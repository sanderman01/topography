//! Authored world layouts (world-layout-spec-v1): a layout spec becomes the L0 input of the
//! terrain cascade, a soft 128 m mid surface with local relief, terrain classes, lakes and
//! cover hints, plus a fit-check report. Large landforms come from the cascade models; the
//! layout only places them.

use crate::acquisition::{Result, digest, ensure, io_context, read_small, write_new};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

/// L0 cell: the L1 model's 128 m grid, so the cascade reads the layout without resampling.
const CELL_M: f32 = 128.0;
const CLASSES: u8 = 13;
/// Neighbouring regions blend over about this distance (Gaussian sigma).
const BLEND_M: f32 = 1000.0;
/// Fallback blend for cells far from any region with its own ground (the volcano interior).
const FAR_BLEND_M: f32 = 4000.0;
/// Sea floor falls from the shelf toward the edge depth with this e-folding distance.
const SEA_FALL_M: f32 = 2000.0;
/// Ridges fade in over this distance inland so arm tips end in cliffs, not sea mounts.
const RIDGE_COAST_M: f32 = 300.0;
/// Passes are saddles: ground rises from the pass height at no more than this grade, out to
/// PASS_OUTER_M, so the gap is a corridor through the ridge rather than a dimple in it.
const PASS_GRADE: f32 = 0.15;
const PASS_INNER_M: f32 = 200.0;
const PASS_OUTER_M: f32 = 1500.0;
const BASIN_RIM_M: f32 = 1000.0;
const LAKE_DEPTH_M: f32 = 30.0;
/// Relief fades out over this distance offshore (except in archipelagos).
const SEA_RELIEF_M: f32 = 1500.0;

