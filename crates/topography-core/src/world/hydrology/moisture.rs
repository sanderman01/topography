//! Orographic precipitation on a 128 m grid: the linear upslope model of Smith and Barstad
//! (2004), solved by FFT, plus a uniform background rate, both depleted downwind by the
//! water already rained out over land.

use super::flood::SEA;
use rustfft::{FftPlanner, num_complex::Complex};
use serde_json::{Value, json};
use std::f64::consts::PI;

/// Precipitation grid spacing.
pub(super) const MOISTURE_M: f64 = 128.0;

/// Judgment constants with literature-typical values (Smith and Barstad 2004). Only the ratio
/// of the orographic term to the background shapes discharge and moisture.
pub(super) struct Params {
    pub wind_ms: f64,
    /// Moist buoyancy frequency (s⁻¹).
    pub nm: f64,
    /// Water vapour scale height (m).
    pub hw_m: f64,
    /// Condensation and fallout time constants (s).
    pub tau_c_s: f64,
    pub tau_f_s: f64,
    /// Uplift sensitivity factor (kg m⁻³).
    pub cw: f64,
    /// Uniform background rate (mm/h), added after clipping: the lee is dry, not rainless.
    pub background_mm_h: f64,
    /// Column water budget (mm): the air keeps e⁻¹ of its water after raining this much out
    /// over land, so rain fades downwind across the island.
    pub budget_mm: f64,
}

pub(super) const PARAMS: Params = Params {
    wind_ms: 10.0,
    nm: 0.005,
    hw_m: 2500.0,
    tau_c_s: 300.0,
    tau_f_s: 300.0,
    cw: 0.004,
    background_mm_h: 1.5,
    budget_mm: 3.0,
};

impl Params {
    pub fn json(&self) -> Value {
        json!({
            "model": "Smith and Barstad (2004) linear upslope, FFT on a 128 m grid padded by \
                half its size each side with 0 m ground; ground below 0 m counts as 0 m; negative \
                rates clipped to 0, then the background added",
            "wind_ms": self.wind_ms,
            "nm_per_s": self.nm,
            "hw_m": self.hw_m,
            "tau_c_s": self.tau_c_s,
            "tau_f_s": self.tau_f_s,
            "cw_kg_m3": self.cw,
            "background_mm_h": self.background_mm_h,
            "budget_mm": self.budget_mm,
            "depletion": "rain scaled by the moisture left after what rained out over land upwind, \
                exp(-rained mm / budget_mm), full at the upwind grid edge",
        })
    }
}

/// Precipitation (mm/h) on a grid of `k` x `k` input cells per cell.
pub(super) struct Rain {
    pub w: usize,
    pub h: usize,
    pub k: usize,
    pub mm_h: Vec<f32>,
}

