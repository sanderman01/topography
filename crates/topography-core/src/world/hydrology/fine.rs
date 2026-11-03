//! Finer levels hold the coarse network fixed. Coarse lakes keep their id and level on the
//! fine ground; each coarse river gets a fine centreline inside a corridor of one coarse cell
//! around it, found by a priority flood over that corridor, burned into the ground so the fine
//! routing follows it, and later carved as a channel with the coarse discharge below a surface
//! no higher than the coarse one.

use super::coarse::Coarse;
use super::flood::{self, CENTRE, Dims, FIXED, OUTLET, RIVER, SEA};
use super::lakes::{Basin, Kind, NO_EXIT};
use serde_json::{Map, Value, json};
use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, HashSet},
};

/// Judgment constants: a coarse lake with no fine ground below its level keeps one cell this
/// far below it; a sill that stops a lake leaking stands this far above the level.
pub(super) const LAKE_BED_M: f32 = 0.25;
pub(super) const SHORE_M: f32 = 0.05;
/// Fine water of a coarse body may stand this far above the coarse surface (f32 rounding).
pub(super) const SURFACE_TOLERANCE_M: f32 = 0.01;

/// Coarse lakes on the fine ground: one lake basin per coarse lake in the window, in `basins`
/// order with the coarse id. Wet where the fine ground is below the level, connected to fine
/// cells below the level inside the coarse lake's cells (seeds; a lake with none keeps its
/// lowest cell, lowered to just below the level). The region grows as far as the ground stays
/// below the level, unless it leaks to the sea, the world's edge, the grid's edge or a lake
/// with another level: then the leaking branch is cut more than one coarse cell out from the
/// seeds by a sill raised to the level plus `SHORE_M` (a leak nearer the seeds is raised where
/// it is, but not on a band edge). `fixed_lake` gives the lake id and level of cells copied
/// from the previous band (seeds too); that band's lakes the coarse level lacks (made at this
/// level, such as a river sink filled to its spill) go on here the same way. Coarse lake cells
/// whose fine ground is above the level stay dry.
pub(super) fn lakes(
    dims: Dims,
    ground: &mut [f32],
    flags: &[u8],
    body: &mut [u32],
    basins: &mut Vec<Basin>,
    c: &Coarse,
    fixed_lake: impl Fn(usize) -> Option<(u32, f32)>,
) -> Value {
    let (n, w, h) = (ground.len(), dims.w, dims.h);
    let mut ids: Vec<u32> = (0..c.body.len()).filter_map(|q| c.lake(q)).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut taken: Vec<(u32, f32)> = (0..n)
        .filter(|&i| flags[i] & FIXED != 0)
        .filter_map(&fixed_lake)
        .filter(|(id, _)| !c.lakes.contains_key(id))
        .collect();
    taken.sort_unstable_by_key(|&(id, _)| id);
    taken.dedup_by_key(|&mut (id, _)| id);
    let fixed_lake = |i: usize| fixed_lake(i).map(|(id, _)| id);
    let mut index: HashMap<u32, u32> = HashMap::new();
    let first = basins.len() as u32;
    let levels = ids
        .iter()
        .map(|id| (*id, c.lakes[id].level))
        .chain(taken.iter().copied());
    for (id, level) in levels.collect::<Vec<_>>() {
        let mut basin = Basin::kept(Kind::Lake, level, NO_EXIT);
        basin.over_budget = false;
        basin.id = id;
        basins.push(basin);
        index.insert(id, basins.len() as u32);
    }
    let level_of = |b: u32| basins[b as usize - 1].level;
    let free = |flags: &[u8], i: usize| flags[i] & (SEA | FIXED) == 0;
    // A lake with no fine ground below its level keeps its lowest cell, just below.
    let mut lowest: HashMap<u32, usize> = HashMap::new();
    let mut seeded: HashSet<u32> = HashSet::new();
    for i in 0..n {
        let id = if flags[i] & FIXED != 0 {
            fixed_lake(i).filter(|id| index.contains_key(id))
        } else {
            c.lake(c.cell(w, i)).filter(|_| free(flags, i))
        };
        let Some(id) = id else { continue };
        if flags[i] & FIXED != 0 || ground[i] < c.lakes[&id].level {
            seeded.insert(id);
        } else {
            let e = lowest.entry(id).or_insert(i);
            if ground[i] < ground[*e] {
                *e = i;
            }
        }
    }
    let (mut restored, mut bed_max, mut bed_at) = (0u64, 0f32, 0usize);
    let mut lowest: Vec<(u32, usize)> = lowest.into_iter().collect();
    lowest.sort_unstable();
    for (id, i) in lowest {
        if !seeded.contains(&id) {
            let to = c.lakes[&id].level - LAKE_BED_M;
            if ground[i] - to > bed_max {
                (bed_max, bed_at) = (ground[i] - to, i);
            }
            ground[i] = ground[i].min(to);
            restored += 1;
        }
    }
    let seed_of = |flags: &[u8], ground: &[f32], i: usize| -> Option<u32> {
        if flags[i] & FIXED != 0 {
            return fixed_lake(i).and_then(|id| index.get(&id).copied());
        }
        c.lake(c.cell(w, i))
            .filter(|id| free(flags, i) && ground[i] < c.lakes[id].level)
            .map(|id| index[&id])
    };
    let reach = c.k as u32;
    // Coarse river cells are not lake: the river keeps its course (beside the lake, a shore).
    let on_river = |j: usize| c.river(c.cell(w, j)).is_some();
    let mut depth = vec![u32::MAX; n];
    // Leaking ground cut off from a lake: the cutting level; kept out of lakes as high.
    let mut cut = vec![f32::INFINITY; n];
    let (mut sills, mut sill_max, mut sill_at, mut removed) = (0u64, 0f32, Vec::new(), 0u64);
    let mut queue = std::collections::VecDeque::new();
    let mut raise = |ground: &mut [f32], i: usize, top: f32| {
        sills += 1;
        sill_max = sill_max.max(top - ground[i]);
        if sill_at.len() < 3 {
            sill_at.push([i % w, i / w]);
        }
        ground[i] = top;
    };
    // Growth stops this far from the seeds; a lake still growing there counts as leaking.
    let bound = 4 * reach;
    let levels: Vec<f32> = basins.iter().map(|b| b.level).collect();
    let grows = |body: &[u32], ground: &[f32], cut: &[f32], b: u32, j: usize| {
        body[j] == 0
            && free(flags, j)
            && ground[j] < level_of(b)
            && !on_river(j)
            && level_of(b) < cut[j]
    };
    let mut leaks_left = 0usize;
    for round in 0..=8 {
        // Grow every lake breadth first from its seeds, so `depth` is the fine distance.
        for i in 0..n {
            if body[i] > first {
                body[i] = 0;
            }
            depth[i] = u32::MAX;
            if let Some(b) = seed_of(flags, ground, i) {
                body[i] = b;
                depth[i] = 0;
                queue.push_back(i as u32);
            }
        }
        while let Some(a) = queue.pop_front() {
            let a = a as usize;
            let b = body[a];
            if depth[a] >= bound {
                continue;
            }
            for (_, j) in dims.neighbours(a) {
                if grows(body, ground, &cut, b, j) {
                    body[j] = b;
                    depth[j] = depth[a] + 1;
                    queue.push_back(j as u32);
                }
            }
        }
        // Leaks: water beside the sea, at an edge, beside a lake with a lower level, or still
        // growing at the bound.
        let mut leaks = leak_cells(dims, flags, body, first, &levels, |_| true);
        leaks.extend((0..n).filter(|&i| {
            body[i] > first
                && depth[i] == bound
                && dims
                    .neighbours(i)
                    .any(|(_, j)| grows(body, ground, &cut, body[i], j))
        }));
        if round == 8 || leaks.is_empty() {
            // Leaks left (a band edge excepted: the lake may go on there), bound included.
            leaks_left = leaks
                .iter()
                .filter(|&&i| !dims.is_edge(i) || c.world_edge(w, h, i))
                .count();
            break;
        }
        // Cut each leaking branch at its narrowest layer between `reach` and twice that out
        // from the seeds; a leak nearer is raised where it is (not on a band edge: the lake
        // may go on there).
        let mut stack: Vec<usize> = Vec::new();
        let mut branch = vec![false; n];
        let mut acted = false;
        for &i in &leaks {
            if depth[i] > reach {
                if !branch[i] {
                    branch[i] = true;
                    stack.push(i);
                }
            } else if !dims.is_edge(i) || c.world_edge(w, h, i) {
                raise(ground, i, level_of(body[i]) + SHORE_M);
                acted = true;
            }
        }
        while let Some(a) = stack.pop() {
            for (_, j) in dims.neighbours(a) {
                if !branch[j] && body[j] == body[a] && depth[j] > reach {
                    branch[j] = true;
                    stack.push(j);
                }
            }
        }
        // Narrowest layer of each lake's leaking branches, nearer than its nearest leak.
        let mut nearest: HashMap<u32, u32> = HashMap::new();
        for &i in leaks.iter().filter(|&&i| depth[i] > reach) {
            let e = nearest.entry(body[i]).or_insert(u32::MAX);
            *e = (*e).min(depth[i]);
        }
        let mut width: HashMap<(u32, u32), u32> = HashMap::new();
        for i in (0..n).filter(|&i| branch[i] && depth[i] <= 2 * reach) {
            if depth[i] < nearest.get(&body[i]).copied().unwrap_or(u32::MAX) {
                *width.entry((body[i], depth[i])).or_insert(0) += 1;
            }
        }
        let mut layer: HashMap<u32, (u32, u32)> = HashMap::new();
        for (&(b, d), &cells) in &width {
            let e = layer.entry(b).or_insert((u32::MAX, reach));
            if (cells, d) < *e {
                *e = (cells, d);
            }
        }
        let layer_of = |b: u32| layer.get(&b).map_or(reach, |&(_, d)| d);
        for i in 0..n {
            if branch[i] && depth[i] <= layer_of(body[i]) {
                branch[i] = false;
            } else if branch[i] {
                if cut[i].is_infinite() {
                    removed += 1;
                }
                cut[i] = level_of(body[i]);
                acted = true;
            }
        }
        for i in 0..n {
            if body[i] > first
                && depth[i] == layer_of(body[i])
                && dims.neighbours(i).any(|(_, j)| branch[j])
            {
                raise(ground, i, level_of(body[i]) + SHORE_M);
                acted = true;
            }
        }
        if !acted {
            // Leaks left (a band edge excepted: the lake may go on there), bound included.
            leaks_left = leaks
                .iter()
                .filter(|&&i| !dims.is_edge(i) || c.world_edge(w, h, i))
                .count();
            break;
        }
    }
    drop(depth);
    // One piece per lake: the largest, and any that may join it beyond the grid (fixed
    // cells or the top and bottom rows); a smaller piece cut off by fine ground above the
    // level stays dry.
    let mut stack: Vec<u32> = Vec::new();
    let mut piece = vec![0u32; n];
    let mut best: HashMap<u32, (u64, u32)> = HashMap::new();
    let mut open: HashSet<u32> = HashSet::new();
    let mut count = 0u32;
    for s in 0..n {
        if body[s] <= first || piece[s] != 0 {
            continue;
        }
        count += 1;
        let (b, mut size, mut edge) = (body[s], 0u64, false);
        piece[s] = count;
        stack.push(s as u32);
        while let Some(a) = stack.pop() {
            let a = a as usize;
            size += 1;
            edge |= flags[a] & FIXED != 0 || a < w || a >= n - w;
            for (_, j) in dims.neighbours(a) {
                if body[j] == b && piece[j] == 0 {
                    piece[j] = count;
                    stack.push(j as u32);
                }
            }
        }
        if edge {
            open.insert(count);
        }
        let e = best.entry(b).or_insert((0, 0));
        if size > e.0 {
            *e = (size, count);
        }
    }
    let mut dropped = 0u64;
    for i in 0..n {
        let p = piece[i];
        if p != 0 && !open.contains(&p) && best[&body[i]].1 != p && flags[i] & FIXED == 0 {
            body[i] = 0;
            dropped += 1;
        }
    }
    // Shore: dry ground beside a lake stays above its level (a coarse river's cells by a thin
    // margin, so the lake leaves only by its exit).
    let (mut shore, mut shore_max) = (0u64, 0f32);
    for i in 0..n {
        if body[i] <= first || flags[i] & SEA != 0 {
            continue;
        }
        let level = level_of(body[i]);
        for (_, j) in dims.neighbours(i) {
            let top = if on_river(j) { level + SHORE_M } else { level };
            if body[j] == 0 && free(flags, j) && ground[j] < top {
                shore += 1;
                shore_max = shore_max.max(level + SHORE_M - ground[j]);
                ground[j] = level + SHORE_M;
            }
        }
    }
    json!({
        "coarse_lakes": ids.len(),
        "lakes_from_previous_band": taken.len(),
        "lakes_given_one_cell": restored,
        "bed_max_lowering_m": bed_max,
        "bed_max_lowering_at_cell": [bed_at % w, bed_at / w],
        "cut_off_cells_left_dry": dropped,
        "leaking_cells_cut_off": removed,
        "leaks_left": leaks_left,
        "sill_cells_raised": sills,
        "sill_max_raise_m": sill_max,
        "sill_at_cells": sill_at,
        "shore_cells_raised": shore,
        "shore_max_raise_m": shore_max,
    })
}

