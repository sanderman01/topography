//! Finer levels hold the coarse network fixed. Coarse lakes keep their id and level on the
//! fine ground; each coarse river gets a fine centreline inside a corridor of one coarse cell
//! around it, found by a priority flood over that corridor, burned into the ground so the fine
//! routing follows it, and later carved as a channel with the coarse discharge below a surface
//! no higher than the coarse one.

use super::coarse::Coarse;
use super::flood::{self, CENTRE, Dims, FIXED, OUTLET, SEA};
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
/// it is, but not on a band edge). `fixed_lake` gives the coarse lake id of cells copied from
/// the previous band (seeds too). Coarse lake cells whose fine ground is above the level stay
/// dry.
pub(super) fn lakes(
    dims: Dims,
    ground: &mut [f32],
    flags: &[u8],
    body: &mut [u32],
    basins: &mut Vec<Basin>,
    c: &Coarse,
    fixed_lake: impl Fn(usize) -> Option<u32>,
) -> Value {
    let (n, w, h) = (ground.len(), dims.w, dims.h);
    let mut ids: Vec<u32> = (0..c.body.len()).filter_map(|q| c.lake(q)).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut index: HashMap<u32, u32> = HashMap::new();
    let first = basins.len() as u32;
    for &id in &ids {
        let mut basin = Basin::kept(Kind::Lake, c.lakes[&id].level, NO_EXIT);
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
/// path's cap where higher: the interpolation along the path) by more than the tolerance.
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

/// Lower each path to a running minimum from its head and to its surface cap (not below 0 m
/// or a lake floor beside it), then let each cut taper off upstream at the trench side slope
/// instead of ending in a cliff (still never rising downstream). Returns the cells lowered and
/// the deepest cut below the ground the path had and where.
pub(super) fn burn(
    dims: Dims,
    ground: &mut [f32],
    body: &[u32],
    basins: &[Basin],
    paths: &[Path],
    cap: &HashMap<u32, f32>,
    spacing_m: f64,
) -> (Vec<u32>, f32, usize) {
    let tan = super::erosion::SIDE_SLOPE_DEG.to_radians().tan();
    let (mut cells, mut max, mut at) = (Vec::new(), 0f32, 0usize);
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
    (cells, max, at)
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
                match next {
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
                if !corridor.contains(&(j as u32)) && !seeds.contains(&(j as u32)) {
                    if let Some(z) = is_seed(j) {
                        seeds.insert(j as u32);
                        top.insert(j as u32, rank_of(i).saturating_sub(1));
                        heap.push(Item(z, j as u32));
                    }
                }
            }
        }
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
                        if ends && seeds.insert(j as u32) {
                            top.insert(j as u32, rank_of(i as usize).saturating_sub(1));
                            heap.push(Item(ground[j], j as u32));
                        }
                    }
                }
            }
            if pass == 2 {
                for &i in &corridor {
                    if recv.contains_key(&i) {
                        continue;
                    }
                    for (_, j) in dims.neighbours(i as usize) {
                        if claimed.contains_key(&(j as u32)) && seeds.insert(j as u32) {
                            top.insert(j as u32, rank_of(i as usize).saturating_sub(1));
                            heap.push(Item(ground[j], j as u32));
                        }
                    }
                }
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
        // (cell, lake it leaves, entering surface and discharge)
        type Head = (u32, u32, Option<(f32, f32)>);
        let mut heads: Vec<Head> = Vec::new();
        for &q in own {
            if !c.centre(q) || fully_fixed(q) {
                continue;
            }
            let donor = cd.neighbours(q).any(|(_, p)| {
                c.centre(p)
                    && c.body[p] == id
                    && c.dir(p) != 0
                    && cd.neighbour(p, c.dir(p) as usize - 1) == Some(q)
                    && inside(p)
                    && (!fully_fixed(p) || !my_inflows.is_empty())
            });
            if donor {
                continue;
            }
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
                while let Some(&r) = recv.get(&at).filter(|&&r| r != u32::MAX) {
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
                    let core = |i: usize| (core_rows[0]..core_rows[1]).contains(&(i / w));
                    if c.fine_cells(q, w, h)
                        .any(|i| flags[i] & (SEA | FIXED) == 0 && core(i))
                    {
                        out.unrouted.push(id);
                    }
                }
            }
        }
        for f in &my_inflows {
            heads.push((f.cell, 0, Some((f.surface, f.q))));
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
                // A grid-edge outlet in the corridor ends the path on it.
                match recv[&at] {
                    u32::MAX => break None,
                    r if seeds.contains(&r) && !corridor.contains(&r) => break Some(r),
                    r => at = r,
                }
            };
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
            claimed.extend(cells.iter().map(|&x| (x, id)));
            out.paths.push(Path {
                river,
                cells,
                next,
                source,
            });
        }
    }
    out.unrouted.sort_unstable();
    out.unrouted.dedup();
    let (cells, max, at) = burn(dims, ground, body, basins, &out.paths, &out.cap, spacing_m);
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
}
