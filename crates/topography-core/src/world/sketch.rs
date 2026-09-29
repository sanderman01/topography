//! Procedural world sketches: an authored recipe of island, landforms and biomes becomes a
//! diffusion world input (32 m coarse surface, 32 m classes, 1 m game cover) without source data.

use crate::acquisition::{
    AcquisitionError, Result, digest, ensure, io_context, read_small, write_new,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

const BLOCK_M: usize = 32;
/// Sketch sampling; coarse blocks average 4 × 4 samples, 1 m layers interpolate them.
const STEP_M: usize = 8;
const CLASSES: usize = 13;
/// Local plane for invented worlds; metres east and north of the south-west corner.
pub(crate) const SKETCH_CRS: &str = r#"ENGCRS["Topography sketch plane",EDATUM["Topography sketch"],CS[Cartesian,2],AXIS["easting (X)",east,ORDER[1],LENGTHUNIT["metre",1]],AXIS["northing (Y)",north,ORDER[2],LENGTHUNIT["metre",1]]]"#;
/// WorldCover classes that count as vegetation (tree, shrub, grass, crop, wetland, mangrove, moss).
const VEGETATED: [u8; 7] = [10, 20, 30, 40, 90, 95, 100];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Island {
    center: [f32; 2],
    radius_m: f32,
    coast_noise_m: f32,
    lowland_m: f32,
    hills_m: f32,
    shelf_m: f32,
}

#[derive(Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Caldera,
    Range,
    Glacier,
    Fjord,
    Archipelago,
    Cliffs,
    Dunes,
    Mesas,
    Gorge,
    Karst,
    BraidedRiver,
    Cones,
}

impl Kind {
    /// Terrain legend v2 class painted by the feature.
    fn class(self) -> u8 {
        match self {
            Self::Karst => 1,
            Self::Gorge => 2,
            Self::Fjord => 3,
            Self::Caldera => 4,
            Self::Archipelago => 5,
            Self::Range => 6,
            Self::Mesas => 7,
            Self::Dunes => 8,
            Self::BraidedRiver => 9,
            Self::Glacier => 10,
            Self::Cones => 11,
            Self::Cliffs => 12,
        }
    }
}

/// One landform. Areas use `at` + `radii_m`; linear features use `path` + `width_m`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FeatureConfig {
    kind: Kind,
    at: Option<[f32; 2]>,
    radii_m: Option<[f32; 2]>,
    path: Option<Vec<[f32; 2]>>,
    width_m: Option<f32>,
    height_m: Option<f32>,
    /// Bed level at the start and end of a carved path.
    floor_m: Option<[f32; 2]>,
    crater_radius_m: Option<f32>,
    crater_floor_m: Option<f32>,
    spacing_m: Option<f32>,
    wavelength_m: Option<f32>,
    step_m: Option<f32>,
    river_m: Option<f32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recipe {
    schema_version: u32,
    name: String,
    seed: u64,
    extent_m: u32,
    /// Class for land and sea no feature claims; 0 = unknown.
    default_class: u8,
    /// Compass bearing the prevailing wind comes from.
    wind_from_deg: f32,
    windward_snowline_m: f32,
    lee_snowline_m: f32,
    treeline_m: f32,
    island: Island,
    #[serde(default)]
    features: Vec<FeatureConfig>,
}

// ---- noise -------------------------------------------------------------------------------

fn hash(seed: u64, i: i32, j: i32) -> u64 {
    let mut h = seed
        ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (j as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^ (h >> 33)
}

fn unit(h: u64) -> f32 {
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Gradient noise, roughly in [-1, 1], with unit lattice spacing.
fn noise(seed: u64, x: f32, y: f32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (i, j) = (x0 as i32, y0 as i32);
    let grad = |di: i32, dj: i32| {
        let a = unit(hash(seed, i + di, j + dj)) * std::f32::consts::TAU;
        a.cos() * (fx - di as f32) + a.sin() * (fy - dj as f32)
    };
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (u, v) = (fade(fx), fade(fy));
    let a = grad(0, 0) + u * (grad(1, 0) - grad(0, 0));
    let b = grad(0, 1) + u * (grad(1, 1) - grad(0, 1));
    (a + v * (b - a)) * 1.4
}

/// Fractal noise in about [-1, 1] at the given base wavelength.
fn fbm(seed: u64, x: f32, y: f32, wavelength: f32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut norm, mut f) = (0.0, 1.0, 0.0, 1.0 / wavelength);
    for o in 0..octaves {
        sum += amp * noise(seed.wrapping_add(u64::from(o)), x * f, y * f);
        norm += amp;
        amp *= 0.5;
        f *= 2.03;
    }
    sum / norm
}

/// Ridged fractal noise in [0, 1]: sharp crests along noise zero lines.
fn ridged(seed: u64, x: f32, y: f32, wavelength: f32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut norm, mut f, mut weight) = (0.0, 1.0, 0.0, 1.0 / wavelength, 1.0);
    for o in 0..octaves {
        let r = (1.0 - noise(seed.wrapping_add(u64::from(o)), x * f, y * f).abs()).powi(2);
        sum += amp * r * weight;
        weight = r.clamp(0.3, 1.0);
        norm += amp;
        amp *= 0.5;
        f *= 2.1;
    }
    sum / norm
}

