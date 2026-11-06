//! Breach or fill (Lindsay 2016's hybrid). A full depression is cut through when a graded
//! gorge drains it for less volume than it would hold: the gorge runs from the depression's
//! pit up its receivers to the spill and down the outlet's receivers to where the gentlest
//! grade from the pit meets the ground; its bed falls linearly over at least GORGE_RATIO × its
//! deepest cut, it is at least 3 cells wide (each cut tapered across the section, so no wall
//! loses more than its centre), and its deepest cut stays within the level's breach cap (at 1 m the 2 m
//! channel cap for every river, R9; the channels' raw − 2 m clamp keeps breach plus channel
//! within it; and only depressions a river flows into). Raw faces at least `NO_CUT_DEG` steep
//! beside the path are not cut, each cut's taper is wide enough for sides under TAPER_DEG, and a
//! gorge that would still leave a changed cell steeper than max(40°, raw) (the `walls` check)
//! is refused. Otherwise the depression keeps its water. Gorges lie within
//! `Params::breach_radius` cells of their spill, so every tiling decides alike.

use super::GORGE_RATIO;
use super::balance::mm;
use super::channel::NO_CUT_DEG;
use super::checks::{CHANGED_M, horn_deg, is_wall};
use super::geom::{Rect, World, step_len};
use super::hydraulic::Geometry;
use super::layers::Layer;
use super::pipeline::{Model, Setup, crop};
use super::rain::Q_PER_KM2;
use super::receivers::receivers;
use super::store::{Work, for_tiles};
use crate::acquisition::Result;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

/// Each cut's parabolic taper reaches out to where its edge slope (2·d/R) is this steep.
pub(crate) const TAPER_DEG: f32 = 35.0;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Candidate {
    /// The node's key (R28).
    pub key: u64,
    pub inside: u64,
    pub outside: u64,
    pub vfill_mm: i64,
    /// Gorge half-width in cells (at least 1: 3 cells wide).
    pub half: u64,
    pub cap: f32,
    /// Gorge cells lie within this many cells of `inside`.
    pub radius: u64,
    /// The depression's filled depth (level − lowest cell), m.
    pub fill_m: f32,
}

/// Why a candidate keeps its water.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Refused {
    /// Out of reach, over the cap or worth less than it drains.
    Fill,
    /// Its gorge (deepest cut, m) would leave a wall.
    Walls(f32),
}

/// Full nodes (never sea: sea is no node); at 1 m only those a river flows into. The cap is
/// the level's breach cap for every river (I1): at 1 m the channels' raw − 2 m clamp keeps
/// breach plus channel within 2 m.
pub(crate) fn candidates(s: &Setup, m: &Model, geo: &Geometry) -> Vec<Candidate> {
    let thr = (s.params.river_km2 * Q_PER_KM2) as u64;
    m.water
        .nodes
        .0
        .iter()
        .filter(|n| n.full)
        .filter_map(|n| {
            let inflow = m.flow.node_total.get(&n.key).copied().unwrap_or(0);
            let half = if s.params.channels {
                if inflow < thr {
                    return None;
                }
                (geo.width_m(inflow as f64 / Q_PER_KM2) / 2.0 / s.spacing_m)
                    .ceil()
                    .max(1.0) as u64
            } else {
                1
            };
            Some(Candidate {
                key: n.key,
                inside: n.outlet[0],
                outside: n.outlet[1],
                vfill_mm: n.vfill_mm,
                half,
                cap: s.params.breach_cap_m,
                radius: s.params.breach_radius,
                fill_m: n.level - n.low,
            })
        })
        .collect()
}

#[derive(Clone, Debug)]
pub(crate) struct Gorge {
    pub key: u64,
    /// (world cell, new ground), sorted by cell.
    pub cut: Vec<(u64, f32)>,
    pub depth: f32,
    pub volume_mm: i64,
    pub length_m: f64,
    /// The graded path, from the pit to where the grade meets the ground (tests check the walls).
    #[cfg(test)]
    pub axis: Vec<u64>,
}

