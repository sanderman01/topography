//! Automatic source-artifact checks on native elevation grids (checks `artifacts_v1`).
//!
//! Heights are row-major, north-up, NaN where not valid. Mask bits: FACET marks
//! interpolated triangles (exactly planar cells), STRIPE periodic axis-aligned striping,
//! SEAM long straight axis-aligned lines. Stripes are repairable with [`destripe`];
//! facets and seams are excluded from training.

use serde_json::{Value, json};

pub(crate) const FACET: u8 = 1;
pub(crate) const STRIPE: u8 = 2;
pub(crate) const SEAM: u8 = 4;
pub(crate) const CHECKS: &str = "artifacts_v1";

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Axis {
    X,
    Y,
}

/// Periodic striping along one grid axis.
#[derive(Clone, Copy)]
pub(crate) struct Stripe {
    pub axis: Axis,
    pub period_cells: f64,
    pub strength: f64,
}

pub(crate) struct Detection {
    pub mask: Vec<u8>,
    pub stripes: Vec<Stripe>,
}

struct Grid<'a> {
    h: &'a [f32],
    w: usize,
    rows: usize,
}

impl Grid<'_> {
    fn at(&self, x: usize, y: usize) -> f64 {
        f64::from(self.h[y * self.w + x])
    }
    /// Second difference across the axis at (x, y); NaN when a sample is missing.
    fn curvature(&self, axis: Axis, x: usize, y: usize) -> f64 {
        match axis {
            Axis::X => self.at(x + 1, y) - 2.0 * self.at(x, y) + self.at(x - 1, y),
            Axis::Y => self.at(x, y + 1) - 2.0 * self.at(x, y) + self.at(x, y - 1),
        }
    }
}

/// Run all checks at about 1 m: finer grids are averaged in k x k blocks first, and the
/// mask and stripe periods are mapped back to native cells.
pub(crate) fn detect_metre(height: &[f32], w: usize, rows: usize, spacing_m: f64) -> Detection {
    let k = (1.0 / spacing_m).round().max(1.0) as usize;
    if k == 1 {
        return detect(height, w, rows);
    }
    let (bw, br) = (w / k, rows / k);
    let mut coarse = vec![f32::NAN; bw * br];
    for by in 0..br {
        for bx in 0..bw {
            let mut sum = 0f64;
            let mut ok = true;
            for y in by * k..(by + 1) * k {
                for x in bx * k..(bx + 1) * k {
                    let v = height[y * w + x];
                    ok &= v.is_finite();
                    sum += f64::from(v);
                }
            }
            if ok {
                coarse[by * bw + bx] = (sum / (k * k) as f64) as f32;
            }
        }
    }
    let found = detect(&coarse, bw, br);
    let mut mask = vec![0u8; height.len()];
    for y in 0..rows {
        for x in 0..w {
            let (bx, by) = (
                (x / k).min(bw.saturating_sub(1)),
                (y / k).min(br.saturating_sub(1)),
            );
            if height[y * w + x].is_finite() && bw > 0 && br > 0 {
                mask[y * w + x] = found.mask[by * bw + bx];
            }
        }
    }
    let stripes = found
        .stripes
        .into_iter()
        .map(|s| Stripe {
            period_cells: s.period_cells * k as f64,
            ..s
        })
        .collect();
    Detection { mask, stripes }
}

/// Run all checks on the given grid. Stripes are detected on the input; seams after
/// stripe repair.
fn detect(height: &[f32], w: usize, rows: usize) -> Detection {
    let mut mask = vec![0u8; height.len()];
    if w < 3 || rows < 3 {
        return Detection {
            mask,
            stripes: Vec::new(),
        };
    }
    facets(&Grid { h: height, w, rows }, &mut mask);
    let stripes = find_stripes(&Grid { h: height, w, rows });
    let mut repaired = height.to_vec();
    for stripe in &stripes {
        mark_stripe(&Grid { h: height, w, rows }, stripe, &mut mask);
    }
    destripe(&mut repaired, w, rows, &stripes);
    seams(
        &Grid {
            h: &repaired,
            w,
            rows,
        },
        &mut mask,
    );
    for (m, h) in mask.iter_mut().zip(height) {
        if !h.is_finite() {
            *m = 0;
        }
    }
    Detection { mask, stripes }
}

/// Interpolated triangles are exactly planar; natural 1 m terrain almost never is.
fn facets(g: &Grid, mask: &mut [u8]) {
    let mut exact = vec![0f32; g.h.len()];
    for y in 1..g.rows - 1 {
        for x in 1..g.w - 1 {
            let (dxx, dyy) = (g.curvature(Axis::X, x, y), g.curvature(Axis::Y, x, y));
            let slope =
                (g.at(x + 1, y) - g.at(x - 1, y)).hypot(g.at(x, y + 1) - g.at(x, y - 1)) / 2.0;
            // Float32 storage limits how planar a plane can look at high elevation.
            let tol = 1e-4 + 2.0 * g.at(x, y).abs() * f64::from(f32::EPSILON);
            if dxx.abs() < tol && dyy.abs() < tol && slope > 0.05 {
                exact[y * g.w + x] = 1.0;
            }
        }
    }
    let density = box_mean(&box_mean(&exact, g.w, g.rows, 8), g.w, g.rows, 8);
    for (m, d) in mask.iter_mut().zip(density) {
        if d > 0.01 {
            *m |= FACET;
        }
    }
}

