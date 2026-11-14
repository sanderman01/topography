//! 1 m channels. Each thalweg cell's section (width and depth from its discharge, hydraulic
//! geometry) is cut below the river's surface: the routing surface R (the ground on dry
//! thalweg cells, the pond level inside ponds), averaged along the river's stem on dry cells
//! (STEM_CELLS each way, within STEM_DEV_M of R) and swept between neighbouring stem cells,
//! so steep reaches get smooth banks. R never rises along a path, so neither does the river. The section is the thalweg's centre cut (down to surface − depth) tapered by
//! 1 − (r/R)² across the half-width R, so no cell loses more than its centre (no wall is cut
//! by its height above a line). A cell within several rivers' half-widths belongs to the
//! river with the larger discharge (its body's largest), then the nearest cell, then the
//! smaller index (review I3b): a tributary inside a larger river's section joins that river
//! there instead of running beside it; a thalweg cell the larger river does not take keeps its
//! own. A cell is lowered only when it ends up under its river's surface, and never more than
//! CHANNEL_CAP_M below raw (R9). Rims beside lakes and ponds are cut like any section cell
//! (F1): a labelled lake whose spill on the carved ground falls below its label is lowered to
//! it afterwards (`relevel`, outlet incision). Lakes, pools and the sea keep their level; their
//! cells whose ground reaches the level are dry shore, as is a lake's exit cell at the level.
//! Other river water over a bed left uncut runs MIN_DEPTH_M deep (export ruling). A river
//! cell below the surface of the cell it flows into is raised to it over BACKWATER cells
//! (N18). At coarser spacings (no channels) rivers are their thalweg cells at R and nothing
//! is cut. Owner rule (2026-11-08): no pass takes a cut back to satisfy a check (`walls`,
//! `bed_rises` and `carve_pits` are figures).