/// The gorge draining `c`, or why not: `Fill` when it would leave the radius, cut deeper than
/// the cap or remove more than the depression holds; `Walls` when a changed cell would end
/// steeper than max(40°, raw). `zw` is the raw ground over `win`, which must hold `c.inside`
/// grown by `c.radius + 2` (clipped to the world) for every tiling to agree.
pub(crate) fn gorge(
    world: World,
    win: Rect,
    zw: &[f32],
    c: &Candidate,
    spacing_m: f64,
) -> std::result::Result<Gorge, Refused> {
    use Refused::Fill;
    let (ix, iy) = world.xy(c.inside);
    // Within the radius, with the 3 x 3 block receivers need inside the window.
    let near = |cell: u64| {
        let (x, y) = world.xy(cell);
        x.abs_diff(ix).max(y.abs_diff(iy)) <= c.radius
            && win.contains(x.saturating_sub(1), y.saturating_sub(1))
            && win.contains((x + 1).min(world.w - 1), (y + 1).min(world.h - 1))
    };
    let z = |cell: u64| {
        let (x, y) = world.xy(cell);
        zw[win.local(x, y).expect("near cells are in the window")]
    };
    let receiver = |cell: u64| -> Option<u64> {
        let (x, y) = world.xy(cell);
        let d = receivers(
            world,
            win,
            zw,
            Rect {
                x0: x,
                y0: y,
                w: 1,
                h: 1,
            },
        )[0];
        (d != 0).then(|| {
            let (nx, ny) = world.neighbour(x, y, d as usize - 1).expect("inside");
            world.index(nx, ny)
        })
    };
    if !near(c.inside) || !near(c.outside) {
        return Err(Fill);
    }
    // Up: from the spill's inside cell down to its pit; the gorge starts at the pit.
    let mut path = vec![c.inside];
    while let Some(next) = receiver(*path.last().ok_or(Fill)?) {
        if !near(next) {
            return Err(Fill);
        }
        path.push(next);
    }
    path.reverse();
    let zp = z(path[0]);
    // Down: the outlet's receivers as far as the radius reaches.
    let spill = path.len();
    let mut cur = c.outside;
    path.push(cur);
    while let Some(next) = receiver(cur).filter(|&n| near(n)) {
        path.push(next);
        cur = next;
    }
    let mut s = vec![0.0f64; path.len()];
    for i in 1..path.len() {
        let (a, b) = (world.xy(path[i - 1]), world.xy(path[i]));
        let diag = a.0 != b.0 && a.1 != b.1;
        s[i] = s[i - 1] + step_len(if diag { 5 } else { 1 }) * spacing_m;
    }
    let line = |k: f64, i: usize| (f64::from(zp) - k * s[i]) as f32;
    let cut_at = |k: f64, i: usize| (z(path[i]) - line(k, i)).max(0.0);
    // The grade: from the pit at the gentlest slope that meets the ground at a downstream cell
    // e at least GORGE_RATIO × its deepest cut away (the smallest cut; ties to the nearer e).
    let mut best: Option<(f64, usize)> = None;
    for e in spill..path.len() {
        let drop = f64::from(zp - z(path[e]));
        if drop <= 0.0 {
            continue;
        }
        let k = drop / s[e];
        if best.is_some_and(|(b, _)| k >= b) {
            continue;
        }
        let depth = (0..=e).map(|i| cut_at(k, i)).fold(0.0f32, f32::max);
        if s[e] >= f64::from(GORGE_RATIO * depth) {
            best = Some((k, e));
        }
    }
    let (k, e) = best.ok_or(Fill)?;
    // Raw at least NO_CUT_DEG steep (world-edge cells count as gentle); near cells have their
    // 3 x 3 in the window.
    let steep = |cell: u64| {
        let (x, y) = world.xy(cell);
        let j = win.local(x, y).expect("near cells are in the window");
        !world.is_edge(x, y) && horn_deg(zw, win.w, j % win.w, j / win.w, spacing_m) >= NO_CUT_DEG
    };
    let on_path: HashSet<u64> = path[..=e].iter().copied().collect();
    // The section: around each path cell that needs a cut d, a Euclidean disc of radius
    // max(half + 1, 2·d / tan TAPER_DEG) cells lowered by that cut tapered parabolically to
    // nothing at its edge, never below the grade at that cell (no new pits), so the gorge is at
    // least 3 cells wide, has no square walls and cuts no wall more than its centre. Path cells
    // are always cut; steep raw faces beside them are not.
    let mut new: HashMap<u64, f32> = HashMap::new();
    let tan = TAPER_DEG.to_radians().tan();
    for (i, &p) in path.iter().enumerate().take(e + 1) {
        let d = cut_at(k, i);
        if d <= 0.0 {
            continue;
        }
        let radius = (c.half as f32 + 1.0).max(2.0 * d / tan);
        let h = radius.ceil() as i64;
        let (px, py) = world.xy(p);
        for dy in -h..=h {
            for dx in -h..=h {
                let r = ((dx * dx + dy * dy) as f32).sqrt();
                if r >= radius {
                    continue;
                }
                let (Some(x), Some(y)) = (px.checked_add_signed(dx), py.checked_add_signed(dy))
                else {
                    continue;
                };
                if x >= world.w || y >= world.h {
                    continue;
                }
                let cell = world.index(x, y);
                if !near(cell) {
                    return Err(Fill);
                }
                if !on_path.contains(&cell) && steep(cell) {
                    continue;
                }
                let ground = z(cell);
                let bed = (ground - d * (1.0 - (r / radius).powi(2))).max(line(k, i));
                if bed < ground {
                    let v = new.entry(cell).or_insert(bed);
                    *v = v.min(bed);
                }
            }
        }
    }
    let depth = new
        .iter()
        .map(|(&cell, &v)| z(cell) - v)
        .fold(0.0f32, f32::max);
    if depth <= 0.0 || depth > c.cap {
        return Err(Fill);
    }
    let volume_mm: i64 = new.iter().map(|(&cell, &v)| mm(z(cell)) - mm(v)).sum();
    if volume_mm >= c.vfill_mm {
        return Err(Fill);
    }
    // Walls: with this gorge cut, no cell changed by more than CHANGED_M (only cut cells
    // changed; their 8-rings are not) may end steeper than max(40°, raw).
    let block = |x: u64, y: u64, cut: bool| -> [f32; 9] {
        std::array::from_fn(|i| {
            let cell = world.index(x + (i % 3) as u64 - 1, y + (i / 3) as u64 - 1);
            match new.get(&cell) {
                Some(&v) if cut => v,
                _ => z(cell),
            }
        })
    };
    let wall = new.iter().any(|(&cell, &v)| {
        let (x, y) = world.xy(cell);
        z(cell) - v > CHANGED_M
            && !world.is_edge(x, y)
            && is_wall(
                horn_deg(&block(x, y, true), 3, 1, 1, spacing_m),
                horn_deg(&block(x, y, false), 3, 1, 1, spacing_m),
            )
    });
    if wall {
        return Err(Refused::Walls(depth));
    }
    let mut cut: Vec<(u64, f32)> = new.into_iter().collect();
    cut.sort_unstable_by_key(|p| p.0);
    Ok(Gorge {
        key: c.key,
        cut,
        depth,
        volume_mm,
        length_m: s[e],
        #[cfg(test)]
        axis: path[..=e].to_vec(),
    })
}