/// Coarse lake cells (basin index above `first`, levels by index − 1) that leak: beside the
/// sea while above 0 m, beside a lake with a lower level, or at an edge where `edge` holds.
pub(super) fn leak_cells(
    dims: Dims,
    flags: &[u8],
    body: &[u32],
    first: u32,
    levels: &[f32],
    edge: impl Fn(usize) -> bool,
) -> Vec<usize> {
    let level = |b: u32| levels[b as usize - 1];
    (0..body.len())
        .filter(|&i| {
            let b = body[i];
            b > first
                && flags[i] & FIXED == 0
                && ((dims.is_edge(i) && edge(i))
                    || dims.neighbours(i).any(|(_, j)| {
                        (flags[j] & SEA != 0 && level(b) > 0.0)
                            || (body[j] > first && level(body[j]) < level(b))
                    }))
        })
        .collect()
}

/// Coarse-consistency checks (each must be 0): every coarse lake with fine cells here present
/// at the same level, every coarse river present and routed, no fine water of a coarse river
/// above its coarse surface (the highest over its coarse cell and the 8 around it, or its
/// path's cap where higher: the interpolation along the path, or a coarse lake's level beside
/// it, or sea level for a river ending in the sea) by more than the tolerance.
#[allow(clippy::too_many_arguments)]
pub(super) fn consistency(
    dims: Dims,
    flags: &[u8],
    surface: &[f32],
    basins: &[Basin],
    c: &Coarse,
    unrouted: &[u32],
    core_rows: [usize; 2],
    cap: &HashMap<u32, f32>,
    body_id: &dyn Fn(usize) -> u32,
    to_sea: &HashSet<u32>,
) -> (Map<String, Value>, Value) {
    let (w, h) = (dims.w, dims.h);
    let mut present: HashSet<u32> = HashSet::new();
    let mut above = 0u64;
    let (mut above_max, mut above_at) = (0f32, Vec::new());
    for i in 0..w * h {
        let id = body_id(i);
        if id == 0 {
            continue;
        }
        present.insert(id);
        if flags[i] & (FIXED | CENTRE) != CENTRE || !c.rivers.contains_key(&id) {
            continue;
        }
        let [cx, cy] = c.abs(c.cell(w, i));
        let top = (-1..=1)
            .flat_map(|dy| (-1..=1).map(move |dx| [cx as isize + dx, cy as isize + dy]))
            .filter_map(|p| c.at(p))
            .filter(|&q| c.body[q] == id)
            .map(|q| c.surface[q])
            .fold(f32::NEG_INFINITY, f32::max)
            .max(cap.get(&(i as u32)).copied().unwrap_or(f32::NEG_INFINITY));
        // Beside a coarse lake a river stands at least at its level (as `rivers::extract`
        // raises it): that level is coarse too.
        let beside = dims
            .neighbours(i)
            .filter_map(|(_, j)| c.lakes.get(&body_id(j)))
            .map(|l| l.level)
            .fold(f32::NEG_INFINITY, f32::max);
        // A river reaching the sea here (`to_sea`) never stands below sea level (it rises to
        // the sea it flows into), though its coarse surface may (a coastal hollow).
        let sea = if to_sea.contains(&id) {
            0.0
        } else {
            f32::NEG_INFINITY
        };
        let top = if top.is_finite() {
            top.max(beside).max(sea)
        } else {
            top
        };
        if top.is_finite() && surface[i] > top + SURFACE_TOLERANCE_M {
            above += 1;
            above_max = above_max.max(surface[i] - top);
            if above_at.len() < 3 {
                above_at.push([i % w, i / w]);
            }
        }
    }
    // Coarse bodies with cells this run makes.
    let mut lakes_here: HashSet<u32> = HashSet::new();
    let mut rivers_here: HashSet<u32> = HashSet::new();
    for q in 0..c.body.len() {
        let free = c.fine_cells(q, w, h).any(|i| flags[i] & (FIXED | SEA) == 0);
        if !free {
            continue;
        }
        if let Some(id) = c.lake(q) {
            lakes_here.insert(id);
        }
        // A river is due only where it has free cells in the core rows, as for unrouted
        // heads: a halo is made again by the next band.
        let in_core = || {
            c.fine_cells(q, w, h).any(|i| {
                flags[i] & (FIXED | SEA) == 0 && (core_rows[0]..core_rows[1]).contains(&(i / w))
            })
        };
        if let Some(id) = c.river(q).filter(|_| c.centre(q) && in_core()) {
            rivers_here.insert(id);
        }
    }
    let lakes_missing = lakes_here.iter().filter(|id| !present.contains(id)).count();
    let levels = basins
        .iter()
        .filter(|b| b.kind == Kind::Lake && c.lakes.get(&b.id).is_some_and(|l| l.level != b.level))
        .count();
    let mut lost: HashSet<u32> = rivers_here
        .iter()
        .copied()
        .filter(|id| !present.contains(id))
        .collect();
    lost.extend(unrouted.iter().copied());
    let mut m = Map::new();
    m.insert("coarse_lakes_missing".into(), json!(lakes_missing));
    m.insert("guard_coarse_lake_levels_changed".into(), json!(levels));
    m.insert(
        "coarse_rivers_missing_or_unrouted".into(),
        json!(lost.len()),
    );
    m.insert("water_above_coarse_surface".into(), json!(above));
    let mut lost: Vec<u32> = lost.into_iter().collect();
    lost.sort_unstable();
    let figures = json!({
        "water_above_coarse_max_m": above_max,
        "water_above_coarse_at_cells": above_at,
        "coarse_rivers_missing_or_unrouted": lost,
    });
    (m, figures)
}

/// A coarse river's fine centreline: cells from a head down to `next` (the sea, lake or dry
/// basin cell it ends in, or none at a grid-edge outlet).
pub(super) struct Path {
    /// Index into `Imprint::ids`.
    pub river: usize,
    pub cells: Vec<u32>,
    pub next: Option<u32>,
    /// Lake basin (index + 1) this path leaves at its head, 0 for none.
    pub source: u32,
    /// Lowest bed per cell: the previous band's bed it ends on and the seam limit downstream.
    pub floors: Vec<f32>,
}

pub(super) struct Imprint {
    /// Coarse river ids in the order of `Path::river`.
    pub ids: Vec<u32>,
    pub paths: Vec<Path>,
    /// Surface cap (the coarse surface interpolated along the path) and coarse discharge.
    pub cap: HashMap<u32, f32>,
    pub q: HashMap<u32, f32>,
    /// Coarse rivers with a head the corridor flood did not reach.
    pub unrouted: Vec<u32>,
    /// Cells the first burn lowered (they widen as breach trenches do).
    pub burned: Vec<u32>,
    pub burn_max_m: f32,
    pub burn_max_at: [usize; 2],
    /// Path cells filled to the previous band's bed they end on.
    pub fill: Fill,
    /// Heads whose path walk met a receiver loop or a cell without a receiver (unrouted).
    pub receiver_loops: u64,
    /// Paths laid again along the way that climbs least (`PATH_CLIMB_M`).
    pub relaid: u64,
    /// Grid edge cells that become outlets where a coarse river leaves the grid.
    pub outlets: Vec<u32>,
}

