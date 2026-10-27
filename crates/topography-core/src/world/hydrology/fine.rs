//! Finer levels hold the coarse network fixed. Coarse lakes keep their id and level on the
//! fine ground; each coarse river gets a fine centreline inside a corridor of one coarse cell
//! around it, found by a priority flood over that corridor, burned into the ground so the fine
//! routing follows it, and later carved as a channel with the coarse discharge below a surface
//! no higher than the coarse one.

use super::coarse::Coarse;
use super::flood::{Dims, FIXED, OUTLET, SEA};
use super::lakes::{Basin, Kind, NO_EXIT};
use serde_json::{Map, Value, json};
use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, HashSet},
};

/// Judgment constants: deep coarse lake cells keep fine ground this far below the level; dry
/// shore cells beside a lake are at least this far above it, so a lake leaves only by its
/// coarse exit.
pub(super) const LAKE_BED_M: f32 = 0.25;
pub(super) const SHORE_M: f32 = 0.05;
/// Coarse lake cells at least this deep get the fine bed below the level.
const DEEP_LAKE_M: f32 = 1.0;
/// Fine water of a coarse body may stand this far above the coarse surface (f32 rounding).
pub(super) const SURFACE_TOLERANCE_M: f32 = 0.01;

/// Coarse lakes on the fine ground: one lake basin per coarse lake in the window, in `basins`
/// order with the coarse id. Wet where the fine ground is below the level, connected to the
/// coarse lake's cells within one coarse cell of them; deep coarse cells get the bed below the
/// level; dry ground beside the lake below `level + SHORE_M` is raised to it. `fixed_lake`
/// gives the coarse lake id of cells copied from the previous band.
pub(super) fn lakes(
    dims: Dims,
    ground: &mut [f32],
    flags: &[u8],
    body: &mut [u32],
    basins: &mut Vec<Basin>,
    c: &Coarse,
    fixed_lake: impl Fn(usize) -> Option<u32>,
) -> Value {
    let n = ground.len();
    let cd = c.dims();
    // Lake each coarse cell is near: its own, else the lowest id among its neighbours.
    let near: Vec<u32> = (0..c.body.len())
        .map(|q| {
            c.lake(q).unwrap_or_else(|| {
                cd.neighbours(q)
                    .filter_map(|(_, p)| c.lake(p))
                    .min()
                    .unwrap_or(0)
            })
        })
        .collect();
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
    let free = |flags: &[u8], i: usize| flags[i] & (SEA | FIXED) == 0;
    let (mut bed_cells, mut restored) = (0u64, 0u64);
    let mut stack: Vec<u32> = Vec::new();
    let mut bed: Vec<u32> = Vec::new();
    for i in 0..n {
        if flags[i] & FIXED != 0 {
            if let Some(b) = fixed_lake(i).and_then(|id| index.get(&id)) {
                body[i] = *b;
                stack.push(i as u32);
            }
            continue;
        }
        let q = c.cell(dims.w, i);
        let Some(id) = c.lake(q).filter(|_| free(flags, i)) else {
            continue;
        };
        let level = c.lakes[&id].level;
        // Deep coarse cells are wet; their bed is lowered once the lake's piece is known.
        let deep = level - c.ground[q] >= DEEP_LAKE_M && ground[i] >= level - LAKE_BED_M;
        if deep {
            bed.push(i as u32);
        }
        if ground[i] < level || deep {
            body[i] = index[&id];
            stack.push(i as u32);
        }
    }
    // A lake whose fine ground is all above its level keeps one cell below it.
    let mut lowest: HashMap<u32, usize> = HashMap::new();
    for i in 0..n {
        let q = c.cell(dims.w, i);
        if let Some(id) = c.lake(q).filter(|_| free(flags, i)) {
            let e = lowest.entry(id).or_insert(i);
            if ground[i] < ground[*e] {
                *e = i;
            }
        }
    }
    let mut present = vec![false; basins.len() + 1];
    for i in 0..n {
        present[body[i] as usize] = true;
    }
    for (id, i) in lowest {
        if !present[index[&id] as usize] {
            ground[i] = ground[i].min(c.lakes[&id].level - LAKE_BED_M);
            body[i] = index[&id];
            stack.push(i as u32);
            restored += 1;
        }
    }
    // Connected ground below the level near the coarse lake.
    let level_of = |b: u32| basins[b as usize - 1].level;
    let id_of: HashMap<u32, u32> = index.iter().map(|(&id, &b)| (b, id)).collect();
    while let Some(a) = stack.pop() {
        let b = body[a as usize];
        for (_, j) in dims.neighbours(a as usize) {
            if body[j] == 0
                && free(flags, j)
                && ground[j] < level_of(b)
                && near[c.cell(dims.w, j)] == id_of[&b]
            {
                body[j] = b;
                stack.push(j as u32);
            }
        }
    }
    // One piece per lake: the largest, and any that may join it beyond the grid (fixed
    // cells or the top and bottom rows); a smaller piece cut off by fine ground above the
    // level stays dry.
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
            edge |= flags[a] & FIXED != 0 || a < dims.w || a >= n - dims.w;
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
    drop(piece);
    for &i in &bed {
        let i = i as usize;
        if body[i] != 0 {
            ground[i] = basins[body[i] as usize - 1].level - LAKE_BED_M;
            bed_cells += 1;
        }
    }
    // Shore: dry ground beside a lake stays above its level.
    let (mut shore, mut shore_max) = (0u64, 0f32);
    for i in 0..n {
        if body[i] <= first || flags[i] & SEA != 0 {
            continue;
        }
        let top = level_of(body[i]) + SHORE_M;
        for (_, j) in dims.neighbours(i) {
            if body[j] == 0 && free(flags, j) && ground[j] < top {
                shore += 1;
                shore_max = shore_max.max(top - ground[j]);
                ground[j] = top;
            }
        }
    }
    json!({
        "coarse_lakes": ids.len(),
        "bed_cells_lowered": bed_cells,
        "lakes_given_one_cell": restored,
        "cut_off_cells_left_dry": dropped,
        "shore_cells_raised": shore,
        "shore_max_raise_m": shore_max,
    })
}