/// Distance to the nearest jittered cell point and that cell's random value.
fn worley(seed: u64, x: f32, y: f32, spacing: f32) -> (f32, f32) {
    let (cx, cy) = ((x / spacing).floor() as i32, (y / spacing).floor() as i32);
    let mut best = (f32::MAX, 0.0);
    for dj in -1..=1 {
        for di in -1..=1 {
            let h = hash(seed, cx + di, cy + dj);
            let px = (cx + di) as f32 + 0.15 + 0.7 * unit(h);
            let py = (cy + dj) as f32 + 0.15 + 0.7 * unit(h.rotate_left(24));
            let d = ((x / spacing - px).powi(2) + (y / spacing - py).powi(2)).sqrt() * spacing;
            if d < best.0 {
                best = (d, unit(h.rotate_left(48)));
            }
        }
    }
    best
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

// ---- features ----------------------------------------------------------------------------

enum Shape {
    Area {
        at: [f32; 2],
        radii: [f32; 2],
    },
    Path {
        points: Vec<[f32; 2]>,
        cumulative: Vec<f32>,
        half_width: f32,
    },
}

/// Distance to a polyline, fraction of its length at the nearest point, and signed side.
fn along(points: &[[f32; 2]], cumulative: &[f32], x: f32, y: f32) -> (f32, f32, f32) {
    let mut best = (f32::MAX, 0.0, 0.0);
    for (k, seg) in points.windows(2).enumerate() {
        let ([ax, ay], [bx, by]) = (seg[0], seg[1]);
        let (dx, dy) = (bx - ax, by - ay);
        let len2 = dx * dx + dy * dy;
        let t = (((x - ax) * dx + (y - ay) * dy) / len2).clamp(0.0, 1.0);
        let (px, py) = (ax + t * dx, ay + t * dy);
        let d = ((x - px).powi(2) + (y - py).powi(2)).sqrt();
        if d < best.0 {
            let side = (dx * (y - ay) - dy * (x - ax)).signum();
            best = (d, cumulative[k] + t * len2.sqrt(), side * d);
        }
    }
    let total = cumulative.last().copied().unwrap_or(1.0).max(1.0);
    (best.0, best.1 / total, best.2)
}

struct Feature {
    kind: Kind,
    seed: u64,
    shape: Shape,
    config: FeatureConfig,
}

/// Displace a point by smooth noise so shapes and paths meander instead of looking drawn.
fn warp(seed: u64, x: f32, y: f32, amplitude: f32, wavelength: f32) -> (f32, f32) {
    (
        x + amplitude * fbm(seed ^ 0x51, x, y, wavelength, 3),
        y + amplitude * fbm(seed ^ 0x52, x, y, wavelength, 3),
    )
}

/// Seamless noise around a circle: `k` sets how many lobes fit in one turn.
fn around(seed: u64, angle: f32, k: f32) -> f32 {
    noise(seed, k * angle.cos(), k * angle.sin())
}

impl Feature {
    fn get(&self, value: Option<f32>) -> f32 {
        value.unwrap_or_default()
    }

    /// Meander amplitude and wavelength in metres for this kind.
    fn meander(&self) -> (f32, f32) {
        match self.kind {
            Kind::Gorge => (260.0, 1100.0),
            Kind::Fjord | Kind::Glacier => (450.0, 2600.0),
            Kind::BraidedRiver => (600.0, 4000.0),
            Kind::Range => (1400.0, 6000.0),
            _ => (0.2 * self.size(), 0.9 * self.size()),
        }
    }

    fn size(&self) -> f32 {
        match &self.shape {
            Shape::Area { radii, .. } => radii[0].min(radii[1]),
            Shape::Path { half_width, .. } => 2.0 * half_width,
        }
    }

    /// Normalized distance from the feature core (0) to its edge (1), measured on warped
    /// coordinates, plus the fraction along a path and the signed offset from it.
    fn reach(&self, x: f32, y: f32) -> (f32, f32, f32) {
        let (amp, wl) = self.meander();
        let (x, y) = warp(self.seed, x, y, amp, wl);
        match &self.shape {
            Shape::Area { at, radii } => {
                let q =
                    (((x - at[0]) / radii[0]).powi(2) + ((y - at[1]) / radii[1]).powi(2)).sqrt();
                (q, 0.0, 0.0)
            }
            Shape::Path {
                points,
                cumulative,
                half_width,
            } => {
                let (d, t, signed) = along(points, cumulative, x, y);
                // Troughs narrow toward their head.
                let taper = match self.kind {
                    Kind::Fjord | Kind::Glacier => lerp(0.5, 1.0, t.sqrt()),
                    _ => 1.0,
                };
                (d / (half_width * taper), t, signed)
            }
        }
    }

    /// Soft membership in [0, 1]: full in the core, fading over the outer part.
    fn weight(&self, x: f32, y: f32) -> f32 {
        1.0 - smoothstep(0.3, 1.0, self.reach(x, y).0)
    }

    /// Normalized reach and weight together, for features that also offer their class.
    fn reach_weight(&self, x: f32, y: f32) -> (f32, f32) {
        let q = self.reach(x, y).0;
        (q, 1.0 - smoothstep(0.3, 1.0, q))
    }

    fn apply(&self, s: &mut Sample, x: f32, y: f32) {
        let c = &self.config;
        let seed = self.seed;
        match self.kind {
            Kind::Caldera => self.caldera(s, x, y),
            Kind::Range => {
                let q = self.reach(x, y).0;
                let bell = (1.0 - q).clamp(0.0, 1.0);
                let crest = 0.35 + 0.65 * ridged(seed, x, y, 2800.0, 6).powf(1.2);
                let along_crest = 0.8 + 0.2 * fbm(seed ^ 1, x, y, 7000.0, 2);
                s.h += self.get(c.height_m) * bell.powf(0.8) * crest * along_crest;
                self.offer(s, q);
            }
            Kind::Glacier => {
                if c.floor_m.is_some() {
                    self.carve(s, x, y);
                }
                let w = self.weight(x, y);
                s.ice = s
                    .ice
                    .max(2.0 * (w - 0.45) + 0.15 * noise(seed, x / 250.0, y / 250.0));
                if w > 0.5 {
                    s.hard(10);
                }
            }
            Kind::Fjord | Kind::Gorge | Kind::BraidedRiver => self.carve(s, x, y),
            Kind::Archipelago => {
                let (q, w) = self.reach_weight(x, y);
                let n = fbm(seed, x, y, 2200.0, 5) * 0.5 + 0.5;
                let isle = (n - 0.56) / 0.44;
                let target = if isle > 0.0 {
                    3.0 + self.get(c.height_m) * isle.powf(0.6)
                } else {
                    (isle * 200.0).max(-40.0)
                };
                s.h += w * (s.h.max(target) - s.h);
                self.offer(s, q);
            }
            Kind::Cliffs => {
                let (q, w) = self.reach_weight(x, y);
                if w <= 0.0 {
                    return;
                }
                let top = self.get(c.height_m) * (0.8 + 0.4 * fbm(seed, x, y, 1500.0, 3));
                if s.coast > 0.0 {
                    let plateau = top
                        * smoothstep(0.0, 70.0, s.coast)
                        * (1.0 - smoothstep(1500.0, 4000.0, s.coast));
                    s.h = s.h.max(s.base + w * plateau);
                } else if s.coast > -700.0 {
                    let (f1, v) = worley(seed, x, y, self.get(c.spacing_m).max(100.0));
                    let r0 = 30.0 + 35.0 * v;
                    // Stacks thin out away from the cliff foot.
                    if v > 0.45 + 0.5 * (-s.coast / 700.0) && f1 < r0 {
                        let stack = top * (0.4 + 0.5 * v) * (1.0 - (f1 / r0).powi(6));
                        s.h = s.h.max(lerp(s.h, stack, w));
                    }
                }
                self.offer(s, q);
            }
            Kind::Dunes => {
                let (q, w) = self.reach_weight(x, y);
                let (dx, dy) = s.downwind;
                let lambda = self.get(c.wavelength_m).max(50.0);
                let bend = 0.8 * lambda * fbm(seed, x, y, 4.0 * lambda, 4);
                let phase = ((x * dx + y * dy + bend) / lambda).rem_euclid(1.0);
                // Gentle stoss slope upwind, steep slip face downwind.
                let profile = if phase < 0.8 {
                    phase / 0.8
                } else {
                    (1.0 - phase) / 0.2
                };
                let crest = (0.55 + 0.45 * fbm(seed ^ 1, x, y, 1.5 * lambda, 3)).max(0.0);
                let swell = 0.3 * fbm(seed ^ 2, x, y, 5000.0, 2);
                s.h += w * self.get(c.height_m) * (smoothstep(0.0, 1.0, profile) * crest + swell);
                s.bare = s.bare.max(w);
                self.offer(s, q);
            }
            Kind::Mesas => {
                let (q, w) = self.reach_weight(x, y);
                let (wx, wy) = warp(seed, x, y, 500.0, 1800.0);
                let v = (fbm(seed, wx, wy, 2600.0, 5) * 0.8 + 0.55).clamp(0.0, 1.0);
                let step = self.get(c.step_m).max(5.0);
                let level = self.get(c.height_m) * v / step;
                let terraced = step * (level.floor() + smoothstep(0.7, 1.0, level.fract()));
                let gullies = 40.0 * ridged(seed ^ 2, x, y, 600.0, 4) * v;
                s.h += w * (terraced - gullies);
                s.bare = s.bare.max(w * 0.8);
                self.offer(s, q);
            }
            Kind::Karst => {
                let (q, w) = self.reach_weight(x, y);
                let spacing = self.get(c.spacing_m).max(60.0);
                let (wx, wy) = warp(seed, x, y, 0.4 * spacing, 2.0 * spacing);
                let (f1, v) = worley(seed, wx, wy, spacing);
                // Towers cluster; gaps between clusters are flat, flooded or farmed.
                let cluster = smoothstep(-0.25, 0.2, fbm(seed ^ 3, x, y, 5.0 * spacing, 3));
                let r0 = spacing * (0.18 + 0.2 * v);
                let tower = (1.0 - f1 / r0).clamp(0.0, 1.0).powf(0.3) * cluster;
                let plain = s.h.min(s.base.clamp(-12.0, 40.0));
                s.h = lerp(s.h, plain, w) + w * self.get(c.height_m) * (0.35 + 0.65 * v) * tower;
                s.forest = s.forest.max(w * tower.min(1.0));
                self.offer(s, q);
            }
            Kind::Cones => {
                let (q, w) = self.reach_weight(x, y);
                let spacing = self.get(c.spacing_m).max(300.0);
                let (wx, wy) = warp(seed, x, y, 0.3 * spacing, 1.5 * spacing);
                let (f1, v) = worley(seed, wx, wy, spacing);
                let r0 = spacing * (0.15 + 0.2 * v);
                let mut cone = 0.0;
                if v > 0.55 && f1 < r0 {
                    let hc = self.get(c.height_m) * (0.4 + 0.6 * v);
                    cone = hc * (1.0 - f1 / r0).powf(1.3);
                    // Summit crater.
                    cone -= 0.3 * hc * (1.0 - smoothstep(0.0, 0.25 * r0, f1));
                }
                let lobes = 16.0 * smoothstep(0.05, 0.2, fbm(seed ^ 3, x, y, 1800.0, 5));
                let rough = 6.0 * ridged(seed ^ 4, x, y, 180.0, 3);
                s.h += w * (cone + lobes + rough);
                s.bare = s.bare.max(w * 0.7);
                self.offer(s, q);
            }
        }
    }

    /// Stratovolcano with a noisy rim, radial gullies and an ice-domed crater.
    fn caldera(&self, s: &mut Sample, x: f32, y: f32) {
        let c = &self.config;
        let seed = self.seed;
        let (Shape::Area { at, radii }, Some(crater)) = (&self.shape, c.crater_radius_m) else {
            return;
        };
        let (dx, dy) = (x - at[0], y - at[1]);
        let d = (dx * dx + dy * dy).sqrt();
        let angle = dy.atan2(dx);
        let rc = crater * (1.0 + 0.12 * around(seed, angle, 2.5));
        let outer = radii[0] * (1.0 + 0.18 * around(seed ^ 1, angle, 1.6));
        let height = self.get(c.height_m) * (1.0 + 0.1 * around(seed ^ 2, angle, 3.0));
        let t = ((d - rc) / (outer - rc)).clamp(0.0, 1.0);
        // Radial valleys that curve slowly outward.
        let bend = d / 4000.0 + 0.3 * fbm(seed ^ 6, x, y, 2500.0, 2);
        let valleys = ridged(
            seed ^ 3,
            1.8 * angle.cos() + bend,
            1.8 * angle.sin(),
            1.0,
            2,
        ) * (0.6 + 0.4 * fbm(seed ^ 7, x, y, 3000.0, 2));
        let gully = 1.0 - 0.3 * valleys * (4.0 * t * (1.0 - t)).min(1.0);
        let rough = 1.0 + 0.06 * fbm(seed ^ 4, x, y, 900.0, 4);
        let cone = height * (1.0 - t).powf(1.7) * gully * rough;
        let rim = s.h + height;
        if d >= rc {
            s.h += cone;
        } else {
            let dome = 140.0 * (1.0 - (d / rc).powi(2)) + 30.0 * fbm(seed ^ 5, x, y, 700.0, 4);
            let floor = self.get(c.crater_floor_m) + dome;
            s.h = lerp(floor, rim, smoothstep(0.78 * rc, rc, d).powi(2));
        }
        // Crater and rim are caldera; the upper flanks are volcanic cones and lava.
        if d < 1.4 * rc {
            s.hard(4);
        } else {
            s.offer(11, d / outer);
        }
    }

    fn offer(&self, s: &mut Sample, q: f32) {
        s.offer(self.kind.class(), q);
    }

    /// Cut a descending bed along the path: U troughs, slot gorges or a flat braided plain.
    fn carve(&self, s: &mut Sample, x: f32, y: f32) {
        let c = &self.config;
        let (q, t, signed) = self.reach(x, y);
        let [a, b] = c.floor_m.unwrap_or_default();
        let floor = lerp(a, b, t);
        if q < 1.0 {
            let wall = match self.kind {
                Kind::Fjord | Kind::Glacier => smoothstep(0.3, 1.0, q),
                Kind::Gorge => q.powi(8),
                _ => smoothstep(0.45, 1.0, q),
            };
            let bed = floor + 3.0 * fbm(self.seed ^ 5, x, y, 500.0, 3);
            s.h = s.h.min(bed + (s.h - bed).max(0.0) * wall);
        }
        let river = self.get(c.river_m);
        let half = self.size() / 2.0;
        match self.kind {
            Kind::Gorge => s.river = s.river.max(river / 2.0 - q * half),
            Kind::BraidedRiver => {
                let belt = 1.0 - q * half / (river / 2.0);
                if belt > 0.0 {
                    // Threads that split and rejoin across the gravel belt.
                    let w1 = fbm(self.seed ^ 6, x, y, 1100.0, 4);
                    let w2 = fbm(self.seed ^ 7, x, y, 700.0, 4);
                    let tau = std::f32::consts::TAU;
                    let braid = (tau * (signed / 210.0 + 1.4 * w1))
                        .cos()
                        .max((tau * (signed / 330.0 + 1.1 * w2)).cos());
                    let broken = smoothstep(-0.35, 0.0, fbm(self.seed ^ 8, x, y, 1500.0, 3));
                    let edge = smoothstep(0.0, 0.25, belt);
                    s.river = s
                        .river
                        .max(60.0 * (braid - 0.88) * broken * edge - 1.0 + edge);
                    s.bare = s.bare.max(smoothstep(0.0, 0.2, belt));
                }
            }
            _ => {}
        }
        let class_band = if self.kind == Kind::Fjord { 1.6 } else { 1.2 };
        if q < class_band {
            s.hard(self.kind.class());
        }
    }
}

// ---- sampling ----------------------------------------------------------------------------

struct Sample {
    h: f32,
    /// Island height before features, used as the reference for plateaus and plains.
    base: f32,
    /// Metres inland from the coastline (negative offshore).
    coast: f32,
    class: u8,
    /// Class set by a carve, glacier or crater; later ones win.
    fixed: bool,
    /// Nearest area feature by normalized reach, for everything else.
    near: (f32, u8),
    /// Positive inside rivers, metres of margin.
    river: f32,
    /// Positive inside glacier ice.
    ice: f32,
    /// Bias toward bare ground (dunes, gravel, lava) and forest (karst), 0..1.
    bare: f32,
    forest: f32,
    downwind: (f32, f32),
}

impl Sample {
    fn hard(&mut self, class: u8) {
        self.class = class;
        self.fixed = true;
    }

    fn offer(&mut self, class: u8, q: f32) {
        if q < self.near.0 {
            self.near = (q, class);
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Cell {
    h: f32,
    river: f32,
    snow: f32,
    class: u8,
    /// WorldCover class ignoring water and snow, which the 1 m pass decides.
    cover: u8,
}

struct Sketch {
    recipe: Recipe,
    features: Vec<Feature>,
    downwind: (f32, f32),
}

impl Sketch {
    fn sample(&self, x: f32, y: f32) -> Cell {
        let r = &self.recipe;
        let isl = &r.island;
        let seed = r.seed;
        let (wx, wy) = warp(seed ^ 0x77, x, y, 2200.0, 9000.0);
        let dist = ((wx - isl.center[0]).powi(2) + (wy - isl.center[1]).powi(2)).sqrt();
        let coast = isl.radius_m + isl.coast_noise_m * fbm(seed, x, y, 6000.0, 6) - dist;
        // Rolling, ridged foothills everywhere inland; features build on top.
        let base = if coast >= 0.0 {
            let relief = 0.35 * (0.5 + 0.5 * fbm(seed ^ 7, x, y, 3000.0, 5))
                + 0.65 * ridged(seed ^ 11, x, y, 2400.0, 5);
            isl.lowland_m * smoothstep(0.0, 1500.0, coast)
                + 0.02 * coast.min(9000.0)
                + isl.hills_m * relief * smoothstep(200.0, 2500.0, coast)
        } else {
            -2.0 + isl.shelf_m * smoothstep(0.0, 2500.0, -coast)
        };
        let mut s = Sample {
            h: base,
            base,
            coast,
            class: r.default_class,
            fixed: false,
            // Beyond 2.5 radii the default class stays.
            near: (2.5, r.default_class),
            river: -1.0,
            ice: -1.0,
            bare: 0.0,
            forest: 0.0,
            downwind: self.downwind,
        };
        for f in &self.features {
            f.apply(&mut s, x, y);
        }
        if !s.fixed {
            s.class = s.near.1;
        }
        // Windward side is wet; the lee of the central massif is dry.
        let lee = ((x - isl.center[0]) * self.downwind.0 + (y - isl.center[1]) * self.downwind.1)
            / isl.radius_m;
        let moisture = (0.55 - 0.55 * lee + 0.15 * fbm(seed ^ 8, x, y, 4000.0, 3)).clamp(0.0, 1.0)
            * (1.0 - s.bare);
        let snowline = lerp(
            r.windward_snowline_m,
            r.lee_snowline_m,
            smoothstep(-1.0, 1.0, lee),
        ) + 120.0 * fbm(seed ^ 9, x, y, 1200.0, 3);
        let snow = s.ice.max((s.h - snowline) / 150.0);
        let n = 0.5 + 0.5 * fbm(seed ^ 10, x, y, 350.0, 3);
        let cover = land_cover(&s, moisture, n, r.treeline_m);
        Cell {
            h: s.h,
            river: s.river,
            snow,
            class: s.class,
            cover,
        }
    }
}

/// Plausible WorldCover class from biome, elevation and moisture; a game judgment layer.
fn land_cover(s: &Sample, moisture: f32, n: f32, treeline: f32) -> u8 {
    let above = s.h - treeline;
    match s.class {
        8 => 60,
        7 if n > 0.62 => 20,
        7 => 60,
        11 if s.bare > 0.3 => {
            if n > 0.6 {
                100
            } else {
                60
            }
        }
        1 if s.forest > 0.3 => 10,
        1 if n > 0.55 => 40,
        9 if s.bare > 0.5 => 60,
        9 if n > 0.72 => 90,
        _ if above > 450.0 => {
            if n > 0.45 {
                60
            } else {
                100
            }
        }
        _ if above > 0.0 => {
            if moisture > 0.45 {
                30
            } else if n > 0.5 {
                60
            } else {
                20
            }
        }
        _ if moisture > 0.58 => 10,
        _ if moisture > 0.4 => {
            if n > 0.4 {
                20
            } else {
                10
            }
        }
        _ if moisture > 0.18 => {
            if n > 0.35 {
                30
            } else {
                20
            }
        }
        _ => 60,
    }
}

fn parse(text: &str) -> Result<Sketch> {
    let mut recipe: Recipe = toml::from_str(text)
        .map_err(|e| AcquisitionError::Invalid(format!("sketch recipe: {e}")))?;
    ensure(
        recipe.schema_version == 1,
        "sketch schema_version must be 1",
    )?;
    ensure(
        recipe.extent_m.is_power_of_two() && (1024..=32768).contains(&recipe.extent_m),
        "extent_m must be a power of two from 1024 to 32768",
    )?;
    ensure(
        usize::from(recipe.default_class) < CLASSES,
        "default_class must be 0..=12",
    )?;
    let isl = &recipe.island;
    ensure(
        isl.radius_m > 0.0
            && [isl.coast_noise_m, isl.lowland_m, isl.hills_m, isl.shelf_m]
                .iter()
                .all(|v| v.is_finite()),
        "island needs a positive radius and finite heights",
    )?;
    let mut features = Vec::new();
    for (i, config) in recipe.features.iter().enumerate() {
        let context = |what: &str| format!("feature {} ({}): {what}", i + 1, config.kind.class());
        let shape = match (&config.at, &config.radii_m, &config.path, config.width_m) {
            (Some(at), Some(radii), None, None) if radii.iter().all(|r| *r > 0.0) => Shape::Area {
                at: *at,
                radii: *radii,
            },
            (None, None, Some(points), Some(width)) if points.len() >= 2 && width > 0.0 => {
                let mut cumulative = vec![0.0];
                for seg in points.windows(2) {
                    let len =
                        ((seg[1][0] - seg[0][0]).powi(2) + (seg[1][1] - seg[0][1]).powi(2)).sqrt();
                    ensure(len > 0.0, context("path points must differ"))?;
                    cumulative.push(cumulative.last().copied().unwrap_or(0.0) + len);
                }
                Shape::Path {
                    points: points.clone(),
                    cumulative,
                    half_width: width / 2.0,
                }
            }
            _ => {
                return Err(AcquisitionError::Invalid(context(
                    "use either at + positive radii_m, or a path of 2+ points + positive width_m",
                )));
            }
        };
        let is_path = matches!(shape, Shape::Path { .. });
        let need = |ok: bool, what: &str| ensure(ok, context(what));
        match config.kind {
            Kind::Caldera => need(
                !is_path
                    && config.height_m.is_some()
                    && config.crater_floor_m.is_some()
                    && config
                        .crater_radius_m
                        .zip(config.radii_m)
                        .is_some_and(|(c, r)| c > 0.0 && c < r[0]),
                "caldera needs an area, height_m, crater_floor_m and crater_radius_m inside radii_m",
            )?,
            Kind::Fjord | Kind::Gorge | Kind::BraidedRiver => need(
                is_path
                    && config.floor_m.is_some()
                    && (config.kind == Kind::Fjord || config.river_m.is_some_and(|r| r > 0.0)),
                "carved features need a path and floor_m; gorges and braided rivers need river_m",
            )?,
            Kind::Glacier => need(
                config.floor_m.is_none() || is_path,
                "glacier floor_m needs a path",
            )?,
            _ => need(config.height_m.is_some(), "needs height_m")?,
        }
        let extra = match config.kind {
            Kind::Dunes => config.wavelength_m.is_some(),
            Kind::Mesas => config.step_m.is_some(),
            Kind::Karst | Kind::Cones | Kind::Cliffs => config.spacing_m.is_some(),
            _ => true,
        };
        need(
            extra,
            "dunes need wavelength_m, mesas step_m, karst/cones/cliffs spacing_m",
        )?;
        features.push((config.kind, shape, i));
    }
    let bearing = (recipe.wind_from_deg + 180.0).to_radians();
    // (east, south) components, since sketch rows run north to south.
    let downwind = (bearing.sin(), -bearing.cos());
    let seed = recipe.seed;
    let configs = std::mem::take(&mut recipe.features);
    let features = features
        .into_iter()
        .zip(configs)
        .map(|((kind, shape, i), config)| Feature {
            kind,
            seed: hash(seed, i as i32, 17),
            shape,
            config,
        })
        .collect();
    Ok(Sketch {
        recipe,
        features,
        downwind,
    })
}

/// Fill `out` row by row on all cores; `f(row, row_slice)`.
fn par_rows<T: Send>(out: &mut [T], width: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows = out.len() / width;
    let per = rows.div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        for (k, chunk) in out.chunks_mut(per * width).enumerate() {
            let f = &f;
            scope.spawn(move || {
                for (r, row) in chunk.chunks_mut(width).enumerate() {
                    f(k * per + r, row);
                }
            });
        }
    });
}

/// Bilinear sample of an 8 m field at a 1 m cell centre.
fn bilinear(cells: &[Cell], n: usize, x: usize, y: usize, get: impl Fn(&Cell) -> f32) -> f32 {
    let s = STEP_M as f32;
    let fx = ((x as f32 + 0.5) / s - 0.5).clamp(0.0, (n - 1) as f32);
    let fy = ((y as f32 + 0.5) / s - 0.5).clamp(0.0, (n - 1) as f32);
    let (x0, y0) = (fx as usize, fy as usize);
    let (x1, y1) = ((x0 + 1).min(n - 1), (y0 + 1).min(n - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let v = |xx: usize, yy: usize| get(&cells[yy * n + xx]);
    lerp(
        lerp(v(x0, y0), v(x1, y0), tx),
        lerp(v(x0, y1), v(x1, y1), tx),
        ty,
    )
}

/// Build a sketch world input: `world.json`, `coarse.f32`, `classes.u8` and 1 m `water.u8`,
/// `snow-ice.u8`, `vegetation.u8` and `land-cover.u8`, in a new directory.
pub fn build_sketch(config_path: &Path, output: &Path) -> Result<PathBuf> {
    ensure(!output.exists(), "world input output already exists")?;
    let text = read_small(config_path)?;
    let text = String::from_utf8_lossy(&text).into_owned();
    let sketch = parse(&text)?;
    let extent = sketch.recipe.extent_m as usize;
    let n = extent / STEP_M;

    let mut cells = vec![Cell::default(); n * n];
    par_rows(&mut cells, n, |row, out| {
        for (col, cell) in out.iter_mut().enumerate() {
            let (x, y) = ((col * STEP_M) as f32 + 4.0, (row * STEP_M) as f32 + 4.0);
            *cell = sketch.sample(x, y);
        }
    });

    let blocks = extent / BLOCK_M;
    let per = BLOCK_M / STEP_M;
    let mut coarse = vec![0f32; blocks * blocks];
    let mut classes = vec![0u8; blocks * blocks];
    for by in 0..blocks {
        for bx in 0..blocks {
            let (mut sum, mut votes) = (0.0, [0u32; CLASSES]);
            for yy in by * per..(by + 1) * per {
                for xx in bx * per..(bx + 1) * per {
                    let c = &cells[yy * n + xx];
                    sum += c.h;
                    votes[usize::from(c.class)] += 1;
                }
            }
            coarse[by * blocks + bx] = sum / (per * per) as f32;
            classes[by * blocks + bx] = (0..CLASSES).max_by_key(|&k| votes[k]).unwrap_or(0) as u8;
        }
    }

    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    io_context("create world input parent", fs::create_dir_all(parent))?;
    let stage = io_context("stage world input", tempfile::tempdir_in(parent))?;
    let mut digests = serde_json::Map::new();
    let mut publish = |name: &str, bytes: &[u8]| -> Result<()> {
        write_new(&stage.path().join(name), bytes)?;
        digests.insert(name.to_owned(), json!(digest(bytes)));
        Ok(())
    };
    let coarse_bytes: Vec<u8> = coarse.iter().flat_map(|v| v.to_le_bytes()).collect();
    publish("coarse.f32", &coarse_bytes)?;
    publish("classes.u8", &classes)?;

    // 1 m layers, one at a time to bound memory: water and snow from interpolated scores.
    let wet = |x: usize, y: usize| {
        bilinear(&cells, n, x, y, |c| c.h) < 0.0 || bilinear(&cells, n, x, y, |c| c.river) > 0.0
    };
    let snowy = |x: usize, y: usize| bilinear(&cells, n, x, y, |c| c.snow) > 0.0;
    let cover = |x: usize, y: usize| cells[(y / STEP_M) * n + x / STEP_M].cover;
    let mut layer = vec![0u8; extent * extent];
    let mut counts = serde_json::Map::new();
    for name in ["water.u8", "snow-ice.u8", "vegetation.u8", "land-cover.u8"] {
        par_rows(&mut layer, extent, |y, row| {
            for (x, v) in row.iter_mut().enumerate() {
                let (w, s) = (wet(x, y), snowy(x, y));
                *v = match name {
                    "water.u8" => {
                        if w {
                            255
                        } else {
                            0
                        }
                    }
                    "snow-ice.u8" => {
                        if s && !w {
                            2
                        } else {
                            1
                        }
                    }
                    "vegetation.u8" => {
                        if !w && !s && VEGETATED.contains(&cover(x, y)) {
                            2
                        } else {
                            1
                        }
                    }
                    _ => {
                        if w {
                            80
                        } else if s {
                            70
                        } else {
                            cover(x, y)
                        }
                    }
                };
            }
        });
        let present = layer
            .iter()
            .filter(|&&v| if name == "water.u8" { v == 255 } else { v == 2 })
            .count();
        if name != "land-cover.u8" {
            counts.insert(name.trim_end_matches(".u8").into(), json!(present));
        }
        publish(name, &layer)?;
    }

    let mut class_blocks = serde_json::Map::new();
    for k in 0..CLASSES {
        let count = classes.iter().filter(|&&c| usize::from(c) == k).count();
        if count > 0 {
            class_blocks.insert(k.to_string(), json!(count));
        }
    }
    let land = coarse.iter().filter(|&&h| h >= 0.0).count();
    let (lo, hi) = coarse
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), &h| (a.min(h), b.max(h)));
    let manifest = json!({
        "schema_version": 2,
        "kind": "diffusion_world_input",
        "name": sketch.recipe.name,
        "size_blocks": [blocks, blocks],
        "block_m": BLOCK_M,
        "crs": SKETCH_CRS,
        "origin": [0.0, extent as f64],
        "extent_m": extent,
        "wgs84_bbox": null,
        "config_sha256": digest(text.as_bytes()),
        "config_text": text,
        "base": {
            "method": "procedural sketch (topography world sketch); 8 m samples averaged to 32 m blocks",
            "attribution": "Procedural sketch; no source elevation data",
        },
        "cover": {
            "policy": "sketch_cover_v1",
            "attribution": [],
            "note": "water: sketch height below 0 m or inside authored rivers; snow/ice: glaciers or above the windward/lee snowline; land cover: WorldCover codes chosen from biome, elevation and wind-side moisture at 8 m; vegetation: WorldCover tree, shrub, grass, crop, wetland, mangrove and moss. Game judgment layers.",
            "present_cells": counts,
        },
        "summary": {
            "land_block_fraction": land as f64 / (blocks * blocks) as f64,
            "coarse_range_m": [lo, hi],
            "class_blocks": class_blocks,
        },
        "layers": digests,
    });
    write_new(
        &stage.path().join("world.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    io_context("publish world input", fs::rename(stage.path(), output))?;
    Ok(output.join("world.json"))
}
