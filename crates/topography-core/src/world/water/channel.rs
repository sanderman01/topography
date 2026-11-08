//! 1 m channels. Each thalweg cell's section (width and depth from its discharge, hydraulic
//! geometry) is cut below the river's surface, which is the routing surface R: the ground on
//! dry thalweg cells, the pond level inside ponds. R never rises along a path, so neither does
//! the river. The section is the thalweg's centre cut (down to surface − depth) tapered by
//! 1 − (r/R)² across the half-width R, so no cell loses more than its centre (no wall is cut
//! by its height above a line). A cell within several rivers' half-widths belongs to the
//! river with the larger discharge (its body's largest), then the nearest cell, then the
//! smaller index (review I3b): a tributary inside a larger river's section joins that river
//! there instead of running beside it; a thalweg cell the larger river does not take keeps its
//! own. A cell is lowered only when it ends up under its river's surface, and never more than
//! CHANNEL_CAP_M below raw (R9); a section cell next to a lake whose level stands above the
//! river's surface is left as it is (R14). Lakes, pools and the sea keep their level; their
//! cells whose ground reaches the level are dry shore, as is a lake's exit cell at the level.
//! Other river water over a bed left uncut runs MIN_DEPTH_M deep (export ruling). A river
//! cell below the surface of the cell it flows into is raised to it over BACKWATER cells
//! (N18). At coarser spacings (no channels) rivers are their thalweg cells at R and nothing
//! is cut. Owner rule (2026-11-08): no pass takes a cut back to satisfy a check (`walls`,
//! `bed_rises` and `carve_pits` are figures).