#[derive(PartialEq)]
struct Item(f32, u32);
impl Eq for Item {}
impl Ord for Item {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
    }
}
impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// Min-heap item of a lowest way: the highest ground so far, then the cell's own ground (on a
/// way below its start, down the fall line rather than by index), then the cell.
#[derive(PartialEq)]
struct Way(f32, f32, u32);
impl Eq for Way {}
impl Ord for Way {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0)
            .then(o.1.total_cmp(&self.1))
            .then(o.2.cmp(&self.2))
    }
}
impl PartialOrd for Way {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// A head entering from the previous band: the free cell a fixed centreline of coarse river
/// `id` drains into, with that cell's surface and discharge.
pub(super) struct Inflow {
    pub cell: u32,
    pub id: u32,
    pub surface: f32,
    pub q: f32,
}

/// Highest ground a path cell must keep beside a lake (its level plus `SHORE_M`; the lake the
/// path leaves, `source`, only its level).
fn floor(dims: Dims, body: &[u32], basins: &[Basin], source: u32, i: usize) -> f32 {
    dims.neighbours(i)
        .filter(|&(_, j)| body[j] != 0 && basins[body[j] as usize - 1].kind == Kind::Lake)
        .map(|(_, j)| {
            let l = basins[body[j] as usize - 1].level;
            if body[j] == source { l } else { l + SHORE_M }
        })
        .fold(f32::NEG_INFINITY, f32::max)
}

/// Path cells filled above their ground to the path's floors (`Path::floors`): most and count.
#[derive(Clone, Copy, Default)]
pub(super) struct Fill {
    pub max_m: f32,
    pub cells: u64,
}

/// Lower each path to a running minimum from its head and to its surface cap (not below 0 m
/// or a lake floor beside it), then let each cut taper off upstream at the trench side slope
/// instead of ending in a cliff (still never rising downstream). Returns the cells lowered and
/// the deepest cut below the ground the path had and where, and the fill to `Path::floor`.
pub(super) fn burn(
    dims: Dims,
    ground: &mut [f32],
    body: &[u32],
    basins: &[Basin],
    paths: &[Path],
    cap: &HashMap<u32, f32>,
    spacing_m: f64,
) -> (Vec<u32>, f32, usize, Fill) {
    let tan = super::erosion::SIDE_SLOPE_DEG.to_radians().tan();
    let (mut cells, mut max, mut at) = (Vec::new(), 0f32, 0usize);
    let mut fill = Fill::default();
    let mut seen: HashSet<u32> = HashSet::new();
    for p in paths {
        let before: Vec<f32> = p.cells.iter().map(|&c| ground[c as usize]).collect();
        let mut g = f32::INFINITY;
        for &c in &p.cells {
            let f = floor(dims, body, basins, p.source, c as usize);
            let top = cap.get(&c).copied().unwrap_or(f32::INFINITY);
            let c = c as usize;
            // Not below 0 m: channels cut mouths below sea level themselves.
            g = ground[c].min(g.min(top).max(f).max(0.0));
            ground[c] = g;
        }
        // The cut tapers upstream: a cell's cut is at least the next one's less the slope's
        // rise over the step, without rising above the next cell.
        for k in (0..p.cells.len().saturating_sub(1)).rev() {
            let (c, d) = (p.cells[k] as usize, p.cells[k + 1] as usize);
            let cut = before[k + 1] - ground[d];
            let step = Dims::step(dims.towards(c, d)) * spacing_m;
            let target = before[k] - (cut - tan * step as f32);
            let f = floor(dims, body, basins, p.source, c);
            let to = ground[c].min(target.max(ground[d]).max(f).max(0.0));
            ground[c] = ground[c].min(to);
        }
        // Not below the path's floors (the bed it ends on, the carve floor downstream): a cut
        // stops there and a hollow below them is filled (counted).
        for (k, &c) in p.cells.iter().enumerate() {
            let c = c as usize;
            if p.floors[k] > ground[c] {
                ground[c] = p.floors[k];
                let above = p.floors[k] - before[k];
                if above > 0.0 {
                    fill.max_m = fill.max_m.max(above);
                    fill.cells += 1;
                }
            }
        }
        for (k, &c) in p.cells.iter().enumerate() {
            let cut = before[k] - ground[c as usize];
            if cut > 0.0 && seen.insert(c) {
                cells.push(c);
            }
            if cut > max {
                (max, at) = (cut, c as usize);
            }
        }
    }
    (cells, max, at, fill)
}

/// Point each path cell at the next. A path ending where its flow comes back (a dry basin
/// whose exit is the path's last cell) would close a cycle: that cell outside the path spills
/// elsewhere. Returns the cells spilled.
pub(super) fn route(
    dims: Dims,
    ground: &[f32],
    flags: &[u8],
    dir: &mut [u8],
    paths: &[Path],
) -> u64 {
    let mut on_path = vec![false; dir.len()];
    for p in paths {
        for (k, &c) in p.cells.iter().enumerate() {
            on_path[c as usize] = true;
            let to = p.cells.get(k + 1).copied().or(p.next);
            if let Some(to) = to {
                dir[c as usize] = dims.towards(c as usize, to as usize);
            }
        }
    }
    flood::spill_cycles(dims, ground, flags, dir, |i| !on_path[i])
}

/// Judgment constant: rows beside the seam a coarse path keeps off, and columns either side of
/// where its river crosses the seam where it may run there (`rivers`).
const SEAM_STRIP: usize = 16;

/// Judgment constant: a path whose ground rises more than this above its lowest point upstream
/// is laid again along the way that climbs least.
const PATH_CLIMB_M: f32 = 5.0;

/// How far a path's ground rises above its lowest point upstream.
fn climb(ground: &[f32], cells: &[u32]) -> f32 {
    let mut low = f32::INFINITY;
    let mut most = 0f32;
    for &c in cells {
        let g = ground[c as usize];
        low = low.min(g);
        most = most.max(g - low);
    }
    most
}

/// The way from `head` over free land (`open` cells, not water or fixed) to a cell beside
/// `end` that climbs least in total (rises along it summed, then length): its cells.
#[allow(clippy::too_many_arguments)]
fn route_descending(
    dims: Dims,
    ground: &[f32],
    flags: &[u8],
    body: &[u32],
    basins: &[Basin],
    head: usize,
    end: usize,
    open: impl Fn(usize) -> bool,
) -> Option<Vec<u32>> {
    // Cells a search may visit before it gives up; the length weight breaks ties.
    const MAX_CELLS: usize = 1 << 22;
    const PER_CELL: f32 = 1e-3;
    let free = |i: usize| {
        flags[i] & (SEA | FIXED) == 0
            && (body[i] == 0 || basins[body[i] as usize - 1].kind == Kind::Dry)
            && open(i)
    };
    let mut cost: HashMap<usize, f32> = HashMap::from([(head, 0.0)]);
    let mut from: HashMap<usize, usize> = HashMap::from([(head, head)]);
    let mut heap = BinaryHeap::from([Item(0.0, head as u32)]);
    while let Some(Item(k, c)) = heap.pop() {
        let c = c as usize;
        if k > cost[&c] {
            continue;
        }
        if dims.neighbours(c).any(|(_, j)| j == end) {
            let mut way = vec![c as u32];
            let mut x = c;
            while from[&x] != x {
                x = from[&x];
                way.push(x as u32);
            }
            way.reverse();
            return Some(way);
        }
        for (d, j) in dims.neighbours(c) {
            if !free(j) {
                continue;
            }
            let step = Dims::step(d as u8 + 1) as f32 * PER_CELL;
            let kj = k + (ground[j] - ground[c]).max(0.0) + step;
            if cost.get(&j).is_none_or(|&old| kj < old) {
                cost.insert(j, kj);
                from.insert(j, c);
                heap.push(Item(kj, j as u32));
            }
        }
        if cost.len() > MAX_CELLS {
            break;
        }
    }
    None
}

/// Judgment constant: cells a path into the previous band's water keeps from that band's dry
/// ground below the water (room for its channel and a bank).
const SEAM_CLEAR: isize = 4;

/// Rise a seam bank may stand below the water beside it (as the report's bank check).
const SEAM_BANK_M: f32 = super::report::BANK_TOLERANCE_M;

/// The lowest way (least highest ground) from `head` over free land, not through `avoid`
/// cells, to a cell beside an `end`: the way's cells and that end.
#[allow(clippy::too_many_arguments)]
fn route_round(
    dims: Dims,
    ground: &[f32],
    flags: &[u8],
    body: &[u32],
    basins: &[Basin],
    head: usize,
    end: impl Fn(usize) -> bool,
    avoid: impl Fn(usize) -> bool,
) -> Option<(Vec<u32>, u32)> {
    // Cells a search may visit before it gives up.
    const MAX_CELLS: usize = 1 << 22;
    let open = |i: usize| {
        flags[i] & (SEA | FIXED) == 0
            && (body[i] == 0 || basins[body[i] as usize - 1].kind == Kind::Dry)
            && !avoid(i)
    };
    let mut from: HashMap<usize, usize> = HashMap::from([(head, head)]);
    let mut heap = BinaryHeap::from([Way(ground[head], ground[head], head as u32)]);
    while let Some(Way(z, _, c)) = heap.pop() {
        let c = c as usize;
        for (_, j) in dims.neighbours(c) {
            if end(j) {
                let mut way = vec![c as u32];
                let mut x = c;
                while from[&x] != x {
                    x = from[&x];
                    way.push(x as u32);
                }
                way.reverse();
                return Some((way, j as u32));
            }
            if !from.contains_key(&j) && open(j) {
                from.insert(j, c);
                heap.push(Way(ground[j].max(z), ground[j], j as u32));
            }
        }
        if from.len() > MAX_CELLS {
            break;
        }
    }
    None
}

/// Route a head the corridor flood missed (`sources`: its free cells) by the lowest way
/// (least highest ground) over free land to the nearest end: a cell already routed (`recv`),
/// the sea, a lake, another river's path (`claimed`), the previous band's water whose flow
/// does not come back here unrouted (`back`), or a grid-edge outlet (or the bottom edge of
/// the halo from row `halo`). Where the previous band holds this river's own water (`own`, a
/// river flowing north out of the grid whose path that band laid off its coarse cells), the
/// way that climbs least to it or a routed cell comes first. The way joins `recv` and
/// `corridor`; ends outside the corridor join `seeds`, new edge outlets `outlets`.
#[allow(clippy::too_many_arguments)]
fn route_stranded(
    dims: Dims,
    ground: &[f32],
    flags: &[u8],
    body: &[u32],
    basins: &[Basin],
    sources: &[usize],
    claimed: impl Fn(usize) -> bool,
    back: &impl Fn(usize) -> Option<usize>,
    halo: usize,
    own: Option<&dyn Fn(usize) -> bool>,
    recv: &mut HashMap<u32, u32>,
    seeds: &mut HashSet<u32>,
    corridor: &mut HashSet<u32>,
    outlets: &mut Vec<u32>,
) {
    // Cells a search may visit before it gives up.
    const MAX_CELLS: usize = 1 << 22;
    let w = dims.w;
    let open = |i: usize| {
        flags[i] & (SEA | FIXED) == 0
            && (body[i] == 0 || basins[body[i] as usize - 1].kind == Kind::Dry)
    };
    let lake = |i: usize| {
        flags[i] & FIXED == 0 && body[i] != 0 && basins[body[i] as usize - 1].kind == Kind::Lake
    };
    // Never from a routed cell or seed: a new receiver there could close a loop.
    let starts: Vec<usize> = sources
        .iter()
        .copied()
        .filter(|&s| open(s) && !recv.contains_key(&(s as u32)) && !seeds.contains(&(s as u32)))
        .collect();
    let mut from: HashMap<usize, usize> = HashMap::new();
    // (last cell of the way, the end it drains into: None for an edge outlet)
    let mut end: Option<(usize, Option<usize>)> = None;
    // This river's own water first: the way that climbs least (rises summed, then length).
    if let Some(own) = own.filter(|_| !starts.is_empty()) {
        const PER_CELL: f32 = 1e-3;
        let mut cost: HashMap<usize, f32> = starts.iter().map(|&s| (s, 0.0)).collect();
        from.extend(starts.iter().map(|&s| (s, s)));
        let mut heap: BinaryHeap<Item> = starts.iter().map(|&s| Item(0.0, s as u32)).collect();
        let mut found = false;
        'own: while let Some(Item(k, c)) = heap.pop() {
            let c = c as usize;
            if k > cost[&c] {
                continue;
            }
            for (d, j) in dims.neighbours(c) {
                let mine = flags[j] & (FIXED | RIVER) == FIXED | RIVER
                    && own(j)
                    && back(j).is_none_or(|r| recv.contains_key(&(r as u32)));
                if mine || (!cost.contains_key(&j) && recv.contains_key(&(j as u32))) {
                    end = Some((c, Some(j)));
                    found = true;
                    break 'own;
                }
                if !open(j) || seeds.contains(&(j as u32)) {
                    continue;
                }
                let kj = k
                    + (ground[j] - ground[c]).max(0.0)
                    + Dims::step(d as u8 + 1) as f32 * PER_CELL;
                if cost.get(&j).is_none_or(|&old| kj < old) {
                    cost.insert(j, kj);
                    from.insert(j, c);
                    heap.push(Item(kj, j as u32));
                }
            }
            if cost.len() > MAX_CELLS {
                break;
            }
        }
        if !found {
            from.clear();
        }
    }
    let mut heap = BinaryHeap::new();
    if end.is_none() {
        for &s in &starts {
            from.insert(s, s);
            heap.push(Way(ground[s], ground[s], s as u32));
        }
    }
    'search: while let Some(Way(z, _, c)) = heap.pop() {
        let c = c as usize;
        let edge =
            dims.is_edge(c) && (flags[c] & OUTLET != 0 || (c / w + 1 == dims.h && c / w >= halo));
        if edge {
            end = Some((c, None));
            break;
        }
        for (_, j) in dims.neighbours(c) {
            if from.contains_key(&j) {
                continue;
            }
            let fixed_water = flags[j] & FIXED != 0
                && flags[j] & (RIVER | SEA) != 0
                && back(j).is_none_or(|r| recv.contains_key(&(r as u32)));
            // A seed (such as a dry basin mouth cell) ends the way: routed through, it would
            // take a receiver and close a loop with the chains already ending in it.
            if recv.contains_key(&(j as u32))
                || seeds.contains(&(j as u32))
                || flags[j] & SEA != 0
                || lake(j)
                || claimed(j)
                || fixed_water
            {
                end = Some((c, Some(j)));
                break 'search;
            }
            if open(j) {
                from.insert(j, c);
                heap.push(Way(ground[j].max(z), ground[j], j as u32));
            }
        }
        if from.len() > MAX_CELLS {
            break;
        }
    }
    let Some((last, into)) = end else { return };
    match into {
        Some(j) => {
            recv.insert(last as u32, j as u32);
            if !corridor.contains(&(j as u32)) {
                seeds.insert(j as u32);
            }
        }
        None => {
            recv.insert(last as u32, u32::MAX);
            seeds.insert(last as u32);
            if flags[last] & OUTLET == 0 {
                outlets.push(last as u32);
            }
        }
    }
    let mut x = last;
    corridor.insert(x as u32);
    while from[&x] != x {
        let p = from[&x];
        recv.insert(p as u32, x as u32);
        corridor.insert(p as u32);
        x = p;
    }
}