use super::checks::SILL_M;
use super::geom::{Rect, World, tile_of};
use super::hydraulic::Geometry;
use super::layers::Layer;
use super::nodes::Nodes;
use super::pipeline::{Rasters, Setup, Water};
use super::store::{Blob, Dec, Enc, Work, cached, for_tiles};
use super::{CHANNEL_CAP_M, MIN_DEPTH_M};
use crate::acquisition::{Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

/// Window halo: the widest section's radius plus the backwater fits inside it.
pub(crate) const HALO: u64 = 64;
/// A river cell's surface is raised to that of the river cell it flows into over at most
/// this many cells upstream (N18). HALO covers the widest radius + BACKWATER.
pub(crate) const BACKWATER: u64 = 16;
/// Section radii are clamped here (cells) so every section reaching a tile's area lies in
/// its window; real rivers stay far below (30 m wide at most, R5).
const MAX_RADIUS: f64 = (HALO - BACKWATER) as f64;
/// The area's margin beyond the core: the backwater (16) and one cell, so every tiling agrees.
/// Owner rule (2026-11-08): no wall, grade or pit rounds undo a cut, so none need a margin.
const MARGIN: u64 = BACKWATER + 1;
/// A thalweg's river surface is the mean routing level over this many stem cells each way.
const STEM_CELLS: usize = 4;
/// ... and at most this far (m) from its own routing level.
const STEM_DEV_M: f32 = CHANNEL_CAP_M / 2.0;

pub(crate) struct Layers {
    pub carved: Layer<f32>,
    pub surface: Layer<f32>,
    pub body: Layer<u32>,
}

impl Layers {
    #[cfg(test)]
    pub fn memory(world: World) -> Self {
        Self {
            carved: Layer::filled(world, 0.0),
            surface: Layer::filled(world, f32::NAN),
            body: Layer::filled(world, 0),
        }
    }
    pub fn files(world: World, dir: &Path) -> Result<Self> {
        Ok(Self {
            carved: Layer::open_or_create(world, &dir.join("carved.f32"), 0.0)?,
            surface: Layer::open_or_create(world, &dir.join("water-surface.f32"), f32::NAN)?,
            body: Layer::open_or_create(world, &dir.join("water-body.u32"), 0)?,
        })
    }
}

/// Per tile: lowered cells, deepest lowering below the ground, volume m³, river cells,
/// deepest lowering below raw; and cells by WetFix (index − 1), then thalwegs returned to
/// their own river.
type Figure = ((u64, f32, f64, u64, f32), [u64; 4]);
/// How the export ruling changed a cell: river water raised to MIN_DEPTH_M over its bed, lake
/// or sea ground at its level left dry, a lake's exit cell left dry.
#[derive(Clone, Copy, PartialEq)]
enum WetFix {
    None,
    MinDepth,
    Shore,
    Exit,
}
/// Per tile: (river body, largest discharge) and (pond ordinal, crossing body).
type Sizes = (Vec<(u32, f32)>, Vec<(u32, u32)>);

/// Keeps the two best candidates of distinct river bodies, least first (a body's candidate
/// is replaced only by a better one of the same body). Exact under any insertion order.
fn offer(slots: &mut [(f32, u32, u32); 2], cand: (f32, u32, u32), body: impl Fn(u32) -> u32) {
    let less = |a: (f32, u32, u32), b: (f32, u32, u32)| {
        b.2 == u32::MAX || a.0 < b.0 || (a.0 == b.0 && (a.1, a.2) < (b.1, b.2))
    };
    let b = body(cand.2);
    let same = |s: (f32, u32, u32)| s.2 != u32::MAX && body(s.2) == b;
    if same(slots[0]) {
        if less(cand, slots[0]) {
            slots[0] = cand;
        }
    } else if same(slots[1]) {
        if less(cand, slots[1]) {
            slots[1] = cand;
            if less(slots[1], slots[0]) {
                slots.swap(0, 1);
            }
        }
    } else if less(cand, slots[0]) {
        slots[1] = slots[0];
        slots[0] = cand;
    } else if less(cand, slots[1]) {
        slots[1] = cand;
    }
}

/// Point `p` projected on the segment from the origin to `b`: (fraction along it in 0..=1,
/// distance²).
fn project(p: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let len2 = b.0 * b.0 + b.1 * b.1;
    let f = ((p.0 * b.0 + p.1 * b.1) / len2).clamp(0.0, 1.0);
    let (dx, dy) = (p.0 - b.0 * f, p.1 - b.1 * f);
    (f, dx * dx + dy * dy)
}

/// A river cell beside a lake below its river's surface `rs`, cut below that lake's level
/// `lake`, stands at the lake's level: no drainage rise at the mouth (fix round 2).
fn mouth_surface(rs: f32, cut: f32, lake: Option<f32>) -> f32 {
    match lake {
        Some(lv) if cut < lv => lv,
        _ => rs,
    }
}

/// A section cell its smaller cut leaves at or above its river's surface is dry again (no
/// surface, no label), unless it is its own m2 thalweg (`own`: its river's surface R and
/// body), which returns to its own river: (surface, body, river, keep).
fn dry_again(own: Option<(f32, u32)>) -> (f32, u32, bool, bool) {
    match own {
        Some((rs, id)) => (rs, id, true, true),
        None => (f32::NAN, 0, false, false),
    }
}

/// A river cell's surface over its bed `carved`: kept above it; none (dry shore) where a lake
/// beside it stands at or above the bed (`sill`: its exit at the level); else flowing water
/// over a bed left uncut, MIN_DEPTH_M deep (export ruling).
fn above_bed(surface: f32, carved: f32, sill: bool) -> Option<f32> {
    if surface > carved {
        Some(surface)
    } else if sill {
        None
    } else {
        Some(carved + MIN_DEPTH_M)
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn channels(
    s: &Setup,
    w: &Water,
    raw: &Layer<f32>,
    ground: &Layer<f32>,
    r: &Rasters,
    body: &Layer<u32>,
    out: &Layers,
    geo: &Geometry,
    work: Option<&Work>,
    tag: &str,
) -> Result<Value> {
    let world = s.world;
    let tiles = s.tiles();
    let cells_per_m = 1.0 / s.spacing_m;
    let is_lake_node = |n: u32| n >= 2 && w.nodes.at(n - 2).lake;

    // Each river body's largest discharge (the larger river takes a shared section, I3), and
    // the bodies whose thalwegs cross each full pond (by node ordinal).
    let sizes: Vec<Sizes> = for_tiles(
        &tiles,
        s.threads,
        &format!("{tag} river sizes"),
        |i, core| {
            cached(work, &format!("{tag}/chq/{i:05}"), || {
                let (node, flow) = (r.node.read(core)?, r.flow.read(core)?);
                let (q, b) = (r.discharge.read(core)?, body.read(core)?);
                let mut m: BTreeMap<u32, f32> = BTreeMap::new();
                let mut crossing: Vec<(u32, u32)> = Vec::new();
                for k in 0..core.cells() {
                    if flow[k] & 16 == 0 || node[k] == 1 || is_lake_node(node[k]) {
                        continue;
                    }
                    let (x, y) = core.world_xy(k);
                    ensure(b[k] >= 2, format!("thalweg cell ({x}, {y}) has no body"))?;
                    let e = m.entry(b[k]).or_insert(0.0);
                    *e = e.max(q[k]);
                    if node[k] >= 2 && w.nodes.at(node[k] - 2).full {
                        crossing.push((node[k] - 2, b[k]));
                    }
                }
                crossing.sort_unstable();
                crossing.dedup();
                Ok((m.into_iter().collect(), crossing))
            })
        },
    )?;
    let mut size: HashMap<u32, f32> = HashMap::new();
    for (id, q) in sizes.iter().flat_map(|p| p.0.iter()) {
        let e = size.entry(*id).or_insert(0.0);
        *e = e.max(*q);
    }
    // A pond a river crosses is wetted whole at its level as part of the largest river
    // crossing it (smaller id on ties): ruling extending R7/R26; labels only.
    let mut pond_body: HashMap<u32, u32> = HashMap::new();
    for &(o, id) in sizes.iter().flat_map(|p| p.1.iter()) {
        let better = pond_body
            .get(&o)
            .is_none_or(|&b| size[&id] > size[&b] || (size[&id] == size[&b] && id < b));
        if better {
            pond_body.insert(o, id);
        }
    }
    drop(sizes);

    let parts: Vec<Figure> =
        for_tiles(&tiles, s.threads, &format!("{tag} channels"), |i, core| {
            let name = format!("{tag}/ch/{i:05}");
            if let Some(wk) = work {
                if wk.is_done(&name) {
                    return wk.cached(&format!("{name}-figures"), || {
                        Ok(((0, 0.0, 0.0, 0, 0.0), [0; 4]))
                    });
                }
            }
            // Every section reaching the area lies in the window (MAX_RADIUS + BACKWATER = HALO),
            // with its thalweg's stem out to STEM_CELLS and their neighbours (+2), and 2 spare.
            let win = core.grow(HALO + MARGIN - BACKWATER + STEM_CELLS as u64 + 4, world);
            let area = core.grow(MARGIN, world);
            let (g, rw) = (ground.read(win)?, raw.read(win)?);
            let (level, node, flow) = (r.level.read(win)?, r.node.read(win)?, r.flow.read(win)?);
            let (q, bw) = (r.discharge.read(win)?, body.read(win)?);
            let is_lake = |j: usize| is_lake_node(node[j]);
            let thalweg = |j: usize| flow[j] & 16 != 0 && node[j] != 1 && !is_lake(j);
            let radius = |t: usize| {
                if s.params.channels {
                    (geo.width_m(f64::from(q[t])) / 2.0 * cells_per_m).min(MAX_RADIUS)
                } else {
                    0.0
                }
            };
            let routing = |t: usize| if level[t].is_finite() { level[t] } else { g[t] };
            // A thalweg's D8 neighbour `j` on its own stem (a body is one main stem).
            let same_stem = |t: usize, j: usize| thalweg(j) && bw[j] == bw[t];
            let down = |t: usize| {
                let d = flow[t] & 15;
                let (x, y) = win.world_xy(t);
                (d != 0)
                    .then(|| world.neighbour(x, y, d as usize - 1))
                    .flatten()
                    .and_then(|(nx, ny)| win.local(nx, ny))
                    .filter(|&j| same_stem(t, j))
            };
            // Its upstream stem cell: the most discharge, then the smaller index.
            let up = |t: usize| {
                let (x, y) = win.world_xy(t);
                (0..8)
                    .filter_map(|k| {
                        world
                            .neighbour(x, y, k)
                            .and_then(|(nx, ny)| win.local(nx, ny))
                    })
                    .filter(|&j| same_stem(t, j) && down(j) == Some(t))
                    .min_by(|&a, &b| q[b].total_cmp(&q[a]).then(a.cmp(&b)))
            };
            // The river's surface at a dry thalweg: the mean routing level over its stem's dry
            // cells, the same number each way, at most STEM_CELLS (fewer near a pond, lake or
            // the stem's ends, which keep their level). A thalweg following 1 m ground noise
            // steps 0.04–0.9 m per cell; a section taking those steps gives jagged banks. A
            // mean of a level that never rises downstream never rises either.
            let mut stem = vec![f32::NAN; win.cells()];
            for t in 0..win.cells() {
                if !thalweg(t) || level[t].is_finite() {
                    continue;
                }
                let walk = |step: &dyn Fn(usize) -> Option<usize>| {
                    let mut cells = Vec::with_capacity(STEM_CELLS);
                    let mut c = t;
                    while cells.len() < STEM_CELLS {
                        let Some(o) = step(c).filter(|&o| level[o].is_nan()) else {
                            break;
                        };
                        cells.push(o);
                        c = o;
                    }
                    cells
                };
                let (below, above) = (walk(&down), walk(&up));
                let m = below.len().min(above.len());
                let sum: f64 = below[..m]
                    .iter()
                    .chain(&above[..m])
                    .fold(f64::from(routing(t)), |s, &o| s + f64::from(routing(o)));
                let n = (2 * m + 1) as f64;
                // Within STEM_DEV_M of its own level, so a cut within the cap still reaches it
                // below a fall (a clamp of non-rising levels does not rise).
                let r = routing(t);
                stem[t] = ((sum / n) as f32).clamp(r - STEM_DEV_M, r + STEM_DEV_M);
            }
            let river = |t: usize| {
                if stem[t].is_finite() {
                    stem[t]
                } else {
                    routing(t)
                }
            };
            // The centre cut: down to the surface − depth, within the raw − cap clamp.
            let centre = |t: usize| {
                if !s.params.channels {
                    return 0.0;
                }
                let floor = (river(t) - geo.depth_m(f64::from(q[t]))).max(rw[t] - CHANNEL_CAP_M);
                (g[t] - floor).max(0.0)
            };
            // The section of thalweg `t` at cell (x, y), swept along its stem: surface, centre
            // cut and radius interpolated on the nearer of the segments to its upstream and
            // downstream stem cells, and the distance² to that segment.
            let section = |t: usize, x: u64, y: u64| {
                let (tx, ty) = win.world_xy(t);
                let p = (x as f64 - tx as f64, y as f64 - ty as f64);
                let mut best = (river(t), centre(t), radius(t), p.0 * p.0 + p.1 * p.1);
                // Ties (inside bends) go to the downstream segment.
                for o in [down(t), up(t)].into_iter().flatten() {
                    let (ox, oy) = win.world_xy(o);
                    let b = (ox as f64 - tx as f64, oy as f64 - ty as f64);
                    let (f, d2) = project(p, b);
                    if d2 < best.3 {
                        let lerp = |a: f64, c: f64| a + (c - a) * f;
                        best = (
                            lerp(f64::from(river(t)), f64::from(river(o))) as f32,
                            lerp(f64::from(centre(t)), f64::from(centre(o))) as f32,
                            lerp(radius(t), radius(o)),
                            d2,
                        );
                    }
                }
                best
            };

            // The two best thalwegs of distinct rivers for every area cell: (−body size,
            // distance², window index), least first.
            let none = (0.0, 0, u32::MAX);
            let mut best: Vec<[(f32, u32, u32); 2]> = vec![[none; 2]; area.cells()];
            for (t, id) in bw.iter().enumerate() {
                if !thalweg(t) {
                    continue;
                }
                let qb = size.get(id).copied().ok_or_else(|| {
                    crate::acquisition::AcquisitionError::Invalid(format!(
                        "river body {id} has no size"
                    ))
                })?;
                let (tx, ty) = win.world_xy(t);
                let rad = radius(t);
                let reach = rad.floor() as i64;
                for dy in -reach..=reach {
                    for dx in -reach..=reach {
                        let d2 = (dx * dx + dy * dy) as u32;
                        if f64::from(d2) > rad * rad {
                            continue;
                        }
                        let (Some(x), Some(y)) =
                            (tx.checked_add_signed(dx), ty.checked_add_signed(dy))
                        else {
                            continue;
                        };
                        let Some(a) = area.local(x, y) else { continue };
                        offer(&mut best[a], (-qb, d2, t as u32), |c| bw[c as usize]);
                    }
                }
            }

            let n = area.cells();
            let (mut carved, mut surface, mut ids) =
                (vec![0f32; n], vec![f32::NAN; n], vec![0u32; n]);
            let (mut lowered, mut is_river) = (vec![false; n], vec![false; n]);
            // River cells that stay wet whatever their cut: own thalwegs and wetted ponds.
            let mut keep = vec![false; n];
            let (mut fixed, mut returned) = (vec![WetFix::None; n], vec![false; n]);
            for a in 0..n {
                let (x, y) = area.world_xy(a);
                let j = win.local(x, y).expect("area in window");
                carved[a] = g[j];
                ids[a] = bw[j];
                if node[j] == 1 || is_lake(j) {
                    // Ground at or above the level is dry shore.
                    if g[j] < level[j] {
                        surface[a] = level[j];
                    } else {
                        (ids[a], fixed[a]) = (0, WetFix::Shore);
                    }
                    continue;
                }
                let ranked: Vec<(usize, u32)> = best[a]
                    .iter()
                    .filter(|c| c.2 != u32::MAX)
                    .map(|c| (c.2 as usize, c.1))
                    .collect();
                let own = (thalweg(j) && ranked.iter().all(|c| c.0 != j)).then_some((j, 0));
                let around: Vec<usize> = (0..8)
                    .filter_map(|k| {
                        world
                            .neighbour(x, y, k)
                            .and_then(|(nx, ny)| win.local(nx, ny))
                    })
                    .collect();
                // Rims beside other bodies are cut like any section cell (F1): a lake whose
                // outlet this lowers is re-levelled on the carved ground (`relevel`).
                // The lowest neighbouring lake level below a river surface `rs`.
                let lake_below = |rs: f32| {
                    around
                        .iter()
                        .filter(|&&j2| is_lake(j2) && level[j2] < rs)
                        .map(|&j2| level[j2])
                        .reduce(f32::min)
                };
                for (t, _) in ranked.into_iter().chain(own) {
                    let (rs, mid, rad, d2) = section(t, x, y);
                    let taper = if d2 == 0.0 {
                        1.0
                    } else if rad > 0.0 {
                        (1.0 - d2 / (rad * rad)).max(0.0) as f32
                    } else {
                        0.0
                    };
                    let cut = (g[j] - mid * taper).max(rw[j] - CHANNEL_CAP_M).min(g[j]);
                    if cut < rs || t == j {
                        lowered[a] = cut < g[j];
                        carved[a] = cut;
                        surface[a] = mouth_surface(rs, cut, lake_below(rs));
                        ids[a] = bw[t];
                        is_river[a] = true;
                        keep[a] = t == j;
                        break;
                    }
                }
                // A full pond a river crosses: wet at its level (or a section's higher
                // surface) as one river, like an R26 pool.
                if let Some(&id) = (node[j] >= 2)
                    .then(|| pond_body.get(&(node[j] - 2)))
                    .flatten()
                {
                    surface[a] = if surface[a].is_finite() {
                        surface[a].max(level[j])
                    } else {
                        level[j]
                    };
                    ids[a] = id;
                    is_river[a] = true;
                    keep[a] = true;
                }
            }
            let in_area = |a: usize| {
                let (x, y) = area.world_xy(a);
                win.local(x, y).expect("area in window")
            };
            let ga: Vec<f32> = (0..n).map(|a| g[in_area(a)]).collect();
            for a in 0..n {
                lowered[a] = carved[a] < ga[a];
                // A section cell its smaller cut leaves above the river's surface is dry again;
                // an own m2 thalweg returns to its own river.
                if is_river[a] && !keep[a] && carved[a] >= surface[a] {
                    let j = in_area(a);
                    (surface[a], ids[a], is_river[a], keep[a]) =
                        dry_again(thalweg(j).then(|| (river(j), bw[j])));
                    returned[a] = keep[a];
                }
            }
            // Every river cell stands above its bed (the backwater pass and settle absorb the
            // rises this makes, raising water only).
            for a in 0..n {
                if !is_river[a] || surface[a].is_nan() || surface[a] > carved[a] {
                    continue;
                }
                let (x, y) = area.world_xy(a);
                let sill = (0..8)
                    .filter_map(|k| {
                        world
                            .neighbour(x, y, k)
                            .and_then(|(nx, ny)| win.local(nx, ny))
                    })
                    .any(|j2| is_lake(j2) && level[j2] >= carved[a]);
                match above_bed(surface[a], carved[a], sill) {
                    Some(v) => (surface[a], fixed[a]) = (v, WetFix::MinDepth),
                    None => {
                        (surface[a], ids[a], is_river[a]) = (f32::NAN, 0, false);
                        fixed[a] = WetFix::Exit;
                    }
                }
            }
            // Backwater (N18): a river cell whose surface lies below that of the cell it flows
            // into is raised to it, and a dry cell draining into a river surface above its
            // ground is wetted to it as that river, BACKWATER Jacobi steps upstream (water
            // deepens; ground is not touched). Every core cell's chain lies in `area`, so
            // every tiling agrees.
            // (cell, receiver) in `area`, as u32 pairs to keep 4096² tiles small.
            let links: Vec<(u32, u32)> = (0..n)
                .filter(|&a| is_river[a] || !surface[a].is_finite())
                .filter_map(|a| {
                    let (x, y) = area.world_xy(a);
                    let d = flow[win.local(x, y).expect("area in window")] & 15;
                    (d != 0)
                        .then(|| world.neighbour(x, y, d as usize - 1))
                        .flatten()
                        .and_then(|(nx, ny)| area.local(nx, ny))
                        .map(|b| (a as u32, b as u32))
                })
                .collect();
            for _ in 0..BACKWATER {
                // Read every value before writing any (Jacobi).
                let change: Vec<(usize, f32, u32)> = links
                    .iter()
                    .map(|&(a, b)| (a as usize, b as usize))
                    .filter(|&(a, b)| {
                        is_river[b]
                            && if is_river[a] {
                                surface[b] > surface[a]
                            } else {
                                surface[b] > carved[a]
                            }
                    })
                    .map(|(a, b)| (a, surface[b], ids[b]))
                    .collect();
                if change.is_empty() {
                    break;
                }
                for (a, v, id) in change {
                    if !is_river[a] {
                        ids[a] = id;
                        is_river[a] = true;
                    }
                    surface[a] = v;
                }
            }
            let at = |k: usize| {
                let (x, y) = core.world_xy(k);
                area.local(x, y).expect("core in area")
            };
            let cc: Vec<f32> = (0..core.cells()).map(|k| carved[at(k)]).collect();
            let cs: Vec<f32> = (0..core.cells()).map(|k| surface[at(k)]).collect();
            let ci: Vec<u32> = (0..core.cells()).map(|k| ids[at(k)]).collect();
            out.carved.write(core, &cc)?;
            out.surface.write(core, &cs)?;
            out.body.write(core, &ci)?;
            let (mut fig, mut fixes): Figure = ((0, 0.0, 0.0, 0, 0.0), [0; 4]);
            for k in 0..core.cells() {
                let (x, y) = core.world_xy(k);
                let j = win.local(x, y).expect("core in window");
                let a = at(k);
                if fixed[a] != WetFix::None {
                    fixes[fixed[a] as usize - 1] += 1;
                }
                fixes[3] += u64::from(returned[a]);
                let drop = g[j] - cc[k];
                if drop > 0.0 {
                    fig.0 += 1;
                    fig.1 = fig.1.max(drop);
                    fig.2 += f64::from(drop) * s.spacing_m * s.spacing_m;
                    fig.4 = fig.4.max(rw[j] - cc[k]);
                }
                if cs[k].is_finite() && node[j] != 1 && !is_lake(j) {
                    fig.3 += 1;
                }
            }
            let fig = (fig, fixes);
            if let Some(wk) = work {
                wk.cached(&format!("{name}-figures"), || Ok(fig))?;
                wk.mark_done(&name)?;
            }
            Ok(fig)
        })?;
    let fixes = |i: usize| parts.iter().map(|p| p.1[i]).sum::<u64>();
    let parts: Vec<_> = parts.iter().map(|p| p.0).collect();
    Ok(json!({
        "lowered_cells": parts.iter().map(|p| p.0).sum::<u64>(),
        "max_lowering_m": parts.iter().map(|p| p.1).fold(0.0f32, f32::max),
        "max_below_raw_m": parts.iter().map(|p| p.4).fold(0.0f32, f32::max),
        "volume_m3": parts.iter().map(|p| p.2).sum::<f64>().round(),
        "river_cells": parts.iter().map(|p| p.3).sum::<u64>(),
        // Export ruling: river cells raised to MIN_DEPTH_M over their bed, lake and sea shore
        // and lake exit cells left dry, thalwegs returned to their own river.
        "min_depth_cells": fixes(0), "shore_dry_cells": fixes(1),
        "exit_dry_cells": fixes(2), "thalweg_returned_cells": fixes(3),
        "width": "3 * Q^0.5 m, Q in m³/s", "depth": "min(0.3 * Q^0.4, 2) m",
        "m3s_per_km2": geo.m3s_per_km2,
    }))
}

/// What drains at a cell, with its label: written water at its surface (its body), a pond at
/// the final routing's level, else ground (no label).
type Drained = (f32, u32);
/// The highest drained value on a cell's path downstream inside its tile (with its label),
/// and the cell the path enters outside the tile (NO_EXIT when it ends inside).
type PathMax = (f32, u32, u64);
const NO_EXIT: u64 = u64::MAX;
/// Settle's figures: cells raised, of them wetted, the largest lift (m), its world cell
/// (u64::MAX for none) and body.
pub(crate) type Settled = (u64, u64, f32, u64, u32);
/// A tile's settle figures and its changed cells (core index, surface, body).
pub(crate) type SettledTile = (Settled, Vec<(u64, f32, u32)>);

/// The larger lift of two; ties keep the smaller cell (exact under any tile order).
fn more_lift(a: Settled, b: Settled) -> Settled {
    let top = if b.2 > a.2 || (b.2 == a.2 && b.3 < a.3) {
        b
    } else {
        a
    };
    (a.0 + b.0, a.1 + b.1, top.2, top.3, top.4)
}

/// The higher of a cell's own value and its path downstream; ties keep the cell's own.
fn downstream(own: Drained, down: PathMax) -> PathMax {
    if down.0 > own.0 {
        down
    } else {
        (own.0, own.1, down.2)
    }
}

/// Each core cell's PathMax along the final routing's receivers (`flow`, core-sized).
fn path_max(world: World, core: Rect, flow: &[u8], d0: &[Drained]) -> Result<Vec<PathMax>> {
    let n = core.cells();
    let mut res: Vec<Option<PathMax>> = vec![None; n];
    let mut stack = Vec::new();
    for k in 0..n {
        let mut c = k;
        // Down to a resolved cell or the end of the path in this tile.
        let below = loop {
            if let Some(r) = res[c] {
                break r;
            }
            ensure(stack.len() <= n, "a flow cycle in the final routing")?;
            let d = flow[c] & 15;
            let (x, y) = core.world_xy(c);
            match (d != 0)
                .then(|| world.neighbour(x, y, d as usize - 1))
                .flatten()
            {
                None => {
                    res[c] = Some((d0[c].0, d0[c].1, NO_EXIT));
                    break res[c].expect("set");
                }
                Some((nx, ny)) => match core.local(nx, ny) {
                    Some(next) => {
                        stack.push(c);
                        c = next;
                    }
                    None => {
                        res[c] = Some((d0[c].0, d0[c].1, world.index(nx, ny)));
                        break res[c].expect("set");
                    }
                },
            }
        };
        let mut below = below;
        while let Some(c) = stack.pop() {
            below = downstream(d0[c], below);
            res[c] = Some(below);
        }
    }
    Ok(res.into_iter().map(|r| r.expect("resolved")).collect())
}

/// What drains along the final routing (`r3`): each cell's drained value (`d0`) and the
/// whole-path maxima of the tile ring cells across tiles, from which any tile's cells get
/// theirs (`top`). Labelled lakes (`lakes`) drain at their written surfaces.
struct Paths<'a> {
    s: &'a Setup,
    r3: &'a Rasters,
    ground: &'a Layer<f32>,
    out: &'a Layers,
    lakes: &'a HashSet<u32>,
    full: HashMap<u64, Drained>,
}

impl<'a> Paths<'a> {
    fn new(
        s: &'a Setup,
        r3: &'a Rasters,
        ground: &'a Layer<f32>,
        out: &'a Layers,
        lakes: &'a HashSet<u32>,
        work: Option<&Work>,
        tag: &str,
    ) -> Result<Self> {
        let world = s.world;
        let mut p = Self {
            s,
            r3,
            ground,
            out,
            lakes,
            full: HashMap::new(),
        };
        // 1: the ring cells' path maxima (every path entering a tile enters at its ring).
        let rings: Vec<Vec<(u64, PathMax)>> = for_tiles(
            &s.tiles(),
            s.threads,
            &format!("{tag} settle 1"),
            |i, core| {
                cached(work, &format!("{tag}/s1/{i:05}"), || {
                    let (flow, d0) = p.d0(core)?;
                    let pm = path_max(world, core, &flow, &d0)?;
                    Ok((0..core.cells())
                        .filter_map(|k| {
                            let (x, y) = core.world_xy(k);
                            core.on_ring(x, y).then(|| (world.index(x, y), pm[k]))
                        })
                        .collect())
                })
            },
        )?;
        let ring: HashMap<u64, PathMax> = rings.into_iter().flatten().collect();
        // The whole path's maximum from a ring cell, across tiles (memoized).
        let full = &mut p.full;
        for &start in ring.keys() {
            let mut chain = vec![start];
            let mut tail = loop {
                let c = *chain.last().expect("start");
                if let Some(&f) = full.get(&c) {
                    chain.pop();
                    break f;
                }
                let pm = *ring.get(&c).ok_or_else(|| {
                    crate::acquisition::AcquisitionError::Invalid(format!(
                        "the path from ring cell {c} enters no tile ring"
                    ))
                })?;
                if pm.2 == NO_EXIT {
                    chain.pop();
                    full.insert(c, (pm.0, pm.1));
                    break (pm.0, pm.1);
                }
                ensure(chain.len() <= ring.len(), "a flow cycle across tiles")?;
                chain.push(pm.2);
            };
            while let Some(c) = chain.pop() {
                let pm = ring[&c];
                tail = if tail.0 > pm.0 { tail } else { (pm.0, pm.1) };
                full.insert(c, tail);
            }
        }
        Ok(p)
    }

    /// A tile's receivers and drained values. A wet river or pool cell inside a higher pond
    /// stands at the pond's level (bank pockets, scale diagnosis 2); a labelled lake at its
    /// surface.
    fn d0(&self, core: Rect) -> Result<(Vec<u8>, Vec<Drained>)> {
        let (sf, b) = (self.out.surface.read(core)?, self.out.body.read(core)?);
        let (lv, z) = (self.r3.level.read(core)?, self.ground.read(core)?);
        let d0 = (0..core.cells())
            .map(|k| {
                if sf[k].is_finite() && self.lakes.contains(&b[k]) {
                    (sf[k], b[k])
                } else if sf[k].is_finite() {
                    (sf[k].max(lv[k]), b[k])
                } else if lv[k].is_finite() {
                    (lv[k], 0)
                } else {
                    (z[k], 0)
                }
            })
            .collect();
        Ok((self.r3.flow.read(core)?, d0))
    }

    /// Each core cell's whole-path maximum (its own value included) with its label.
    fn top(&self, core: Rect, flow: &[u8], d0: &[Drained]) -> Result<Vec<Drained>> {
        let pm = path_max(self.s.world, core, flow, d0)?;
        pm.into_iter()
            .map(|(v, id, exit)| match exit {
                NO_EXIT => Ok((v, id)),
                e => {
                    let f = self.full.get(&e).copied().ok_or_else(|| {
                        crate::acquisition::AcquisitionError::Invalid(format!(
                            "cell {e} is on no tile ring"
                        ))
                    })?;
                    Ok(if f.0 > v { f } else { (v, id) })
                }
            })
            .collect()
    }
}

/// Settles the written water on the final routing (review: drainage_rises): what drains at a
/// cell never lies below what its receiver (`r3`, the routing on the carved ground) drains
/// at. A wet cell inside a higher pond of that routing stands at the pond's level. Each cell
/// takes the highest labelled water on its path downstream when that lies above it: a dry or
/// pond cell is wetted as that water's body, a wet one raised. Labels and
/// surfaces only. Exact over tiles: per-tile path maxima, then the paths across tile rings.
/// Labelled lakes (`lakes`, body ids) keep their level and extent (gate 1 ruling: labels
/// from m3 would be circular; `relevel` has set them on the carved ground).
pub(crate) fn settle(
    s: &Setup,
    r3: &Rasters,
    ground: &Layer<f32>,
    out: &Layers,
    lakes: &HashSet<u32>,
    work: Option<&Work>,
    tag: &str,
) -> Result<Settled> {
    let world = s.world;
    let tiles = s.tiles();
    let paths = Paths::new(s, r3, ground, out, lakes, work, tag)?;
    // 2: every cell's whole-path maximum; raise where it lies above the cell.
    let none: Settled = (0, 0, 0.0, u64::MAX, 0);
    let settle_tile = |core: Rect| -> Result<SettledTile> {
        let (flow, d0) = paths.d0(core)?;
        let tops = paths.top(core, &flow, &d0)?;
        let (sf, b) = (out.surface.read(core)?, out.body.read(core)?);
        let (mut fig, mut changed) = (none, Vec::new());
        for k in 0..core.cells() {
            // Labelled lakes keep their surface and body.
            if sf[k].is_finite() && lakes.contains(&b[k]) {
                continue;
            }
            let top = tops[k];
            let (was, mut v) = (sf[k], (sf[k], b[k]));
            // A wet cell keeps its body at its pond's level (d0).
            if was.is_finite() {
                v.0 = d0[k].0;
            }
            // Only labelled water wets a cell (a pond or dry source has no body). A wet cell a
            // labelled lake raises keeps its body: a lake grows over dry cells only, so river
            // water it holds up stays river and goes down with it (`flood`).
            if top.0 > d0[k].0 && top.1 >= 1 {
                v = if was.is_finite() && lakes.contains(&top.1) {
                    (top.0, b[k])
                } else {
                    top
                };
            }
            if v.0 != was && !(v.0.is_nan() && was.is_nan()) {
                let lift = v.0 - if was.is_finite() { was } else { d0[k].0 };
                let (x, y) = core.world_xy(k);
                let cell = (1, u64::from(!was.is_finite()), lift, world.index(x, y), v.1);
                fig = more_lift(fig, cell);
                changed.push((k as u64, v.0, v.1));
            }
        }
        Ok((fig, changed))
    };
    let parts: Vec<Settled> =
        for_tiles(&tiles, s.threads, &format!("{tag} settle 2"), |i, core| {
            let name = format!("{tag}/s2/{i:05}");
            let changes = format!("{name}-changes");
            if let Some(wk) = work {
                if wk.is_done(&name) {
                    let done: SettledTile = wk.cached(&changes, || Ok((none, Vec::new())))?;
                    return Ok(done.0);
                }
            }
            // Settle rewrites the channels' rasters in place: its changes are cached before
            // either raster is written, so a kill between the two writes resumes by rewriting
            // both from the cache, never from half-settled rasters.
            let (fig, changed) = cached(work, &changes, || settle_tile(core))?;
            if !changed.is_empty() {
                let (mut sf, mut b) = (out.surface.read(core)?, out.body.read(core)?);
                for &(k, v, id) in &changed {
                    (sf[k as usize], b[k as usize]) = (v, id);
                }
                out.surface.write(core, &sf)?;
                out.body.write(core, &b)?;
            }
            if let Some(wk) = work {
                wk.mark_done(&name)?;
            }
            Ok(fig)
        })?;
    Ok(parts.into_iter().fold(none, more_lift))
}

/// A labelled lake: its m2 level and, when full, its spill edge [lake cell, outside cell].
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Label {
    pub level: f32,
    pub outlet: Option<[u64; 2]>,
}

/// What `relevel` did to a labelled lake whose label stood above its spill (outlet incision, F1).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Relevelled {
    pub id: u32,
    /// The label's level.
    pub from: f32,
    /// The new level, its spill; NaN when the lake drained (no cell left below its spill).
    pub to: f32,
    /// The spill edge [lake cell, outside cell] (u64::MAX when drained).
    pub outlet: [u64; 2],
    /// The m3 node (layer value) the lake is confined to when its bulk stayed behind a sill
    /// (`bulk_pocket`); 0 otherwise.
    pub basin: u32,
    /// Lake cells left at the new level, cells left dry, and river cells lowered with it
    /// (`flood`).
    pub cells: u64,
    pub dried: u64,
    pub flooded: u64,
}

impl Blob for Relevelled {
    fn put(&self, e: &mut Enc) {
        (self.id, self.from, self.to, self.outlet, self.basin).put(e);
        (self.cells, self.dried, self.flooded).put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let (id, from, to, outlet, basin) = Blob::get(d)?;
        let (cells, dried, flooded) = Blob::get(d)?;
        Ok(Self {
            id,
            from,
            to,
            outlet,
            basin,
            cells,
            dried,
            flooded,
        })
    }
}

/// Relevel passes before the loop stops unconverged (one more settle then leaves the water
/// settled; `stages.relevelled.converged` is false).
pub(crate) const MAX_PASSES: u64 = 16;
/// Longest outflow path `outflow` lowers (cells); its window is this wide around the outlet.
const OUTFLOW_CELLS: u64 = 64;
/// `relevel`'s figures beyond the lakes: settle's over all passes, then (passes run, whether
/// the last pass planned nothing, outflow cells lowered).
pub(crate) type Levelled = (Settled, (u64, bool, u64));
/// Cell changes by world index: (cell, surface, body).
type Changes = Vec<(u64, f32, u32)>;

/// A lake's spill edge: (value, lake cell, outside cell).
type Edge = (f32, u64, u64);
/// Per tile and lake: its lowest spill edge and its cells below its level.
type LakeEdges = Vec<(u32, Option<Edge>, u64)>;
/// Per tile: lake cells by (id, m3 node layer value, count).
type LakeNodes = Vec<(u32, u32, u64)>;
/// Per tile: changed cells (core index, surface, body) and (id, kept, dried, flooded) per lake.
type Relevel2 = (Vec<(u64, f32, u32)>, Vec<(u32, u64, u64, u64)>);

/// The lower of two spill edges; ties keep the smaller cells (exact under any tile order).
fn lower_edge(a: Option<Edge>, b: Edge) -> Option<Edge> {
    match a {
        Some(a) if (a.0, a.1, a.2) <= (b.0, b.1, b.2) => Some(a),
        _ => Some(b),
    }
}

/// Outlet incision (F1): channels cut lake rims like any other ground, so a labelled lake
/// (`labels`: body id → label level) can spill lower than its label: into a river whose water
/// drains below it, or over a rim cut below it. Its spill is the lowest edge, from a lake cell
/// j (ground below the level) to a cell n outside it, of max(z(j), D(n)), where D(n) is what
/// n drains at once settled on the routing on the carved ground (`r3`; `settle`'s rule: its
/// own drained value, raised to the highest labelled water on its path). A lake whose spill
/// lies more than SILL_M below its label drops to it, again and again (its cells at or above
/// the level are dry, so its edge moves) until it stands at its spill; one left with no cell
/// below its spill drains. Its own outflow river, whose water stands at the outlet's height
/// over its cut bed, keeps it at its label. A lake whose bulk stays behind a sill above its
/// spill stands at that basin's own spill instead (`bulk_pocket`). Rounds repeat on the rewritten surfaces until no
/// lake moves (a lake draining through another one).
///
/// First a full lake inside a full, higher m3 basin takes that basin's level (`basins`). Then
/// each pass settles the water (`settle`), lowers lakes' outflows standing above them
/// (`outflow`) and plans on the result; a pass that plans nothing ends the loop, so the water
/// left is settled and every lake stands at its spill on it (diagnosis A: settle wets cut
/// banks as the lake, which the next pass sees). At most MAX_PASSES passes. Surfaces and
/// labels only, never the ground; every change is cached before it is written, so a killed run
/// resumes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn relevel(
    s: &Setup,
    r3: &Rasters,
    n3: &Nodes,
    carved: &Layer<f32>,
    out: &Layers,
    labels: &BTreeMap<u32, Label>,
    work: Option<&Work>,
    tag: &str,
) -> Result<(Vec<Relevelled>, Levelled)> {
    let lakes: HashSet<u32> = labels.keys().copied().collect();
    let mut moved: BTreeMap<u32, Relevelled> = BTreeMap::new();
    for r in basins(
        s,
        r3,
        n3,
        carved,
        out,
        labels,
        work,
        &format!("{tag}/basins"),
    )? {
        moved.insert(r.id, r);
    }
    let mut settled: Settled = (0, 0, 0.0, u64::MAX, 0);
    let (mut passes, mut converged, mut lowered) = (0, false, 0);
    for pass in 0..=MAX_PASSES {
        let tag = format!("{tag}/rv{pass}");
        let fig = settle(s, r3, carved, out, &lakes, work, &format!("{tag}/st"))?;
        settled = more_lift(settled, fig);
        // Each lake's current level and spill edge.
        let exits: BTreeMap<u32, (f32, [u64; 2])> = labels
            .iter()
            .filter_map(|(&id, l)| match moved.get(&id) {
                // Drained and closed lakes have no spill edge.
                Some(r) => {
                    (r.to.is_finite() && r.outlet[1] != u64::MAX).then_some((id, (r.to, r.outlet)))
                }
                None => l.outlet.map(|o| (id, (l.level, o))),
            })
            .collect();
        lowered += outflow(
            s,
            r3,
            carved,
            out,
            &lakes,
            &exits,
            work,
            &format!("{tag}/outflow"),
        )?;
        passes = pass + 1;
        if pass == MAX_PASSES {
            break;
        }
        let plan: Vec<Relevelled> = cached(work, &format!("{tag}/plan"), || {
            let paths = Paths::new(s, r3, carved, out, &lakes, work, &tag)?;
            plan_relevel(s, &paths, n3, carved, out, &lakes, &tag)
        })?;
        if plan.is_empty() {
            converged = true;
            break;
        }
        let counts = write_relevel(s, r3, carved, out, &lakes, &plan, work, &tag)?;
        for (r, (kept, dried, flooded)) in plan.iter().zip(counts) {
            let m = moved.entry(r.id).or_insert(Relevelled {
                from: labels[&r.id].level,
                ..*r
            });
            (m.to, m.outlet, m.basin, m.cells) = (r.to, r.outlet, r.basin, kept);
            (m.dried, m.flooded) = (m.dried + dried, m.flooded + flooded);
        }
    }
    let moved = moved.into_values().collect();
    Ok((moved, (settled, (passes, converged, lowered))))
}