/// Coarse-consistency checks (each must be 0): every coarse lake with fine cells here present
/// at the same level, every coarse river present and routed, no fine water of a coarse river
/// above its coarse surface (the highest within two coarse cells) by more than the tolerance.
#[allow(clippy::too_many_arguments)]
pub(super) fn consistency(
    dims: Dims,
    flags: &[u8],
    surface: &[f32],
    basins: &[Basin],
    c: &Coarse,
    unrouted: &[u32],
    body_id: &dyn Fn(usize) -> u32,
) -> (Map<String, Value>, f32) {
    let (w, h) = (dims.w, dims.h);
    let mut present: HashSet<u32> = HashSet::new();
    let mut above = 0u64;
    let mut above_max = 0f32;
    for i in 0..w * h {
        let id = body_id(i);
        if id == 0 {
            continue;
        }
        present.insert(id);
        if flags[i] & FIXED != 0 || !c.rivers.contains_key(&id) {
            continue;
        }
        let [cx, cy] = c.abs(c.cell(w, i));
        let top = (-2..=2)
            .flat_map(|dy| (-2..=2).map(move |dx| [cx as isize + dx, cy as isize + dy]))
            .filter_map(|p| c.at(p))
            .filter(|&q| c.body[q] == id)
            .map(|q| c.surface[q])
            .fold(f32::NEG_INFINITY, f32::max);
        if top.is_finite() && surface[i] > top + SURFACE_TOLERANCE_M {
            above += 1;
            above_max = above_max.max(surface[i] - top);
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
        if let Some(id) = c.river(q).filter(|_| c.centre(q)) {
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
    (m, above_max)
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

/// Lower each path to a running minimum from its head (not below a lake floor beside it).
pub(super) fn burn(
    dims: Dims,
    ground: &mut [f32],
    body: &[u32],
    basins: &[Basin],
    paths: &[Path],
) -> (Vec<u32>, f32) {
    let (mut cells, mut max) = (Vec::new(), 0f32);
    for p in paths {
        let mut g = ground[p.cells[0] as usize];
        for &c in &p.cells[1..] {
            let f = floor(dims, body, basins, p.source, c as usize);
            let c = c as usize;
            g = ground[c].min(g.max(f));
            if g < ground[c] {
                cells.push(c as u32);
                max = max.max(ground[c] - g);
                ground[c] = g;
            }
        }
    }
    (cells, max)
}

/// Point each path cell at the next.
pub(super) fn route(dims: Dims, dir: &mut [u8], paths: &[Path]) {
    for p in paths {
        for (k, &c) in p.cells.iter().enumerate() {
            let to = p.cells.get(k + 1).copied().or(p.next);
            if let Some(to) = to {
                dir[c as usize] = dims.towards(c as usize, to as usize);
            }
        }
    }
}

/// Fine centrelines of the coarse rivers (largest first). A river's corridor is the fine cells
/// of its coarse cells and their neighbours (not other coarse rivers or lakes, the sea, fixed
/// cells or earlier paths). A priority flood over it starts from where the coarse river ends
/// (the sea, its lake, a dry basin cell, or a grid-edge outlet in the corridor); each coarse
/// head (a centreline cell without a centreline donor in the window) starts its path at the
/// lowest fine cell it holds, beside the lake when the river leaves one, and follows the flood
/// down. The surface cap is the coarse surface interpolated along the path between the middles
/// of its runs through the river's coarse cells.
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
    let fully_fixed = |q: usize| c.fine_cells(q, w, h).all(|i| flags[i] & FIXED != 0);
    let mut claimed: HashSet<u32> = HashSet::new();
    let mut out = Imprint {
        ids: Vec::new(),
        paths: Vec::new(),
        cap: HashMap::new(),
        q: HashMap::new(),
        unrouted: Vec::new(),
        burned: Vec::new(),
        burn_max_m: 0.0,
    };
    for id in ids {
        let own = &cells_of[&id];
        let parent = c.rivers[&id].parent;
        let mut cq: HashSet<usize> = HashSet::new();
        for &q in own {
            cq.insert(q);
            for (_, p) in cd.neighbours(q) {
                let other = c.lake(p).is_some() || c.river(p).is_some_and(|r| r != id);
                if !other && c.dry_basin(p).is_none() {
                    cq.insert(p);
                }
            }
        }
        // A path touches only the lake it ends in, and a lake it leaves within that lake's
        // exit cell. Elsewhere a river beside a lake would stand below its level, and shores
        // hold ground above it, which would dam the burned path.
        let end_lake = lake_basin.get(&parent).copied();
        let shore = |i: usize| {
            let q = c.abs(c.cell(w, i));
            let leaves = exits.get(&q).and_then(|l| lake_basin.get(l)).copied();
            dims.neighbours(i).any(|(_, j)| {
                body[j] != 0
                    && basins[body[j] as usize - 1].kind == Kind::Lake
                    && Some(body[j]) != end_lake
                    && Some(body[j]) != leaves
            })
        };
        let mut corridor: HashSet<u32> = HashSet::new();
        for &q in &cq {
            for i in c.fine_cells(q, w, h) {
                if flags[i] & (SEA | FIXED) == 0
                    && body[i] == 0
                    && !claimed.contains(&(i as u32))
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
        let dry_end = c.dry.contains(&parent);
        let is_seed = |j: usize| -> Option<f32> {
            if flags[j] & SEA != 0 && parent == 1 {
                Some(0.0)
            } else if flags[j] & FIXED != 0 {
                (north_end && fixed_id(j) == id).then_some(ground[j])
            } else if lake_end.is_some_and(|b| body[j] == b) {
                Some(basins[body[j] as usize - 1].level)
            } else if dry_end && c.dry_basin(c.cell(w, j)).is_some() && body[j] == 0 {
                Some(ground[j])
            } else {
                None
            }
        };
        let mut heap = BinaryHeap::new();
        let mut recv: HashMap<u32, u32> = HashMap::new();
        let mut seeds: HashSet<u32> = HashSet::new();
        for &i in &corridor {
            let i = i as usize;
            if flags[i] & OUTLET != 0 {
                seeds.insert(i as u32);
                recv.insert(i as u32, u32::MAX);
                heap.push(Item(ground[i], i as u32));
            }
            for (_, j) in dims.neighbours(i) {
                if !corridor.contains(&(j as u32)) && !seeds.contains(&(j as u32)) {
                    if let Some(z) = is_seed(j) {
                        seeds.insert(j as u32);
                        heap.push(Item(z, j as u32));
                    }
                }
            }
        }
        while let Some(Item(z, a)) = heap.pop() {
            for (_, j) in dims.neighbours(a as usize) {
                let jj = j as u32;
                if corridor.contains(&jj) && !recv.contains_key(&jj) {
                    recv.insert(jj, a);
                    heap.push(Item(ground[j].max(z), jj));
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
            let lowest = |f: &dyn Fn(usize) -> bool| {
                reached
                    .iter()
                    .copied()
                    .filter(|&i| f(i))
                    .min_by(|&a, &b| ground[a].total_cmp(&ground[b]).then(a.cmp(&b)))
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
                    if c.fine_cells(q, w, h).any(|i| flags[i] & (SEA | FIXED) == 0) {
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
                match recv[&at] {
                    u32::MAX => break None,
                    r if seeds.contains(&r) => break Some(r),
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
                // Never above this river's coarse surface beside the cell.
                let [qx, qy] = c.abs(q);
                let local = (-1..=1)
                    .flat_map(|dy| (-1..=1).map(move |dx| [qx as isize + dx, qy as isize + dy]))
                    .filter_map(|p| c.at(p))
                    .filter(|&p| c.body[p] == id)
                    .map(|p| c.surface[p])
                    .fold(f32::NEG_INFINITY, f32::max);
                let v = if local.is_finite() { v.min(local) } else { v };
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
            claimed.extend(cells.iter().copied());
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
    let (cells, max) = burn(dims, ground, body, basins, &out.paths);
    out.burned = cells;
    out.burn_max_m = max;
    out
}
