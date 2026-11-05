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
//! river's surface is left as it is (R14). Lakes, pools and the sea keep their level. A river
//! cell below the surface of the cell it flows into is raised to it over BACKWATER cells
//! (N18). At coarser spacings (no channels) rivers are their thalweg cells at R and nothing
//! is cut.

use super::CHANNEL_CAP_M;
use super::checks::{CHANGED_M, horn_deg, is_wall};
use super::geom::{Rect, World};
use super::hydraulic::Geometry;
use super::layers::Layer;
use super::pipeline::{Rasters, Setup, Water};
use super::store::{Work, cached, for_tiles};
use crate::acquisition::{Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Window halo: the widest section's radius plus the backwater fits inside it.
pub(crate) const HALO: u64 = 64;
/// A river cell's surface is raised to that of the river cell it flows into over at most
/// this many cells upstream (N18). HALO covers the widest radius + BACKWATER.
pub(crate) const BACKWATER: u64 = 16;
/// Section radii are clamped here (cells) so every section reaching a tile's area lies in
/// its window; real rivers stay far below (30 m wide at most, R5).
const MAX_RADIUS: f64 = (HALO - BACKWATER) as f64;
/// Wall rounds: a cut that leaves a changed cell steeper than max(40°, raw) is halved, at
/// most this many Jacobi rounds; each round reaches 2 cells further, so the area carries a
/// margin of 2 per round (+ 1 for the slope stencil) and every tiling agrees.
pub(crate) const WALL_ROUNDS: u64 = 16;
/// The area's margin beyond the core: backwater and the wall rounds.
const MARGIN: u64 = BACKWATER + 2 * WALL_ROUNDS + 1;

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
/// deepest lowering below raw.
type Figure = (u64, f32, f64, u64, f32);
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
                    return wk.cached(&format!("{name}-figures"), || Ok((0, 0.0, 0.0, 0, 0.0)));
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
            for a in 0..n {
                let (x, y) = area.world_xy(a);
                let j = win.local(x, y).expect("area in window");
                carved[a] = g[j];
                ids[a] = bw[j];
                if node[j] == 1 || is_lake(j) {
                    surface[a] = level[j];
                    continue;
                }
                let ranked: Vec<(usize, u32)> = best[a]
                    .iter()
                    .filter(|c| c.2 != u32::MAX)
                    .map(|c| (c.2 as usize, c.1))
                    .collect();
                let own = (thalweg(j) && ranked.iter().all(|c| c.0 != j)).then_some((j, 0));
                for (t, d2) in ranked.into_iter().chain(own) {
                    let rs = river(t);
                    let below_a_lake = (0..8).any(|k| {
                        world
                            .neighbour(x, y, k)
                            .and_then(|(nx, ny)| win.local(nx, ny))
                            .is_some_and(|j2| is_lake(j2) && level[j2] > rs)
                    });
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
                    if cut < rs || t == j {
                        lowered[a] = cut < g[j];
                        carved[a] = cut;
                        surface[a] = rs;
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
            // Walls: no changed cell may end steeper than max(40°, raw) (the `walls` check).
            // The cut of every lowered cell in the 3 × 3 of such a cell is halved, Jacobi, at
            // most WALL_ROUNDS rounds.
            let in_area = |a: usize| {
                let (x, y) = area.world_xy(a);
                win.local(x, y).expect("area in window")
            };
            let (ga, ra): (Vec<f32>, Vec<f32>) =
                (0..n).map(|a| (g[in_area(a)], rw[in_area(a)])).unzip();
            let ring = |a: usize| {
                let (lx, ly) = (a % area.w, a / area.w);
                lx == 0 || ly == 0 || lx + 1 == area.w || ly + 1 == area.h
            };
            let nbrs = |a: usize| {
                let (lx, ly) = ((a % area.w) as i64, (a / area.w) as i64);
                (-1i64..=1)
                    .flat_map(move |dy| (-1i64..=1).map(move |dx| (lx + dx, ly + dy)))
                    .filter(move |&(x, y)| {
                        x >= 0 && y >= 0 && (x as usize) < area.w && (y as usize) < area.h
                    })
                    .map(move |(x, y)| y as usize * area.w + x as usize)
            };
            // Only cells next to a cut can be steepened by it.
            let mut near_cut: Vec<usize> = (0..n)
                .filter(|&a| !ring(a) && nbrs(a).any(|b| carved[b] < ga[b]))
                .collect();
            for _ in 0..WALL_ROUNDS {
                let steep: Vec<usize> = near_cut
                    .iter()
                    .copied()
                    .filter(|&a| {
                        let (lx, ly) = (a % area.w, a / area.w);
                        (carved[a] - ra[a]).abs() > CHANGED_M
                            && is_wall(
                                horn_deg(&carved, area.w, lx, ly, s.spacing_m),
                                horn_deg(&ra, area.w, lx, ly, s.spacing_m),
                            )
                    })
                    .collect();
                let mut shrink: Vec<usize> = steep
                    .iter()
                    .flat_map(|&a| nbrs(a))
                    .filter(|&b| carved[b] < ga[b])
                    .collect();
                if shrink.is_empty() {
                    break;
                }
                shrink.sort_unstable();
                shrink.dedup();
                for b in shrink {
                    let cut = (ga[b] - carved[b]) / 2.0;
                    carved[b] = if cut < 1e-3 { ga[b] } else { ga[b] - cut };
                    // A section cell no longer under its river's surface is not cut at all
                    // (it turns dry below).
                    if is_river[b] && !keep[b] && carved[b] >= surface[b] {
                        carved[b] = ga[b];
                    }
                }
                near_cut.retain(|&a| nbrs(a).any(|b| carved[b] < ga[b]));
            }
            for a in 0..n {
                lowered[a] = carved[a] < ga[a];
                // A section cell its smaller cut leaves above the river's surface is dry again.
                if is_river[a] && !keep[a] && carved[a] >= surface[a] {
                    (surface[a], ids[a], is_river[a]) = (f32::NAN, bw[in_area(a)], false);
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
            let mut fig: Figure = (0, 0.0, 0.0, 0, 0.0);
            for k in 0..core.cells() {
                let (x, y) = core.world_xy(k);
                let j = win.local(x, y).expect("core in window");
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
            if let Some(wk) = work {
                wk.cached(&format!("{name}-figures"), || Ok(fig))?;
                wk.mark_done(&name)?;
            }
            Ok(fig)
        })?;
    Ok(json!({
        "lowered_cells": parts.iter().map(|p| p.0).sum::<u64>(),
        "max_lowering_m": parts.iter().map(|p| p.1).fold(0.0f32, f32::max),
        "max_below_raw_m": parts.iter().map(|p| p.4).fold(0.0f32, f32::max),
        "volume_m3": parts.iter().map(|p| p.2).sum::<f64>().round(),
        "river_cells": parts.iter().map(|p| p.3).sum::<u64>(),
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
/// at. Each cell takes the highest labelled water on its path downstream when that lies
/// above it: a dry or pond cell is wetted as that water's body, a wet one raised. Labels and
/// surfaces only. Exact over tiles: per-tile path maxima, then the paths across tile rings.
pub(crate) fn settle(
    s: &Setup,
    r3: &Rasters,
    ground: &Layer<f32>,
    out: &Layers,
    work: Option<&Work>,
    tag: &str,
) -> Result<(u64, u64, f32)> {
    let world = s.world;
    let tiles = s.tiles();
    let read = |core: Rect| -> Result<(Vec<u8>, Vec<Drained>)> {
        let (sf, b) = (out.surface.read(core)?, out.body.read(core)?);
        let (lv, z) = (r3.level.read(core)?, ground.read(core)?);
        let d0 = (0..core.cells())
            .map(|k| {
                if sf[k].is_finite() {
                    (sf[k], b[k])
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
    let parts: Vec<(u64, u64, f32)> =
        for_tiles(&tiles, s.threads, &format!("{tag} settle 2"), |i, core| {
            let name = format!("{tag}/s2/{i:05}");
            if let Some(wk) = work {
                if wk.is_done(&name) {
                    return wk.cached(&format!("{name}-figures"), || Ok((0, 0, 0.0)));
                }
            }
            let (flow, d0) = read(core)?;
            let pm = path_max(world, core, &flow, &d0)?;
            let (mut sf, mut b) = (out.surface.read(core)?, out.body.read(core)?);
            let mut fig = (0u64, 0u64, 0f32);
            for k in 0..core.cells() {
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
                // Only labelled water wets a cell (a pond or dry source has no body).
                if top.0 > d0[k].0 && top.1 >= 1 {
                    fig.0 += 1;
                    fig.1 += u64::from(!sf[k].is_finite());
                    fig.2 = fig.2.max(top.0 - d0[k].0);
                    (sf[k], b[k]) = (top.0, top.1);
                }
            }
            if fig.0 > 0 {
                out.surface.write(core, &sf)?;
                out.body.write(core, &b)?;
            }
            if let Some(wk) = work {
                wk.cached(&format!("{name}-figures"), || Ok(fig))?;
                wk.mark_done(&name)?;
            }
            Ok(fig)
        })?;
    Ok((
        parts.iter().map(|p| p.0).sum(),
        parts.iter().map(|p| p.1).sum(),
        parts.iter().map(|p| p.2).fold(0.0, f32::max),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::water::bodies::label;
    use crate::world::water::geom::OFFSETS;
    use crate::world::water::pipeline::model;
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
        channels(&s, &m.water, &raw, &raw, &r, &body, &out, &geo, None, "c").expect("channels");
        if let Some(v) = rasters {
            *v = (r.node.to_vec().unwrap(), r.level.to_vec().unwrap());
            // Pond cells are no lake nodes.
            assert!(v.0.iter().all(|&n| n < 2 || !m.water.nodes.at(n - 2).lake));
        }
        (
            raw.to_vec().unwrap(),
            out.carved.to_vec().unwrap(),
            out.surface.to_vec().unwrap(),
            out.body.to_vec().unwrap(),
            r.flow.to_vec().unwrap(),
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
                assert!(surface[j] <= surface[k] + 1e-5, "rises from {k} to {j}");
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

    /// A V valley with 1:1 walls: each wall cell loses at most the centre cut (the taper of
    /// the thalweg's cut), never its height above the river (Task 11 lesson).
    #[test]
    fn steep_walls_lose_no_more_than_the_centre_cut() {
        let world = World { w: 128, h: 41 };
        let z: Vec<f32> = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
                40.0 - 0.05 * x + (y - 20.0).abs() + ((i * 7919) % 7) as f32 * 0.001
            })
            .collect();
        // About 5 m wide and 0.4 m deep where the valley leaves the map.
        let m3s = 1e3;
        let (raw, carved, surface, _, flow) = run_on(world, z, 32, m3s, 1e-3);
        let geo = Geometry::new(m3s);
        let (s, r) = (setup(world, 32, 1e-3), Rasters::memory(world));
        model(&s, &Layer::memory(world, raw.clone()), None, "m", Some(&r)).expect("model");
        let qmax = r
            .discharge
            .to_vec()
            .unwrap()
            .into_iter()
            .fold(0f32, f32::max);
        let dmax = geo.depth_m(f64::from(qmax));
        assert!(dmax < 1.0 && geo.width_m(f64::from(qmax)) > 4.0);
        let mut bed = 0;
        for k in 0..raw.len() {
            let drop = raw[k] - carved[k];
            assert!(
                drop <= dmax + 1e-4,
                "cut {drop} m at {k}, deeper than the centre"
            );
            if drop > 0.0 {
                assert!(carved[k] < surface[k]);
                bed += usize::from(flow[k] & 16 != 0);
            }
            // A wall cell a metre or more above the river would end above its surface.
            if (k / 128).abs_diff(20) >= 2 {
                assert_eq!(drop, 0.0, "a wall cell was cut at {k}");
            }
        }
        assert!(bed > 50, "the valley floor carries a channel");
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
            assert_eq!(surface[k], level[k], "pond cell {k} at the level");
            assert_eq!(body[k], body[cells[0]], "pond cell {k} in one body");
        }
        assert!(body[cells[0]] > 1);
    }

    /// No cut leaves a changed cell steeper than max(40°, raw) (the `walls` check): the
    /// synthetic world's rivers run down 42-47° slopes.
    #[test]
    fn no_cut_leaves_a_wall() {
        let (raw, carved, ..) = run(32);
        let (w, mut walls) = (256usize, 0);
        for y in 1..w - 1 {
            for x in 1..w - 1 {
                let k = y * w + x;
                if (carved[k] - raw[k]).abs() > CHANGED_M
                    && is_wall(
                        horn_deg(&carved, w, x, y, 1.0),
                        horn_deg(&raw, w, x, y, 1.0),
                    )
                {
                    walls += 1;
                }
            }
        }
        assert_eq!(walls, 0);
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
            let fig = settle(&s, &r3, &ground, &out, None, "s").expect("settle");
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