impl Rain {
    /// Precipitation at input cell `i` of a `fine_w`-wide grid, bilinear between cell centres.
    pub fn at(&self, fine_w: usize, i: usize) -> f32 {
        let k = self.k as f32;
        let pos = |c: usize, n: usize| ((c as f32 + 0.5) / k - 0.5).clamp(0.0, (n - 1) as f32);
        let (x, y) = (pos(i % fine_w, self.w), pos(i / fine_w, self.h));
        let (x0, y0) = (x as usize, y as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let p = |x: usize, y: usize| self.mm_h[y * self.w + x];
        let top = p(x0, y0) * (1.0 - fx) + p(x1, y0) * fx;
        let bottom = p(x0, y1) * (1.0 - fx) + p(x1, y1) * fx;
        top * (1.0 - fy) + bottom * fy
    }
}

/// Land precipitation figures: the mean weights discharge, the 95th percentile scales
/// `moisture.f32`.
pub(super) struct Stats {
    pub mean_land_mm_h: f32,
    pub p95_land_mm_h: f32,
    /// Land means by map quadrant: NW, NE, SW, SE.
    pub quadrants_mm_h: [f32; 4],
    pub quadrants_moisture: [f32; 4],
    /// No wind: rain is uniform and moisture is `NEUTRAL_MOISTURE` everywhere.
    pub windless: bool,
}

/// Judgment value: moisture without a wind, neither wet nor dry for vegetation.
pub(super) const NEUTRAL_MOISTURE: f32 = 0.5;

impl Stats {
    /// Land is input cells without `SEA` in `flags`; a rain cell is land when most of its
    /// block is. Falls back to the background rate without land.
    pub fn new(rain: &Rain, fine_w: usize, flags: &[u8], windless: bool) -> Self {
        let fine_h = flags.len() / fine_w;
        let mut land = vec![0u32; rain.mm_h.len()];
        let mut cells = vec![0u32; rain.mm_h.len()];
        let (mut sum, mut quad, mut quad_n) = (0f64, [0f64; 4], [0u64; 4]);
        for (i, &f) in flags.iter().enumerate() {
            let (x, y) = (i % fine_w, i / fine_w);
            let c = (y / rain.k) * rain.w + x / rain.k;
            cells[c] += 1;
            if f & SEA == 0 {
                land[c] += 1;
                let p = rain.at(fine_w, i) as f64;
                sum += p;
                let q = usize::from(x >= fine_w / 2) + 2 * usize::from(y >= fine_h / 2);
                quad[q] += p;
                quad_n[q] += 1;
            }
        }
        let n: u64 = quad_n.iter().sum();
        let fallback = PARAMS.background_mm_h as f32;
        let mut wet: Vec<f32> = (0..land.len())
            .filter(|&c| 2 * land[c] > cells[c])
            .map(|c| rain.mm_h[c])
            .collect();
        wet.sort_by(f32::total_cmp);
        let p95 = wet.get(wet.len() * 95 / 100).copied().unwrap_or(fallback);
        let mean = if n > 0 {
            (sum / n as f64) as f32
        } else {
            fallback
        };
        let mut stats = Self {
            // Positive so weights and moisture stay finite on rainless land.
            mean_land_mm_h: mean.max(1e-6),
            p95_land_mm_h: p95.max(1e-6),
            quadrants_mm_h: std::array::from_fn(|q| (quad[q] / quad_n[q].max(1) as f64) as f32),
            quadrants_moisture: [0.0; 4],
            windless,
        };
        let mut quad = [0f64; 4];
        for (i, &f) in flags.iter().enumerate() {
            if f & SEA == 0 {
                let (x, y) = (i % fine_w, i / fine_w);
                let q = usize::from(x >= fine_w / 2) + 2 * usize::from(y >= fine_h / 2);
                quad[q] += stats.moisture(rain.at(fine_w, i)) as f64;
            }
        }
        stats.quadrants_moisture =
            std::array::from_fn(|q| (quad[q] / quad_n[q].max(1) as f64) as f32);
        stats
    }

    /// `moisture.f32` value: rain over the 95th percentile, so most land spans 0 to 1.
    pub fn moisture(&self, mm_h: f32) -> f32 {
        if self.windless {
            return NEUTRAL_MOISTURE;
        }
        (mm_h / self.p95_land_mm_h).clamp(0.0, 1.0)
    }