/// Writes cell changes (world index, surface, body), a tile at a time.
fn apply(s: &Setup, out: &Layers, changes: &[(u64, f32, u32)]) -> Result<()> {
    let (world, tiles) = (s.world, s.tiles());
    let mut by_tile: BTreeMap<usize, Vec<(u64, f32, u32)>> = BTreeMap::new();
    for &c in changes {
        let (x, y) = world.xy(c.0);
        by_tile
            .entry(tile_of(world, s.tile, x, y))
            .or_default()
            .push(c);
    }
    for (i, cells) in by_tile {
        let core = tiles[i];
        let (mut sf, mut b) = (out.surface.read(core)?, out.body.read(core)?);
        for (c, v, id) in cells {
            let (x, y) = world.xy(c);
            let k = core.local(x, y).expect("cell in its tile");
            (sf[k], b[k]) = (v, id);
        }
        out.surface.write(core, &sf)?;
        out.body.write(core, &b)?;
    }
    Ok(())
}

/// Applies changes cached under `name` (computed by `f` when missing) once: a done marker
/// keeps a resumed run from writing them over later changes.
fn apply_once<T: Blob>(
    s: &Setup,
    out: &Layers,
    work: Option<&Work>,
    name: &str,
    f: impl FnOnce() -> Result<T>,
    changes: impl Fn(&T) -> &Changes,
) -> Result<T> {
    let v = cached(work, &format!("{name}/changes"), f)?;
    if !work.is_some_and(|wk| wk.is_done(name)) {
        apply(s, out, changes(&v))?;
        if let Some(wk) = work {
            wk.mark_done(name)?;
        }
    }
    Ok(v)
}