/// Fine centrelines of the coarse rivers (largest first). A river's corridor is the fine cells
/// of its coarse cells and their neighbours (not other coarse rivers or lakes, the sea, fixed
/// cells or earlier paths). A priority flood over it starts from where the coarse river ends
/// (the sea, its lake, a dry basin cell, or a grid-edge outlet in the corridor); each coarse
/// head (a centreline cell without a centreline donor in the window) starts its path at the
/// fine cell it holds whose path climbs least (then the lowest), beside the lake when the river
/// leaves one, and follows the flood
/// down. A head the flood cannot reach counts as unrouted only in the core rows (a halo is
/// made again by the next band). The surface cap is described where it is computed. `back`
/// gives the free cell a fixed cell's flow in the previous band comes back into.
#[allow(clippy::too_many_arguments)]
pub(super) fn rivers(
    dims: Dims,
    ground: &mut [f32],
    flags: &[u8],
    body: &[u32],
    basins: &[Basin],
    c: &Coarse,
    inflows: &[Inflow],
    fixed_id: impl Fn(usize) -> u32,
    fixed_surface: impl Fn(usize) -> f32,
    path_floor: impl Fn(usize) -> f32,
    back: impl Fn(usize) -> Option<usize>,
    core_rows: [usize; 2],
    spacing_m: f64,
) -> Imprint {
    let (w, h) = (dims.w, dims.h);
    let cd = c.dims();
    let mut cells_of: HashMap<u32, Vec<usize>> = HashMap::new();
    for q in 0..c.body.len() {
        if let Some(id) = c.river(q) {
            cells_of.entry(id).or_default().push(q);
        }
    }
    let mut ids: Vec<u32> = cells_of.keys().copied().collect();
    ids.sort_by(|a, b| {
        c.rivers[b]
            .drainage_km2
            .total_cmp(&c.rivers[a].drainage_km2)
            .then(a.cmp(b))
    });
    let lake_basin: HashMap<u32, u32> = basins
        .iter()
        .enumerate()
        .filter(|(_, b)| b.kind == Kind::Lake && b.id != 0)
        .map(|(k, b)| (b.id, k as u32 + 1))
        .collect();
    // Lake each coarse exit cell drains.
    let exits: HashMap<[usize; 2], u32> = c
        .lakes
        .iter()
        .filter_map(|(&id, l)| l.exit.map(|e| (e, id)))
        .collect();
    // Coarse cells of the margin around the grid hold no fine cells.
    let inside = |q: usize| c.fine_cells(q, w, h).next().is_some();
    let fully_fixed = |q: usize| inside(q) && c.fine_cells(q, w, h).all(|i| flags[i] & FIXED != 0);
    let mut claimed: HashMap<u32, u32> = HashMap::new();
    let mut out = Imprint {
        ids: Vec::new(),
        paths: Vec::new(),
        cap: HashMap::new(),
        q: HashMap::new(),
        unrouted: Vec::new(),
        burned: Vec::new(),
        burn_max_m: 0.0,
        burn_max_at: [0, 0],
        fill: Fill::default(),
        receiver_loops: 0,
        relaid: 0,
        outlets: Vec::new(),
    };
    for id in ids {
        let own = &cells_of[&id];
        let parent = c.rivers[&id].parent;
        let mut cq: HashSet<usize> = HashSet::new();
        for &q in own {
            cq.insert(q);
            // The river or lake it ends in is in reach (a coarse lake cell can be dry ground at
            // the finer level), so the path can run to it.
            for (_, p) in cd.neighbours(q) {
                let other = c.lake(p).is_some_and(|l| l != parent)
                    || c.river(p).is_some_and(|r| r != id && r != parent);
                if !other && c.dry_basin(p).is_none() {
                    cq.insert(p);
                }
            }
        }
        // All of the lake it ends in: its fine water may lie further in.
        if c.lakes.contains_key(&parent) {
            cq.extend((0..c.body.len()).filter(|&q| c.lake(q) == Some(parent)));
        }
        // A path touches only the lake it ends in, and a lake it leaves within that lake's
        // exit cell. Elsewhere a river beside a lake would stand below its level, and shores
        // hold ground above it, which would dam the burned path.
        let end_lake = lake_basin.get(&parent).copied();
        // The lake this river leaves (its exit is one of the river's cells): the fine lake may
        // reach past the exit cell, beside another of its heads.
        let left_lake = own.iter().find_map(|&q| {
            exits
                .get(&c.abs(q))
                .and_then(|l| lake_basin.get(l))
                .copied()
        });
        let shore = |i: usize| {
            let q = c.abs(c.cell(w, i));
            let leaves = exits.get(&q).and_then(|l| lake_basin.get(l)).copied();
            dims.neighbours(i).any(|(_, j)| {
                body[j] != 0
                    && basins[body[j] as usize - 1].kind == Kind::Lake
                    && Some(body[j]) != end_lake
                    && Some(body[j]) != leaves
                    && Some(body[j]) != left_lake
            })
        };
        let mut corridor: HashSet<u32> = HashSet::new();
        for &q in &cq {
            for i in c.fine_cells(q, w, h) {
                // Dry basin floors stay open (at 1 m a glacier is a field of ice basins).
                let open = body[i] == 0 || basins[body[i] as usize - 1].kind == Kind::Dry;
                if flags[i] & (SEA | FIXED) == 0
                    && open
                    && !claimed.contains_key(&(i as u32))
                    && !shore(i)
                {
                    corridor.insert(i as u32);
                }
            }
        }
        let my_inflows: Vec<&Inflow> = inflows.iter().filter(|f| f.id == id).collect();
        for f in &my_inflows {
            corridor.insert(f.cell);
        }
        // Flows north into the previous band: its fixed cells of this river are an end.
        let north_end = own.iter().any(|&q| {
            c.centre(q)
                && !fully_fixed(q)
                && c.dir(q) != 0
                && cd
                    .neighbour(q, c.dir(q) as usize - 1)
                    .is_some_and(fully_fixed)
        });
        let lake_end = lake_basin.get(&parent).copied();
        // Ending in a dry basin: the basin cells the coarse river flows into, not any dry
        // basin beside its corridor.
        let mut dry_mouth: HashSet<usize> = HashSet::new();
        if c.dry.contains(&parent) {
            for &q in own.iter().filter(|&&q| c.centre(q) && c.dir(q) != 0) {
                if let Some(p) = cd
                    .neighbour(q, c.dir(q) as usize - 1)
                    .filter(|&p| c.dry_basin(p).is_some())
                {
                    dry_mouth.insert(p);
                    dry_mouth.extend(
                        cd.neighbours(p)
                            .map(|(_, r)| r)
                            .filter(|&r| c.dry_basin(r).is_some()),
                    );
                }
            }
        }
        let is_seed = |j: usize| -> Option<f32> {
            if flags[j] & SEA != 0 && parent == 1 {
                Some(0.0)
            } else if flags[j] & FIXED != 0 {
                // Not a fixed cell that drains into this grid (that is where rivers enter).
                let ends = back(j).is_none()
                    && ((north_end && fixed_id(j) == id) || fixed_id(j) == parent);
                ends.then_some(ground[j])
            } else if claimed.get(&(j as u32)) == Some(&parent) {
                // A confluence: the larger river's path (made before this one).
                Some(ground[j])
            } else if lake_end.is_some_and(|b| body[j] == b) {
                Some(basins[body[j] as usize - 1].level)
            } else if dry_mouth.contains(&c.cell(w, j)) && body[j] == 0 {
                Some(ground[j])
            } else {
                None
            }
        };
        // Coarse order: centreline steps down to where the river ends in the window; other
        // corridor cells take their lowest neighbour's.
        let mut rank: HashMap<usize, u32> = HashMap::new();
        let on_line = |q: usize| c.centre(q) && c.body[q] == id;
        for &q in own.iter().filter(|&&q| on_line(q)) {
            if rank.contains_key(&q) {
                continue;
            }
            let (mut chain, mut base) = (vec![q], 0);
            loop {
                let last = chain[chain.len() - 1];
                let d = c.dir(last);
                let next = (d != 0)
                    .then(|| cd.neighbour(last, d as usize - 1))
                    .flatten()
                    .filter(|&p| on_line(p));
                // A coarse loop (none expected) ends the chain instead of hanging.
                match next.filter(|_| chain.len() <= own.len()) {
                    Some(p) => match rank.get(&p) {
                        Some(&r) => {
                            base = r + 1;
                            break;
                        }
                        None => chain.push(p),
                    },
                    None => break,
                }
            }
            for (k, &x) in chain.iter().rev().enumerate() {
                rank.insert(x, base + k as u32);
            }
        }
        let corridor_rank: HashMap<usize, u32> = cq
            .iter()
            .map(|&q| {
                let r = rank.get(&q).copied().or_else(|| {
                    cd.neighbours(q)
                        .filter_map(|(_, p)| rank.get(&p).copied())
                        .min()
                });
                (q, r.unwrap_or(0))
            })
            .collect();
        let rank_of = |i: usize| corridor_rank.get(&c.cell(w, i)).copied().unwrap_or(0);
        // Where the coarse river leaves the grid, the grid edge within a coarse cell of it is
        // an outlet, starting the flood at that exit's order.
        let mut leave: HashMap<usize, u32> = HashMap::new();
        for &q in own
            .iter()
            .filter(|&&q| c.centre(q) && inside(q) && !fully_fixed(q))
        {
            let [qx, qy] = c.abs(q);
            let world_edge = qx == 0 || qy == 0 || qx + 1 == c.size[0] || qy + 1 == c.size[1];
            let out_of_grid = match c.dir(q) {
                0 => world_edge && parent == 0,
                d => cd.neighbour(q, d as usize - 1).is_none_or(|p| !inside(p)),
            };
            if out_of_grid {
                let r = rank.get(&q).copied().unwrap_or(0);
                for p in cd.neighbours(q).map(|(_, p)| p).chain([q]) {
                    let e = leave.entry(p).or_insert(r);
                    *e = (*e).max(r);
                }
            }
        }
        // The flood climbs the river upstream: each branch carries the highest order it has
        // reached and enters no cell more than one coarse step below it, so a stretch that
        // leaves the grid and comes back is not routed back to where it left.
        let mut heap = BinaryHeap::new();
        let mut recv: HashMap<u32, u32> = HashMap::new();
        let mut top: HashMap<u32, u32> = HashMap::new();
        let mut seeds: HashSet<u32> = HashSet::new();
        // Order of a seed set beside the corridor: the lowest of its corridor neighbours', so
        // it does not depend on the (random) set iteration order.
        let mut fresh: HashMap<u32, u32> = HashMap::new();
        for &i in &corridor {
            let i = i as usize;
            let exit = leave
                .get(&c.cell(w, i))
                .copied()
                .filter(|_| dims.is_edge(i));
            if let Some(r) = exit {
                if flags[i] & OUTLET == 0 {
                    out.outlets.push(i as u32);
                }
                seeds.insert(i as u32);
                recv.insert(i as u32, u32::MAX);
                top.insert(i as u32, r.max(rank_of(i)));
                heap.push(Item(ground[i], i as u32));
            }
            for (_, j) in dims.neighbours(i) {
                let (jj, t) = (j as u32, rank_of(i).saturating_sub(1));
                if corridor.contains(&jj) {
                    continue;
                }
                if let Some(e) = fresh.get_mut(&jj) {
                    *e = (*e).min(t);
                } else if !seeds.contains(&jj) {
                    if let Some(z) = is_seed(j) {
                        seeds.insert(jj);
                        fresh.insert(jj, t);
                        heap.push(Item(z, jj));
                    }
                }
            }
        }
        top.extend(fresh.drain());
        // Corridor cells the river's own end cannot reach end at its fixed cells whose flow
        // comes back into a cell already reached (a stretch dipping south of the seam), and
        // then (another river's path cuts across) at that river instead: a confluence.
        for pass in 0..3 {
            if pass == 1 {
                for &i in &corridor {
                    if recv.contains_key(&i) {
                        continue;
                    }
                    for (_, j) in dims.neighbours(i as usize) {
                        let ends = flags[j] & FIXED != 0
                            && fixed_id(j) == id
                            && back(j).is_some_and(|r| recv.contains_key(&(r as u32)));
                        let t = rank_of(i as usize).saturating_sub(1);
                        if let Some(e) = fresh.get_mut(&(j as u32)) {
                            *e = (*e).min(t);
                        } else if ends && seeds.insert(j as u32) {
                            fresh.insert(j as u32, t);
                            heap.push(Item(ground[j], j as u32));
                        }
                    }
                }
                top.extend(fresh.drain());
            }
            if pass == 2 {
                for &i in &corridor {
                    if recv.contains_key(&i) {
                        continue;
                    }
                    for (_, j) in dims.neighbours(i as usize) {
                        let t = rank_of(i as usize).saturating_sub(1);
                        if let Some(e) = fresh.get_mut(&(j as u32)) {
                            *e = (*e).min(t);
                        } else if claimed.contains_key(&(j as u32)) && seeds.insert(j as u32) {
                            fresh.insert(j as u32, t);
                            heap.push(Item(ground[j], j as u32));
                        }
                    }
                }
                top.extend(fresh.drain());
            }
            while let Some(Item(z, a)) = heap.pop() {
                let ra = top[&a];
                for (_, j) in dims.neighbours(a as usize) {
                    let jj = j as u32;
                    if corridor.contains(&jj) && !recv.contains_key(&jj) && rank_of(j) + 1 >= ra {
                        recv.insert(jj, a);
                        top.insert(jj, ra.max(rank_of(j)));
                        heap.push(Item(ground[j].max(z), jj));
                    }
                }
            }
        }
        // Heads: coarse centreline cells without a centreline donor of this river in the
        // window (donors in the previous band count only when it hands the river over).
        let is_head = |q: usize| {
            c.centre(q)
                && !fully_fixed(q)
                && !cd.neighbours(q).any(|(_, p)| {
                    c.centre(p)
                        && c.body[p] == id
                        && c.dir(p) != 0
                        && cd.neighbour(p, c.dir(p) as usize - 1) == Some(q)
                        && inside(p)
                        && (!fully_fixed(p) || !my_inflows.is_empty())
                })
        };
        // Last resort for a head the corridor flood missed: the lowest way over free cells
        // to any end (this river's routed cells, the sea, a lake, another path, the previous
        // band's water or a grid-edge outlet), made part of the corridor.
        // A head is due when its coarse river reaches free core cells from it (a head in the
        // halo may flow into the core); other halo heads are made again by the next band.
        let due = |q: usize| {
            let core = |p: usize| {
                c.fine_cells(p, w, h).any(|i| {
                    flags[i] & (SEA | FIXED) == 0 && (core_rows[0]..core_rows[1]).contains(&(i / w))
                })
            };
            if !c.fine_cells(q, w, h).any(|i| flags[i] & (SEA | FIXED) == 0) {
                return false;
            }
            let mut p = q;
            for _ in 0..=own.len() {
                if core(p) {
                    return true;
                }
                match (c.dir(p) != 0)
                    .then(|| cd.neighbour(p, c.dir(p) as usize - 1))
                    .flatten()
                    .filter(|&n| on_line(n))
                {
                    Some(n) => p = n,
                    None => return false,
                }
            }
            false
        };
        // A head whose fine cells are all sea at this level (a mouth the fine coast moved
        // inland) starts at the first coarse cell down the river that has land.
        let land = |q: usize| c.fine_cells(q, w, h).any(|i| flags[i] & (SEA | FIXED) == 0);
        let start = |q: usize| {
            let mut p = q;
            for _ in 0..=own.len() {
                if land(p) || fully_fixed(p) {
                    return Some(p);
                }
                p = (c.dir(p) != 0)
                    .then(|| cd.neighbour(p, c.dir(p) as usize - 1))
                    .flatten()
                    .filter(|&n| on_line(n))?;
            }
            None
        };
        // A head whose coarse river comes from the previous band, which did not hand it over
        // (that band routed the water its own way), is no head while another one is.
        let taken_above = |q: usize| {
            my_inflows.is_empty()
                && cd.neighbours(q).any(|(_, p)| {
                    c.centre(p)
                        && c.body[p] == id
                        && c.dir(p) != 0
                        && cd.neighbour(p, c.dir(p) as usize - 1) == Some(q)
                        && inside(p)
                        && fully_fixed(p)
                })
        };
        let heads_here: Vec<usize> = own.iter().copied().filter(|&q| is_head(q)).collect();
        let own_head = heads_here.iter().any(|&q| !taken_above(q));
        let mut starts: Vec<usize> = heads_here
            .iter()
            .filter(|&&q| !own_head || !taken_above(q))
            .filter_map(|&q| start(q))
            .collect();
        starts.sort_unstable();
        starts.dedup();
        let mut stranded: Vec<Vec<usize>> = starts
            .iter()
            .filter(|&&q| due(q))
            .map(|&q| c.fine_cells(q, w, h).collect::<Vec<_>>())
            .filter(|cells| !cells.iter().any(|&i| recv.contains_key(&(i as u32))))
            .collect();
        stranded.extend(
            my_inflows
                .iter()
                .filter(|f| !recv.contains_key(&f.cell))
                .map(|f| vec![f.cell as usize]),
        );
        for sources in stranded {
            // An earlier way may have run through this head: it is reached now.
            if sources.iter().any(|&s| recv.contains_key(&(s as u32))) {
                continue;
            }
            route_stranded(
                dims,
                ground,
                flags,
                body,
                basins,
                &sources,
                |j| claimed.contains_key(&(j as u32)),
                &back,
                core_rows[1],
                north_end.then_some(&|j| fixed_id(j) == id),
                &mut recv,
                &mut seeds,
                &mut corridor,
                &mut out.outlets,
            );
        }
        // (cell, lake it leaves, entering surface and discharge)
        type Head = (u32, u32, Option<(f32, f32)>);
        let mut heads: Vec<Head> = Vec::new();
        for &q in &starts {
            let source = exits
                .get(&c.abs(q))
                .and_then(|l| lake_basin.get(l))
                .copied()
                .unwrap_or(0);
            let reached: Vec<usize> = c
                .fine_cells(q, w, h)
                .filter(|&i| recv.contains_key(&(i as u32)))
                .collect();
            let beside =
                |i: usize| source != 0 && dims.neighbours(i).any(|(_, j)| body[j] == source);
            // How far the path from a cell climbs above it: a head low on a steep cell whose
            // path climbs to the stem would have the stem burned down to it.
            let climb = |i: usize| {
                let (mut at, mut top) = (i as u32, ground[i]);
                // Bounded: a loop (never made) cannot hang the run.
                for _ in 0..=recv.len() {
                    let Some(&r) = recv.get(&at).filter(|&&r| r != u32::MAX) else {
                        break;
                    };
                    top = top.max(ground[r as usize]);
                    if seeds.contains(&r) {
                        break;
                    }
                    at = r;
                }
                top - ground[i]
            };
            let lowest = |f: &dyn Fn(usize) -> bool| {
                reached
                    .iter()
                    .copied()
                    .filter(|&i| f(i))
                    .map(|i| (climb(i), ground[i], i))
                    .min_by(|a, b| {
                        a.0.total_cmp(&b.0)
                            .then(a.1.total_cmp(&b.1))
                            .then(a.2.cmp(&b.2))
                    })
                    .map(|(_, _, i)| i)
            };
            match lowest(&beside).or_else(|| lowest(&|_| true)) {
                Some(i) => {
                    let source = if beside(i) { source } else { 0 };
                    if source != 0 {
                        // The lake leaves here: the head is its sill at the level.
                        ground[i] = ground[i].min(basins[source as usize - 1].level);
                    }
                    heads.push((i as u32, source, None));
                }
                None => {
                    if due(q) {
                        out.unrouted.push(id);
                    }
                }
            }
        }
        for f in &my_inflows {
            heads.push((f.cell, 0, Some((f.surface, f.q))));
        }
        // Columns near where this river (or what it ends in) crosses the seam, in the previous
        // band's last row.
        let fixed_rows = (0..h).find(|&y| flags[y * w] & FIXED == 0).unwrap_or(h);
        let mut near_cross = vec![false; w];
        if fixed_rows > 0 {
            for x in 0..w {
                let f = fixed_id((fixed_rows - 1) * w + x);
                if f != 0 && (f == id || f == parent) {
                    let (a, b) = (x.saturating_sub(SEAM_STRIP), (x + SEAM_STRIP).min(w - 1));
                    near_cross[a..=b].fill(true);
                }
            }
        }
        let river = out.ids.len();
        out.ids.push(id);
        for (head, source, inflow) in heads {
            if !recv.contains_key(&head) {
                out.unrouted.push(id);
                continue;
            }
            let mut cells = Vec::new();
            let mut at = head;
            let next = loop {
                cells.push(at);
                // A loop in the receivers or a cell without one (neither is made) counts as
                // unrouted instead of hanging or panicking.
                let Some(&r) = recv.get(&at).filter(|_| cells.len() <= recv.len() + 1) else {
                    break Some(u32::MAX);
                };
                // A grid-edge outlet in the corridor ends the path on it.
                match r {
                    u32::MAX => break None,
                    r if seeds.contains(&r) && !corridor.contains(&r) => break Some(r),
                    r => at = r,
                }
            };
            if next == Some(u32::MAX) {
                out.receiver_loops += 1;
                out.unrouted.push(id);
                continue;
            }
            // A path that climbs (down into a hollow, then up to its end, as where the coarse
            // river crosses the seam above a valley this band's ground holds) is laid again
            // along the way to that end that climbs least, so it is neither cut nor filled.
            // So is one running along the seam away from where its river crosses it (it
            // would be cut or filled against the ground the previous band keeps), if a way
            // that keeps off the seam there climbs no more.
            let along = |c: u32| {
                let c = c as usize;
                (fixed_rows..fixed_rows + SEAM_STRIP).contains(&(c / w)) && !near_cross[c % w]
            };
            let runs_along = fixed_rows > 0 && cells.iter().any(|&c| along(c));
            let climbs = climb(ground, &cells);
            if let Some(end) = next.filter(|_| climbs > PATH_CLIMB_M || runs_along) {
                // Near its end (on the seam too, where it joins another river) it may reach it,
                // and near a head on the seam it may leave it.
                let near = |i: usize, at: usize| (i % w).abs_diff(at % w) <= SEAM_STRIP;
                let open = |i: usize| {
                    !claimed.contains_key(&(i as u32))
                        && (!along(i as u32) || near(i, end as usize) || near(i, head as usize))
                };
                if let Some(way) = route_descending(
                    dims,
                    ground,
                    flags,
                    body,
                    basins,
                    head as usize,
                    end as usize,
                    open,
                ) {
                    // Off the seam a climb is cut or filled within the carve floor; along it,
                    // only below the floor (a trench at the seam): any such way is better.
                    let better = if runs_along {
                        true
                    } else {
                        climb(ground, &way) < climbs
                    };
                    if better {
                        cells = way;
                        out.relaid += 1;
                    }
                }
            }
            // Ending in the previous band's water (surface `end_s`): a way beside that band's
            // dry ground below it (which cannot be raised) goes round, to this river's water
            // there; the path stands at least at that surface and its bed at the end's.
            let mut next = next;
            let end_s = next
                .filter(|&r| flags[r as usize] & FIXED != 0)
                .map(|r| fixed_surface(r as usize))
                .filter(|s| s.is_finite());
            if let Some(s) = end_s {
                // Within `SEAM_CLEAR` cells: the path's channel and banks, raised to its
                // water, would reach there too (a levee on the seam).
                let low_bank = |i: usize| {
                    let (x, y) = ((i % w) as isize, (i / w) as isize);
                    flags[i] & FIXED == 0
                        && (-SEAM_CLEAR..=SEAM_CLEAR).any(|dy| {
                            (-SEAM_CLEAR..=SEAM_CLEAR).any(|dx| {
                                let (a, b) = (x + dx, y + dy);
                                if a < 0 || b < 0 || a >= w as isize || b >= h as isize {
                                    return false;
                                }
                                let k = b as usize * w + a as usize;
                                flags[k] & FIXED != 0
                                    && flags[k] & (RIVER | SEA) == 0
                                    && fixed_id(k) == 0
                                    && ground[k] + SEAM_BANK_M < s
                            })
                        })
                };
                if cells.iter().any(|&x| low_bank(x as usize)) {
                    let end = |j: usize| {
                        flags[j] & (FIXED | RIVER) == FIXED | RIVER
                            && fixed_id(j) == id
                            && back(j).is_none()
                    };
                    let avoid = |i: usize| low_bank(i) || claimed.contains_key(&(i as u32));
                    // Not a way that climbs much more (a crossing above a hollow).
                    if let Some((way, j)) =
                        route_round(dims, ground, flags, body, basins, head as usize, end, avoid)
                            .filter(|(way, _)| {
                                climb(ground, way) <= climb(ground, &cells) + PATH_CLIMB_M
                            })
                    {
                        (cells, next) = (way, Some(j));
                    }
                }
            }
            let end_s = next
                .filter(|&r| flags[r as usize] & FIXED != 0)
                .map(|r| fixed_surface(r as usize))
                .filter(|s| s.is_finite());
            // Lowest bed each cell may get: never below the previous band's bed the path ends
            // on, nor below the seam limit (`path_floor`) of any cell downstream (a path that
            // must not cut beside the seam stays up and fills what lies lower above it, so it
            // never climbs).
            let end_bed = next
                .filter(|_| end_s.is_some())
                .map_or(f32::NEG_INFINITY, |r| ground[r as usize]);
            let mut floors = vec![end_bed; cells.len()];
            let mut low = end_bed;
            for k in (0..cells.len()).rev() {
                low = low.max(path_floor(cells[k] as usize));
                floors[k] = low;
            }
            // Cap: coarse surfaces at the middle of each run through one coarse cell of
            // this river, linear between them by path length.
            let mut s = vec![0f64; cells.len()];
            for k in 1..cells.len() {
                let d = dims.towards(cells[k - 1] as usize, cells[k] as usize);
                s[k] = s[k - 1] + Dims::step(d);
            }
            let mut anchors: Vec<(f64, f32)> = Vec::new();
            if let Some((surface, _)) = inflow {
                anchors.push((-1.0, surface));
            }
            let mut k = 0;
            while k < cells.len() {
                let q = c.cell(w, cells[k] as usize);
                let mut e = k;
                while e + 1 < cells.len() && c.cell(w, cells[e + 1] as usize) == q {
                    e += 1;
                }
                if c.body[q] == id && c.surface[q].is_finite() {
                    anchors.push(((s[k] + s[e]) / 2.0, c.surface[q]));
                }
                k = e + 1;
            }
            let mut qmax = inflow.map_or(0f32, |(_, q)| q);
            // Coarse order of the path's current place on the river.
            let mut here = rank_of(cells[0] as usize);
            let mut last_q: Option<usize> = None;
            for (k, &cell) in cells.iter().enumerate() {
                let q = c.cell(w, cell as usize);
                if c.body[q] == id {
                    qmax = qmax.max(c.discharge[q]);
                }
                let e = out.q.entry(cell).or_insert(0.0);
                *e = e.max(qmax);
                let a = anchors.partition_point(|&(x, _)| x <= s[k]);
                let v = match (a.checked_sub(1).map(|p| anchors[p]), anchors.get(a)) {
                    (Some((x0, v0)), Some(&(x1, v1))) => {
                        v0 + (v1 - v0) * ((s[k] - x0) / (x1 - x0)) as f32
                    }
                    (Some((_, v0)), None) => v0,
                    (None, Some(&(_, v1))) => v1,
                    (None, None) => continue,
                };
                // The highest coarse surface of this river over the cell's coarse cell and the
                // 8 around it, from cells not downstream of where the path is along the coarse
                // river (a downstream cell's surface beside a steep drop would cut a gash), and
                // never below the interpolation along the path.
                if c.body[q] == id {
                    here = corridor_rank.get(&q).copied().unwrap_or(here);
                } else if Some(q) != last_q {
                    // Beside the river: abreast of its upstream-most neighbour. (A downstream
                    // cell's low surface cannot cut: the cap is never below the interpolation.)
                    let abreast = cd
                        .neighbours(q)
                        .filter(|&(_, p)| c.body[p] == id)
                        .filter_map(|(_, p)| corridor_rank.get(&p).copied())
                        .max()
                        .unwrap_or(here);
                    here = here.min(abreast);
                }
                last_q = Some(q);
                let [qx, qy] = c.abs(q);
                let local = (-1..=1)
                    .flat_map(|dy| (-1..=1).map(move |dx| [qx as isize + dx, qy as isize + dy]))
                    .filter_map(|p| c.at(p))
                    .filter(|&p| c.body[p] == id)
                    .filter(|p| corridor_rank.get(p).is_none_or(|&r| r >= here))
                    .map(|p| c.surface[p])
                    .fold(f32::NEG_INFINITY, f32::max);
                // Away from the coarse river (a path laid off its corridor) its surface says
                // nothing of the ground here: no cap, the path only keeps descending.
                let near_river = (-1..=1)
                    .flat_map(|dy| (-1..=1).map(move |dx| [qx as isize + dx, qy as isize + dy]))
                    .filter_map(|p| c.at(p))
                    .any(|p| c.body[p] == id);
                if !near_river {
                    continue;
                }
                let v = v.max(local);
                let e = out.cap.entry(cell).or_insert(f32::INFINITY);
                *e = e.min(v);
            }
            // The first cells may have taken no discharge before the first coarse cell.
            let first = cells
                .iter()
                .map(|&x| out.q[&x])
                .find(|&v| v > 0.0)
                .unwrap_or(0.0);
            for &x in &cells {
                let e = out.q.get_mut(&x).expect("set above");
                *e = e.max(first);
            }
            // That water cannot change and the previous band's checks held it. The water may
            // also stand on the floors the path keeps.
            for (k, x) in cells.iter().enumerate() {
                if let Some(e) = out.cap.get_mut(x) {
                    *e = e.max(end_s.unwrap_or(f32::NEG_INFINITY)).max(floors[k]);
                }
            }
            claimed.extend(cells.iter().map(|&x| (x, id)));
            out.paths.push(Path {
                river,
                cells,
                next,
                source,
                floors,
            });
        }
    }
    out.unrouted.sort_unstable();
    out.unrouted.dedup();
    let (cells, max, at, fill) = burn(dims, ground, body, basins, &out.paths, &out.cap, spacing_m);
    out.fill = fill;
    out.burned = cells;
    out.burn_max_m = max;
    out.burn_max_at = [at % w, at / w];
    out
}