use super::checks::SILL_M;
use super::geom::{Rect, World};
use super::hydraulic::Geometry;
use super::layers::Layer;
use super::pipeline::{Rasters, Setup, Water};
use super::store::{Work, cached, for_tiles};
use super::{CHANNEL_CAP_M, MIN_DEPTH_M};
use crate::acquisition::{Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
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

/// A cut beside another m2 node (lakes included) stops at the floor `rim` (or the ground, if
/// lower), so carving never lowers a pond's outlet (fix round 2).
fn rim_floor(ground: f32, cut: f32, rim: Option<f32>) -> f32 {
    rim.map_or(cut, |lv| cut.max(ground.min(lv)))
}

/// The rim floor of a cell: SILL_M above the highest level of the other m2 nodes beside it, so
/// carving never makes a new spill at their level (gate 2 fix 2b), or its own node's level
/// where it stands at it (its spill cell); the higher of the two.
fn rim_level(own: Option<f32>, others: impl Iterator<Item = f32>) -> Option<f32> {
    others.map(|lv| lv + SILL_M).chain(own).reduce(f32::max)
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
            // Every section reaching the area lies in the window (MAX_RADIUS + BACKWATER = HALO).
            let win = core.grow(HALO + MARGIN - BACKWATER, world);
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
            let river = |t: usize| if level[t].is_finite() { level[t] } else { g[t] };
            let depth = |t: usize| geo.depth_m(f64::from(q[t]));
            // Whether node `n`'s rim holds against a river `d` deep: lakes always, ponds as
            // deep as the river or deeper; shallower ponds drown in its channel (Task 13).
            let kept_rim = |n: u32, d: f32| {
                let p = w.nodes.at(n - 2);
                p.lake || p.level - p.low >= d
            };
            // The centre cut: down to the surface − depth, within the raw − cap clamp.
            let centre = |t: usize| {
                if !s.params.channels {
                    return 0.0;
                }
                let floor = (river(t) - geo.depth_m(f64::from(q[t]))).max(rw[t] - CHANNEL_CAP_M);
                (g[t] - floor).max(0.0)
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
                // Fix round 2: a rim beside a neighbouring m2 node (lakes included) other than
                // j's own is not cut to within SILL_M of its level; j's own spill cell not
                // below its level. Ponds shallower than the cutting river's depth `d` drown in
                // its channel and keep no rim (Task 13: else their rims comb the bed).
                let own_rim = (node[j] >= 2 && g[j] >= level[j]).then_some(level[j]);
                let pond_rim = |d: f32| {
                    rim_level(
                        own_rim.filter(|_| kept_rim(node[j], d)),
                        around
                            .iter()
                            .filter(|&&j2| {
                                node[j2] >= 2 && node[j2] != node[j] && kept_rim(node[j2], d)
                            })
                            .map(|&j2| level[j2]),
                    )
                };
                // The lowest neighbouring lake level below a river surface `rs`.
                let lake_below = |rs: f32| {
                    around
                        .iter()
                        .filter(|&&j2| is_lake(j2) && level[j2] < rs)
                        .map(|&j2| level[j2])
                        .reduce(f32::min)
                };
                for (t, d2) in ranked.into_iter().chain(own) {
                    let rs = river(t);
                    let below_a_lake = around.iter().any(|&j2| is_lake(j2) && level[j2] > rs);
                    if below_a_lake && t != j {
                        continue;
                    }
                    let rad = radius(t);
                    let taper = if d2 == 0 {
                        1.0
                    } else {
                        1.0 - (f64::from(d2) / (rad * rad)) as f32
                    };
                    let cut = (g[j] - centre(t) * taper)
                        .max(rw[j] - CHANNEL_CAP_M)
                        .min(g[j]);
                    let cut = rim_floor(g[j], cut, pond_rim(depth(t)));
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

/// Settles the written water on the final routing (review: drainage_rises): what drains at a
/// cell never lies below what its receiver (`r3`, the routing on the carved ground) drains
/// at. A wet cell inside a higher pond of that routing stands at the pond's level. Each cell
/// takes the highest labelled water on its path downstream when that lies above it: a dry or
/// pond cell is wetted as that water's body, a wet one raised. Labels and
/// surfaces only. Exact over tiles: per-tile path maxima, then the paths across tile rings.
/// Labelled lakes (`lakes`, body ids) keep their label's level and extent (gate 1 ruling:
/// labels from m3 would be circular).
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
    let read = |core: Rect| -> Result<(Vec<u8>, Vec<Drained>)> {
        let (sf, b) = (out.surface.read(core)?, out.body.read(core)?);
        let (lv, z) = (r3.level.read(core)?, ground.read(core)?);
        // A wet river or pool cell inside a higher pond stands at the pond's level (bank
        // pockets, scale diagnosis 2); a labelled lake at its label.
        let d0 = (0..core.cells())
            .map(|k| {
                if sf[k].is_finite() && lakes.contains(&b[k]) {
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
        Ok((r3.flow.read(core)?, d0))
    };
    // 1: the ring cells' path maxima (every path entering a tile enters at its ring).
    let rings: Vec<Vec<(u64, PathMax)>> =
        for_tiles(&tiles, s.threads, &format!("{tag} settle 1"), |i, core| {
            cached(work, &format!("{tag}/s1/{i:05}"), || {
                let (flow, d0) = read(core)?;
                let pm = path_max(world, core, &flow, &d0)?;
                Ok((0..core.cells())
                    .filter_map(|k| {
                        let (x, y) = core.world_xy(k);
                        core.on_ring(x, y).then(|| (world.index(x, y), pm[k]))
                    })
                    .collect())
            })
        })?;
    let ring: HashMap<u64, PathMax> = rings.into_iter().flatten().collect();
    // The whole path's maximum from a ring cell, across tiles (memoized).
    let mut full: HashMap<u64, Drained> = HashMap::new();
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
    // 2: every cell's whole-path maximum; raise where it lies above the cell.
    let none: Settled = (0, 0, 0.0, u64::MAX, 0);
    let settle_tile = |core: Rect| -> Result<SettledTile> {
        let (flow, d0) = read(core)?;
        let pm = path_max(world, core, &flow, &d0)?;
        let (sf, b) = (out.surface.read(core)?, out.body.read(core)?);
        let (mut fig, mut changed) = (none, Vec::new());
        for k in 0..core.cells() {
            // Labelled lakes keep their surface and body.
            if sf[k].is_finite() && lakes.contains(&b[k]) {
                continue;
            }
            let (v, id, exit) = pm[k];
            let top = match exit {
                NO_EXIT => (v, id),
                e => {
                    let f = full.get(&e).copied().ok_or_else(|| {
                        crate::acquisition::AcquisitionError::Invalid(format!(
                            "cell {e} is on no tile ring"
                        ))
                    })?;
                    if f.0 > v { f } else { (v, id) }
                }
            };
            let (was, mut v) = (sf[k], (sf[k], b[k]));
            // A wet cell keeps its body at its pond's level (d0).
            if was.is_finite() {
                v.0 = d0[k].0;
            }
            // Only labelled water wets a cell (a pond or dry source has no body).
            if top.0 > d0[k].0 && top.1 >= 1 {
                v = top;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::water::bodies::label;
    use crate::world::water::geom::OFFSETS;
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

    /// The channels run with its figures, the model's node and level layers, whether any node
    /// is a lake, and the model's discharge. No breach: the ground channels are cut into is raw.
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
        label(&s, &m.water, &r, &body, &BTreeMap::new(), None, "m").expect("bodies");
        let out = Layers::memory(world);
        // Wide channels on a small grid at 3.6e4 m³/s per km² (radii up to about 25 cells,
        // inside HALO with BACKWATER).
        let geo = Geometry::new(m3s_per_km2);
        let fig =
            channels(&s, &m.water, &raw, &raw, &r, &body, &out, &geo, None, "c").expect("channels");
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
        let (_, _, surface, _, flow) = run(32);
        assert_no_rise(256, &surface, &flow);
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

    /// Channels, then the routing on the carved ground (m3, keeping raw's receivers) and settle
    /// on it, as `run` does (no breach, no lakes): raw, carved, surface, body, m3 flow, m2 node
    /// and level.
    #[allow(clippy::type_complexity)]
    fn settled_run(
        s: &Setup,
        z: Vec<f32>,
    ) -> (
        Vec<f32>,
        Vec<f32>,
        Vec<f32>,
        Vec<u32>,
        Vec<u8>,
        Vec<u32>,
        Vec<f32>,
    ) {
        let world = s.world;
        let raw = Layer::memory(world, z);
        let r = Rasters::memory(world);
        let m = model(s, &raw, None, "m", Some(&r)).expect("model");
        let body = Layer::filled(world, 0u32);
        label(s, &m.water, &r, &body, &BTreeMap::new(), None, "m").expect("bodies");
        let out = Layers::memory(world);
        let geo = Geometry::new(1e3);
        channels(s, &m.water, &raw, &raw, &r, &body, &out, &geo, None, "c").expect("channels");
        let carved = Layer::memory(world, out.carved.to_vec().unwrap());
        let r3 = Rasters::memory(world);
        model_on(s, &carved, Some(&raw), None, "m3", Some(&r3)).expect("m3");
        settle(s, &r3, &carved, &out, &HashSet::new(), None, "s").expect("settle");
        (
            raw.to_vec().unwrap(),
            carved.to_vec().unwrap(),
            out.surface.to_vec().unwrap(),
            out.body.to_vec().unwrap(),
            r3.flow.to_vec().unwrap(),
            r.node.to_vec().unwrap(),
            r.level.to_vec().unwrap(),
        )
    }

    /// Export ruling: a thalweg down a 45° slope and a pond's spill cell (left at
    /// the level) carry MIN_DEPTH_M of water, and after settle no surface rises downstream.
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
        let (raw, carved, surface, body, flow, ..) = settled_run(&s, steep);
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
        let (_, carved, surface, body, flow, node, level) = settled_run(&s, dam);
        for k in 0..body.len() {
            assert!(body[k] == 0 || surface[k] > carved[k], "wet cell {k}");
        }
        let spill: Vec<usize> = (0..node.len())
            .filter(|&k| node[k] >= 2 && carved[k] >= level[k] && surface[k].is_finite())
            .collect();
        assert!(!spill.is_empty(), "the pond has a wet spill cell");
        for k in spill {
            assert!(surface[k] - carved[k] >= MIN_DEPTH_M - 1e-4, "spill {k}");
        }
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

    /// Fix round 2: the river over the dam cuts neither the pond's rim nor its spill cell (in
    /// the node, at the level) below the level, so the routing on the carved ground (m3) keeps
    /// every pond cell (the rim alone lost 7: the spill cell was cut 5.5 cm).
    #[test]
    fn a_river_over_a_dam_keeps_the_pond_behind_it() {
        let world = World { w: 96, h: 41 };
        let z: Vec<f32> = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
                let dam = if (46.0..=50.0).contains(&x) { 1.2 } else { 0.0 };
                40.0 - 0.05 * x + 0.3 * (y - 20.0).abs() + dam + ((i * 7919) % 7) as f32 * 0.001
            })
            .collect();
        let s = || {
            let mut s = setup(world, 32, 1e-5);
            s.table.lake_min_m2 = 1e9;
            s
        };
        let mut nl = (Vec::new(), Vec::new());
        let (raw, carved, ..) = run_setup(s(), z, 1e3, Some(&mut nl));
        let (node, level) = nl;
        let pond = node[20 * 96 + 40];
        assert!(pond >= 2, "the dam holds a pond");
        let w = 96i64;
        let mut rim = 0;
        for k in 0..node.len() {
            let (x, y) = ((k as i64) % w, (k as i64) / w);
            let lv = OFFSETS
                .iter()
                .map(|&(dy, dx)| (x + dx, y + dy))
                .filter(|&(nx, ny)| nx >= 0 && ny >= 0 && nx < w && ny < 41)
                .map(|(nx, ny)| (ny * w + nx) as usize)
                .filter(|&k2| node[k2] == pond && node[k] != pond)
                .map(|k2| level[k2])
                .reduce(f32::max);
            if let Some(lv) = lv {
                rim += 1;
                assert!(
                    carved[k] >= raw[k].min(lv),
                    "rim cell {k} cut to {} below the pond at {lv}",
                    carved[k]
                );
            }
        }
        assert!(rim > 10);
        let r3 = Rasters::memory(world);
        model(
            &s(),
            &Layer::memory(world, carved.clone()),
            None,
            "m3",
            Some(&r3),
        )
        .expect("m3");
        let n3 = r3.node.to_vec().unwrap();
        let lost = (0..node.len())
            .filter(|&k| node[k] == pond && n3[k] == 0)
            .count();
        assert_eq!(lost, 0, "pond cells lost on the carved ground");
    }

    #[test]
    fn a_river_cell_cut_below_a_lake_beside_it_stands_at_the_lake() {
        // Cut below the lake (9.0 m) under a river at 9.3 m: at the lake.
        assert_eq!(mouth_surface(9.3, 8.8, Some(9.0)), 9.0);
        // Not cut below it, or no lower lake beside it: the river's surface.
        assert_eq!(mouth_surface(9.3, 9.1, Some(9.0)), 9.3);
        assert_eq!(mouth_surface(9.3, 8.8, None), 9.3);
        // The rim floor: never below the neighbour's level, nor above the ground.
        assert_eq!(rim_floor(10.0, 8.0, Some(9.0)), 9.0);
        assert_eq!(rim_floor(8.5, 8.0, Some(9.0)), 8.5);
        assert_eq!(rim_floor(10.0, 9.5, Some(9.0)), 9.5);
        assert_eq!(rim_floor(10.0, 8.0, None), 8.0);
    }

    #[test]
    fn a_rim_beside_another_pond_stays_a_sill_above_its_level() {
        // Gate 2 case 2 (E12336 N12293): rim cells beside a one-cell pond at 700.7356 were cut
        // to exactly its level and became spills; they now stop a centimetre above it.
        let lv = 700.7356f32;
        let floor = rim_level(None, [lv].into_iter()).unwrap();
        assert_eq!(floor, lv + SILL_M);
        assert_eq!(rim_floor(700.879, 700.2, Some(floor)), lv + SILL_M);
        // Ground below that floor is never raised.
        assert_eq!(rim_floor(700.74, 700.2, Some(floor)), 700.74);
        // Its own spill cell keeps the level itself; the higher floor wins.
        assert_eq!(rim_level(Some(lv), std::iter::empty()), Some(lv));
        assert_eq!(rim_level(Some(lv), [lv].into_iter()), Some(lv + SILL_M));
        assert_eq!(rim_level(Some(lv + 1.0), [lv].into_iter()), Some(lv + 1.0));
        assert_eq!(rim_level(None, std::iter::empty()), None);
    }

    /// Task 13 (B26, E41058 N56173): a river down a raw flat dotted with one-cell ponds a few
    /// cm deep. Their rims held the cut a centimetre above each pond, so only the pond cells
    /// were cut: a comb of pits along the bed. Ponds shallower than the river's depth are
    /// drowned in its channel and their rims are cut like any section cell.
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
}