/// Ruling on diagnosis C (gate 1 for full lakes): a full labelled lake whose cells all lie in
/// one wet node of the routing on the carved ground (`n3`, layer value N) standing higher
/// than its label, with no other labelled lake in it, takes that node's level and extent (the
/// node's cells below the level). A full node gives its spill edge; one that is not full
/// (lakes 103, 1183: the merge level of full children, endorheic in m3) leaves the lake closed,
/// outlet [u64::MAX; 2], as m3 has it. Surfaces and labels only.
#[allow(clippy::too_many_arguments)]
fn basins(
    s: &Setup,
    r3: &Rasters,
    n3: &Nodes,
    carved: &Layer<f32>,
    out: &Layers,
    labels: &BTreeMap<u32, Label>,
    work: Option<&Work>,
    name: &str,
) -> Result<Vec<Relevelled>> {
    let (world, tiles) = (s.world, s.tiles());
    let compute = || -> Result<(Vec<Relevelled>, Changes)> {
        // Lake cells by (lake, node layer value).
        let parts: Vec<Vec<(u32, u32, u64)>> =
            for_tiles(&tiles, s.threads, "relevel basins", |_, core| {
                let (sf, b, nd) = (
                    out.surface.read(core)?,
                    out.body.read(core)?,
                    r3.node.read(core)?,
                );
                let mut m: BTreeMap<(u32, u32), u64> = BTreeMap::new();
                for k in 0..core.cells() {
                    if sf[k].is_finite() && labels.contains_key(&b[k]) {
                        *m.entry((b[k], nd[k])).or_default() += 1;
                    }
                }
                Ok(m.into_iter().map(|((id, n), c)| (id, n, c)).collect())
            })?;
        let mut nodes_of: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
        let mut lakes_in: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
        for &(id, n, _) in parts.iter().flatten() {
            nodes_of.entry(id).or_default().insert(n);
            lakes_in.entry(n).or_default().insert(id);
        }
        // Node layer value → lake.
        let take: BTreeMap<u32, u32> = nodes_of
            .iter()
            .filter_map(|(&id, ns)| {
                let &n = ns.iter().next().filter(|_| ns.len() == 1)?;
                let nd = (n >= 2).then(|| n3.at(n - 2))?;
                let l = labels[&id];
                (l.outlet.is_some() && nd.level > l.level && lakes_in[&n].len() == 1)
                    .then_some((n, id))
            })
            .collect();
        if take.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let parts: Vec<(Changes, Vec<(u32, u64)>)> =
            for_tiles(&tiles, s.threads, "relevel basins write", |_, core| {
                let nd = r3.node.read(core)?;
                let (sf, b, z) = (
                    out.surface.read(core)?,
                    out.body.read(core)?,
                    carved.read(core)?,
                );
                let (mut changed, mut cells) = (Vec::new(), BTreeMap::new());
                for k in 0..core.cells() {
                    let Some(&id) = take.get(&nd[k]) else {
                        continue;
                    };
                    let lv = n3.at(nd[k] - 2).level;
                    let (x, y) = core.world_xy(k);
                    if z[k] < lv {
                        *cells.entry(id).or_default() += 1;
                        if sf[k] != lv || b[k] != id {
                            changed.push((world.index(x, y), lv, id));
                        }
                    } else if sf[k].is_finite() && b[k] == id {
                        changed.push((world.index(x, y), f32::NAN, 0));
                    }
                }
                Ok((changed, cells.into_iter().collect()))
            })?;
        let mut cells: BTreeMap<u32, u64> = BTreeMap::new();
        for &(id, c) in parts.iter().flat_map(|p| p.1.iter()) {
            *cells.entry(id).or_default() += c;
        }
        let records = take
            .iter()
            .map(|(&n, &id)| {
                let nd = n3.at(n - 2);
                Relevelled {
                    id,
                    from: labels[&id].level,
                    to: nd.level,
                    outlet: if nd.full { nd.outlet } else { [u64::MAX; 2] },
                    basin: n,
                    cells: cells.get(&id).copied().unwrap_or(0),
                    ..Default::default()
                }
            })
            .collect();
        Ok((records, parts.into_iter().flat_map(|p| p.0).collect()))
    };
    Ok(apply_once(s, out, work, name, compute, |v| &v.1)?.0)
}

/// Diagnosis D: a lake's outflow does not stand above the lake. From each cell of its spill
/// edge (`exits`: level and edge per lake), the path along the final routing (`r3`) through
/// river water standing above the lake, over beds at least MIN_DEPTH_M below it and in no
/// higher m3 pond, to a cell that drains at or below the lake (or into it) within
/// OUTFLOW_CELLS is lowered to the lake's level: water settle carried up to the outlet from a
/// stale section surface downstream. A path meeting anything else stays. The water left is
/// settled (each lowered cell drains at the lake's level). Returns the cells lowered.
#[allow(clippy::too_many_arguments)]
fn outflow(
    s: &Setup,
    r3: &Rasters,
    carved: &Layer<f32>,
    out: &Layers,
    lakes: &HashSet<u32>,
    exits: &BTreeMap<u32, (f32, [u64; 2])>,
    work: Option<&Work>,
    name: &str,
) -> Result<u64> {
    let world = s.world;
    let compute = || -> Result<Changes> {
        let mut low: BTreeMap<u64, (f32, u32)> = BTreeMap::new();
        for (&id, &(lv, edge)) in exits {
            let (ox, oy) = world.xy(edge[1]);
            let one = Rect {
                x0: ox,
                y0: oy,
                w: 1,
                h: 1,
            };
            let win = one.grow(OUTFLOW_CELLS + 1, world);
            let (sf, b) = (out.surface.read(win)?, out.body.read(win)?);
            let (z, lv3, flow) = (carved.read(win)?, r3.level.read(win)?, r3.flow.read(win)?);
            let walk = |start: u64| -> Option<Vec<usize>> {
                let (x, y) = world.xy(start);
                let mut c = win.local(x, y)?;
                let mut path = Vec::new();
                loop {
                    let wet = sf[c].is_finite();
                    if wet && b[c] == id {
                        return Some(path);
                    }
                    // What `settle` values the cell at.
                    let d = if wet && lakes.contains(&b[c]) {
                        sf[c]
                    } else if wet {
                        sf[c].max(lv3[c])
                    } else if lv3[c].is_finite() {
                        lv3[c]
                    } else {
                        z[c]
                    };
                    if d <= lv {
                        return Some(path);
                    }
                    let river = wet && b[c] > 1 && !lakes.contains(&b[c]);
                    if !river || lv3[c] > lv || z[c] + MIN_DEPTH_M > lv {
                        return None;
                    }
                    path.push(c);
                    let f = flow[c] & 15;
                    if f == 0 || path.len() as u64 > OUTFLOW_CELLS {
                        return None;
                    }
                    let (x, y) = win.world_xy(c);
                    let (nx, ny) = world.neighbour(x, y, f as usize - 1)?;
                    c = win.local(nx, ny)?;
                }
            };
            for c in edge.iter().filter_map(|&e| walk(e)).flatten() {
                let (x, y) = win.world_xy(c);
                let e = low.entry(world.index(x, y)).or_insert((lv, b[c]));
                e.0 = e.0.min(lv);
            }
        }
        Ok(low.into_iter().map(|(c, (v, id))| (c, v, id)).collect())
    };
    Ok(apply_once(s, out, work, name, compute, |v| v)?.len() as u64)
}

/// One pass of `relevel`: the lakes that move, from their written surfaces (`from`), with
/// their new level and spill edge.
fn plan_relevel(
    s: &Setup,
    paths: &Paths,
    n3: &Nodes,
    carved: &Layer<f32>,
    out: &Layers,
    lakes: &HashSet<u32>,
    tag: &str,
) -> Result<Vec<Relevelled>> {
    let world = s.world;
    let tiles = s.tiles();
    // The lakes in each tile, and their written level.
    let held: Vec<Vec<(u32, f32)>> = for_tiles(
        &tiles,
        s.threads,
        &format!("{tag} relevel lakes"),
        |_, core| {
            let (sf, b) = (out.surface.read(core)?, out.body.read(core)?);
            let mut ids: Vec<(u32, f32)> = (0..core.cells())
                .filter(|&k| sf[k].is_finite() && lakes.contains(&b[k]))
                .map(|k| (b[k], sf[k]))
                .collect();
            ids.sort_unstable_by_key(|v| v.0);
            ids.dedup_by_key(|v| v.0);
            Ok(ids)
        },
    )?;
    let mut lv: BTreeMap<u32, f32> = held.iter().flatten().copied().collect();
    let from = lv.clone();
    // A tile computes the edges into its cells, so a lake's tiles and their neighbours.
    let per_tile = (world.w.div_ceil(s.tile), world.h.div_ceil(s.tile));
    let near = |i: usize, active: &HashSet<u32>| {
        let (tx, ty) = (i as u64 % per_tile.0, i as u64 / per_tile.0);
        (ty.saturating_sub(1)..=(ty + 1).min(per_tile.1 - 1)).any(|y| {
            (tx.saturating_sub(1)..=(tx + 1).min(per_tile.0 - 1)).any(|x| {
                held[(y * per_tile.0 + x) as usize]
                    .iter()
                    .any(|(id, _)| active.contains(id))
            })
        })
    };
    let mut active: HashSet<u32> = lv.keys().copied().collect();
    let mut plan: BTreeMap<u32, Relevelled> = BTreeMap::new();
    let mut empty: BTreeSet<u32> = BTreeSet::new();
    for round in 0.. {
        ensure(round < 1000, "a lake's spill does not settle")?;
        let busy: Vec<Rect> = (0..tiles.len())
            .filter(|&i| near(i, &active))
            .map(|i| tiles[i])
            .collect();
        if busy.is_empty() {
            break;
        }
        let (lv_now, active_now) = (&lv, &active);
        let parts: Vec<LakeEdges> = for_tiles(
            &busy,
            s.threads,
            &format!("{tag} relevel round {round}"),
            |_, core| {
                let win = core.grow(1, world);
                let (sf, b, z) = (
                    out.surface.read(win)?,
                    out.body.read(win)?,
                    carved.read(win)?,
                );
                let lv3 = paths.r3.level.read(win)?;
                // A cell under an active lake's current level.
                let wet = |j: usize| {
                    sf[j].is_finite()
                        && active_now.contains(&b[j])
                        && under(z[j], lv3[j], lv_now[&b[j]], from[&b[j]], 0, 0)
                };
                let (flow, d0) = paths.d0(core)?;
                let tops = paths.top(core, &flow, &d0)?;
                let mut per: BTreeMap<u32, (Option<Edge>, u64)> = BTreeMap::new();
                for k in 0..core.cells() {
                    let (x, y) = core.world_xy(k);
                    let n = win.local(x, y).expect("core in window");
                    if wet(n) {
                        per.entry(b[n]).or_default().1 += 1;
                    }
                    let dn = drains_at(d0[k], tops[k], None);
                    for d in 0..8 {
                        let Some((jx, jy)) = world.neighbour(x, y, d) else {
                            continue;
                        };
                        let j = win.local(jx, jy).expect("neighbour in window");
                        if !wet(j) || (wet(n) && b[n] == b[j]) {
                            continue;
                        }
                        let edge = (z[j].max(dn), world.index(jx, jy), world.index(x, y));
                        let e = per.entry(b[j]).or_default();
                        e.0 = lower_edge(e.0, edge);
                    }
                }
                Ok(per.into_iter().map(|(id, (e, n))| (id, e, n)).collect())
            },
        )?;
        let mut per: BTreeMap<u32, (Option<Edge>, u64)> = BTreeMap::new();
        for &(id, e, n) in parts.iter().flatten() {
            let p = per.entry(id).or_default();
            p.1 += n;
            if let Some(e) = e {
                p.0 = lower_edge(p.0, e);
            }
        }
        let mut next = HashSet::new();
        for &id in &active {
            let now = lv[&id];
            match per.get(&id).copied().unwrap_or_default() {
                // No cell below the level: drained, unless its bulk stayed in a pocket (below).
                (_, 0) => {
                    plan.entry(id).or_insert_with(|| drained(id, from[&id]));
                    empty.insert(id);
                }
                // Its spill lies below its level (beyond the 1 cm the checks allow on the
                // first round): it drops to it and its edge moves.
                (Some((v, a, b)), _) if v < now && (round > 0 || v < now - SILL_M) => {
                    let r = Relevelled {
                        id,
                        from: from[&id],
                        to: v,
                        outlet: [a, b],
                        ..Default::default()
                    };
                    plan.insert(id, r);
                    lv.insert(id, v);
                    next.insert(id);
                }
                _ => {}
            }
        }
        active = next;
    }
    // Where the lowering left a lake's bulk behind a sill, the lake is that basin.
    let lowered: BTreeMap<u32, (f32, f32)> = plan
        .values()
        .filter(|r| r.to.is_finite())
        .map(|r| (r.id, (r.from, r.to)))
        .collect();
    let parts: Vec<LakeNodes> = for_tiles(
        &tiles,
        s.threads,
        &format!("{tag} relevel bulk"),
        |_, core| {
            let (sf, b, z) = (
                out.surface.read(core)?,
                out.body.read(core)?,
                carved.read(core)?,
            );
            let (lv3, nd) = (paths.r3.level.read(core)?, paths.r3.node.read(core)?);
            let mut m: BTreeMap<(u32, u32), u64> = BTreeMap::new();
            for k in 0..core.cells() {
                let Some(&(from, to)) = lowered.get(&b[k]).filter(|_| sf[k].is_finite()) else {
                    continue;
                };
                if under(z[k], lv3[k], to, from, 0, nd[k]) {
                    *m.entry((b[k], 0)).or_default() += 1;
                } else if lv3[k] > to && lv3[k] < from {
                    *m.entry((b[k], nd[k])).or_default() += 1;
                }
            }
            Ok(m.into_iter().map(|((id, n), c)| (id, n, c)).collect())
        },
    )?;
    let mut held: BTreeMap<u32, BTreeMap<u32, u64>> = BTreeMap::new();
    for &(id, n, c) in parts.iter().flatten() {
        *held.entry(id).or_default().entry(n).or_default() += c;
    }
    for (id, counts) in &held {
        if let Some(basin) = bulk_pocket(counts, n3) {
            let nd = n3.at(basin - 2);
            // Only where the outlet's water allows that level; the lake's own cells and its
            // stale label are about to change, so they do not count.
            let (ox, oy) = world.xy(nd.outlet[1]);
            let core = tiles[tile_of(world, s.tile, ox, oy)];
            let (flow, d0) = paths.d0(core)?;
            let tops = paths.top(core, &flow, &d0)?;
            let k = core.local(ox, oy).expect("outlet in its tile");
            let dn = if d0[k].1 == *id {
                carved.read(core)?[k]
            } else {
                drains_at(d0[k], tops[k], Some(*id))
            };
            if dn > nd.level + SILL_M {
                continue;
            }
            let r = plan.get_mut(id).expect("planned");
            (r.to, r.outlet, r.basin) = (nd.level, nd.outlet, basin);
        }
    }
    for id in empty {
        if plan[&id].basin == 0 {
            plan.insert(id, drained(id, from[&id]));
        }
    }
    Ok(plan.into_values().collect())
}