/// Separable box mean with radius `r`, shrinking the window at the edges.
fn box_mean(a: &[f32], w: usize, rows: usize, r: usize) -> Vec<f32> {
    let pass = |a: &[f32], n: usize, lines: usize, idx: &dyn Fn(usize, usize) -> usize| {
        let mut out = vec![0f32; a.len()];
        let mut prefix = vec![0f64; n + 1];
        for line in 0..lines {
            for i in 0..n {
                prefix[i + 1] = prefix[i] + f64::from(a[idx(line, i)]);
            }
            for i in 0..n {
                let (lo, hi) = (i.saturating_sub(r), (i + r + 1).min(n));
                out[idx(line, i)] = ((prefix[hi] - prefix[lo]) / (hi - lo) as f64) as f32;
            }
        }
        out
    };
    let rows_done = pass(a, w, rows, &|y, x| y * w + x);
    pass(&rows_done, rows, w, &|x, y| y * w + x)
}

/// Mean periodogram of cross-axis curvature at frequency `f` (cycles per cell) over
/// the lines and span given; NaN runs contribute nothing.
fn power(
    g: &Grid,
    axis: Axis,
    f: f64,
    lines: impl Iterator<Item = usize>,
    span: (usize, usize),
) -> f64 {
    let (c, s) = (
        (std::f64::consts::TAU * f).cos(),
        (std::f64::consts::TAU * f).sin(),
    );
    let (mut total, mut count) = (0.0, 0usize);
    for line in lines {
        let (mut re, mut im) = (0.0, 0.0);
        let (mut pr, mut pi) = (1.0f64, 0.0f64);
        let mut n = 0usize;
        for i in span.0..span.1 {
            let d = match axis {
                Axis::X => g.curvature(axis, i, line),
                Axis::Y => g.curvature(axis, line, i),
            };
            if d.is_finite() {
                re += d * pr;
                im -= d * pi;
                n += 1;
            }
            (pr, pi) = (pr * c - pi * s, pr * s + pi * c);
        }
        if n > span.1.saturating_sub(span.0) / 2 {
            total += (re * re + im * im) / n as f64;
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else {
        total / count as f64
    }
}

/// The strongest periodic peak per axis between 2 and 8 cells, if it stands far above
/// the band's median power.
fn find_stripes(g: &Grid) -> Vec<Stripe> {
    let mut stripes = Vec::new();
    for axis in [Axis::X, Axis::Y] {
        let (n, lines) = match axis {
            Axis::X => (g.w, g.rows),
            Axis::Y => (g.rows, g.w),
        };
        if n < 64 || lines < 3 {
            continue;
        }
        // Every 4th line is plenty for a tile-wide spectrum.
        let freqs: Vec<f64> = (0..240)
            .map(|k| 0.125 + k as f64 * (0.495 - 0.125) / 239.0)
            .collect();
        let spectrum: Vec<f64> = freqs
            .iter()
            .map(|&f| power(g, axis, f, (1..lines - 1).step_by(4), (1, n - 1)))
            .collect();
        let mut sorted = spectrum.clone();
        sorted.sort_by(f64::total_cmp);
        let median = sorted[sorted.len() / 2].max(1e-12);
        let (k, peak) = spectrum
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(k, p)| (k, *p))
            .unwrap_or((0, 0.0));
        if peak / median > 5.0 {
            stripes.push(Stripe {
                axis,
                period_cells: 1.0 / freqs[k],
                strength: peak / median,
            });
        }
    }
    stripes
}

/// Mark 64-cell windows where the stripe frequency dominates its neighbourhood.
fn mark_stripe(g: &Grid, stripe: &Stripe, mask: &mut [u8]) {
    const B: usize = 64;
    let f = 1.0 / stripe.period_cells;
    for by in (0..g.rows).step_by(B) {
        for bx in (0..g.w).step_by(B) {
            let (y1, x1) = ((by + B).min(g.rows - 1), (bx + B).min(g.w - 1));
            let (y0, x0) = (by.max(1), bx.max(1));
            if y1 <= y0 + 8 || x1 <= x0 + 8 {
                continue;
            }
            let p = |f: f64| match stripe.axis {
                Axis::X => power(g, Axis::X, f, y0..y1, (x0, x1)),
                Axis::Y => power(g, Axis::Y, f, x0..x1, (y0, y1)),
            };
            let off = (p(f * 0.8) + p((f * 1.2).min(0.5))) / 2.0;
            if p(f) > 4.0 * off.max(1e-12) {
                for y in by..(by + B).min(g.rows) {
                    for m in &mut mask[y * g.w + bx..y * g.w + (bx + B).min(g.w)] {
                        *m |= STRIPE;
                    }
                }
            }
        }
    }
}

/// Remove each stripe frequency with a narrow FIR notch along its axis. Cells whose
/// filter window leaves the grid or touches invalid samples are kept as they are.
pub(crate) fn destripe(height: &mut [f32], w: usize, rows: usize, stripes: &[Stripe]) {
    for stripe in stripes {
        let f = 1.0 / stripe.period_cells;
        let half = (4.0 * stripe.period_cells).round() as usize;
        let t = |j: usize| j as f64 - half as f64;
        let hann: Vec<f64> = (0..=2 * half)
            .map(|j| 0.5 + 0.5 * (std::f64::consts::PI * t(j) / (half as f64 + 1.0)).cos())
            .collect();
        let wave = |j: usize| (std::f64::consts::TAU * f * t(j)).cos();
        // Zero DC gain so heights keep their level; symmetry keeps ramps too.
        let dc = (0..=2 * half).map(|j| hann[j] * wave(j)).sum::<f64>() / hann.iter().sum::<f64>();
        let taps: Vec<f64> = (0..=2 * half).map(|j| hann[j] * (wave(j) - dc)).collect();
        // Unit gain at the stripe frequency.
        let gain: f64 = taps
            .iter()
            .enumerate()
            .map(|(j, k)| k * (std::f64::consts::TAU * f * (j as f64 - half as f64)).cos())
            .sum();
        let (n, lines) = match stripe.axis {
            Axis::X => (w, rows),
            Axis::Y => (rows, w),
        };
        let idx = |line: usize, i: usize| match stripe.axis {
            Axis::X => line * w + i,
            Axis::Y => i * w + line,
        };
        let mut buf = vec![0f64; n];
        for line in 0..lines {
            for (i, b) in buf.iter_mut().enumerate() {
                *b = f64::from(height[idx(line, i)]);
            }
            for i in half..n.saturating_sub(half) {
                let window = &buf[i - half..=i + half];
                if window.iter().all(|v| v.is_finite()) {
                    let band: f64 =
                        window.iter().zip(&taps).map(|(v, k)| v * k).sum::<f64>() / gain;
                    height[idx(line, i)] = (buf[i] - band) as f32;
                }
            }
        }
    }
}

/// Long straight lines: cross-axis curvature that keeps one sign along a grid line for
/// at least 6 consecutive 32-cell segments (192 cells).
fn seams(g: &Grid, mask: &mut [u8]) {
    const S: usize = 32;
    const RUN: usize = 6;
    for axis in [Axis::X, Axis::Y] {
        let (n, along) = match axis {
            Axis::X => (g.w, g.rows),
            Axis::Y => (g.rows, g.w),
        };
        for line in 1..n - 1 {
            let mut run = 0usize;
            for seg in 0..along / S {
                let values: Vec<f64> = (seg * S..(seg + 1) * S)
                    .map(|i| match axis {
                        Axis::X => g.curvature(axis, line, i),
                        Axis::Y => g.curvature(axis, i, line),
                    })
                    .collect();
                let hit = values.iter().all(|v| v.is_finite()) && {
                    let mean = values.iter().sum::<f64>() / S as f64;
                    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / S as f64;
                    mean.abs() / (var.sqrt() / (S as f64).sqrt()).max(1e-6) > 6.0
                };
                run = if hit { run + 1 } else { 0 };
                if run >= RUN {
                    let first = if run == RUN { seg + 1 - RUN } else { seg };
                    for s in first..=seg {
                        for i in s * S..(s + 1) * S {
                            for l in line - 1..=line + 1 {
                                let cell = match axis {
                                    Axis::X => i * g.w + l,
                                    Axis::Y => l * g.w + i,
                                };
                                mask[cell] |= SEAM;
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Fractions of valid cells per check, plus stripe details in metres.
pub(crate) fn summary(height: &[f32], detection: &Detection, spacing_m: [f64; 2]) -> Value {
    let valid = height.iter().filter(|h| h.is_finite()).count().max(1) as f64;
    let fraction = |bit: u8| {
        let n = detection.mask.iter().filter(|m| *m & bit != 0).count() as f64 / valid;
        (n * 1e4).round() / 1e4
    };
    let stripes: Vec<Value> = detection
        .stripes
        .iter()
        .map(|s| {
            let (axis, spacing) = match s.axis {
                Axis::X => ("x", spacing_m[0]),
                Axis::Y => ("y", spacing_m[1]),
            };
            json!({"axis": axis, "period_m": (s.period_cells * spacing * 100.0).round() / 100.0,
                "strength": s.strength.round()})
        })
        .collect();
    json!({
        "checks": CHECKS,
        "facet_fraction": fraction(FACET),
        "stripe_fraction": fraction(STRIPE),
        "seam_fraction": fraction(SEAM),
        "excluded_fraction": fraction(FACET | SEAM),
        "stripes": stripes,
    })
}