type Point = [f32; 2];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Sea {
    shelf_m: f32,
    edge_m: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Island {
    target_km2: f32,
    coast: Vec<Point>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tiers {
    starter_ceiling_m: f32,
    mid_ceiling_m: f32,
    clearance_m: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Region {
    id: String,
    #[allow(dead_code)]
    name: String,
    class_id: u8,
    #[serde(default)]
    refs: Vec<String>,
    outline: Option<Vec<Point>>,
    center: Option<Point>,
    radius_m: Option<f32>,
    path: Option<Vec<Point>>,
    width_m: Option<f32>,
    target_km2: f32,
    /// Ground at the coast and fully inland; None stands on the neighbours' ground.
    ground_m: Option<[f32; 2]>,
    #[serde(default)]
    ramp_m: f32,
    relief_m: f32,
    forest: f32,
    #[allow(dead_code)]
    coast: Option<String>,
    #[allow(dead_code)]
    cover: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ridge {
    id: String,
    path: Vec<Point>,
    crest_m: [f32; 2],
    width_m: f32,
    #[allow(dead_code)]
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Volcano {
    id: String,
    class_id: u8,
    at: Point,
    radius_m: f32,
    summit_m: f32,
    concavity: f32,
    crater_diameter_m: f32,
    crater_floor_m: f32,
    #[allow(dead_code)]
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Glacier {
    id: String,
    class_id: u8,
    at: Option<Point>,
    radius_m: Option<f32>,
    path: Option<Vec<Point>>,
    width_m: Option<f32>,
    #[allow(dead_code)]
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cone {
    id: String,
    class_id: u8,
    at: Point,
    radius_m: f32,
    summit_m: f32,
    #[allow(dead_code)]
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cones {
    id: String,
    class_id: u8,
    points: Vec<Point>,
    radius_m: f32,
    height_m: f32,
    #[allow(dead_code)]
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Basin {
    id: String,
    class_id: u8,
    at: Point,
    radius_m: f32,
    floor_m: f32,
    rim_m: f32,
    lip_m: f32,
    lip_at: Point,
    #[allow(dead_code)]
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Dunes {
    id: String,
    class_id: u8,
    at: Point,
    radii_m: [f32; 2],
    height_m: f32,
    relief_m: f32,
    #[allow(dead_code)]
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Carve {
    id: String,
    class_id: Option<u8>,
    path: Vec<Point>,
    width_m: f32,
    rim_width_m: f32,
    /// Bed at the first and last point.
    bed_m: [f32; 2],
    /// A river runs along it from the first point and must reach the sea.
    #[serde(default)]
    river: bool,
    #[allow(dead_code)]
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Feature {
    Volcano(Volcano),
    Glacier(Glacier),
    Cone(Cone),
    Peak(Cone),
    Cones(Cones),
    Basin(Basin),
    Dunes(Dunes),
    Fjord(Carve),
    Gorge(Carve),
    BraidedRiver(Carve),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
/// A dammed hollow, round (`at` + `radius_m`) or along a valley (`path` + `width_m`). The
/// cascade keeps the generated hollow there as a lake; its shoreline comes from the terrain.
struct Lake {
    id: String,
    at: Option<Point>,
    radius_m: Option<f32>,
    path: Option<Vec<Point>>,
    width_m: Option<f32>,
    surface_m: f32,
    #[allow(dead_code)]
    note: Option<String>,
}

#[derive(Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Tier {
    Starter,
    Mid,
    Specialist,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pass {
    id: String,
    tier: Tier,
    at: Point,
    height_m: f32,
    #[allow(dead_code)]
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Destination {
    id: String,
    #[allow(dead_code)]
    tier: Tier,
    at: Point,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Site {
    region: String,
    at: Point,
    #[allow(dead_code)]
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Layout {
    schema_version: u32,
    name: String,
    extent_m: u32,
    /// Compass bearing the prevailing wind comes from.
    wind_from_deg: f32,
    windward_snowline_m: f32,
    lee_snowline_m: f32,
    treeline_m: f32,
    height_budget_m: [f32; 2],
    /// Class of land no region claims.
    default_class: u8,
    sea: Sea,
    island: Island,
    tiers: Tiers,
    regions: Vec<Region>,
    #[serde(default)]
    ridges: Vec<Ridge>,
    #[serde(default)]
    features: Vec<Feature>,
    #[serde(default)]
    lakes: Vec<Lake>,
    #[serde(default)]
    passes: Vec<Pass>,
    #[serde(default)]
    destinations: Vec<Destination>,
    #[serde(default)]
    sites: Vec<Site>,
}

impl Lake {
    /// Distance outside the lake's edge (negative inside).
    fn edge(&self, x: f32, y: f32) -> f32 {
        match (self.at, self.radius_m, &self.path, self.width_m) {
            (Some(at), Some(r), _, _) => dist(at, x, y) - r,
            (_, _, Some(p), Some(w)) => along(p, x, y).0 - w / 2.0,
            _ => f32::MAX,
        }
    }
}

impl Feature {
    fn id(&self) -> &str {
        match self {
            Self::Volcano(v) => &v.id,
            Self::Glacier(g) => &g.id,
            Self::Cone(c) | Self::Peak(c) => &c.id,
            Self::Cones(c) => &c.id,
            Self::Basin(b) => &b.id,
            Self::Dunes(d) => &d.id,
            Self::Fjord(c) | Self::Gorge(c) | Self::BraidedRiver(c) => &c.id,
        }
    }
}

// ---- geometry ----------------------------------------------------------------------------

fn inside(polygon: &[Point], x: f32, y: f32) -> bool {
    let mut c = false;
    for (i, &[x1, y1]) in polygon.iter().enumerate() {
        let [x2, y2] = polygon[(i + 1) % polygon.len()];
        if (y1 > y) != (y2 > y) && x < x1 + (y - y1) * (x2 - x1) / (y2 - y1) {
            c = !c;
        }
    }
    c
}

/// Distance to a polyline and the fraction of its length at the nearest point.
fn along(points: &[Point], x: f32, y: f32) -> (f32, f32) {
    let (mut best, mut at, mut walked) = (f32::MAX, 0.0, 0.0);
    for seg in points.windows(2) {
        let ([ax, ay], [bx, by]) = (seg[0], seg[1]);
        let (dx, dy) = (bx - ax, by - ay);
        let len = (dx * dx + dy * dy).sqrt().max(1e-3);
        let t = (((x - ax) * dx + (y - ay) * dy) / (len * len)).clamp(0.0, 1.0);
        let d = ((x - ax - t * dx).powi(2) + (y - ay - t * dy).powi(2)).sqrt();
        if d < best {
            (best, at) = (d, walked + t * len);
        }
        walked += len;
    }
    (best, at / walked.max(1e-3))
}

fn length(points: &[Point]) -> f32 {
    points
        .windows(2)
        .map(|s| ((s[1][0] - s[0][0]).powi(2) + (s[1][1] - s[0][1]).powi(2)).sqrt())
        .sum()
}

fn dist(a: Point, x: f32, y: f32) -> f32 {
    ((x - a[0]).powi(2) + (y - a[1]).powi(2)).sqrt()
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Separable Gaussian blur with edge clamping.
fn blur(field: &[f32], n: usize, sigma_m: f32) -> Vec<f32> {
    let s = sigma_m / CELL_M;
    let r = (3.0 * s).ceil() as isize;
    let kernel: Vec<f32> = (-r..=r)
        .map(|i| (-(i * i) as f32 / (2.0 * s * s)).exp())
        .collect();
    let total: f32 = kernel.iter().sum();
    let pass = |src: &[f32], horizontal: bool| -> Vec<f32> {
        let mut out = vec![0.0; n * n];
        for y in 0..n {
            for x in 0..n {
                let mut acc = 0.0;
                for (k, w) in kernel.iter().enumerate() {
                    let o = k as isize - r;
                    let (xx, yy) = if horizontal {
                        ((x as isize + o).clamp(0, n as isize - 1) as usize, y)
                    } else {
                        (x, (y as isize + o).clamp(0, n as isize - 1) as usize)
                    };
                    acc += w * src[yy * n + xx];
                }
                out[y * n + x] = acc / total;
            }
        }
        out
    };
    pass(&pass(field, true), false)
}

impl Region {
    fn contains(&self, x: f32, y: f32) -> bool {
        if let Some(outline) = &self.outline {
            inside(outline, x, y)
        } else if let (Some(c), Some(r)) = (self.center, self.radius_m) {
            dist(c, x, y) < r
        } else if let (Some(p), Some(w)) = (&self.path, self.width_m) {
            along(p, x, y).0 < w / 2.0
        } else {
            false
        }
    }
}

fn validate(layout: &Layout) -> Result<()> {
    ensure(
        layout.schema_version == 1,
        "unsupported layout schema_version",
    )?;
    ensure(
        layout.extent_m.is_power_of_two() && layout.extent_m >= 4096,
        "extent_m must be a power of two of at least 4,096 m",
    )?;
    ensure(
        layout.island.coast.len() >= 3,
        "island coast needs 3 or more points",
    )?;
    let class_ok = |c: u8| c < CLASSES;
    ensure(class_ok(layout.default_class), "default_class out of range")?;
    for r in &layout.regions {
        let shapes = [
            r.outline.is_some(),
            r.center.is_some() && r.radius_m.is_some(),
            r.path.is_some() && r.width_m.is_some(),
        ];
        ensure(
            shapes.iter().filter(|&&s| s).count() == 1,
            format!(
                "region {} needs exactly one of outline, center + radius_m, path + width_m",
                r.id
            ),
        )?;
        ensure(
            class_ok(r.class_id) && r.class_id > 0,
            format!("region {} class_id out of range", r.id),
        )?;
        ensure(
            (0.0..=1.0).contains(&r.forest),
            format!("region {} forest must be 0-1", r.id),
        )?;
        ensure(
            r.ground_m.is_none() || r.ramp_m > 0.0,
            format!("region {} with ground_m needs ramp_m", r.id),
        )?;
    }
    for f in &layout.features {
        if let Feature::Glacier(g) = f {
            ensure(
                (g.at.is_some() && g.radius_m.is_some())
                    != (g.path.is_some() && g.width_m.is_some()),
                format!("glacier {} needs at + radius_m or path + width_m", g.id),
            )?;
        }
    }
    // Ids are unique across the spec so downstream reports can name things.
    let mut ids: Vec<&str> = layout.regions.iter().map(|r| r.id.as_str()).collect();
    ids.extend(layout.ridges.iter().map(|r| r.id.as_str()));
    ids.extend(layout.features.iter().map(Feature::id));
    ids.extend(layout.lakes.iter().map(|l| l.id.as_str()));
    ids.extend(layout.passes.iter().map(|p| p.id.as_str()));
    ids.extend(layout.destinations.iter().map(|d| d.id.as_str()));
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    ensure(sorted.len() == ids.len(), "layout ids must be unique")?;
    let extent = layout.extent_m as f32;
    let within = |p: Point| (0.0..extent).contains(&p[0]) && (0.0..extent).contains(&p[1]);
    ensure(
        layout
            .destinations
            .iter()
            .map(|d| d.at)
            .chain(layout.passes.iter().map(|p| p.at))
            .all(within),
        "passes and destinations must lie inside the extent",
    )?;
    for l in &layout.lakes {
        ensure(
            (l.at.is_some() && l.radius_m.is_some()) != (l.path.is_some() && l.width_m.is_some()),
            format!("lake {} needs at + radius_m or path + width_m", l.id),
        )?;
    }
    for s in &layout.sites {
        ensure(
            layout.regions.iter().any(|r| r.id == s.region),
            format!("site names unknown region {}", s.region),
        )?;
    }
    Ok(())
}

// ---- rendering ---------------------------------------------------------------------------

struct Rendered {
    n: usize,
    /// Signed distance to the coast in metres, positive inland.
    coast: Vec<f32>,
    mid: Vec<f32>,
    relief: Vec<f32>,
    class: Vec<u8>,
    /// Index of the region that claims each cell, or None.
    owner: Vec<Option<usize>>,
    lake: Vec<u8>,
    lake_level: Vec<f32>,
    snowline: Vec<f32>,
    forest: Vec<u8>,
    /// How strongly the cascade holds `mid` while sampling (0-1): the landmarks.
    hold: Vec<f32>,
}

/// Hold weights: high where a landmark's authored shape must survive L1 sampling, fading
/// out so the model joins it to the generated terrain around it.
fn hold_weights(layout: &Layout, n: usize) -> Vec<f32> {
    let centre = |i: usize| (i as f32 + 0.5) * CELL_M;
    let fade = |w: f32, inner: f32, outer: f32, d: f32| w * (1.0 - smoothstep(inner, outer, d));
    (0..n * n)
        .map(|k| {
            let (x, y) = (centre(k % n), centre(k / n));
            let mut w: f32 = 0.0;
            for f in &layout.features {
                w = w.max(match f {
                    Feature::Volcano(v) => {
                        fade(0.85, 0.3 * v.radius_m, v.radius_m, dist(v.at, x, y))
                    }
                    Feature::Cone(c) | Feature::Peak(c) => {
                        fade(0.75, 0.3 * c.radius_m, c.radius_m, dist(c.at, x, y))
                    }
                    Feature::Basin(b) => {
                        let rim = b.radius_m + BASIN_RIM_M;
                        fade(0.85, b.radius_m, rim, dist(b.at, x, y))
                    }
                    Feature::Gorge(c) | Feature::Fjord(c) | Feature::BraidedRiver(c) => {
                        let strength = match f {
                            Feature::Gorge(_) if c.river => 0.9,
                            Feature::Gorge(_) => 0.6,
                            _ => 0.5,
                        };
                        let rw = (c.rim_width_m / 2.0).max(CELL_M);
                        fade(strength, rw, rw + 2.0 * CELL_M, along(&c.path, x, y).0)
                    }
                    _ => 0.0,
                });
            }
            for l in &layout.lakes {
                w = w.max(fade(1.0, 0.0, 400.0, l.edge(x, y)));
            }
            for p in &layout.passes {
                w = w.max(fade(0.7, 400.0, PASS_OUTER_M, dist(p.at, x, y)));
            }
            w
        })
        .collect()
}

fn render(layout: &Layout) -> Rendered {
    let n = (layout.extent_m as f32 / CELL_M) as usize;
    let centre = |i: usize| (i as f32 + 0.5) * CELL_M;
    let cells = || (0..n * n).map(move |k| (k, centre(k % n), centre(k / n)));

    // Coast distance from the polygon edges.
    let coast_pts = &layout.island.coast;
    let mut ring = coast_pts.clone();
    ring.push(coast_pts[0]);
    let coast: Vec<f32> = cells()
        .map(|(_, x, y)| {
            let d = along(&ring, x, y).0;
            if inside(coast_pts, x, y) { d } else { -d }
        })
        .collect();

    // Region membership, class and owner in order.
    let members: Vec<Vec<f32>> = layout
        .regions
        .iter()
        .map(|r| {
            cells()
                .map(|(_, x, y)| if r.contains(x, y) { 1.0 } else { 0.0 })
                .collect()
        })
        .collect();
    let mut owner = vec![None; n * n];
    for (ri, m) in members.iter().enumerate() {
        for k in 0..n * n {
            if m[k] > 0.0 {
                owner[k] = Some(ri);
            }
        }
    }

    // Ground, relief and forest blend between regions; far cells use a wide blend.
    let near: Vec<Vec<f32>> = members.iter().map(|m| blur(m, n, BLEND_M)).collect();
    let far: Vec<Vec<f32>> = members.iter().map(|m| blur(m, n, FAR_BLEND_M)).collect();
    let blend = |value: &dyn Fn(usize, usize) -> Option<f32>| -> Vec<f32> {
        (0..n * n)
            .map(|k| {
                let mix = |w: &Vec<Vec<f32>>| {
                    let (mut sum, mut total) = (0.0, 0.0);
                    for (ri, wr) in w.iter().enumerate() {
                        if let Some(v) = value(ri, k) {
                            sum += wr[k] * v;
                            total += wr[k];
                        }
                    }
                    (if total > 0.0 { sum / total } else { 0.0 }, total)
                };
                let (a, total) = mix(&near);
                let (b, _) = mix(&far);
                lerp(b, a, smoothstep(0.05, 0.3, total))
            })
            .collect()
    };
    let ground = blend(&|ri, k| {
        let r = &layout.regions[ri];
        r.ground_m
            .map(|[g0, g1]| lerp(g0, g1, smoothstep(0.0, r.ramp_m, coast[k])))
    });
    let mut relief = blend(&|ri, _| Some(layout.regions[ri].relief_m));
    let forest = blend(&|ri, _| Some(layout.regions[ri].forest));

    let sea = &layout.sea;
    let mut mid: Vec<f32> = (0..n * n)
        .map(|k| {
            if coast[k] >= 0.0 {
                ground[k]
            } else {
                sea.edge_m + (sea.shelf_m - sea.edge_m) * (coast[k] / SEA_FALL_M).exp()
            }
        })
        .collect();

    let mut class: Vec<u8> = (0..n * n)
        .map(|k| match owner[k] {
            Some(ri) => layout.regions[ri].class_id,
            None if coast[k] >= 0.0 => layout.default_class,
            None => 0,
        })
        .collect();

    // The X: rounded ridges raise the land toward their crest.
    for ridge in &layout.ridges {
        let hw = ridge.width_m / 2.0;
        for (k, x, y) in cells() {
            let (d, t) = along(&ridge.path, x, y);
            if d < hw && coast[k] > 0.0 {
                let crest = lerp(ridge.crest_m[0], ridge.crest_m[1], t);
                let rise = (crest - mid[k]).max(0.0) * (1.0 - (d / hw).powi(2));
                mid[k] += rise * smoothstep(0.0, RIDGE_COAST_M, coast[k]);
            }
        }
    }

    // Passes cut the ridges before peaks and cones rise, so a nearby peak keeps its height.
    for p in &layout.passes {
        for (k, x, y) in cells() {
            let d = dist(p.at, x, y);
            if d < PASS_OUTER_M {
                mid[k] = mid[k].min(p.height_m + PASS_GRADE * (d - PASS_INNER_M).max(0.0));
            }
        }
    }

    let mut lake = vec![0u8; n * n];
    let mut lake_level = vec![f32::NAN; n * n];
    // Raised features first, then carving, then lakes.
    let order = |f: &Feature| match f {
        Feature::Fjord(_) | Feature::Gorge(_) | Feature::BraidedRiver(_) => 1,
        _ => 0,
    };
    let mut features: Vec<&Feature> = layout.features.iter().collect();
    features.sort_by_key(|f| order(f));
    for feature in features {
        match feature {
            Feature::Volcano(v) => {
                for (k, x, y) in cells() {
                    let r = dist(v.at, x, y);
                    if r < v.radius_m {
                        let p = (1.0 - r / v.radius_m).powf(v.concavity);
                        mid[k] += (v.summit_m - mid[k]) * p;
                        relief[k] = lerp(relief[k], relief[k].min(150.0), p);
                    }
                    let cr = v.crater_diameter_m / 2.0;
                    if r < cr {
                        let bowl = lerp(v.crater_floor_m, v.summit_m, (r / cr).powi(2));
                        mid[k] = mid[k].min(bowl);
                        class[k] = v.class_id;
                    }
                }
            }
            Feature::Cone(c) | Feature::Peak(c) => {
                let p = if matches!(feature, Feature::Peak(_)) {
                    1.0
                } else {
                    1.3
                };
                for (k, x, y) in cells() {
                    let r = dist(c.at, x, y);
                    if r < c.radius_m {
                        let lifted =
                            mid[k] + (c.summit_m - mid[k]) * (1.0 - r / c.radius_m).powf(p);
                        mid[k] = mid[k].max(lifted);
                        if r < c.radius_m / 2.0 {
                            class[k] = c.class_id;
                        }
                    }
                }
            }
            Feature::Cones(c) => {
                for (k, x, y) in cells() {
                    for &at in &c.points {
                        let r = dist(at, x, y);
                        if r < c.radius_m {
                            mid[k] += c.height_m * (1.0 - (r / c.radius_m).powi(2)).powi(2);
                            class[k] = c.class_id;
                        }
                    }
                }
            }
            Feature::Dunes(d) => {
                for (k, x, y) in cells() {
                    let e = ((x - d.at[0]) / d.radii_m[0]).powi(2)
                        + ((y - d.at[1]) / d.radii_m[1]).powi(2);
                    if e < 1.0 {
                        mid[k] += d.height_m * (1.0 - e);
                        relief[k] = relief[k].max(d.relief_m);
                        class[k] = d.class_id;
                    }
                }
            }
            Feature::Basin(b) => {
                for (k, x, y) in cells() {
                    let r = dist(b.at, x, y);
                    if r < b.radius_m {
                        mid[k] = b.floor_m;
                    } else if r < b.radius_m + BASIN_RIM_M {
                        let fade =
                            smoothstep(b.radius_m + 0.3 * BASIN_RIM_M, b.radius_m + BASIN_RIM_M, r);
                        mid[k] = mid[k].max(lerp(b.rim_m, mid[k], fade));
                    }
                    if dist(b.lip_at, x, y) < 400.0 {
                        mid[k] = mid[k].min(b.lip_m);
                    }
                    if r < b.radius_m + BASIN_RIM_M / 2.0 {
                        class[k] = b.class_id;
                    }
                }
            }
            Feature::Glacier(g) => {
                for (k, x, y) in cells() {
                    let hit = match (g.at, g.radius_m, &g.path, g.width_m) {
                        (Some(at), Some(r), _, _) => dist(at, x, y) < r,
                        (_, _, Some(p), Some(w)) => along(p, x, y).0 < w / 2.0,
                        _ => false,
                    };
                    // The crater keeps its own class inside the ice cap.
                    if hit && class[k] != 4 {
                        class[k] = g.class_id;
                    }
                }
            }
            Feature::Fjord(c) | Feature::Gorge(c) | Feature::BraidedRiver(c) => {
                let class_id = c.class_id.unwrap_or(match feature {
                    Feature::Fjord(_) => 3,
                    Feature::Gorge(_) => 2,
                    _ => 9,
                });
                // Narrower than a cell still lowers the cell it runs through.
                let hw = (c.width_m / 2.0).max(CELL_M / 2.0);
                let rw = (c.rim_width_m / 2.0).max(hw + CELL_M);
                for (k, x, y) in cells() {
                    let (d, t) = along(&c.path, x, y);
                    if d < rw {
                        let bed = lerp(c.bed_m[0], c.bed_m[1], t);
                        let target = bed + (mid[k] - bed).max(0.0) * smoothstep(hw, rw, d);
                        mid[k] = mid[k].min(target);
                        class[k] = class_id;
                    }
                }
            }
        }
    }
    for l in &layout.lakes {
        for (k, x, y) in cells() {
            let e = l.edge(x, y);
            if e < 0.0 {
                // A shallow bowl below the surface, so the models see a basin to keep.
                mid[k] = l.surface_m - LAKE_DEPTH_M * smoothstep(0.0, 300.0, -e);
                lake[k] = 255;
                lake_level[k] = l.surface_m;
            }
        }
    }
    // Open sea no region claims takes the class of the nearest region (by wide blend).
    for k in 0..n * n {
        if class[k] == 0 && mid[k] < 0.0 {
            let best = (0..far.len()).max_by(|&a, &b| far[a][k].total_cmp(&far[b][k]));
            if let Some(ri) = best.filter(|&ri| far[ri][k] > 1e-4) {
                class[k] = layout.regions[ri].class_id;
            } else {
                class[k] = layout.default_class;
            }
        }
    }

    // Open sea keeps little relief, or the models raise islands; archipelagos keep theirs.
    for k in 0..n * n {
        if coast[k] < 0.0 && class[k] != 5 {
            relief[k] *= smoothstep(-SEA_RELIEF_M, 0.0, coast[k]);
        }
    }

    // Snowline from wind exposure across the island: windward low, lee high.
    let bearing = layout.wind_from_deg.to_radians();
    let (tx, ty) = (-bearing.sin(), bearing.cos()); // direction the wind blows toward (y south)
    let (cx, cy) = coast_pts
        .iter()
        .fold((0.0, 0.0), |(a, b), p| (a + p[0], b + p[1]));
    let (cx, cy) = (cx / coast_pts.len() as f32, cy / coast_pts.len() as f32);
    let half = layout.extent_m as f32 / 2.0;
    let snowline = cells()
        .map(|(_, x, y)| {
            let e = (((x - cx) * tx + (y - cy) * ty) / half).clamp(-1.0, 1.0);
            lerp(
                layout.windward_snowline_m,
                layout.lee_snowline_m,
                (e + 1.0) / 2.0,
            )
        })
        .collect();
    let forest = forest
        .iter()
        .map(|f| (f.clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect();

    Rendered {
        n,
        coast,
        mid,
        relief,
        class,
        owner,
        lake,
        lake_level,
        snowline,
        forest,
        hold: hold_weights(layout, n),
    }
}

// ---- fit check ---------------------------------------------------------------------------

fn sample(r: &Rendered, field: &[f32], p: Point) -> f32 {
    let i = ((p[0] / CELL_M) as usize).min(r.n - 1);
    let j = ((p[1] / CELL_M) as usize).min(r.n - 1);
    field[j * r.n + i]
}

fn slope_deg(r: &Rendered, p: Point) -> f32 {
    let i = ((p[0] / CELL_M) as usize).clamp(1, r.n - 2);
    let j = ((p[1] / CELL_M) as usize).clamp(1, r.n - 2);
    let gx = (r.mid[j * r.n + i + 1] - r.mid[j * r.n + i - 1]) / (2.0 * CELL_M);
    let gy = (r.mid[(j + 1) * r.n + i] - r.mid[(j - 1) * r.n + i]) / (2.0 * CELL_M);
    (gx * gx + gy * gy).sqrt().atan().to_degrees()
}

/// Human-readable checks; returns the report and the number of warnings.
fn fit_check(layout: &Layout, r: &Rendered) -> (String, usize) {
    let mut out = String::new();
    let mut warnings = 0;
    let mut warn = |out: &mut String, ok: bool, line: String| {
        if !ok {
            warnings += 1;
        }
        let _ = writeln!(out, "{} {line}", if ok { "  ok  " } else { "  WARN" });
    };
    let km2 = CELL_M * CELL_M / 1e6;
    let land: Vec<bool> = r.mid.iter().map(|&h| h >= 0.0).collect();
    let _ = writeln!(
        out,
        "Layout {} at {} m cells ({}²)\n",
        layout.name, CELL_M, r.n
    );

    let _ = writeln!(out, "Areas");
    let island = land.iter().filter(|&&l| l).count() as f32 * km2;
    let near = |a: f32, b: f32| (a - b).abs() <= 0.2 * b + 1.0;
    warn(
        &mut out,
        near(island, layout.island.target_km2),
        format!(
            "island land {island:.0} km² (target {:.0})",
            layout.island.target_km2
        ),
    );
    for (ri, region) in layout.regions.iter().enumerate() {
        let a = (0..r.n * r.n)
            .filter(|&k| land[k] && r.owner[k] == Some(ri))
            .count() as f32
            * km2;
        warn(
            &mut out,
            near(a, region.target_km2),
            format!(
                "{:<10} {a:>5.0} km² land (target {:.0}); refs {}",
                region.id,
                region.target_km2,
                region.refs.join(", ")
            ),
        );
    }
    let default = (0..r.n * r.n)
        .filter(|&k| land[k] && r.owner[k].is_none())
        .count() as f32
        * km2;
    let _ = writeln!(
        out,
        "  info unclaimed land (class {}) {default:.0} km²",
        layout.default_class
    );
    // Land and near-shore water: archipelago classes live in the sea.
    let mut classes = BTreeMap::new();
    for k in 0..r.n * r.n {
        if r.coast[k] > -2000.0 {
            *classes.entry(r.class[k]).or_insert(0usize) += 1;
        }
    }
    let missing: Vec<u8> = (1..CLASSES).filter(|c| !classes.contains_key(c)).collect();
    warn(
        &mut out,
        missing.is_empty(),
        format!(
            "classes on land and within 2 km offshore: {} of 12; missing {missing:?}",
            classes.keys().filter(|&&c| c > 0).count()
        ),
    );

    let _ = writeln!(out, "\nHeights");
    let (mut top, mut top_at) = (f32::MIN, 0);
    for (k, &h) in r.mid.iter().enumerate() {
        if h > top {
            (top, top_at) = (h, k);
        }
    }
    let lo = r.mid.iter().copied().fold(f32::MAX, f32::min);
    let [b0, b1] = layout.height_budget_m;
    warn(
        &mut out,
        lo >= b0 - 1.0 && top <= b1 + 1.0,
        format!("mid surface {lo:.0} to {top:.0} m (budget {b0:.0} to {b1:.0})"),
    );
    let volcano = layout.features.iter().find_map(|f| match f {
        Feature::Volcano(v) => Some(v),
        _ => None,
    });
    if let Some(v) = volcano {
        let rival = (0..r.n * r.n)
            .filter(|&k| {
                dist(
                    v.at,
                    (k % r.n) as f32 * CELL_M + 64.0,
                    (k / r.n) as f32 * CELL_M + 64.0,
                ) > 3000.0
            })
            .map(|k| r.mid[k] + r.relief[k] / 2.0)
            .fold(f32::MIN, f32::max);
        warn(
            &mut out,
            v.summit_m - rival >= 500.0,
            format!(
                "{} summit {:.0} m; highest envelope top beyond 3 km {rival:.0} m (margin {:.0})",
                v.id,
                v.summit_m,
                v.summit_m - rival
            ),
        );
        let _ = top_at;
    }

    let _ = writeln!(out, "\nHero features");
    for f in &layout.features {
        match f {
            Feature::Fjord(c) | Feature::Gorge(c) | Feature::BraidedRiver(c) => {
                let end = *c.path.last().unwrap_or(&[0.0, 0.0]);
                let mut line = format!(
                    "{:<13} {:>5.1} km, bed {:.0} to {:.0} m",
                    c.id,
                    length(&c.path) / 1000.0,
                    c.bed_m[0],
                    c.bed_m[1]
                );
                let mut ok = true;
                if c.river {
                    let reaches = sample(r, &r.mid, end) <= 0.0;
                    ok = c.bed_m[0] > c.bed_m[1] && reaches;
                    line += if reaches {
                        "; reaches the sea"
                    } else {
                        "; does NOT reach the sea"
                    };
                }
                if matches!(f, Feature::Fjord(_)) {
                    ok = c.bed_m[0] < 0.0 && sample(r, &r.mid, c.path[0]) < 0.0;
                    line += "; mouth below sea level";
                }
                warn(&mut out, ok, line);
            }
            Feature::Cone(c) | Feature::Peak(c) => {
                let h = sample(r, &r.mid, c.at);
                warn(
                    &mut out,
                    (h - c.summit_m).abs() < 150.0,
                    format!(
                        "{:<13} {h:.0} m at its centre cell (spec {:.0})",
                        c.id, c.summit_m
                    ),
                );
            }
            Feature::Basin(b) => {
                let floor = sample(r, &r.mid, b.at);
                let lip = sample(r, &r.mid, b.lip_at);
                let starter = layout.tiers.starter_ceiling_m;
                warn(
                    &mut out,
                    floor > starter && lip > starter,
                    format!(
                        "{:<13} floor {floor:.0} m, lip {lip:.0} m (starter ceiling {starter:.0})",
                        b.id
                    ),
                );
            }
            _ => {}
        }
    }

    let _ = writeln!(out, "\nLakes");
    for l in &layout.lakes {
        // Lowest ground on a ring just outside the lake: the spill point.
        let rim = (0..r.n * r.n)
            .filter(|&k| {
                let d = l.edge(
                    (k % r.n) as f32 * CELL_M + 64.0,
                    (k / r.n) as f32 * CELL_M + 64.0,
                );
                d > 64.0 && d < 192.0
            })
            .map(|k| r.mid[k])
            .fold(f32::MAX, f32::min);
        warn(
            &mut out,
            rim >= l.surface_m - 50.0,
            format!(
                "{:<13} surface {:.0} m, lowest rim {rim:.0} m",
                l.id, l.surface_m
            ),
        );
    }

    let _ = writeln!(out, "\nPasses (mid surface at the saddle)");
    let t = &layout.tiers;
    for p in &layout.passes {
        let h = sample(r, &r.mid, p.at);
        let (ok, rule) = match p.tier {
            Tier::Starter => (
                h <= t.starter_ceiling_m - t.clearance_m,
                format!("starter: below {:.0}", t.starter_ceiling_m - t.clearance_m),
            ),
            Tier::Mid => (
                h > t.starter_ceiling_m && h <= t.mid_ceiling_m - t.clearance_m,
                format!(
                    "mid: {:.0} to {:.0}",
                    t.starter_ceiling_m,
                    t.mid_ceiling_m - t.clearance_m
                ),
            ),
            Tier::Specialist => (
                h > t.mid_ceiling_m,
                format!("specialist: above {:.0}", t.mid_ceiling_m),
            ),
        };
        warn(
            &mut out,
            ok,
            format!("{:<17} {h:>5.0} m (spec {:.0}; {rule})", p.id, p.height_m),
        );
    }

    let _ = writeln!(out, "\nSettlement sites (L0 slope; detail comes later)");
    for s in &layout.sites {
        let _ = writeln!(
            out,
            "  info {:<10} {:>5.0} m, {:.1}°{}",
            s.region,
            sample(r, &r.mid, s.at),
            slope_deg(r, s.at),
            if sample(r, &r.coast, s.at) < 0.0 {
                " (offshore!)"
            } else {
                ""
            }
        );
    }
    let _ = writeln!(
        out,
        "\n{warnings} warning(s). Tier reachability: python -m topography_training.diffusion.layout reach"
    );
    (out, warnings)
}

/// Render a layout spec to L0 rasters and a fit-check report; returns the manifest path.
pub fn build_layout(config_path: &Path, output: &Path) -> Result<PathBuf> {
    ensure(!output.exists(), "layout output already exists")?;
    let text = String::from_utf8_lossy(&read_small(config_path)?).into_owned();
    let layout: Layout = toml::from_str(&text)
        .map_err(|e| crate::acquisition::AcquisitionError::Invalid(format!("layout spec: {e}")))?;
    validate(&layout)?;
    let r = render(&layout);
    let (report, warnings) = fit_check(&layout, &r);

    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    io_context("create layout parent", fs::create_dir_all(parent))?;
    let stage = io_context("stage layout", tempfile::tempdir_in(parent))?;
    let mut digests = serde_json::Map::new();
    let mut publish = |name: &str, bytes: &[u8]| -> Result<()> {
        write_new(&stage.path().join(name), bytes)?;
        digests.insert(name.to_owned(), json!(digest(bytes)));
        Ok(())
    };
    let f32s = |v: &[f32]| -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() };
    publish("mid.f32", &f32s(&r.mid))?;
    publish("relief.f32", &f32s(&r.relief))?;
    publish("coast.f32", &f32s(&r.coast))?;
    publish("class.u8", &r.class)?;
    publish("lake.u8", &r.lake)?;
    publish("lake-level.f32", &f32s(&r.lake_level))?;
    publish("snowline.f32", &f32s(&r.snowline))?;
    publish("forest.u8", &r.forest)?;
    let hold: Vec<u8> = r
        .hold
        .iter()
        .map(|w| (w.clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect();
    publish("hold.u8", &hold)?;
    publish("fit-check.txt", report.as_bytes())?;
    let manifest = json!({
        "schema_version": 1,
        "kind": "world_layout_l0",
        "name": layout.name,
        "cell_m": CELL_M,
        "size": [r.n, r.n],
        "extent_m": layout.extent_m,
        "orientation": "row-major, north-up; row 0 is y = 0 (north)",
        "config_path": config_path.display().to_string(),
        "config_sha256": digest(text.as_bytes()),
        "config_text": text,
        "treeline_m": layout.treeline_m,
        "fit_check_warnings": warnings,
        "layers": digests,
        "notes": {
            "mid.f32": "intended mean ground (m) at the 2 km envelope scale; sea is ground below 0 m",
            "relief.f32": "local relief (m) on top of mid",
            "coast.f32": "signed distance to the authored coast (m), positive inland",
            "class.u8": "terrain legend v2 class per cell",
            "lake.u8": "authored lakes (255)",
            "lake-level.f32": "lake surface (m), NaN elsewhere",
            "snowline.f32": "snowline (m) from wind exposure",
            "forest.u8": "forest share 0-255 below the treeline",
            "hold.u8": "weight 0-255 with which L1 sampling holds mid: the landmarks",
        },
    });
    write_new(
        &stage.path().join("layout.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    io_context("publish layout", fs::rename(stage.path(), output))?;
    print!("{report}");
    Ok(output.join("layout.json"))
}