/// What a cell drains at once settled (`settle`'s rule): its own value `d0`, raised to the highest
/// labelled water `top` on its path unless that is the lake `own`.
fn drains_at(d0: Drained, top: Drained, own: Option<u32>) -> f32 {
    if top.1 >= 1 && Some(top.1) != own && top.0 > d0.0 {
        top.0
    } else {
        d0.0
    }
}

/// The basin a lake's bulk stays in: the largest pocket (`counts`: cells by m3 node layer
/// value, 0 for those still under the lake) when it holds more than the part that reached
/// the spill and its node is full (it stands at its own spill). Ties keep the smaller value.
fn bulk_pocket(counts: &BTreeMap<u32, u64>, n3: &Nodes) -> Option<u32> {
    let kept = counts.get(&0).copied().unwrap_or(0);
    counts
        .iter()
        .filter(|&(&n, &c)| n >= 2 && c > kept)
        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
        .map(|(&n, _)| n)
        .filter(|&n| n3.at(n - 2).full)
}

/// Whether a cell of a lake lowered from `from` to `lv` (ground `z`, level `l3` of the routing
/// on the carved ground) lies under it: below `lv`, and in no pocket the lowering cut off (a
/// pond of its own whose rim stood under the old level and stands above the new one). A lake
/// inside a higher pond of that routing stays (gate 1 ruling). A lake confined to `basin`
/// (node layer value; 0: none) lies only in that node (`node`, the cell's).
fn under(z: f32, l3: f32, lv: f32, from: f32, basin: u32, node: u32) -> bool {
    z < lv
        && if basin != 0 {
            node == basin
        } else {
            !(l3 > lv && l3 < from)
        }
}

/// A drained lake's record.
fn drained(id: u32, from: f32) -> Relevelled {
    Relevelled {
        id,
        from,
        to: f32::NAN,
        outlet: [u64::MAX; 2],
        ..Default::default()
    }
}

/// Diagnosis B: water a lowered lake held up goes down with it. From each lowered lake's old
/// extent (`by_id`: from, to), through 8-connected river water (no labelled lake, no sea)
/// standing in (to, from], every cell is lowered to max(to, bed + MIN_DEPTH_M), never raised;
/// the lowest value wins, then the smaller lake id. Returns (cell, surface, lake), by cell.
/// Exact over tiles: each round floods tile cores from the lakes and the cells reached across
/// their edges in earlier rounds, until a round reaches nothing new.
fn flood(
    s: &Setup,
    carved: &Layer<f32>,
    out: &Layers,
    lakes: &HashSet<u32>,
    by_id: &HashMap<u32, (f32, f32, u32)>,
    tag: &str,
) -> Result<Changes> {
    let lowered: HashMap<u32, (f32, f32)> = by_id
        .iter()
        .filter(|(_, v)| v.1.is_finite() && v.1 < v.0)
        .map(|(&id, &(from, to, _))| (id, (from, to)))
        .collect();
    let (world, tiles) = (s.world, s.tiles());
    // Cells reached: world index → (lake, value, surface before).
    let mut reached: HashMap<u64, Vec<(u32, f32, f32)>> = HashMap::new();
    let mut busy: BTreeSet<usize> = if lowered.is_empty() {
        BTreeSet::new()
    } else {
        (0..tiles.len()).collect()
    };
    for round in 0.. {
        ensure(round < 100_000, "a lowered lake's flood does not end")?;
        if busy.is_empty() {
            break;
        }
        let rects: Vec<Rect> = busy.iter().map(|&i| tiles[i]).collect();
        let seen_before = &reached;
        let parts: Vec<Vec<(u64, u32, f32, f32)>> = for_tiles(
            &rects,
            s.threads,
            &format!("{tag} flood round {round}"),
            |_, core| {
                let win = core.grow(1, world);
                let (sf, b, z) = (
                    out.surface.read(win)?,
                    out.body.read(win)?,
                    carved.read(win)?,
                );
                let gi = |j: usize| {
                    let (x, y) = win.world_xy(j);
                    world.index(x, y)
                };
                let around = |j: usize| {
                    let (x, y) = win.world_xy(j);
                    (0..8).filter_map(move |k| {
                        world
                            .neighbour(x, y, k)
                            .and_then(|(nx, ny)| win.local(nx, ny))
                    })
                };
                let in_core = |j: usize| {
                    let (x, y) = win.world_xy(j);
                    core.contains(x, y)
                };
                let had = |j: usize, id: u32| {
                    seen_before
                        .get(&gi(j))
                        .is_some_and(|v| v.iter().any(|e| e.0 == id))
                };
                let takes = |j: usize, id: u32| {
                    let (from, to) = lowered[&id];
                    sf[j].is_finite()
                        && b[j] > 1
                        && !lakes.contains(&b[j])
                        && sf[j] > to
                        && sf[j] <= from
                        && !had(j, id)
                };
                let mut seen: BTreeSet<(usize, u32)> = BTreeSet::new();
                let mut queue: Vec<(usize, u32)> = Vec::new();
                for k in 0..core.cells() {
                    let (x, y) = core.world_xy(k);
                    let j = win.local(x, y).expect("core in window");
                    for n in around(j) {
                        // Seeds: beside a lake's old extent, or beside a cell reached across
                        // the tile edge.
                        let mut ids: Vec<u32> = Vec::new();
                        if sf[n].is_finite() && lowered.contains_key(&b[n]) {
                            ids.push(b[n]);
                        }
                        if !in_core(n) {
                            ids.extend(seen_before.get(&gi(n)).into_iter().flatten().map(|e| e.0));
                        }
                        for id in ids {
                            if takes(j, id) && seen.insert((j, id)) {
                                queue.push((j, id));
                            }
                        }
                    }
                }
                while let Some((j, id)) = queue.pop() {
                    for n in around(j).filter(|&n| in_core(n)) {
                        if takes(n, id) && seen.insert((n, id)) {
                            queue.push((n, id));
                        }
                    }
                }
                Ok(seen
                    .into_iter()
                    .map(|(j, id)| {
                        let to = lowered[&id].1;
                        (gi(j), id, to.max(z[j] + MIN_DEPTH_M).min(sf[j]), sf[j])
                    })
                    .collect())
            },
        )?;
        busy.clear();
        for (c, id, v, was) in parts.into_iter().flatten() {
            reached.entry(c).or_default().push((id, v, was));
            // The flood goes on in the tiles across this cell's edges.
            let (x, y) = world.xy(c);
            let own = tile_of(world, s.tile, x, y);
            for k in 0..8 {
                if let Some((nx, ny)) = world.neighbour(x, y, k) {
                    let t = tile_of(world, s.tile, nx, ny);
                    if t != own {
                        busy.insert(t);
                    }
                }
            }
        }
    }
    let mut changes: Changes = reached
        .into_iter()
        .filter_map(|(c, v)| {
            let was = v[0].2;
            let (id, low) = v
                .iter()
                .map(|e| (e.0, e.1))
                .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))?;
            (low < was).then_some((c, low, id))
        })
        .collect();
    changes.sort_unstable_by_key(|c| c.0);
    Ok(changes)
}