#[cfg(test)]
mod tests {
    use super::super::coarse::{Lake, Moisture, River};
    use super::*;
    use crate::world::hydrology::flood::{FLOW_CENTRE, RIVER};

    /// 2 x 2 coarse cells of 2 x 2 fine cells: a lake (id 2, level 5 m) top left, a river
    /// (id 3, surface 4 m) top right.
    fn coarse() -> Coarse {
        Coarse {
            k: 2,
            size: [2, 2],
            off: [0, 0],
            x0: 0,
            y0: 0,
            ww: 2,
            wh: 2,
            surface: vec![5.0, 4.0, f32::NAN, f32::NAN],
            body: vec![2, 3, 0, 0],
            flow: vec![0, FLOW_CENTRE | 2, 0, 0],
            discharge: vec![1.0; 4],
            lakes: HashMap::from([(
                2,
                Lake {
                    level: 5.0,
                    exit: None,
                },
            )]),
            rivers: HashMap::from([(
                3,
                River {
                    parent: 1,
                    drainage_km2: 1.0,
                },
            )]),
            dry: HashSet::new(),
            max_id: 3,
            moisture: Moisture {
                w: 1,
                h: 1,
                spacing_m: 128.0,
                nw_m: [0.0, 0.0],
                values: vec![0.5],
                weight: 1.0,
                windless: true,
                wind_from_deg: None,
                entry: Map::new(),
            },
            spacing_m: 2.0,
            digests: Map::new(),
        }
    }