    pub fn json(&self) -> Value {
        let [nw, ne, sw, se] = self.quadrants_mm_h;
        let rel = |v: f32| v / self.mean_land_mm_h;
        let m = self.quadrants_moisture;
        json!({
            "mean_land_mm_h": self.mean_land_mm_h,
            "p95_land_mm_h": self.p95_land_mm_h,
            "land_mm_h_by_quadrant": { "nw": nw, "ne": ne, "sw": sw, "se": se },
            "land_relative_by_quadrant": { "nw": rel(nw), "ne": rel(ne), "sw": rel(sw), "se": rel(se) },
            "land_moisture_by_quadrant": { "nw": m[0], "ne": m[1], "sw": m[2], "se": m[3] },
        })
    }
}

/// Block factor from the input spacing to about 128 m (1 when the input is that coarse).
pub(super) fn block(spacing_m: f64) -> usize {
    ((MOISTURE_M / spacing_m).round() as usize).max(1)
}

/// Precipitation over `ground` (`w` x `h`, `spacing_m`), with the wind coming from
/// `wind_from_deg` (clockwise from north); uniform background rain without a wind. Rain
/// depletes over cells where `land_at` holds (the input's non-sea cells).
pub(super) fn precipitation(
    ground: &[f32],
    land_at: impl Fn(usize) -> bool,
    w: usize,
    h: usize,
    spacing_m: f64,
    wind_from_deg: Option<f64>,
    p: &Params,
) -> Rain {
    let k = block(spacing_m);
    let (cw, ch) = (w.div_ceil(k), h.div_ceil(k));
    let background = p.background_mm_h as f32;
    let Some(from) = wind_from_deg else {
        return Rain {
            w: cw,
            h: ch,
            k,
            mm_h: vec![background; cw * ch],
        };
    };

    // Block means of the ground, sea level as the floor.
    let mut sum = vec![0f64; cw * ch];
    let mut count = vec![0u32; cw * ch];
    let mut land = vec![0u32; cw * ch];
    for (i, &g) in ground.iter().enumerate() {
        let c = (i / w / k) * cw + (i % w) / k;
        sum[c] += g.max(0.0) as f64;
        count[c] += 1;
        land[c] += u32::from(land_at(i));
    }

    // Pad by half the size each side (at least), to a power of two against wrap-around.
    let (pw, ph) = ((2 * cw).next_power_of_two(), (2 * ch).next_power_of_two());
    let (ox, oy) = ((pw - cw) / 2, (ph - ch) / 2);
    let mut field = vec![Complex::new(0.0f64, 0.0); pw * ph];
    for y in 0..ch {
        for x in 0..cw {
            let c = y * cw + x;
            field[(y + oy) * pw + x + ox].re = sum[c] / count[c] as f64;
        }
    }
    let mut planner = FftPlanner::<f64>::new();
    fft2(&mut planner, &mut field, pw, ph, false);

    // Wind toward from + 180°: east component and the component along rows (southward).
    let dx = spacing_m * k as f64;
    let rad = from.to_radians();
    let (u, v) = (-p.wind_ms * rad.sin(), p.wind_ms * rad.cos());
    let wavenumber = |f: usize, n: usize| {
        let f = if f <= n / 2 {
            f as f64
        } else {
            f as f64 - n as f64
        };
        2.0 * PI * f / (n as f64 * dx)
    };
    let i = Complex::new(0.0, 1.0);
    for y in 0..ph {
        let ky = wavenumber(y, ph);
        for x in 0..pw {
            let kx = wavenumber(x, pw);
            let sigma = u * kx + v * ky;
            let cell = &mut field[y * pw + x];
            if sigma == 0.0 {
                *cell = Complex::new(0.0, 0.0);
                continue;
            }
            // Vertical wavenumber: propagating (sign of sigma) or evanescent (decaying).
            let m2 = (p.nm * p.nm - sigma * sigma) / (sigma * sigma) * (kx * kx + ky * ky);
            let m = if m2 >= 0.0 {
                Complex::new(m2.sqrt() * sigma.signum(), 0.0)
            } else {
                Complex::new(0.0, (-m2).sqrt())
            };
            let numerator = i * (p.cw * sigma);
            let denominator = (1.0 - i * m * p.hw_m)
                * (1.0 + i * (sigma * p.tau_c_s))
                * (1.0 + i * (sigma * p.tau_f_s));
            *cell *= numerator / denominator;
        }
    }
    fft2(&mut planner, &mut field, pw, ph, true);

    // kg m⁻² s⁻¹ is mm/s; the inverse FFT is unnormalised.
    let scale = 3600.0 / (pw * ph) as f64;
    let mut mm_h = vec![0f32; cw * ch];
    for y in 0..ch {
        for x in 0..cw {
            let orographic = field[(y + oy) * pw + x + ox].re * scale;
            mm_h[y * cw + x] = (orographic as f32).max(0.0) + background;
        }
    }
    let land: Vec<bool> = (0..cw * ch).map(|c| 2 * land[c] > count[c]).collect();
    deplete(
        &mut mm_h,
        &land,
        cw,
        ch,
        dx,
        u / p.wind_ms,
        v / p.wind_ms,
        p,
    );
    Rain {
        w: cw,
        h: ch,
        k,
        mm_h,
    }
}

/// Scale `mm_h` by the moisture left in the air: it enters full at the upwind grid edge and
/// loses what rains out over land on the way (an upwind march along the wind, mixing the two
/// upwind neighbours that bracket its direction). (`wx`, `wy`) is the unit wind toward east
/// and toward rows (south).
#[allow(clippy::too_many_arguments)]
fn deplete(
    mm_h: &mut [f32],
    land: &[bool],
    w: usize,
    h: usize,
    dx: f64,
    wx: f64,
    wy: f64,
    p: &Params,
) {
    // March along the dominant axis `a`; `b` is the other one.
    let x_major = wx.abs() >= wy.abs();
    let (na, nb) = if x_major { (w, h) } else { (h, w) };
    let (wa, wb) = if x_major { (wx, wy) } else { (wy, wx) };
    let idx = |a: usize, b: usize| if x_major { b * w + a } else { a * w + b };
    let (sa, sb) = (wa.signum() as isize, wb.signum() as isize);
    let t = (wb.abs() / wa.abs()) as f32;
    // Water rained out per step (mm) for 1 mm/h, relative to the column budget.
    let per_step = (dx / wa.abs() / p.wind_ms / 3600.0 / p.budget_mm) as f32;
    let mut q = vec![1f32; na * nb];
    for s in 0..na {
        let a = if sa > 0 { s } else { na - 1 - s };
        let up_a = a as isize - sa;
        for b in 0..nb {
            let q_at = |bb: isize| {
                if up_a < 0 || up_a >= na as isize || bb < 0 || bb >= nb as isize {
                    1.0
                } else {
                    q[idx(up_a as usize, bb as usize)]
                }
            };
            let q_in = (1.0 - t) * q_at(b as isize) + t * q_at(b as isize - sb);
            let c = idx(a, b);
            mm_h[c] *= q_in;
            q[c] = if land[c] {
                q_in * (-mm_h[c] * per_step).exp()
            } else {
                q_in
            };
        }
    }
}

/// In-place 2-D FFT: rows, then columns.
fn fft2(planner: &mut FftPlanner<f64>, data: &mut [Complex<f64>], w: usize, h: usize, inv: bool) {
    let mut plan = |n| {
        if inv {
            planner.plan_fft_inverse(n)
        } else {
            planner.plan_fft_forward(n)
        }
    };
    let rows = plan(w);
    rows.process(data);
    let cols = plan(h);
    let mut column = vec![Complex::new(0.0, 0.0); h];
    for x in 0..w {
        for y in 0..h {
            column[y] = data[y * w + x];
        }
        cols.process(&mut column);
        for y in 0..h {
            data[y * w + x] = column[y];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rain_falls_upwind_and_shadows_the_lee() {
        // A 1 km Gaussian hill (3 km scale) on a 256² grid at 128 m.
        let n = 256;
        let ground: Vec<f32> = (0..n * n)
            .map(|i| {
                let (x, y) = ((i % n) as f64 - 128.0, (i / n) as f64 - 128.0);
                (1000.0 * (-(x * x + y * y) * 128.0 * 128.0 / 3000.0f64.powi(2)).exp()) as f32
            })
            .collect();
        let bg = PARAMS.background_mm_h as f32;
        // From the west: wet windward flank 2.5 km west, only background 6 km east.
        let rain = precipitation(
            &ground,
            |i| ground[i] > 0.0,
            n,
            n,
            128.0,
            Some(270.0),
            &PARAMS,
        );
        let at = |x: usize, y: usize| rain.mm_h[y * n + x];
        assert!(at(108, 128) > 1.5 * bg && at(175, 128) < 1.01 * bg);
        // From the south-west, along the diagonal (rows run south).
        let rain = precipitation(
            &ground,
            |i| ground[i] > 0.0,
            n,
            n,
            128.0,
            Some(225.0),
            &PARAMS,
        );
        let at = |x: usize, y: usize| rain.mm_h[y * n + x];
        assert!(at(114, 142) > 1.5 * bg && at(161, 95) < 1.01 * bg);
    }
}