/// Rewrites the planned lakes' cells: under the new level at it, the others dry; river water
/// the lake held up goes down with it (`flood`). Every tile's changes are cached before any is
/// written, so all read the same rasters and a killed run resumes. Returns (kept, dried,
/// flooded) cells per planned lake, in plan order.
#[allow(clippy::too_many_arguments)]
fn write_relevel(
    s: &Setup,
    r3: &Rasters,
    carved: &Layer<f32>,
    out: &Layers,
    lakes: &HashSet<u32>,
    plan: &[Relevelled],
    work: Option<&Work>,
    tag: &str,
) -> Result<Vec<(u64, u64, u64)>> {
    let by_id: HashMap<u32, (f32, f32, u32)> = plan
        .iter()
        .map(|r| (r.id, (r.from, r.to, r.basin)))
        .collect();
    let (world, tiles) = (s.world, s.tiles());
    let flooded: HashMap<u64, (f32, u32)> = cached(work, &format!("{tag}/flood"), || {
        flood(s, carved, out, lakes, &by_id, tag)
    })?
    .into_iter()
    .map(|(c, v, id)| (c, (v, id)))
    .collect();
    let parts: Vec<Relevel2> =
        for_tiles(&tiles, s.threads, &format!("{tag} relevel"), |i, core| {
            let mut part = cached(work, &format!("{tag}/r2/{i:05}-changes"), || {
                let win = core.grow(1, world);
                let (sf, b, z) = (
                    out.surface.read(win)?,
                    out.body.read(win)?,
                    carved.read(win)?,
                );
                let (lv3, nd) = (r3.level.read(win)?, r3.node.read(win)?);
                // A planned lake's cell under its new level.
                let wet = |j: usize| {
                    sf[j].is_finite()
                        && by_id.get(&b[j]).is_some_and(|&(from, to, basin)| {
                            under(z[j], lv3[j], to, from, basin, nd[j])
                        })
                };
                let (mut changed, mut per) = (Vec::new(), BTreeMap::new());
                for k in 0..core.cells() {
                    let (x, y) = core.world_xy(k);
                    let j = win.local(x, y).expect("core in window");
                    if !sf[j].is_finite() {
                        continue;
                    }
                    if let Some(&(_, to, _)) = by_id.get(&b[j]) {
                        let e: &mut (u64, u64, u64) = per.entry(b[j]).or_default();
                        if wet(j) {
                            e.0 += 1;
                            changed.push((k as u64, to, b[j]));
                        } else {
                            e.1 += 1;
                            changed.push((k as u64, f32::NAN, 0));
                        }
                        continue;
                    }
                    if let Some(&(v, id)) = flooded.get(&world.index(x, y)) {
                        per.entry(id).or_default().2 += 1;
                        changed.push((k as u64, v, b[j]));
                    }
                }
                let per = per
                    .into_iter()
                    .map(|(id, (a, d, m))| (id, a, d, m))
                    .collect();
                Ok((changed, per))
            })?;
            // With a work directory the changes wait on disk.
            if work.is_some() {
                part.0 = Vec::new();
            }
            Ok(part)
        })?;
    for_tiles(
        &tiles,
        s.threads,
        &format!("{tag} relevel write"),
        |i, core| {
            let name = format!("{tag}/r2/{i:05}");
            if let Some(wk) = work
                && wk.is_done(&name)
            {
                return Ok(());
            }
            let changed = match work {
                Some(wk) => {
                    let missing = || {
                        Err(crate::acquisition::AcquisitionError::Invalid(
                            "re-level changes are missing".into(),
                        ))
                    };
                    wk.cached::<Relevel2>(&format!("{name}-changes"), missing)?
                        .0
                }
                None => parts[i].0.clone(),
            };
            if !changed.is_empty() {
                let (mut sf, mut b) = (out.surface.read(core)?, out.body.read(core)?);
                for &(k, v, id) in &changed {
                    (sf[k as usize], b[k as usize]) = (v, id);
                }
                out.surface.write(core, &sf)?;
                out.body.write(core, &b)?;
            }
            if let Some(wk) = work {
                wk.mark_done(&name)?;
            }
            Ok(())
        },
    )?;
    let mut per: HashMap<u32, (u64, u64, u64)> = HashMap::new();
    for &(id, kept, dried, flooded) in parts.iter().flat_map(|p| p.1.iter()) {
        let e = per.entry(id).or_default();
        (e.0, e.1, e.2) = (e.0 + kept, e.1 + dried, e.2 + flooded);
    }
    Ok(plan
        .iter()
        .map(|r| per.get(&r.id).copied().unwrap_or_default())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::water::bodies::label;
    use crate::world::water::geom::OFFSETS;
    use crate::world::water::lake_labels;
    use crate::world::water::pipeline::{model, model_on};
    use crate::world::water::testing::{setup, terrain};

    type Run = (Vec<f32>, Vec<f32>, Vec<f32>, Vec<u32>, Vec<u8>);

    /// The synthetic world at 256²: with 32-cell tiles a window (tile + 2 · 72) is smaller than
    /// the world, so window edges fall among the rivers (N9).
    fn run(tile: u64) -> Run {
        let world = World { w: 256, h: 256 };
        run_on(world, terrain(world), tile, 3.6e4, 1e-5)
    }

    fn run_on(world: World, z: Vec<f32>, tile: u64, m3s_per_km2: f64, river_km2: f64) -> Run {
        run_setup(setup(world, tile, river_km2), z, m3s_per_km2, None)
    }

    /// `rasters` receives the model's node and level layers when given.
    fn run_setup(
        s: Setup,
        z: Vec<f32>,
        m3s_per_km2: f64,
        rasters: Option<&mut (Vec<u32>, Vec<f32>)>,
    ) -> Run {
        let (run, _, node, level, lake, _) = run_full(s, z, m3s_per_km2);
        if let Some(v) = rasters {
            // Pond cells are no lake nodes.
            assert!(!lake, "a lake node");
            *v = (node, level);
        }
        run
    }

    /// The channels run with its figures, then the lakes re-levelled on the carved ground (as
    /// `run`), the model's node and level layers, whether any node is a lake, and the model's
    /// discharge. No breach: the ground channels are cut into is raw.
    #[allow(clippy::type_complexity)]
    fn run_full(
        s: Setup,
        z: Vec<f32>,
        m3s_per_km2: f64,
    ) -> (Run, Value, Vec<u32>, Vec<f32>, bool, Vec<f32>) {
        let world = s.world;
        let raw = Layer::memory(world, z);
        let r = Rasters::memory(world);
        let m = model(&s, &raw, None, "m", Some(&r)).expect("model");
        let body = Layer::filled(world, 0u32);
        let bodies = label(&s, &m.water, &r, &body, &BTreeMap::new(), None, "m").expect("bodies");
        let out = Layers::memory(world);
        // Wide channels on a small grid at 3.6e4 m³/s per km² (radii up to about 25 cells,
        // inside HALO with BACKWATER).
        let geo = Geometry::new(m3s_per_km2);
        let fig =
            channels(&s, &m.water, &raw, &raw, &r, &body, &out, &geo, None, "c").expect("channels");
        let carved = Layer::memory(world, out.carved.to_vec().unwrap());
        let r3 = Rasters::memory(world);
        let m3 = model_on(&s, &carved, Some(&raw), None, "m3", Some(&r3)).expect("m3");
        let labels = lake_labels(&bodies, &m.water.nodes).expect("labels");
        relevel(&s, &r3, &m3.water.nodes, &carved, &out, &labels, None, "r").expect("relevel");
        let node = r.node.to_vec().unwrap();
        let lake = node.iter().any(|&n| n >= 2 && m.water.nodes.at(n - 2).lake);
        (
            (
                raw.to_vec().unwrap(),
                out.carved.to_vec().unwrap(),
                out.surface.to_vec().unwrap(),
                out.body.to_vec().unwrap(),
                r.flow.to_vec().unwrap(),
            ),
            fig,
            node,
            r.level.to_vec().unwrap(),
            lake,
            r.discharge.to_vec().unwrap(),
        )
    }

    #[test]
    fn only_river_cells_are_lowered_and_never_past_the_cap() {
        let (raw, carved, surface, body, _) = run(32);
        let mut lowered = 0;
        for k in 0..raw.len() {
            assert!(carved[k] <= raw[k], "raised at {k}");
            assert!(
                carved[k] >= raw[k] - CHANNEL_CAP_M - 1e-5,
                "cut past the cap at {k}"
            );
            if carved[k] < raw[k] {
                lowered += 1;
                assert!(
                    surface[k].is_finite() && body[k] > 1,
                    "a dry or unlabelled cell was lowered at {k}"
                );
                assert!(
                    carved[k] < surface[k],
                    "a lowered cell above its river at {k}"
                );
            }
        }
        assert!(lowered > 20, "channels were cut");
    }

    /// Every wet cell with a receiver: the receiver's surface is not higher.
    fn assert_no_rise(w: usize, surface: &[f32], flow: &[u8]) {
        for (k, &f) in flow.iter().enumerate() {
            if f & 15 == 0 || !surface[k].is_finite() {
                continue;
            }
            let (dy, dx) = OFFSETS[(f & 15) as usize - 1];
            let j = ((k / w) as i64 + dy) as usize * w + ((k % w) as i64 + dx) as usize;
            if surface[j].is_finite() {
                assert!(
                    surface[j] <= surface[k] + 1e-5,
                    "rises from {k} ({} m) to {j} ({} m)",
                    surface[k],
                    surface[j]
                );
            }
        }
    }

    #[test]
    fn river_surfaces_never_rise_downstream() {
        // Channels, relevel and settle as `run` does (F1: rims cut, lakes re-levelled).
        let world = World { w: 256, h: 256 };
        let s = setup(world, 32, 1e-5);
        let (_, _, surface, body, flow, _, _, lakes) = settled_run(&s, terrain(world), 3.6e4);
        // A lake re-levelled to the lower of two river reaches beside it can lie below the
        // higher one its m3 receiver names: a figure (`lake_routing_rises`). Others stay strict.
        for (k, &f) in flow.iter().enumerate() {
            if f & 15 == 0 || !surface[k].is_finite() {
                continue;
            }
            let (dy, dx) = OFFSETS[(f & 15) as usize - 1];
            let j = ((k / 256) as i64 + dy) as usize * 256 + ((k % 256) as i64 + dx) as usize;
            if surface[j] > surface[k] + 1e-5 {
                assert!(lakes.contains(&body[k]), "rises from {k} to {j}");
            }
        }
    }

    #[test]
    fn a_tributary_meets_a_larger_river_at_its_surface() {
        // A trunk valley along row 40 falling east, a tributary valley down column 80 from the
        // north, joining it; the trunk's section is wide (3.6e4 m³/s per km²), so the tributary's
        // last cells lie inside it (N18).
        let world = World { w: 256, h: 80 };
        let z: Vec<f32> = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
                let trunk =
                    50.0 - 0.1 * x + 0.3 * (y - 40.0).abs() + ((i * 7919) % 7) as f32 * 0.002;
                let trib = if y < 40.0 {
                    50.0 - 0.1 * 80.0 + 0.05 * (40.0 - y) + 0.3 * (x - 80.0).abs()
                } else {
                    f32::INFINITY
                };
                trunk.min(trib)
            })
            .collect();
        let (raw, carved, surface, body, flow) = run_on(world, z, 32, 3.6e4, 1e-5);
        assert_no_rise(256, &surface, &flow);
        // The tributary's cell next to the trunk thalweg carries the trunk's body (I3b).
        let at = |x: usize, y: usize| y * 256 + x;
        assert!(flow[at(80, 30)] & 16 != 0, "the tributary is a river");
        assert_eq!(
            body[at(80, 39)],
            body[at(100, 40)],
            "it joins the trunk inside its section"
        );
        // Its banks inside the trunk's radius stay wet and keep their channel (the next
        // river takes a cell the larger one rejects).
        let mut cut = 0;
        for y in 6..20 {
            for x in [79, 81] {
                assert!(surface[at(x, y)].is_finite(), "dry bank at ({x}, {y})");
                cut += usize::from(carved[at(x, y)] < raw[at(x, y)]);
            }
        }
        assert!(cut >= 20, "tributary banks cut: {cut} of 28");
    }

    /// A river through a 1 m deep pond (a dam across a valley; no lake: the lake minimum is
    /// out of reach): every pond cell is wet at the pond's level, as one river body.
    #[test]
    fn a_pond_a_river_crosses_is_wet_at_its_level() {
        let world = World { w: 96, h: 41 };
        let z: Vec<f32> = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
                let dam = if (46.0..=50.0).contains(&x) { 1.2 } else { 0.0 };
                40.0 - 0.05 * x + 0.3 * (y - 20.0).abs() + dam + ((i * 7919) % 7) as f32 * 0.001
            })
            .collect();
        let mut s = setup(world, 32, 1e-5);
        s.table.lake_min_m2 = 1e9;
        let mut nl = (Vec::new(), Vec::new());
        let (_, carved, surface, body, _) = run_setup(s, z, 1e3, Some(&mut nl));
        let (node, level) = nl;
        let pond = node[20 * 96 + 40];
        assert!(pond >= 2, "the dam holds a pond");
        let cells: Vec<usize> = (0..node.len()).filter(|&k| node[k] == pond).collect();
        let depth = level[cells[0]] - cells.iter().map(|&k| carved[k]).fold(f32::MAX, f32::min);
        assert!(
            cells.len() > 50 && depth > 0.9,
            "{} cells, {depth} m deep",
            cells.len()
        );
        for &k in &cells {
            // The spill cell runs MIN_DEPTH_M deep; backwater lifts the pond behind it.
            assert!(
                surface[k] == level[k] || surface[k] == level[k] + MIN_DEPTH_M,
                "pond cell {k} at {} m, level {} m",
                surface[k],
                level[k]
            );
            assert!(surface[k] > carved[k], "pond cell {k} above its ground");
            assert_eq!(body[k], body[cells[0]], "pond cell {k} in one body");
        }
        assert!(body[cells[0]] > 1);
    }

    /// Channels, then the routing on the carved ground (m3, keeping raw's receivers), relevel
    /// and settle on it, as `run` does (no breach): raw, carved, surface, body, m3 flow, m2 node
    /// and level.
    #[allow(clippy::type_complexity)]
    fn settled_run(
        s: &Setup,
        z: Vec<f32>,
        m3s_per_km2: f64,
    ) -> (
        Vec<f32>,
        Vec<f32>,
        Vec<f32>,
        Vec<u32>,
        Vec<u8>,
        Vec<u32>,
        Vec<f32>,
        HashSet<u32>,
    ) {
        let world = s.world;
        let raw = Layer::memory(world, z);
        let r = Rasters::memory(world);
        let m = model(s, &raw, None, "m", Some(&r)).expect("model");
        let body = Layer::filled(world, 0u32);
        let bodies = label(s, &m.water, &r, &body, &BTreeMap::new(), None, "m").expect("bodies");
        let out = Layers::memory(world);
        let geo = Geometry::new(m3s_per_km2);
        channels(s, &m.water, &raw, &raw, &r, &body, &out, &geo, None, "c").expect("channels");
        let carved = Layer::memory(world, out.carved.to_vec().unwrap());
        let r3 = Rasters::memory(world);
        let m3 = model_on(s, &carved, Some(&raw), None, "m3", Some(&r3)).expect("m3");
        let labels = lake_labels(&bodies, &m.water.nodes).expect("labels");
        // Relevel settles the water too.
        relevel(s, &r3, &m3.water.nodes, &carved, &out, &labels, None, "r").expect("relevel");
        let lakes: HashSet<u32> = labels.keys().copied().collect();
        (
            raw.to_vec().unwrap(),
            carved.to_vec().unwrap(),
            out.surface.to_vec().unwrap(),
            out.body.to_vec().unwrap(),
            r3.flow.to_vec().unwrap(),
            r.node.to_vec().unwrap(),
            r.level.to_vec().unwrap(),
            lakes,
        )
    }

    /// Export ruling: a thalweg down a 45° slope carries MIN_DEPTH_M of water; a pond's spill
    /// cell is cut and wet; after settle no surface rises downstream.
    #[test]
    fn uncut_river_cells_stand_a_centimetre_above_their_bed() {
        let world = World { w: 96, h: 41 };
        let noise = |i: u64| ((i * 7919) % 7) as f32 * 0.001;
        let steep: Vec<f32> = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
                200.0 - x + 0.3 * (y - 20.0).abs() + noise(i)
            })
            .collect();
        let mut s = setup(world, 32, 1e-5);
        s.table.lake_min_m2 = 1e9;
        let (raw, carved, surface, body, flow, ..) = settled_run(&s, steep, 1e3);
        for k in 0..raw.len() {
            if body[k] == 0 {
                continue;
            }
            assert!(surface[k] > carved[k], "wet cell {k} not above its bed");
            if flow[k] & 16 != 0 && carved[k] == raw[k] {
                assert!(surface[k] - carved[k] >= MIN_DEPTH_M - 1e-4, "cell {k}");
            }
        }
        assert_no_rise(96, &surface, &flow);

        // The dam of `a_pond_a_river_crosses_is_wet_at_its_level`: its spill cell.
        let dam: Vec<f32> = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
                let dam = if (46.0..=50.0).contains(&x) { 1.2 } else { 0.0 };
                40.0 - 0.05 * x + 0.3 * (y - 20.0).abs() + dam + noise(i)
            })
            .collect();
        let (raw, carved, surface, body, flow, node, level, _) = settled_run(&s, dam, 1e3);
        for k in 0..body.len() {
            assert!(body[k] == 0 || surface[k] > carved[k], "wet cell {k}");
        }
        // The river cuts the dam's spill cell (F1: no rim floor) and runs over it.
        let spill: Vec<usize> = (0..node.len())
            .filter(|&k| node[k] >= 2 && raw[k] >= level[k] && surface[k].is_finite())
            .collect();
        assert!(!spill.is_empty(), "the pond has a wet spill cell");
        assert!(
            spill.iter().any(|&k| carved[k] < level[k]),
            "the spill is cut"
        );
        assert_no_rise(96, &surface, &flow);
    }

    /// A tributary thalweg a larger river's section claimed, then gave up (its cut undone),
    /// returns to its own river at its own surface; any other such cell is dry and unlabelled.
    /// A river cell at a lake's exit at the level is dry shore; others run MIN_DEPTH_M deep.
    #[test]
    fn cells_a_cut_gives_up_return_to_their_own_river_or_dry() {
        assert_eq!(dry_again(Some((12.5, 7))), (12.5, 7, true, true));
        let (sf, id, river, keep) = dry_again(None);
        assert!(sf.is_nan());
        assert_eq!((id, river, keep), (0, false, false));
        assert_eq!(above_bed(10.5, 10.0, true), Some(10.5));
        assert_eq!(above_bed(10.0, 10.0, true), None);
        assert_eq!(above_bed(10.0, 10.0, false), Some(10.0 + MIN_DEPTH_M));
        assert_eq!(above_bed(9.99, 10.0, false), Some(10.0 + MIN_DEPTH_M));
    }

    /// Lake and sea cells whose ground reaches their level are dry shore; the others stand at
    /// the level, above their ground.
    #[test]
    fn lake_ground_at_the_level_is_dry_shore() {
        let world = World { w: 64, h: 64 };
        // A shelf at exactly sea level along the coast.
        let z: Vec<f32> = terrain(world)
            .into_iter()
            .map(|v| if (-0.5..=0.0).contains(&v) { 0.0 } else { v })
            .collect();
        let s = setup(world, 16, 1e-5);
        let raw = Layer::memory(world, z);
        let r = Rasters::memory(world);
        let m = model(&s, &raw, None, "m", Some(&r)).expect("model");
        let body = Layer::filled(world, 0u32);
        label(&s, &m.water, &r, &body, &BTreeMap::new(), None, "m").expect("bodies");
        let out = Layers::memory(world);
        let geo = Geometry::new(3.6e4);
        let fig =
            channels(&s, &m.water, &raw, &raw, &r, &body, &out, &geo, None, "c").expect("channels");
        let (g, node, level) = (
            raw.to_vec().unwrap(),
            r.node.to_vec().unwrap(),
            r.level.to_vec().unwrap(),
        );
        let (sf, b) = (out.surface.to_vec().unwrap(), out.body.to_vec().unwrap());
        let (mut shore, mut lake) = (0, 0);
        for k in 0..g.len() {
            if node[k] == 1 || (node[k] >= 2 && m.water.nodes.at(node[k] - 2).lake) {
                if g[k] >= level[k] {
                    assert!(sf[k].is_nan() && b[k] == 0, "shore cell {k} is wet");
                    shore += 1;
                } else {
                    assert_eq!(sf[k], level[k], "cell {k} at its level");
                    assert!(b[k] > 0);
                    lake += usize::from(node[k] >= 2);
                }
            }
        }
        assert!(shore > 0 && lake > 0, "{shore} shore, {lake} lake cells");
        assert_eq!(fig["shore_dry_cells"], shore);
    }

    /// A lake lowered to a small part's spill stays with its bulk: the largest pocket, when it
    /// outweighs what is still under the lake and stands at its own spill.
    #[test]
    fn a_lake_cut_off_behind_a_sill_stays_with_its_bulk() {
        use super::super::nodes::Node;
        let node = |key, full| Node {
            key,
            full,
            ..Default::default()
        };
        let n3 = Nodes(vec![node(1, true), node(2, true), node(3, false)]);
        let c = |v: &[(u32, u64)]| v.iter().copied().collect::<BTreeMap<u32, u64>>();
        assert_eq!(
            bulk_pocket(&c(&[(0, 20), (2, 12000), (3, 500)]), &n3),
            Some(2)
        );
        assert_eq!(
            bulk_pocket(&c(&[(0, 900), (2, 100)]), &n3),
            None,
            "lake kept"
        );
        assert_eq!(bulk_pocket(&c(&[(0, 20), (4, 100)]), &n3), None, "not full");
        assert!(under(1.0, f32::NAN, 2.0, 3.0, 0, 0));
        assert!(!under(1.0, 2.5, 2.0, 3.0, 0, 7), "a pocket");
        assert!(under(1.0, 2.5, 2.5, 3.0, 7, 7), "the basin");
        assert!(!under(1.0, f32::NAN, 2.5, 3.0, 7, 0), "outside the basin");
    }

    #[test]
    fn a_river_cell_cut_below_a_lake_beside_it_stands_at_the_lake() {
        // Cut below the lake (9.0 m) under a river at 9.3 m: at the lake.
        assert_eq!(mouth_surface(9.3, 8.8, Some(9.0)), 9.0);
        // Not cut below it, or no lower lake beside it: the river's surface.
        assert_eq!(mouth_surface(9.3, 9.1, Some(9.0)), 9.3);
        assert_eq!(mouth_surface(9.3, 8.8, None), 9.3);
    }

    /// Task 13 (B26, E41058 N56173): a river down a raw flat dotted with one-cell ponds a few
    /// cm deep. Their rims held the cut a centimetre above each pond, so only the pond cells
    /// were cut: a comb of pits along the bed. Rims are now cut like any section cell (F1).
    #[test]
    fn a_river_over_shallow_pits_cuts_an_even_bed() {
        let world = World { w: 96, h: 33 };
        let z: Vec<f32> = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = (i % world.w, i / world.w);
                let pit = if y == 16 && x % 3 == 0 && (9..=84).contains(&x) {
                    0.08
                } else {
                    0.0
                };
                40.0 - 0.01 * x as f32 + 0.3 * (y as f32 - 16.0).abs() - pit
                    + ((i * 7919) % 7) as f32 * 0.001
            })
            .collect();
        let s = || {
            // A river from the west edge on.
            let mut s = setup(world, 32, 1e-4);
            s.table.lake_min_m2 = 1e9;
            s
        };
        let m3s = 1e3;
        let ((raw, carved, ..), _, node, level, lake, q) = run_full(s(), z, m3s);
        let k = |x: usize| 16 * 96 + x;
        assert!(
            !lake && node[k(12)] >= 2 && level[k(12)] - raw[k(12)] < 0.1,
            "a shallow pond"
        );
        // The river is deeper than the ponds from the first pit on.
        let q = q[k(12)];
        assert!(Geometry::new(m3s).depth_m(f64::from(q)) > 0.15, "Q {q}");
        let uncut = (12..78)
            .filter(|&x| carved[k(x)] > raw[k(x)] - 1e-3)
            .count();
        let teeth = (12..78)
            .filter(|&x| carved[k(x)] < carved[k(x - 1)].min(carved[k(x + 1)]) - 0.05)
            .count();
        assert_eq!((uncut, teeth), (0, 0), "bed {:?}", &carved[k(12)..k(30)]);
    }

    /// Cut cells of a rill straight down a plane falling `fall` per cell with `across` per
    /// cell to its sides, at tiles 160 and 32 (asserted bit-identical).
    fn rill(fall: f32, across: f32, m3s: f64) -> usize {
        let world = World { w: 160, h: 41 };
        let z: Vec<f32> = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
                200.0 - fall * x + across * (y - 20.0).abs() + ((i * 7919) % 7) as f32 * 0.002
            })
            .collect();
        let mut first: Option<Vec<u32>> = None;
        for tile in [160, 32] {
            let (run, ..) = run_full(setup(world, tile, 1e-4), z.clone(), m3s);
            let bits: Vec<u32> = run.1.iter().map(|v| v.to_bits()).collect();
            let f = first.get_or_insert_with(|| bits.clone());
            assert_eq!(&bits, f, "tile {tile}");
        }
        let (raw, carved) = (z, first.expect("ran"));
        (0..raw.len())
            .filter(|&k| f32::from_bits(carved[k]) < raw[k])
            .count()
    }

    /// Owner rule: a rill straight down a 46° plane (raw 44–48° across it) is cut like any
    /// other; steep raw is no reason to spare it.
    #[test]
    fn a_rill_down_a_46_degree_slope_is_cut() {
        assert!(rill(1.035, 0.3, 50.0) > 0);
    }

    /// A rill down a 38° plane cut deep: its cut stays (no wall rounds take it back).
    #[test]
    fn a_deep_rill_on_a_38_degree_slope_is_cut() {
        let cut = rill(0.79, 0.05, 1e4);
        assert!(cut > 50, "the rill is cut ({cut} cells)");
    }

    /// Water settles on the final routing: cells draining into water above them are wetted
    /// as that water (labels and surfaces only), alike at every tiling.
    #[test]
    fn settled_water_never_lies_below_what_it_drains_into() {
        let world = World { w: 12, h: 3 };
        let settled = |tile: u64| {
            let s = setup(world, tile, 1e-5);
            let z: Vec<f32> = (0..36).map(|i| 10.0 - 0.1 * (i % 12) as f32).collect();
            let r3 = Rasters::memory(world);
            // Row 1 drains east to x = 11; the rest is a dry basin.
            let flow: Vec<u8> = (0..36)
                .map(|i| if i / 12 == 1 && i % 12 < 11 { 4 } else { 32 })
                .collect();
            r3.flow.write(Rect::whole(world), &flow).unwrap();
            let out = Layers::memory(world);
            let (mut sf, mut b) = (vec![f32::NAN; 36], vec![0u32; 36]);
            // A river cell at x = 8 standing at 10 m, above the ground of x = 1..7.
            (sf[12 + 8], b[12 + 8]) = (10.0, 7);
            out.surface.write(Rect::whole(world), &sf).unwrap();
            out.body.write(Rect::whole(world), &b).unwrap();
            let ground = Layer::memory(world, z);
            let fig = settle(&s, &r3, &ground, &out, &HashSet::new(), None, "s").expect("settle");
            (
                fig,
                out.surface.to_vec().unwrap(),
                out.body.to_vec().unwrap(),
            )
        };
        let (fig, sf, b) = settled(12);
        for x in 1..8 {
            assert_eq!((sf[12 + x], b[12 + x]), (10.0, 7), "x = {x}");
        }
        assert!(sf[12].is_nan(), "x = 0 stands at 10 m itself");
        assert!(
            sf[12 + 9].is_nan() && sf[1].is_nan(),
            "downstream and off the path dry"
        );
        assert_eq!(fig.0, 7);
        let (fig4, sf4, b4) = settled(4);
        assert_eq!(fig4.0, fig.0);
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!((bits(&sf4), b4), (bits(&sf), b), "tiles of 4 alike");
    }

    /// Settle rewrites the channels' rasters in place: a kill after a tile's surface write
    /// but before its body write (the settled cells' bodies lost) resumes to the same rasters.
    #[test]
    fn settle_resumes_after_a_kill_between_its_writes() {
        let world = World { w: 12, h: 3 };
        let dir = tempfile::tempdir().expect("temp");
        let work = Work::open(dir.path(), &json!({})).expect("work");
        let s = setup(world, 4, 1e-5);
        let z: Vec<f32> = (0..36).map(|i| 10.0 - 0.1 * (i % 12) as f32).collect();
        let r3 = Rasters::memory(world);
        let flow: Vec<u8> = (0..36)
            .map(|i| if i / 12 == 1 && i % 12 < 11 { 4 } else { 32 })
            .collect();
        r3.flow.write(Rect::whole(world), &flow).unwrap();
        let out = Layers::memory(world);
        let (mut sf, mut b) = (vec![f32::NAN; 36], vec![0u32; 36]);
        (sf[12 + 8], b[12 + 8]) = (10.0, 7);
        out.surface.write(Rect::whole(world), &sf).unwrap();
        out.body.write(Rect::whole(world), &b).unwrap();
        let ground = Layer::memory(world, z);
        let go = || settle(&s, &r3, &ground, &out, &HashSet::new(), Some(&work), "s");
        let fig = go().expect("settle");
        let (sf, b) = (out.surface.to_vec().unwrap(), out.body.to_vec().unwrap());
        // The kill: settled surfaces written, their bodies not, no done markers.
        let mut cut = b.clone();
        for k in 0..36 {
            if sf[k].is_finite() && k != 12 + 8 {
                cut[k] = 0;
            }
        }
        out.body.write(Rect::whole(world), &cut).unwrap();
        for i in 0..s.tiles().len() {
            let _ = std::fs::remove_file(work.path(&format!("s/s2/{i:05}.done")));
        }
        assert_eq!(go().expect("resume"), fig);
        assert_eq!(out.body.to_vec().unwrap(), b);
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&out.surface.to_vec().unwrap()), bits(&sf));
    }

    /// A bank pocket wet at its river's surface (9.2 m) inside an m3 pond at 9.6 m that spills
    /// into a dry cell at 9.6 m (the 60 `drainage_rises` cells at 8192²): settle raises it to
    /// the pond's level, keeping its body, and the river cell upstream with it.
    #[test]
    fn a_bank_pocket_inside_a_higher_pond_stands_at_the_pond_level() {
        let world = World { w: 12, h: 3 };
        let settled = |tile: u64| {
            let s = setup(world, tile, 1e-5);
            let mut z: Vec<f32> = (0..36).map(|i| 10.0 - 0.1 * (i % 12) as f32).collect();
            (z[12 + 5], z[12 + 6]) = (9.0, 9.6);
            let r3 = Rasters::memory(world);
            let flow: Vec<u8> = (0..36)
                .map(|i| if i / 12 == 1 && i % 12 < 11 { 4 } else { 32 })
                .collect();
            r3.flow.write(Rect::whole(world), &flow).unwrap();
            let mut lv = vec![f32::NAN; 36];
            lv[12 + 5] = 9.6;
            r3.level.write(Rect::whole(world), &lv).unwrap();
            let out = Layers::memory(world);
            let (mut sf, mut b) = (vec![f32::NAN; 36], vec![0u32; 36]);
            (sf[12 + 4], b[12 + 4]) = (9.25, 7);
            (sf[12 + 5], b[12 + 5]) = (9.2, 7);
            out.surface.write(Rect::whole(world), &sf).unwrap();
            out.body.write(Rect::whole(world), &b).unwrap();
            let ground = Layer::memory(world, z.clone());
            settle(&s, &r3, &ground, &out, &HashSet::new(), None, "s").expect("settle");
            let (sf, b) = (out.surface.to_vec().unwrap(), out.body.to_vec().unwrap());
            // What the drainage check reads: no rise along row 1.
            let d = |k: usize| {
                if sf[k].is_finite() {
                    sf[k]
                } else if lv[k].is_finite() {
                    lv[k]
                } else {
                    z[k]
                }
            };
            for x in 0..11 {
                assert!(d(12 + x + 1) <= d(12 + x), "rises at x = {x}");
            }
            (sf, b)
        };
        let (sf, b) = settled(12);
        assert_eq!((sf[12 + 5], b[12 + 5]), (9.6, 7), "the pocket");
        assert_eq!((sf[12 + 4], b[12 + 4]), (9.6, 7), "upstream");
        assert!(sf[12 + 6].is_nan(), "the spill stays dry");
        let (sf4, b4) = settled(4);
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!((bits(&sf4), b4), (bits(&sf), b), "tiles of 4 alike");
    }

    /// Gate 1 ruling (lake 21): a labelled lake (9.0 m) inside a higher m3 pond (9.3 m) keeps
    /// its label's level and extent; a river bank pocket in a higher pond (row 2) is still
    /// raised, and settle names it as the largest lift.
    #[test]
    fn a_labelled_lake_in_a_higher_pond_stays_at_its_label() {
        let world = World { w: 12, h: 4 };
        let settled = |tile: u64| {
            let s = setup(world, tile, 1e-5);
            let mut z: Vec<f32> = (0..48).map(|i| 10.0 - 0.1 * (i % 12) as f32).collect();
            let (mut sf, mut b) = (vec![f32::NAN; 48], vec![0u32; 48]);
            let mut lv = vec![f32::NAN; 48];
            // Row 1: lake 9 at x = 3..6 in an m3 pond over x = 2..7.
            for x in 2..8 {
                lv[12 + x] = 9.3;
                z[12 + x] = if (3..7).contains(&x) { 8.5 } else { 9.1 };
            }
            for x in 3..7 {
                (sf[12 + x], b[12 + x]) = (9.0, 9);
            }
            // Row 2: river 7's bank pocket at x = 5 in a pond at 9.6 m.
            (z[24 + 5], z[24 + 6], lv[24 + 5]) = (9.0, 9.6, 9.6);
            (sf[24 + 4], b[24 + 4]) = (9.25, 7);
            (sf[24 + 5], b[24 + 5]) = (9.2, 7);
            let r3 = Rasters::memory(world);
            let flow: Vec<u8> = (0..48)
                .map(|i| {
                    if (1..3).contains(&(i / 12)) && i % 12 < 11 {
                        4
                    } else {
                        32
                    }
                })
                .collect();
            r3.flow.write(Rect::whole(world), &flow).unwrap();
            r3.level.write(Rect::whole(world), &lv).unwrap();
            let out = Layers::memory(world);
            out.surface.write(Rect::whole(world), &sf).unwrap();
            out.body.write(Rect::whole(world), &b).unwrap();
            let ground = Layer::memory(world, z);
            let lakes = HashSet::from([9]);
            let fig = settle(&s, &r3, &ground, &out, &lakes, None, "s").expect("settle");
            (
                fig,
                out.surface.to_vec().unwrap(),
                out.body.to_vec().unwrap(),
            )
        };
        let (fig, sf, b) = settled(12);
        for x in 3..7 {
            assert_eq!((sf[12 + x], b[12 + x]), (9.0, 9), "lake at x = {x}");
        }
        for x in [0, 1, 2, 7, 8] {
            assert!(sf[12 + x].is_nan(), "row 1 x = {x} stays dry");
        }
        assert_eq!((sf[24 + 5], b[24 + 5]), (9.6, 7), "the pocket");
        assert_eq!((sf[24 + 4], b[24 + 4]), (9.6, 7), "upstream");
        assert_eq!((fig.3, fig.4), (24 + 5, 7), "the pocket lifts most");
        assert!((fig.2 - 0.4).abs() < 1e-5, "{fig:?}");
        let (fig4, sf4, b4) = settled(4);
        assert_eq!(fig4, fig);
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!((bits(&sf4), b4), (bits(&sf), b), "tiles of 4 alike");
    }

    /// The routing on the carved ground (m3, scale diagnosis 3): carving tips the plain Horn
    /// pick of some river cells off the carved river; kept receivers follow m2 wherever its
    /// receiver is not higher, alike at every tiling.
    #[test]
    fn the_final_routing_stays_on_the_carved_rivers() {
        let world = World { w: 256, h: 256 };
        let (raw, carved, _, _, m2) = run(32);
        let route = |z: &[f32], prior: Option<&Layer<f32>>, tile: u64| {
            let r = Rasters::memory(world);
            let ground = Layer::memory(world, z.to_vec());
            model_on(
                &setup(world, tile, 1e-5),
                &ground,
                prior,
                None,
                "m",
                Some(&r),
            )
            .expect("model");
            (r.flow.to_vec().unwrap(), r.node.to_vec().unwrap())
        };
        let m2_node = route(&raw, None, 32).1;
        // The fixture does not breach: channels are cut into raw, so raw is m2's ground.
        let prior = Layer::memory(world, raw);
        let route = |prior: Option<&Layer<f32>>, tile: u64| route(&carved, prior, tile);
        let plain = route(None, 32);
        let kept = route(Some(&prior), 32);
        assert_eq!(kept, route(Some(&prior), 256), "tilings alike");
        // Dry m2 thalweg cells, dry in m3 too, whose m2 receiver is still lower but not taken.
        let left = |(flow, node): &(Vec<u8>, Vec<u32>)| {
            (0..m2.len())
                .filter(|&k| {
                    let d = m2[k] & 15;
                    if m2[k] & 16 == 0 || d == 0 || m2_node[k] != 0 || node[k] != 0 {
                        return false;
                    }
                    let (dy, dx) = OFFSETS[d as usize - 1];
                    let r =
                        ((k / 256) as i64 + dy) as usize * 256 + ((k % 256) as i64 + dx) as usize;
                    carved[r] < carved[k] && flow[k] & 15 != d
                })
                .count()
        };
        assert!(left(&plain) > 0, "carving flips some plain picks");
        assert_eq!(left(&kept), 0);
    }

    #[test]
    fn channels_are_alike_at_every_tiling() {
        let a = run(256);
        let b = run(32);
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&a.1), bits(&b.1));
        assert_eq!(bits(&a.2), bits(&b.2));
        assert_eq!(a.3, b.3);
    }

    /// A hand-built state after channels, 12 × 3 cells: carved ground (20 m walls by default),
    /// the routing on it (flow, level, node and its nodes), the written water and the labels.
    struct Hand {
        z: Vec<f32>,
        flow: Vec<u8>,
        level: Vec<f32>,
        node: Vec<u32>,
        nodes: Nodes,
        sf: Vec<f32>,
        b: Vec<u32>,
        labels: BTreeMap<u32, Label>,
    }

    /// Flow codes: east, west, north; none.
    const E: u8 = 4;
    const W: u8 = 3;
    const N: u8 = 1;
    const HW: World = World { w: 12, h: 3 };

    fn at(x: usize, y: usize) -> usize {
        y * 12 + x
    }

    /// The finished state: surfaces, bodies and the moved lakes' records.
    type Done = (Vec<f32>, Vec<u32>, Vec<Relevelled>);

    impl Hand {
        fn new() -> Self {
            let n = 36;
            Self {
                z: vec![20.0; n],
                flow: vec![0; n],
                level: vec![f32::NAN; n],
                node: vec![0; n],
                nodes: Nodes(Vec::new()),
                sf: vec![f32::NAN; n],
                b: vec![0; n],
                labels: BTreeMap::new(),
            }
        }
        /// Row 1, x = 1..5: lake 9 at 10 m over 9 m ground, draining nowhere (`full`: its
        /// m2 spill edge is (4, 1) → (5, 1)).
        fn lake(mut self, full: bool) -> Self {
            for x in 1..5 {
                (self.z[at(x, 1)], self.sf[at(x, 1)], self.b[at(x, 1)]) = (9.0, 10.0, 9);
            }
            let outlet = full.then_some([at(4, 1) as u64, at(5, 1) as u64]);
            self.labels.insert(
                9,
                Label {
                    level: 10.0,
                    outlet,
                },
            );
            self
        }
        /// River `id` along row `y` from x0 east: (surface, ground) per cell, the last draining
        /// nowhere.
        fn river(mut self, id: u32, y: usize, x0: usize, cells: &[(f32, f32)]) -> Self {
            for (i, &(s, z)) in cells.iter().enumerate() {
                let k = at(x0 + i, y);
                (self.sf[k], self.z[k], self.b[k]) = (s, z, id);
                self.flow[k] = if i + 1 < cells.len() { E } else { 0 };
            }
            self
        }
        /// Relevel and settle as `run` does, at `tile`.
        fn run(&self, tile: u64) -> Done {
            let s = setup(HW, tile, 1e-5);
            let whole = Rect::whole(HW);
            let r3 = Rasters::memory(HW);
            r3.flow.write(whole, &self.flow).unwrap();
            r3.level.write(whole, &self.level).unwrap();
            r3.node.write(whole, &self.node).unwrap();
            let out = Layers::memory(HW);
            out.surface.write(whole, &self.sf).unwrap();
            out.body.write(whole, &self.b).unwrap();
            let carved = Layer::memory(HW, self.z.clone());
            let (moved, (_, (passes, converged, _))) =
                relevel(&s, &r3, &self.nodes, &carved, &out, &self.labels, None, "r")
                    .expect("relevel");
            assert!(converged && passes <= 3, "{passes} passes");
            (
                out.surface.to_vec().unwrap(),
                out.body.to_vec().unwrap(),
                moved,
            )
        }
        /// `run` at one tile and at tiles of 4 (asserted alike).
        fn done(&self) -> Done {
            let one = self.run(12);
            let four = self.run(4);
            let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
            assert_eq!(
                (bits(&four.0), &four.1),
                (bits(&one.0), &one.1),
                "tiles of 4"
            );
            assert_eq!(four.2, one.2, "tiles of 4");
            one
        }
        /// What a cell drains at (the checks' `dsurf`).
        fn dsurf(&self, sf: &[f32], k: usize) -> f32 {
            if sf[k].is_finite() {
                sf[k]
            } else if self.level[k].is_finite() {
                self.level[k]
            } else {
                self.z[k]
            }
        }
        /// `lake_leaks` and the saddle of lake 9 at `lv` with spill edge `outlet`, as the checks
        /// value them: (leaks, lowest saddle).
        fn leaks(&self, d: &Done, lv: f32, outlet: [u64; 2]) -> (usize, f32) {
            let (sf, b) = (&d.0, &d.1);
            let (mut leaks, mut saddle) = (0, f32::INFINITY);
            for j in 0..36 {
                if !(sf[j].is_finite() && b[j] == 9) || self.z[j] >= lv {
                    continue;
                }
                let (x, y) = HW.xy(j as u64);
                for k in 0..8 {
                    let Some((nx, ny)) = HW.neighbour(x, y, k) else {
                        continue;
                    };
                    let n = HW.index(nx, ny) as usize;
                    let same = sf[n].is_finite() && b[n] == 9;
                    if same && self.z[n] < lv {
                        continue;
                    }
                    let dn = if same { lv } else { self.dsurf(sf, n) };
                    saddle = saddle.min(lv.max(dn));
                    if dn.max(self.z[j]) < lv - SILL_M && n as u64 != outlet[1] {
                        leaks += 1;
                    }
                }
            }
            (leaks, saddle)
        }
        /// `lake_exit_surface`'s gap: the spill edge's higher shown value against the level.
        fn exit_gap(&self, d: &Done, lv: f32, outlet: [u64; 2]) -> f32 {
            let shown = |k: u64| {
                let k = k as usize;
                if d.0[k].is_finite() {
                    d.0[k]
                } else {
                    self.z[k]
                }
            };
            (shown(outlet[0]).max(shown(outlet[1])) - lv).abs()
        }
    }

    /// Diagnosis A (lake 174): a bank cell cut below the lake beside a lower river, dry after
    /// channels and draining into the lake, is wetted as the lake by settle. The lake then
    /// spills through it: relevel and settle repeat until a pass plans nothing, so the lake
    /// stands at that river (9.9 m) and does not leak.
    #[test]
    fn a_lake_settle_grows_over_a_cut_bank_drops_to_its_spill() {
        let mut h = Hand::new().lake(false).river(
            7,
            1,
            6,
            &[
                (9.9, 9.5),
                (9.7, 9.3),
                (9.5, 9.0),
                (9.4, 8.9),
                (9.3, 8.8),
                (9.2, 8.7),
            ],
        );
        (h.z[at(5, 1)], h.flow[at(5, 1)]) = (9.85, W);
        let d = h.done();
        assert_eq!((d.0[at(5, 1)], d.1[at(5, 1)]), (9.9, 9), "the bank is lake");
        for x in 1..5 {
            assert_eq!(d.0[at(x, 1)], 9.9, "lake at x = {x}");
        }
        let r = d.2.iter().find(|r| r.id == 9).expect("relevelled");
        assert_eq!((r.from, r.to), (10.0, 9.9));
        assert_eq!(h.leaks(&d, 9.9, r.outlet).0, 0);
    }

    /// Diagnosis B (lake 690): a lake lowered to a sill cell at its new level (9.6 m) whose
    /// receiver is a river standing between the new and the old level (9.8 m, reaching into
    /// the next tile). The river goes down with the lake (a flood from its old extent), so
    /// settle does not wet the sill above the lake.
    #[test]
    fn water_a_lowered_lake_held_up_goes_down_with_it() {
        let mut h = Hand::new()
            .lake(false)
            .river(
                7,
                1,
                6,
                &[
                    (9.5, 9.0),
                    (9.4, 8.9),
                    (9.3, 8.8),
                    (9.2, 8.7),
                    (9.1, 8.6),
                    (9.0, 8.5),
                ],
            )
            .river(
                8,
                0,
                5,
                &[
                    (9.8, 9.0),
                    (9.8, 9.0),
                    (9.8, 9.0),
                    (9.8, 9.0),
                    (9.8, 9.0),
                    (9.3, 8.8),
                    (9.2, 8.7),
                ],
            );
        // The sill: lake water over 9.6 m ground, draining north into river 8.
        (h.z[at(5, 1)], h.sf[at(5, 1)], h.b[at(5, 1)]) = (9.6, 10.0, 9);
        h.flow[at(5, 1)] = N;
        let d = h.done();
        let r = d.2.iter().find(|r| r.id == 9).expect("relevelled");
        assert_eq!((r.to, r.outlet), (9.6, [at(5, 1) as u64, at(6, 1) as u64]));
        assert!(d.0[at(5, 1)].is_nan(), "the sill stays dry");
        assert!(h.exit_gap(&d, 9.6, r.outlet) <= SILL_M);
        // River 8 down to the lake, across the tile edge; below the new level untouched.
        let row: Vec<f32> = (5..12).map(|x| d.0[at(x, 0)]).collect();
        assert_eq!(row, [9.6, 9.6, 9.6, 9.6, 9.6, 9.3, 9.2]);
        assert_eq!(r.flooded, 5);
        assert_eq!(h.leaks(&d, 9.6, r.outlet).0, 0);
    }

    /// Diagnosis D (lake 1403): the outlet cell is cut below the lake; the reach below stands
    /// above the lake (stale section water) before it falls away. Settle carried that water up
    /// to the outlet; the lake's outflow is lowered to the lake instead, down to where it
    /// falls below it.
    #[test]
    fn a_lakes_outflow_does_not_stand_above_the_lake() {
        let h = Hand::new().lake(true).river(
            7,
            1,
            5,
            &[
                (10.0, 9.8),
                (10.07, 9.5),
                (9.9, 9.0),
                (9.5, 8.9),
                (9.4, 8.8),
                (9.3, 8.7),
                (9.2, 8.6),
            ],
        );
        let d = h.done();
        assert!(d.2.is_empty(), "the lake keeps its label: {:?}", d.2);
        let outlet = h.labels[&9].outlet.expect("full");
        assert!(h.exit_gap(&d, 10.0, outlet) <= SILL_M);
        let row: Vec<f32> = (5..12).map(|x| d.0[at(x, 1)]).collect();
        assert_eq!(row, [10.0, 10.0, 9.9, 9.5, 9.4, 9.3, 9.2]);
        assert_eq!(h.leaks(&d, 10.0, outlet).0, 0);
    }

    /// Ruling on diagnosis C (lakes 103, 1183): a full labelled lake (10 m) inside a wet,
    /// higher m3 node (10.2 m, its pit and a river cell included) takes that node's level and
    /// extent. A full node gives its exit; one that is not full (103 and 1183 in the
    /// full-world run: the merge level of full children, endorheic in m3) leaves the lake
    /// closed. A lake that is not full keeps its label (gate 1 ruling).
    #[test]
    fn a_full_lake_inside_a_higher_basin_takes_its_level() {
        let basin = |full_lake: bool, full_node: bool| {
            let mut h = Hand::new().lake(full_lake);
            for x in 1..7 {
                (h.node[at(x, 1)], h.level[at(x, 1)]) = (2, 10.2);
            }
            (h.z[at(1, 1)], h.z[at(5, 1)], h.z[at(6, 1)]) = (10.1, 10.05, 10.15);
            (h.sf[at(1, 1)], h.b[at(1, 1)]) = (f32::NAN, 0);
            // The lake's outflow river starts inside the node.
            (h.sf[at(5, 1)], h.b[at(5, 1)], h.flow[at(5, 1)]) = (10.1, 7, E);
            h.z[at(7, 1)] = 10.2;
            for x in 8..12 {
                h.z[at(x, 1)] = 9.5 - 0.1 * (x - 8) as f32;
            }
            for x in 7..11 {
                h.flow[at(x, 1)] = E;
            }
            h.nodes = Nodes(vec![super::super::nodes::Node {
                key: 1,
                level: 10.2,
                full: full_node,
                lake: true,
                outlet: if full_node {
                    [at(6, 1) as u64, at(7, 1) as u64]
                } else {
                    [0, 0]
                },
                cells: 6,
                ..Default::default()
            }]);
            h
        };
        for full_node in [true, false] {
            let h = basin(true, full_node);
            let d = h.done();
            let r = d.2.iter().find(|r| r.id == 9).expect("raised");
            assert_eq!((r.from, r.to, r.basin, r.cells), (10.0, 10.2, 2, 6));
            for x in 1..7 {
                assert_eq!((d.0[at(x, 1)], d.1[at(x, 1)]), (10.2, 9), "lake at x = {x}");
            }
            if full_node {
                assert_eq!(r.outlet, [at(6, 1) as u64, at(7, 1) as u64]);
                assert!(h.exit_gap(&d, 10.2, r.outlet) <= SILL_M);
            } else {
                assert_eq!(r.outlet, [u64::MAX; 2], "closed");
            }
            let (leaks, saddle) = h.leaks(&d, 10.2, r.outlet);
            assert_eq!(leaks, 0);
            assert!((saddle - 10.2).abs() <= SILL_M, "saddle {saddle}");
        }

        let h = basin(false, true);
        let d = h.done();
        assert!(d.2.is_empty(), "not full: {:?}", d.2);
        for x in 2..5 {
            assert_eq!(d.0[at(x, 1)], 10.0, "lake at x = {x}");
        }
    }
}