    struct Scene {
        flags: Vec<u8>,
        surface: Vec<f32>,
        ids: Vec<u32>,
        basins: Vec<Basin>,
        unrouted: Vec<u32>,
    }

    fn scene() -> Scene {
        let mut s = Scene {
            flags: vec![0; 16],
            surface: vec![f32::NAN; 16],
            ids: vec![0; 16],
            basins: vec![Basin::kept(Kind::Lake, 5.0, NO_EXIT)],
            unrouted: Vec::new(),
        };
        s.basins[0].id = 2;
        (s.ids[0], s.surface[0]) = (2, 5.0);
        for i in [2, 6] {
            (s.ids[i], s.surface[i], s.flags[i]) = (3, 3.9, RIVER | CENTRE);
        }
        s
    }

    fn failed(s: &Scene) -> Vec<String> {
        let dims = Dims { w: 4, h: 4 };
        let (m, _) = consistency(
            dims,
            &s.flags,
            &s.surface,
            &s.basins,
            &coarse(),
            &s.unrouted,
            [0, 4],
            &HashMap::new(),
            &|i| s.ids[i],
            &HashSet::new(),
        );
        m.into_iter()
            .filter(|(_, v)| v.as_u64() != Some(0))
            .map(|(k, _)| k)
            .collect()
    }