/// Breach figures of one gorge: (node key, deepest cut, volume, length, spill cell).
type Figure = (u64, f32, i64, f64, u64);
/// A gorge refused for walls: (node key, deepest cut, spill cell, filled depth).
type Rejected = (u64, f32, u64, f32);

/// Writes `ground` with every gorge cut to `out` (cores only) and returns the breach figures:
/// count, deepest cut, total volume in m³ and the 20 deepest with their cells; and the gorges
/// refused for walls (`breaches_rejected_walls`): count and the 20 deepest.
pub(crate) fn breach(
    s: &Setup,
    m: &Model,
    ground: &Layer<f32>,
    out: &Layer<f32>,
    geo: &Geometry,
    work: Option<&Work>,
    tag: &str,
) -> Result<Value> {
    let world = s.world;
    let cands = candidates(s, m, geo);
    // Every gorge cell lies within the radius of its spill, so a tile takes the candidates
    // whose spill is that close and reads every cell within the radius + 2 of those spills
    // (the wall test's slope stencil).
    let reach = s.params.breach_radius;
    let halo = 2 * reach + 2;
    let per_tile: Vec<(Vec<Figure>, Vec<Rejected>)> = for_tiles(
        &s.tiles(),
        s.threads,
        &format!("{tag} breach"),
        |i, core| {
            let name = format!("{tag}/breach/{i:05}");
            if let Some(wk) = work {
                if wk.is_done(&name) {
                    return wk.cached(&format!("{name}-figures"), || Ok((Vec::new(), Vec::new())));
                }
            }
            let win = core.grow(halo, world);
            let zw = ground.read(win)?;
            let mut new: Vec<f32> = crop(win, &zw, core);
            let reach_rect = core.grow(reach, world);
            let (mut figures, mut rejected) = (Vec::new(), Vec::new());
            for c in &cands {
                let (x, y) = world.xy(c.inside);
                if !reach_rect.contains(x, y) {
                    continue;
                }
                let g = match gorge(world, win, &zw, c, s.spacing_m) {
                    Ok(g) => g,
                    Err(Refused::Walls(depth)) => {
                        if core.contains(x, y) {
                            rejected.push((c.key, depth, c.inside, c.fill_m));
                        }
                        continue;
                    }
                    Err(Refused::Fill) => continue,
                };
                for &(cell, v) in &g.cut {
                    let (cx, cy) = world.xy(cell);
                    if let Some(k) = core.local(cx, cy) {
                        new[k] = new[k].min(v);
                    }
                }
                if core.contains(x, y) {
                    figures.push((g.key, g.depth, g.volume_mm, g.length_m, c.inside));
                }
            }
            out.write(core, &new)?;
            let result = (figures, rejected);
            if let Some(wk) = work {
                wk.cached(&format!("{name}-figures"), || Ok(result.clone()))?;
                wk.mark_done(&name)?;
            }
            Ok(result)
        },
    )?;
    let (mut all, mut refused): (Vec<Figure>, Vec<Rejected>) = (Vec::new(), Vec::new());
    for (f, r) in per_tile {
        all.extend(f);
        refused.extend(r);
    }
    all.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    refused.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let cell_m2 = s.spacing_m * s.spacing_m;
    let m3 = |v: i64| v as f64 / 1000.0 * cell_m2;
    Ok(json!({
        "breaches": all.len(),
        "deepest_cut_m": all.first().map_or(0.0, |b| b.1),
        "volume_m3": all.iter().map(|b| m3(b.2)).sum::<f64>().round(),
        "deepest": all.iter().take(20).map(|b| {
            let (x, y) = world.xy(b.4);
            json!({
                "node": b.0,
                "cut_m": b.1,
                "volume_m3": m3(b.2).round(),
                "length_m": b.3,
                "spill_cell": [x, y],
            })
        }).collect::<Vec<_>>(),
        "breaches_rejected_walls": {
            "count": refused.len(),
            "deepest": refused.iter().take(20).map(|r| {
                let (x, y) = world.xy(r.2);
                json!({"node": r.0, "cut_m": r.1, "spill_cell": [x, y], "filled_depth_m": r.3})
            }).collect::<Vec<_>>(),
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::water::pipeline::{Rasters, model};
    use crate::world::water::testing::setup;

    /// A V valley `w` cells long falling east 5 cm a cell, its sides rising `wall` m a cell,
    /// walled at its west end (so the pond can only leave over the dam, N8) and dammed at `at`
    /// by a one-cell wall `dam` m high at its centre, whose crest rises 2 cm a row (a flat
    /// crest ties with the valley sides, and the spill would wander along it).
    fn dammed_at(w: u64, at: f32, dam: f32, wall: f32) -> (World, Vec<f32>) {
        let world = World { w, h: 21 };
        let z = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
                let side = (y - 10.0).abs();
                let v = 10.0 - 0.05 * x + wall * side + if x <= 1.0 { 30.0 } else { 0.0 };
                if x == at && side <= 6.0 {
                    10.0 - 0.05 * (at - 1.0) + dam + 0.02 * side
                } else {
                    v
                }
            })
            .collect();
        (world, z)
    }

    fn dammed(dam: f32) -> (World, Vec<f32>) {
        dammed_at(100, 40.0, dam, 0.25)
    }

    /// A V valley (100 × 21) falling east 2 cm a cell to x = 40 and 5 cm a cell past it, its
    /// sides rising `wall` m a cell, walled at its west end and crossed at x = 40 by a mound
    /// `dam` m high falling to nothing `len` cells up and down the valley; with `hole`, a 1.5 m
    /// deep hole on cells x 39-41, rows 13-15 (sides 3-5) beside the spill, whose rim is steep.
    fn mound(dam: f32, len: f32, wall: f32, hole: bool) -> (World, Vec<f32>) {
        let world = World { w: 100, h: 21 };
        let z = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
                let side = (y - 10.0).abs();
                let pit = hole && (39.0..=41.0).contains(&x) && (13.0..=15.0).contains(&y);
                let floor = if x <= 40.0 {
                    10.0 - 0.02 * x
                } else {
                    9.2 - 0.05 * (x - 40.0)
                };
                floor
                    + wall * side
                    + dam * (1.0 - (x - 40.0).abs() / len).max(0.0)
                    + if x <= 1.0 { 30.0 } else { 0.0 }
                    - if pit { 3.0 } else { 0.0 }
            })
            .collect();
        (world, z)
    }

    /// Changed cells steeper than max(40°, raw) (the `walls` check) after cutting `g`.
    fn gorge_walls(world: World, z: &[f32], g: &Gorge) -> usize {
        let mut cut = z.to_vec();
        for &(k, v) in &g.cut {
            cut[k as usize] = v;
        }
        let w = world.w as usize;
        let mut n = 0;
        for y in 1..world.h as usize - 1 {
            for x in 1..w - 1 {
                let k = y * w + x;
                n += usize::from(
                    z[k] - cut[k] > CHANGED_M
                        && is_wall(horn_deg(&cut, w, x, y, 1.0), horn_deg(z, w, x, y, 1.0)),
                );
            }
        }
        n
    }

    fn the_candidate(world: World, z: &[f32], cap: f32) -> (Candidate, Rect) {
        let mut s = setup(world, 128, 1e-5);
        s.classes = None;
        s.params.breach_cap_m = cap;
        let m = model(&s, &Layer::memory(world, z.to_vec()), None, "m", None).expect("model");
        let geo = Geometry::new(2.0);
        let c = candidates(&s, &m, &geo)
            .into_iter()
            .max_by_key(|c| c.vfill_mm)
            .expect("the pond behind the dam");
        (c, Rect::whole(world))
    }

    #[test]
    fn a_narrow_dam_is_cut_as_a_graded_gorge() {
        let (world, z) = dammed(1.0);
        let (c, all) = the_candidate(world, &z, 2.0);
        let g = gorge(world, all, &z, &c, 1.0).expect("breached");
        assert!(g.depth > 0.9 && g.depth <= 2.0, "depth {}", g.depth);
        assert!(
            g.length_m >= f64::from(GORGE_RATIO * g.depth),
            "length {} for depth {}",
            g.length_m,
            g.depth
        );
        // At least 3 neighbouring dam cells are cut (the gorge is 3 cells wide, not a slot).
        let cut_dam: Vec<u64> = g
            .cut
            .iter()
            .filter(|&&(c, v)| c % 100 == 40 && v < z[c as usize] - 0.5)
            .map(|p| p.0 / 100)
            .collect();
        assert!(cut_dam.len() >= 3, "{cut_dam:?}");
        assert!(
            cut_dam.windows(2).all(|p| p[1] == p[0] + 1),
            "adjacent rows: {cut_dam:?}"
        );
    }

    #[test]
    fn a_dam_above_the_cap_or_worth_less_than_its_cut_stays() {
        // Steep sides, so the 2.5 m dam's ends stand above its crest.
        let (world, z) = dammed_at(100, 40.0, 2.5, 0.5);
        let (c, all) = the_candidate(world, &z, 2.0);
        // The pond leaves over the dam, not off the map: the cap is what refuses it.
        let (ix, _) = world.xy(c.inside);
        let (ox, _) = world.xy(c.outside);
        assert!(
            (39..=41).contains(&ix) && (39..=41).contains(&ox),
            "outlet at the dam: {ix} {ox}"
        );
        assert_eq!(
            gorge(world, all, &z, &c, 1.0).err(),
            Some(Refused::Fill),
            "a 2.55 m cut, 2 m cap"
        );
        let mut wide = c;
        wide.cap = 3.0;
        assert!(
            matches!(gorge(world, all, &z, &wide, 1.0), Err(Refused::Walls(d)) if d > 2.5),
            "under a 3 m cap the cap no longer refuses it (the one-cell dam's gorge would leave walls)"
        );
        let (world, z) = dammed(1.0);
        let (mut c, all) = the_candidate(world, &z, 2.0);
        c.vfill_mm = 1;
        assert_eq!(
            gorge(world, all, &z, &c, 1.0).err(),
            Some(Refused::Fill),
            "the cut would remove more than it drains"
        );
    }

    #[test]
    fn breaching_is_alike_at_every_tiling() {
        // A dam on the tile seam at x = 64; a breach radius of 24 keeps the windows (tile + 49)
        // smaller than the world, so the gorge is cut across tiles (N9).
        let (world, z) = dammed_at(128, 64.0, 1.0, 0.25);
        let ground = Layer::memory(world, z.clone());
        let mut out = Vec::new();
        for tile in [128, 32] {
            let mut s = setup(world, tile, 1e-5);
            s.classes = None;
            s.params.breach_radius = 24;
            let m = model(&s, &ground, None, "m", Some(&Rasters::memory(world))).expect("model");
            let breached = Layer::filled(world, 0.0f32);
            let figures =
                breach(&s, &m, &ground, &breached, &Geometry::new(2.0), None, "b").expect("breach");
            assert_eq!(figures["breaches"], 1, "tile {tile}");
            out.push(breached.to_vec().unwrap());
        }
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&out[0]), bits(&out[1]));
        let cut: Vec<u64> = (0..z.len())
            .filter(|&k| out[1][k] < z[k])
            .map(|k| k as u64 % 128)
            .collect();
        assert!(
            cut.iter().any(|&x| x < 64) && cut.iter().any(|&x| x >= 64),
            "the gorge crosses the seam"
        );
    }

    /// The disc radius of each axis cell of `g` (fix 1b).
    fn radii(g: &Gorge, z: &[f32], half: u64) -> HashMap<u64, f32> {
        let bed: HashMap<u64, f32> = g.cut.iter().copied().collect();
        let tan = TAPER_DEG.to_radians().tan();
        g.axis
            .iter()
            .filter_map(|a| bed.get(a).map(|b| (*a, z[*a as usize] - b)))
            .map(|(a, d)| (a, (half as f32 + 1.0).max(2.0 * d / tan)))
            .collect()
    }

    fn dist(world: World, a: u64, b: u64) -> f32 {
        let ((ax, ay), (bx, by)) = (world.xy(a), world.xy(b));
        (ax.abs_diff(bx) as f32).hypot(ay.abs_diff(by) as f32)
    }

    #[test]
    fn a_deep_gorge_is_tapered_wide_enough_to_leave_no_wall() {
        let (world, z) = mound(1.8, 8.0, 0.25, false);
        let (c, all) = the_candidate(world, &z, 2.0);
        let g = gorge(world, all, &z, &c, 1.0).expect("breached");
        assert!(g.depth > 1.0 && g.depth <= c.cap, "depth {}", g.depth);
        let most = g
            .cut
            .iter()
            .map(|&(k, v)| z[k as usize] - v)
            .fold(0.0f32, f32::max);
        assert_eq!(most, g.depth, "the depth is the deepest cut of any cell");
        assert_eq!(gorge_walls(world, &z, &g), 0);
        // Wider than half + 1 where the cut is deep; a cell off the axis loses no more than the
        // axis cells whose discs cover it.
        let bed: HashMap<u64, f32> = g.cut.iter().copied().collect();
        let r = radii(&g, &z, c.half);
        let far = g.cut.iter().any(|&(k, _)| {
            g.axis
                .iter()
                .all(|&a| dist(world, a, k) >= c.half as f32 + 1.0)
        });
        assert!(far, "a cut beyond half + 1 cells of the axis");
        for &(k, v) in &g.cut {
            if !g.axis.contains(&k) {
                let centre = r
                    .iter()
                    .filter(|&(&a, &ra)| dist(world, a, k) < ra)
                    .map(|(a, _)| z[*a as usize] - bed[a])
                    .fold(0.0f32, f32::max);
                assert!(
                    z[k as usize] - v <= centre + 1e-6,
                    "wall {:?} cut {} beside an axis cut {centre}",
                    world.xy(k),
                    z[k as usize] - v
                );
            }
        }
    }

    #[test]
    fn steep_raw_beside_the_path_is_not_cut() {
        let (world, z) = mound(1.0, 8.0, 0.2, true);
        let (c, all) = the_candidate(world, &z, 2.0);
        let g = gorge(world, all, &z, &c, 1.0).expect("breached");
        assert_eq!(gorge_walls(world, &z, &g), 0);
        let w = world.w as usize;
        let steep = |k: u64| {
            let (x, y) = world.xy(k);
            !world.is_edge(x, y) && horn_deg(&z, w, x as usize, y as usize, 1.0) >= NO_CUT_DEG
        };
        let cut: HashMap<u64, f32> = g.cut.iter().copied().collect();
        for &k in cut.keys() {
            assert!(
                g.axis.contains(&k) || !steep(k),
                "steep raw cell {:?} cut",
                world.xy(k)
            );
        }
        // The rule mattered: a steep cell inside a disc stays as it is.
        let r = radii(&g, &z, c.half);
        let spared = (0..world.w * world.h).any(|k| {
            steep(k) && !cut.contains_key(&k) && r.iter().any(|(&a, &ra)| dist(world, a, k) < ra)
        });
        assert!(spared, "no steep cell inside a disc");
    }

    #[test]
    fn a_gorge_that_would_leave_walls_fills_and_is_reported() {
        // A 1.8 m one-cell dam between 45° sides: the path's slot steepens its sides.
        let (world, z) = dammed_at(100, 40.0, 1.8, 1.0);
        let (c, all) = the_candidate(world, &z, 2.0);
        assert!(
            matches!(gorge(world, all, &z, &c, 1.0), Err(Refused::Walls(d)) if d > 1.5 && d <= 2.0),
            "refused for walls"
        );
        let mut s = setup(world, 128, 1e-5);
        s.classes = None;
        s.params.breach_cap_m = 2.0;
        let ground = Layer::memory(world, z.clone());
        let m = model(&s, &ground, None, "m", None).expect("model");
        let out = Layer::filled(world, 0.0f32);
        let fig = breach(&s, &m, &ground, &out, &Geometry::new(2.0), None, "b").expect("breach");
        assert_eq!(fig["breaches"], 0, "{fig}");
        let rejected = &fig["breaches_rejected_walls"];
        assert_eq!(rejected["count"], 1, "{fig}");
        let first = &rejected["deepest"][0];
        assert_eq!(first["node"], c.key);
        let (ix, iy) = world.xy(c.inside);
        assert_eq!(first["spill_cell"], json!([ix, iy]));
        let filled = first["filled_depth_m"].as_f64().unwrap();
        assert!((filled - 1.8).abs() < 0.05, "filled depth {filled}");
        assert_eq!(
            out.to_vec().unwrap(),
            z,
            "the depression stays filled: nothing is cut"
        );
    }

    /// A valley with its pit 18 cells upstream of a dam at x = 40 (off the 16-cell seams): the
    /// floor rises 1 cm a cell from the pit to the dam (0.3 m), drops 0.5 m below the pit past
    /// it and falls 1 cm a cell, so the gentlest grade meets the ground at the radius' end and
    /// the gorge spans both sides of the spill. Sides 1:20 so the pond is worth the cut.
    fn spanning() -> (World, Vec<f32>) {
        let world = World { w: 128, h: 41 };
        let z = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
                let side = (y - 20.0).abs();
                let (floor, wall) = if x <= 1.0 {
                    (40.0, 0.05)
                } else if x <= 22.0 {
                    (10.0 + 0.05 * (22.0 - x), 0.05)
                } else if x < 40.0 || (x == 40.0 && side > 16.0) {
                    (10.0 + 0.01 * (x - 22.0), 0.05)
                } else if x == 40.0 {
                    (10.47, 0.02)
                } else {
                    (9.5 - 0.01 * (x - 41.0), 0.05)
                };
                floor + wall * side
            })
            .collect();
        (world, z)
    }

    #[test]
    fn a_gorge_spanning_its_spill_is_cut_alike_at_every_tiling() {
        let (world, z) = spanning();
        let ground = Layer::memory(world, z.clone());
        let mut out = Vec::new();
        for tile in [128, 16] {
            let mut s = setup(world, tile, 1e-5);
            s.classes = None;
            s.params.breach_radius = 24;
            let m = model(&s, &ground, None, "m", Some(&Rasters::memory(world))).expect("model");
            let breached = Layer::filled(world, 0.0f32);
            let figures =
                breach(&s, &m, &ground, &breached, &Geometry::new(2.0), None, "b").expect("breach");
            assert_eq!(figures["breaches"], 1, "tile {tile}: {figures}");
            out.push(breached.to_vec().unwrap());
        }
        let cut: Vec<u64> = (0..z.len())
            .filter(|&k| out[0][k] < z[k])
            .map(|k| k as u64 % 128)
            .collect();
        assert!(
            cut.iter().any(|&x| x <= 25) && cut.iter().any(|&x| x >= 40),
            "the gorge spans the spill: {cut:?}"
        );
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&out[0]), bits(&out[1]));
    }

    /// Expert review: the model, breach and the model again on a 1,024² raw window at E43966
    /// N21633 (the staircase), tiles of 256², the layout's classes, the table's width factor,
    /// river start 0.05 km², uniform rain. Prints the breach figures and node counts before and
    /// after; writes t11-e43966-before.png and -after.png (lakes blue, rivers cyan, cut cells
    /// red by depth) and a cut-depth cross-section through the deepest spill.
    /// cargo test --release … breach -- --ignored --nocapture, under systemd-run MemoryMax=6G.
    #[test]
    #[ignore]
    fn probe_breach_at_e43966() {
        use crate::world::water::layers::Classes;
        use crate::world::water::probe;
        use crate::world::water::table::{DEFAULT, Table};
        let Some((win, z)) = probe::window(43966, 21633, 1024) else {
            return;
        };
        let Some(class) = probe::classes(win) else {
            return;
        };
        let world = World {
            w: win.w as u64,
            h: win.h as u64,
        };
        let mut s = setup(world, 256, 0.05);
        s.table = Table::parse(DEFAULT).expect("table");
        s.nw_m = [win.x0 as f64, (probe::WORLD.h - win.y0) as f64];
        s.classes = Some(Classes::new(Layer::memory(world, class), 1, world).expect("classes"));
        let geo = Geometry::new(s.table.m3s_per_km2);
        let run = |name: &str, ground: &[f32]| {
            let r = Rasters::memory(world);
            let m = model(
                &s,
                &Layer::memory(world, ground.to_vec()),
                None,
                "m",
                Some(&r),
            )
            .expect("model");
            let (node, flow) = (r.node.to_vec().unwrap(), r.flow.to_vec().unwrap());
            let lake = |k: usize| node[k] >= 2 && m.water.nodes.at(node[k] - 2).lake;
            let lakes = m.water.nodes.0.iter().filter(|n| n.lake).count();
            let full = m.water.nodes.0.iter().filter(|n| n.lake && n.full).count();
            let lake_cells = (0..ground.len()).filter(|&k| lake(k)).count();
            let rivers = flow.iter().filter(|&&f| f & 16 != 0).count();
            eprintln!(
                "{name}: nodes {} lakes {lakes} (full {full}) lake cells {lake_cells} river cells \
                 {rivers}",
                m.water.nodes.0.len()
            );
            let tint: Vec<Option<[u8; 3]>> = (0..ground.len())
                .map(|k| {
                    let cut = z[k] - ground[k];
                    if cut > 0.01 {
                        let red = (6.0 * cut / 2.0).clamp(1.0, 6.0) as u8;
                        Some([6, 6 - red, 6 - red])
                    } else if lake(k) {
                        Some([2, 3, 6])
                    } else if flow[k] & 16 != 0 {
                        Some([1, 5, 6])
                    } else {
                        None
                    }
                })
                .collect();
            probe::shade_tinted(name, ground, win.w, |k| tint[k]);
            m
        };
        let m = run("t11-e43966-before", &z);
        eprintln!(
            "candidates (full nodes a river flows into): {}",
            candidates(&s, &m, &geo).len()
        );
        let ground = Layer::memory(world, z.clone());
        let out = Layer::filled(world, 0.0f32);
        let figures = breach(&s, &m, &ground, &out, &geo, None, "b").expect("breach");
        eprintln!(
            "breach figures: {}",
            serde_json::to_string_pretty(&figures).unwrap()
        );
        let after = out.to_vec().unwrap();
        assert!(
            (0..z.len()).all(|k| after[k] <= z[k]),
            "ground is never raised"
        );
        let cut = (0..z.len()).filter(|&k| after[k] < z[k]).count();
        eprintln!("cut cells {cut}");
        // Off-axis cuts: wall cells beside each gorge's graded path.
        for c in candidates(&s, &m, &geo) {
            if let Ok(g) = gorge(world, Rect::whole(world), &z, &c, s.spacing_m) {
                let off = g
                    .cut
                    .iter()
                    .filter(|p| !g.axis.contains(&p.0))
                    .map(|&(k, v)| z[k as usize] - v)
                    .fold(0.0f32, f32::max);
                eprintln!(
                    "gorge {}: depth {:.2} m, length {:.1} m, {} cells, max off-axis cut {off:.2} m",
                    g.key,
                    g.depth,
                    g.length_m,
                    g.cut.len()
                );
            }
        }
        run("t11-e43966-after", &after);
        // A cross-section (cut depth, m) across the deepest gorge at its spill, rows and columns.
        if let Some(d) = figures["deepest"].as_array().and_then(|a| a.first()) {
            let (sx, sy) = (
                d["spill_cell"][0].as_u64().unwrap(),
                d["spill_cell"][1].as_u64().unwrap(),
            );
            for (label, horizontal) in [("row", true), ("column", false)] {
                let line: Vec<String> = (-8i64..=8)
                    .filter_map(|o| {
                        let (x, y) = if horizontal {
                            (sx.checked_add_signed(o)?, sy)
                        } else {
                            (sx, sy.checked_add_signed(o)?)
                        };
                        let k = world.index(x.min(world.w - 1), y.min(world.h - 1)) as usize;
                        Some(format!("{:.2}", z[k] - after[k]))
                    })
                    .collect();
                eprintln!(
                    "deepest spill ({sx}, {sy}) E{} N{}: cut along the {label}: {}",
                    win.x0 + sx,
                    probe::WORLD.h - 1 - (win.y0 + sy),
                    line.join(" ")
                );
            }
        }
    }
}