    #[test]
    fn a_leaking_lake_is_found() {
        // 3 x 3: sea at the top-left corner, lake 1 beside it, lake 2 in the middle.
        let dims = Dims { w: 3, h: 3 };
        let mut flags = vec![0u8; 9];
        flags[0] = SEA;
        let mut body = vec![0u32; 9];
        body[1] = 1;
        let edge = |_: usize| false;
        assert!(leak_cells(dims, &flags, &body, 0, &[0.0, 0.0], edge).is_empty());
        assert_eq!(
            leak_cells(dims, &flags, &body, 0, &[5.0, 0.0], edge),
            vec![1]
        );
        flags[0] = 0;
        body[4] = 2;
        assert_eq!(
            leak_cells(dims, &flags, &body, 0, &[5.0, 3.0], edge),
            vec![1]
        );
        assert!(leak_cells(dims, &flags, &body, 0, &[3.0, 3.0], edge).is_empty());
    }

    #[test]
    fn every_coarse_check_can_fail() {
        assert!(failed(&scene()).is_empty(), "{:?}", failed(&scene()));
        type Mutation = fn(&mut Scene);
        let broken: [(&str, Mutation); 5] = [
            ("coarse_lakes_missing", |s| s.ids[0] = 0),
            ("guard_coarse_lake_levels_changed", |s| {
                s.basins[0].level = 5.5
            }),
            ("coarse_rivers_missing_or_unrouted", |s| {
                s.ids[2] = 0;
                s.ids[6] = 0;
            }),
            ("coarse_rivers_missing_or_unrouted", |s| s.unrouted.push(3)),
            ("water_above_coarse_surface", |s| s.surface[6] = 4.5),
        ];
        for (name, mutate) in broken {
            let mut s = scene();
            mutate(&mut s);
            assert!(
                failed(&s).contains(&name.to_string()),
                "{name}: {:?}",
                failed(&s)
            );
        }
    }

    #[test]
    fn a_head_beside_the_lake_its_river_leaves_is_routed() {
        // 3 x 3 coarse cells of 2 x 2 fine cells. Lake 2 (level 20 m) in the top-right cell
        // drains into river 3 at (2, 1); the river's other head (1, 0) lies where the fine
        // lake reaches past its exit cell. Both heads route west to the world edge.
        let mut c = coarse();
        (c.size, c.ww, c.wh) = ([3, 3], 3, 3);
        c.body = vec![0, 3, 2, 3, 3, 3, 0, 0, 0];
        let line = |d: u8| FLOW_CENTRE | d;
        c.flow = vec![0, line(2), 0, line(3), line(3), line(3), 0, 0, 0];
        c.surface = vec![
            f32::NAN,
            19.0,
            20.0,
            15.0,
            17.0,
            19.5,
            f32::NAN,
            f32::NAN,
            f32::NAN,
        ];
        c.discharge = vec![1.0; 9];
        c.lakes = HashMap::from([(
            2,
            Lake {
                level: 20.0,
                exit: Some([2, 1]),
            },
        )]);
        c.rivers = HashMap::from([(
            3,
            River {
                parent: 0,
                drainage_km2: 1.0,
            },
        )]);
        let dims = Dims { w: 6, h: 6 };
        let mut ground: Vec<f32> = (0..36).map(|i| 10.0 + (i % 6) as f32).collect();
        let mut body = vec![0u32; 36];
        for y in 0..2 {
            for x in 3..6 {
                (ground[y * 6 + x], body[y * 6 + x]) = (15.0, 1);
            }
        }
        let mut lake = Basin::kept(Kind::Lake, 20.0, NO_EXIT);
        lake.id = 2;
        let im = rivers(
            dims,
            &mut ground,
            &[0; 36],
            &body,
            &[lake],
            &c,
            &[],
            |_| 0,
            |_| f32::NAN,
            |_| f32::NEG_INFINITY,
            |_| None,
            [0, 6],
            2.0,
        );
        assert!(im.unrouted.is_empty(), "unrouted {:?}", im.unrouted);
    }

    #[test]
    fn a_stretch_into_the_previous_band_ends_where_its_flow_comes_back() {
        // 6 x 4 coarse cells of 2 x 2 fine cells; the top two coarse rows are the previous
        // band's. River 3 rises at (0, 2), runs north into those rows and east, and comes
        // back at (4, 2) to the sea in the bottom-right corner. The upstream stretch can reach
        // no end in this band: it ends at the fixed river, whose flow re-enters at the inflow.
        let mut c = coarse();
        (c.size, c.ww, c.wh) = ([6, 4], 6, 4);
        let line = |d: u8| FLOW_CENTRE | d;
        let course = [
            ((0, 2), 1),
            ((0, 1), 6),
            ((1, 0), 4),
            ((2, 0), 4),
            ((3, 0), 8),
            ((4, 1), 2),
            ((4, 2), 2),
            ((4, 3), 2),
        ];
        (c.body, c.flow, c.surface) = (vec![0; 24], vec![0; 24], vec![f32::NAN; 24]);
        for (k, ((x, y), d)) in course.into_iter().enumerate() {
            let q = y * 6 + x;
            (c.body[q], c.flow[q], c.surface[q]) = (3, line(d), 20.0 - k as f32);
        }
        c.discharge = vec![1.0; 24];
        c.lakes = HashMap::new();
        c.rivers = HashMap::from([(
            3,
            River {
                parent: 1,
                drainage_km2: 1.0,
            },
        )]);
        let dims = Dims { w: 12, h: 8 };
        let fixed = 4 * 12;
        let mut ground: Vec<f32> = (0..96).map(|i| 20.0 - (i / 12) as f32).collect();
        let mut flags = vec![0u8; 96];
        flags[..fixed].fill(FIXED);
        flags[90..].fill(SEA);
        let river_cell = |j: usize| c.body[c.cell(12, j)] == 3;
        let inflow = Inflow {
            cell: (4 * 12 + 8) as u32,
            id: 3,
            surface: 16.0,
            q: 1.0,
        };
        let fixed_id = |j: usize| if j < fixed && river_cell(j) { 3 } else { 0 };
        let im = rivers(
            dims,
            &mut ground,
            &flags,
            &[0; 96],
            &[],
            &c,
            &[inflow],
            fixed_id,
            |_| f32::NAN,
            |_| f32::NEG_INFINITY,
            |j| (fixed_id(j) == 3).then_some(4 * 12 + 8),
            [4, 8],
            2.0,
        );
        assert!(im.unrouted.is_empty(), "unrouted {:?}", im.unrouted);
        assert!(
            im.paths
                .iter()
                .any(|p| p.next.is_some_and(|n| (n as usize) < fixed))
        );
    }

    /// Whether following `recv` from every cell ends (no loop).
    fn no_loop(recv: &HashMap<u32, u32>) -> bool {
        recv.keys().all(|&k| {
            let mut at = k;
            (0..=recv.len()).any(|_| match recv.get(&at) {
                Some(&r) if r != u32::MAX => {
                    at = r;
                    false
                }
                _ => true,
            })
        })
    }

    #[test]
    fn stranded_ways_close_no_receiver_loop() {
        // 6 x 3, row 1 falling east between high rows, an outlet at (5, 1). Head A (0, 1) is routed first and
        // its way runs through (2, 1), a cell of head B; (4, 1) is a seed (a dry basin mouth).
        // A's way ends at the seed; B, routed after, must not start from its routed cell
        // (re-pointing it into A's way closed the loop that hung band 53).
        let dims = Dims { w: 6, h: 3 };
        let ground: Vec<f32> = (0..18)
            .map(|i| {
                if i / 6 == 1 {
                    10.0 - (i % 6) as f32
                } else {
                    20.0
                }
            })
            .collect();
        let mut flags = vec![0u8; 18];
        flags[11] = OUTLET;
        let (mut recv, mut seeds, mut corridor) = (HashMap::new(), HashSet::new(), HashSet::new());
        seeds.insert(10);
        let mut outlets = Vec::new();
        for sources in [vec![6], vec![8, 2, 14]] {
            route_stranded(
                dims,
                &ground,
                &flags,
                &[0; 18],
                &[],
                &sources,
                |_| false,
                &|_| None,
                3,
                None,
                &mut recv,
                &mut seeds,
                &mut corridor,
                &mut outlets,
            );
            if sources == [6] {
                assert_eq!((recv[&8], recv.get(&10)), (9, None), "ends at the seed");
            }
        }
        assert_eq!(recv[&8], 9, "a routed cell keeps its receiver");
        assert!(no_loop(&recv), "{recv:?}");
    }

    #[test]
    fn a_path_into_the_previous_bands_water_meets_its_bed_and_surface() {
        // 3 x 3 coarse cells of 2 x 2 fine cells; the top coarse row is the previous band's.
        // River 3 runs north from (1, 2) into that band's river at (1, 0), whose bed (9.5 m)
        // and water (10 m) stand above this band's valley (5 m) and the coarse surface (6 m).
        // The path is not cut below that bed (the valley is filled) and its cap allows the
        // water.
        let mut c = coarse();
        (c.size, c.ww, c.wh) = ([3, 3], 3, 3);
        c.body = vec![0, 3, 0, 0, 3, 0, 0, 3, 0];
        let line = FLOW_CENTRE | 1;
        c.flow = vec![0, line, 0, 0, line, 0, 0, line, 0];
        c.surface = vec![
            f32::NAN,
            6.0,
            f32::NAN,
            f32::NAN,
            6.0,
            f32::NAN,
            f32::NAN,
            6.0,
            f32::NAN,
        ];
        c.discharge = vec![1.0; 9];
        c.lakes = HashMap::new();
        c.rivers = HashMap::from([(
            3,
            River {
                parent: 7,
                drainage_km2: 1.0,
            },
        )]);
        let dims = Dims { w: 6, h: 6 };
        let fixed = 2 * 6;
        let river_cell = |j: usize| j < fixed && (2..4).contains(&(j % 6));
        let mut flags = vec![0u8; 36];
        let mut ground = vec![5.0f32; 36];
        for j in 0..fixed {
            flags[j] = FIXED;
            ground[j] = 12.0;
            if river_cell(j) {
                flags[j] |= RIVER | CENTRE;
                ground[j] = 9.5;
            }
        }
        let fixed_id = |j: usize| if river_cell(j) { 3 } else { 0 };
        let im = rivers(
            dims,
            &mut ground,
            &flags,
            &[0; 36],
            &[],
            &c,
            &[],
            fixed_id,
            |j| if river_cell(j) { 10.0 } else { f32::NAN },
            |_| f32::NEG_INFINITY,
            |_| None,
            [2, 6],
            2.0,
        );
        assert!(im.unrouted.is_empty(), "unrouted {:?}", im.unrouted);
        let p = &im.paths[0];
        assert!(p.next.is_some_and(|n| (n as usize) < fixed));
        for &x in &p.cells {
            assert!(
                ground[x as usize] >= 9.5,
                "bed {} at {x}",
                ground[x as usize]
            );
            assert!(im.cap.get(&x).is_none_or(|&s| s >= 10.0));
        }
        assert!(im.fill.cells > 0 && (im.fill.max_m - 4.5).abs() < 1e-4);
    }
}
