//! Coarse-level erosion (spec "Coarse stages"; rulings B1–B7, B23, B29). Each step reads one
//! model run on the current ground and lowers it:
//! 1. implicit stream power (Braun and Willett 2013, n = 1, no uplift) in ascending ground
//!    order: a dry receiver is strictly lower (R12), so it is updated first. Routing is
//!    lake-aware: cells under a node's water do not erode, cells draining into a node take
//!    its level as their base, and a full node's sill cell erodes with the node's whole
//!    discharge while the level follows it (Cordonnier et al. 2019 with one outlet per node);
//! 2. lowering-only hillslope diffusion (convex cells; no deposition, out of the spec's scope);
//! 3. each cut re-expressed as Plan A's parabolic gorge taper, then wall rounds.
//!
//! Never below the floor (the breached ground less the cell's cap) and never raised. One
//! global single-threaded pass in a fixed order: deterministic.

use super::checks::{CHANGED_M, horn_deg, is_wall};
use super::geom::{OFFSETS, World};
use super::nodes::Nodes;
use serde_json::{Value, json};

/// Cuts shallower than this are not tapered.
pub(crate) const SHAPE_MIN_M: f32 = 0.01;
/// Wall rounds after the taper: each halves this routing's lowering in the 3 × 3 of a wall.
pub(crate) const WALL_ROUNDS: usize = 16;
/// The narrowest taper (cells): a disc of radius 2.5 lowers a 5-cell-wide cut, wide enough
/// that even an isolated disc passes `handoff_narrow` (preflight F1).
pub(crate) const TAPER_MIN_CELLS: f64 = 2.5;

/// A bit per cell.
pub(crate) struct Bits(Vec<u64>);

impl Bits {
    pub fn new(n: usize) -> Self {
        Self(vec![0; n.div_ceil(64)])
    }
    pub fn get(&self, i: usize) -> bool {
        (self.0[i >> 6] >> (i & 63)) & 1 == 1
    }
    pub fn set(&mut self, i: usize) {
        self.0[i >> 6] |= 1 << (i & 63);
    }
}

/// A full node's sill: the higher cell of its spill edge, eroded with the node's discharge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sill {
    pub cell: u32,
    /// The cell it drains to: the spill edge's outside cell, or that cell's receiver.
    pub down: u32,
    /// The node's ordinal in `Nodes`.
    pub node: u32,
    /// The other spill-edge cell's ground: the level never falls below it.
    pub other_z: f32,
    /// k·A^m with A the outside cell's discharge (the node's whole flow).
    pub ka: f32,
}

/// One routing: the model run on the current ground, as the passes read it.
pub(crate) struct Routing<'a> {
    pub world: World,
    pub spacing_m: f64,
    /// The breached ground less the cell's cap.
    pub floor: &'a [f32],
    /// k·A^m per cell (k by provenance, A in m²); 0 where nothing erodes.
    pub ka: &'a [f32],
    /// The model's flow layer (low 4 bits: D8 receiver, 0 none).
    pub flow: &'a [u8],
    /// Cells under a node's water (node raster >= 2).
    pub held: &'a Bits,
    pub sea: &'a Bits,
    /// Interior and glacier cells: no diffusion.
    pub interior: &'a Bits,
    /// Held cells some dry cell drains into, with their node ordinal, sorted by cell.
    pub shore: &'a [(u32, u32)],
    /// Sorted by cell.
    pub sills: &'a [Sill],
    pub nodes: &'a Nodes,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Figures {
    pub lowered_cells: u64,
    pub max_lowering_m: f32,
    pub capped_cells: u64,
    pub sills_lowered: u64,
    pub order_ties: u64,
    pub diffused_cells: u64,
    pub tapered_cells: u64,
    pub wall_cells_restored: u64,
    pub walls_left: u64,
    /// Strict local minima of the eroded ground that were none in the breached ground (B33).
    pub new_pits: u64,
    /// Lowering of full-K cells outside the cut footprint (B6), cm-binned p99 and max.
    pub hillslope_p99_m: f32,
    pub hillslope_max_m: f32,
}

impl Figures {
    pub fn to_blob(self) -> (Vec<u64>, f32) {
        (
            vec![
                self.lowered_cells,
                self.capped_cells,
                self.sills_lowered,
                self.order_ties,
                self.diffused_cells,
                self.tapered_cells,
                self.wall_cells_restored,
                self.walls_left,
                self.new_pits,
                u64::from(self.hillslope_p99_m.to_bits()),
                u64::from(self.hillslope_max_m.to_bits()),
            ],
            self.max_lowering_m,
        )
    }
    pub fn from_blob((v, max): (Vec<u64>, f32)) -> Self {
        let g = |i: usize| v.get(i).copied().unwrap_or(0);
        Self {
            lowered_cells: g(0),
            capped_cells: g(1),
            sills_lowered: g(2),
            order_ties: g(3),
            diffused_cells: g(4),
            tapered_cells: g(5),
            wall_cells_restored: g(6),
            walls_left: g(7),
            new_pits: g(8),
            hillslope_p99_m: f32::from_bits(g(9) as u32),
            hillslope_max_m: f32::from_bits(g(10) as u32),
            max_lowering_m: max,
        }
    }
    pub fn add(&mut self, o: Figures) {
        self.lowered_cells += o.lowered_cells;
        self.max_lowering_m = self.max_lowering_m.max(o.max_lowering_m);
        self.capped_cells += o.capped_cells;
        self.sills_lowered += o.sills_lowered;
        self.order_ties += o.order_ties;
        self.diffused_cells += o.diffused_cells;
        self.tapered_cells += o.tapered_cells;
        self.wall_cells_restored += o.wall_cells_restored;
        self.walls_left += o.walls_left;
        self.new_pits = o.new_pits; // the last routing's ground is what m2 reads
        self.hillslope_p99_m = self.hillslope_p99_m.max(o.hillslope_p99_m);
        self.hillslope_max_m = self.hillslope_max_m.max(o.hillslope_max_m);
    }
    pub fn json(&self) -> Value {
        json!({
            "lowered_cells": self.lowered_cells, "max_lowering_m": self.max_lowering_m,
            "capped_cells": self.capped_cells, "sills_lowered": self.sills_lowered,
            "order_ties": self.order_ties, "diffused_cells": self.diffused_cells,
            "tapered_cells": self.tapered_cells, "wall_cells_restored": self.wall_cells_restored,
            "walls_left": self.walls_left, "erosion_new_pits": self.new_pits,
            "hillslope_lowering_p99_m": self.hillslope_p99_m, "hillslope_lowering_max_m": self.hillslope_max_m,
        })
    }
}

/// The D8 receiver of cell `c` for direction byte `d` (1 N … 8 SE; 0 none).
pub(crate) fn receiver(w: usize, h: usize, c: usize, d: u8) -> Option<usize> {
    if d == 0 || d > 8 {
        return None;
    }
    let (dy, dx) = OFFSETS[d as usize - 1];
    let (x, y) = ((c % w) as i64 + dx, (c / w) as i64 + dy);
    (x >= 0 && y >= 0 && x < w as i64 && y < h as i64).then(|| y as usize * w + x as usize)
}

/// f32 → u32 with the same order (negative values included).
fn orderable(v: f32) -> u32 {
    let b = v.to_bits();
    if b >> 31 == 1 { !b } else { b | 0x8000_0000 }
}

fn dist_cells(w: usize, a: usize, b: usize) -> f64 {
    let (dx, dy) = (
        (a % w) as f64 - (b % w) as f64,
        (a / w) as f64 - (b / w) as f64,
    );
    dx.hypot(dy).max(1.0)
}

/// Implicit stream power, one step: z ← (z + F·base) / (1 + F), F = k·A^m / L, in ascending
/// ground order; a cell whose receiver (or, draining into a lake, the lake's sill) is not yet
/// updated has that chain processed first (review M3), so no base is stale. Never below the
/// floor or the base, never raised.
pub(crate) fn stream_power(r: &Routing, z: &mut [f32]) -> Figures {
    let (w, h) = (r.world.w as usize, r.world.h as usize);
    let sill_at = |c: usize| r.sills.binary_search_by_key(&(c as u32), |s| s.cell).ok();
    let erodible = |c: usize| !r.sea.get(c) && (!r.held.get(c) || sill_at(c).is_some());
    let shore_node = |t: usize| {
        r.shore
            .binary_search_by_key(&(t as u32), |p| p.0)
            .ok()
            .map(|i| r.shore[i].1 as usize)
    };
    // The cell whose new height this cell's base needs: its receiver, or the sill of the lake
    // it drains into.
    // Each node's sill cell (first by cell), so a lake's drainers find it without a scan.
    let mut node_sill: Vec<Option<u32>> = vec![None; r.nodes.0.len()];
    for s in r.sills {
        node_sill[s.node as usize].get_or_insert(s.cell);
    }
    let needs = |c: usize| -> Option<usize> {
        if let Some(i) = sill_at(c) {
            return Some(r.sills[i].down as usize);
        }
        let t = receiver(w, h, c, r.flow[c] & 15)?;
        if r.held.get(t) && sill_at(t).is_none() {
            return node_sill[shore_node(t)?].map(|s| s as usize);
        }
        Some(t)
    };
    // u32 indices sorted by (ground, index): 4 B a cell (preflight F11).
    let mut order: Vec<u32> = (0..z.len() as u32)
        .filter(|&c| erodible(c as usize))
        .collect();
    order.sort_unstable_by_key(|&c| (orderable(z[c as usize]), c));
    let mut level: Vec<f32> = r.nodes.0.iter().map(|n| n.level).collect();
    let mut done = Bits::new(z.len());
    let mut fig = Figures::default();
    let mut stack: Vec<usize> = Vec::new();
    let mut on_stack = Bits::new(z.len()); // preflight F30: no linear `contains`
    for c0 in order {
        let c0 = c0 as usize;
        if done.get(c0) {
            continue;
        }
        stack.push(c0);
        on_stack.set(c0);
        while let Some(&c) = stack.last() {
            if let Some(t) = needs(c).filter(|&t| erodible(t) && !done.get(t)) {
                // A tie (receiver not lower) or a lake whose sill is not done: that first.
                if !on_stack.get(t) {
                    fig.order_ties += 1;
                    stack.push(t);
                    on_stack.set(t);
                    continue;
                }
            }
            stack.pop();
            done.set(c);
            let zc = z[c];
            let (to, ka, base) = if let Some(i) = sill_at(c) {
                let s = r.sills[i];
                (s.down as usize, s.ka, z[s.down as usize])
            } else {
                let Some(to) = receiver(w, h, c, r.flow[c] & 15) else {
                    continue;
                };
                let base = if r.held.get(to) && sill_at(to).is_none() {
                    shore_node(to).map_or(z[to], |o| level[o])
                } else {
                    z[to]
                };
                (to, r.ka[c], base)
            };
            // v2: rivers into the sea grade to 0 m, not to the seabed.
            let base = if r.sea.get(to) { base.max(0.0) } else { base };
            if ka <= 0.0 || zc <= base {
                continue;
            }
            let f = f64::from(ka) / (dist_cells(w, c, to) * r.spacing_m);
            let mut v = ((f64::from(zc) + f * f64::from(base)) / (1.0 + f)) as f32;
            if v < r.floor[c] {
                v = r.floor[c];
                fig.capped_cells += 1;
            }
            let v = v.min(zc);
            if zc - v > CHANGED_M {
                fig.lowered_cells += 1;
                fig.max_lowering_m = fig.max_lowering_m.max(zc - v);
            }
            z[c] = v;
            if let Some(i) = sill_at(c) {
                let s = r.sills[i];
                let n = &r.nodes.0[s.node as usize];
                let lv = v.max(s.other_z).max(n.low).min(n.level);
                if lv < level[s.node as usize] - CHANGED_M {
                    fig.sills_lowered += 1;
                }
                level[s.node as usize] = lv;
            }
        }
    }
    fig
}

/// Cells within R_taper + 1 of a cell this routing's stream power lowered by at least
/// SHAPE_MIN_M: where diffusion may act (B6, review I3).
pub(crate) fn footprint(r: &Routing, before: &[f32], z: &[f32], wall_deg: f64) -> Bits {
    let (w, h) = (r.world.w as usize, r.world.h as usize);
    let tan = wall_deg.to_radians().tan();
    let mut near = Bits::new(z.len());
    for c in 0..z.len() {
        let d = before[c] - z[c];
        if d < SHAPE_MIN_M {
            continue;
        }
        let radius = (2.0 * f64::from(d) / (tan * r.spacing_m)).max(TAPER_MIN_CELLS) + 1.0;
        let k = radius.ceil() as i64;
        let (cx, cy) = ((c % w) as i64, (c / w) as i64);
        for dy in -k..=k {
            for dx in -k..=k {
                let (x, y) = (cx + dx, cy + dy);
                if ((dx * dx + dy * dy) as f64).sqrt() <= radius
                    && x >= 0
                    && y >= 0
                    && x < w as i64
                    && y < h as i64
                {
                    near.set(y as usize * w + x as usize);
                }
            }
        }
    }
    near
}

/// Lowering-only hillslope diffusion, one explicit Jacobi step (alpha = D·dt/dx² <= 0.25),
/// only on `near` cells (the river cuts' footprint, B6): a convex cell moves toward its 4
/// neighbours' mean, never below the floor; concave cells, interior, held, sea and edge cells
/// stay. Returns the cells lowered.
pub(crate) fn diffuse(r: &Routing, z: &mut [f32], alpha: f32, near: &Bits) -> u64 {
    let (w, h) = (r.world.w as usize, r.world.h as usize);
    if alpha <= 0.0 || w < 3 || h < 3 {
        return 0;
    }
    let mut above = z[..w].to_vec();
    let mut row = vec![0f32; w];
    let mut n = 0;
    for y in 1..h - 1 {
        row.copy_from_slice(&z[y * w..(y + 1) * w]);
        for x in 1..w - 1 {
            let c = y * w + x;
            if !near.get(c) || r.held.get(c) || r.sea.get(c) || r.interior.get(c) {
                continue;
            }
            // above: row y - 1 before this step; z[c + w]: row y + 1, not yet updated.
            let lap = above[x] + z[c + w] + row[x - 1] + row[x + 1] - 4.0 * row[x];
            if lap >= 0.0 {
                continue;
            }
            let v = (row[x] + alpha * lap).max(r.floor[c]);
            if v < z[c] {
                z[c] = v;
                n += 1;
            }
        }
        std::mem::swap(&mut above, &mut row);
    }
    n
}

/// Each cut (before − z ≥ SHAPE_MIN_M) as a parabolic disc d·(1 − (ρ/R)²), R = max(2.5 cells,
/// 2·d / (tan(wall)·spacing)); the deepest lowering over all discs, not under water or sea,
/// never below the floor. Then up to WALL_ROUNDS rounds, each halving this step's lowering in
/// the 3 × 3 of every changed cell steeper than max(wall, raw) and then grading beds (a
/// changed cell below its receiver is lifted to it, never above `before`; B33, Plan A 16b),
/// until neither changes anything. Returns (cells tapered, cells restored, walls left).
pub(crate) fn taper(
    r: &Routing,
    z: &mut [f32],
    before: &[f32],
    raw: &[f32],
    wall_deg: f64,
    lower: &mut [f32],
) -> (u64, u64, u64) {
    let (w, h) = (r.world.w as usize, r.world.h as usize);
    let tan = wall_deg.to_radians().tan();
    lower.fill(0.0);
    for c in 0..z.len() {
        let d = before[c] - z[c];
        if d < SHAPE_MIN_M || r.held.get(c) || r.sea.get(c) {
            continue;
        }
        let radius = (2.0 * f64::from(d) / (tan * r.spacing_m)).max(TAPER_MIN_CELLS);
        let k = radius.ceil() as i64;
        let (cx, cy) = ((c % w) as i64, (c / w) as i64);
        for dy in -k..=k {
            for dx in -k..=k {
                let rho = ((dx * dx + dy * dy) as f64).sqrt();
                let (x, y) = (cx + dx, cy + dy);
                if rho >= radius || x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                    continue;
                }
                let j = y as usize * w + x as usize;
                if r.held.get(j) || r.sea.get(j) {
                    continue;
                }
                let lw = (f64::from(d) * (1.0 - (rho / radius).powi(2))) as f32;
                if lw > lower[j] {
                    lower[j] = lw;
                }
            }
        }
    }
    let mut changed: Vec<usize> = Vec::new();
    let mut tapered = 0;
    for j in 0..z.len() {
        if lower[j] <= 0.0 {
            continue;
        }
        let v = (before[j] - lower[j]).max(r.floor[j]);
        if v < z[j] {
            z[j] = v;
            tapered += 1;
        }
        if before[j] - z[j] > CHANGED_M {
            changed.push(j);
        }
    }
    // Wall rounds (as Plan A's channel restore): cells near this step's cuts only.
    let inner = |j: usize| {
        let (x, y) = (j % w, j / w);
        x > 0 && y > 0 && x + 1 < w && y + 1 < h
    };
    let mut restored = 0;
    let mut walls = Vec::new();
    for round in 0..=WALL_ROUNDS {
        walls.clear();
        for &c in &changed {
            let (cx, cy) = ((c % w) as i64, (c / w) as i64);
            for (dy, dx) in OFFSETS.iter().copied().chain([(0, 0)]) {
                let (x, y) = (cx + dx, cy + dy);
                if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                    continue;
                }
                let j = y as usize * w + x as usize;
                if inner(j)
                    && raw[j] - z[j] > CHANGED_M
                    && is_wall(
                        horn_deg(z, w, j % w, j / w, r.spacing_m),
                        horn_deg(raw, w, j % w, j / w, r.spacing_m),
                        wall_deg,
                    )
                {
                    walls.push(j);
                }
            }
        }
        walls.sort_unstable();
        walls.dedup();
        // Bed grade (to its fixed point inside the round): no changed cell below its receiver.
        let mut graded = false;
        loop {
            let mut moved = false;
            for &c in &changed {
                let Some(t) = receiver(w, h, c, r.flow[c] & 15) else {
                    continue;
                };
                if r.held.get(t) || r.sea.get(t) {
                    continue;
                }
                let v = before[c].min(z[c].max(z[t]));
                if v > z[c] {
                    z[c] = v;
                    moved = true;
                }
            }
            if !moved {
                break;
            }
            graded = true;
        }
        if (walls.is_empty() && !graded) || round == WALL_ROUNDS {
            break;
        }
        let mut fix: Vec<usize> = Vec::new();
        for &j in &walls {
            let (cx, cy) = ((j % w) as i64, (j / w) as i64);
            for (dy, dx) in OFFSETS.iter().copied().chain([(0, 0)]) {
                let (x, y) = (cx + dx, cy + dy);
                if x >= 0 && y >= 0 && x < w as i64 && y < h as i64 {
                    fix.push(y as usize * w + x as usize);
                }
            }
        }
        fix.sort_unstable();
        fix.dedup();
        for j in fix {
            if before[j] - z[j] > CHANGED_M {
                z[j] = before[j] - (before[j] - z[j]) / 2.0;
                restored += 1;
            }
        }
    }
    (tapered, restored, walls.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::water::nodes::Node;

    /// A w × 1 strip flowing east (direction 4) to an outlet at the east end; discharge 10 km²
    /// everywhere (A^0.5 = 3162 m), k_dt 7e-4 → ka = 2.21; 32 m cells.
    struct Strip {
        z: Vec<f32>,
        floor: Vec<f32>,
        ka: Vec<f32>,
        flow: Vec<u8>,
        held: Bits,
        sea: Bits,
        interior: Bits,
        shore: Vec<(u32, u32)>,
        sills: Vec<Sill>,
        nodes: Nodes,
    }

    impl Strip {
        fn new(z: &[f32]) -> Self {
            let n = z.len();
            let mut flow = vec![4u8; n];
            flow[n - 1] = 0;
            Self {
                z: z.to_vec(),
                floor: vec![f32::NEG_INFINITY; n],
                ka: vec![(7e-4 * (10.0f64 * 1e6).sqrt()) as f32; n],
                flow,
                held: Bits::new(n),
                sea: Bits::new(n),
                interior: Bits::new(n),
                shore: Vec::new(),
                sills: Vec::new(),
                nodes: Nodes::default(),
            }
        }
        fn routing(&self) -> Routing<'_> {
            Routing {
                world: World {
                    w: self.z.len() as u64,
                    h: 1,
                },
                spacing_m: 32.0,
                floor: &self.floor,
                ka: &self.ka,
                flow: &self.flow,
                held: &self.held,
                sea: &self.sea,
                interior: &self.interior,
                shore: &self.shore,
                sills: &self.sills,
                nodes: &self.nodes,
            }
        }
        fn erode(&mut self) -> Figures {
            let mut z = self.z.clone();
            let f = stream_power(&self.routing(), &mut z);
            self.z = z;
            f
        }
    }

    #[test]
    fn a_seam_step_on_a_river_grades_and_nothing_rises() {
        let before = [40.0, 40.0, 40.0, 40.0, 10.0, 10.0, 10.0, 0.0f32];
        let mut s = Strip::new(&before);
        s.erode();
        assert!(s.z[3] < 40.0 - 1.0, "the step's lip is cut, got {}", s.z[3]);
        for (i, b) in before.iter().enumerate() {
            assert!(s.z[i] <= *b, "cell {i} rose");
        }
        for i in 0..before.len() - 1 {
            assert!(s.z[i] >= s.z[i + 1], "cell {i} below its receiver");
        }
    }

    #[test]
    fn interior_k_barely_moves_real_ground() {
        let before = [40.0, 39.0, 38.0, 37.0, 36.0, 35.0, 34.0, 0.0f32];
        let mut s = Strip::new(&before);
        for k in s.ka.iter_mut() {
            *k *= 0.05;
        }
        s.erode();
        let most = (0..before.len())
            .map(|i| before[i] - s.z[i])
            .fold(0.0f32, f32::max);
        assert!(
            most < 1.0,
            "an interior reach moves under 1 m per step, got {most}"
        );
    }

    #[test]
    fn the_floor_caps_lowering() {
        let before = [40.0, 40.0, 40.0, 40.0, 10.0, 10.0, 10.0, 0.0f32];
        let mut s = Strip::new(&before);
        // Unfloored, the lip falls to 38.06 m in one step; a 1 m floor stops it at 39 m.
        s.floor = before.iter().map(|v| v - 1.0).collect();
        let f = s.erode();
        assert!(s.z[3] >= 39.0 - 1e-4 && f.capped_cells > 0);
    }

    #[test]
    fn a_full_lake_drains_through_its_sill_and_its_level_follows() {
        // Cells 2–4 are a lake (node 0) at level 30; its spill edge is [4, 5] with cell 5 the
        // saddle (30 m); cell 1 drains into the lake at cell 2.
        let before = [45.0, 35.0, 25.0, 24.0, 26.0, 30.0, 12.0, 0.0f32];
        let mut s = Strip::new(&before);
        for c in [2, 3, 4] {
            s.held.set(c);
        }
        s.nodes = Nodes(vec![Node {
            key: 3,
            level: 30.0,
            low: 24.0,
            full: true,
            lake: true,
            outlet: [4, 5],
            cells: 3,
            ..Default::default()
        }]);
        s.shore = vec![(2, 0)];
        s.sills = vec![Sill {
            cell: 5,
            down: 6,
            node: 0,
            other_z: 26.0,
            ka: s.ka[5] * 2.0,
        }];
        let f = s.erode();
        assert!(s.z[5] < 30.0 - 1.0, "the sill incises, got {}", s.z[5]);
        assert_eq!(f.sills_lowered, 1);
        assert!(
            s.z[1] < 35.0,
            "the cell above the lake erodes toward the lowered level"
        );
        assert!(
            s.z[1] >= s.z[5].max(26.0) - 1e-4,
            "never below the new level"
        );
        assert_eq!(
            &s.z[2..5],
            &before[2..5],
            "ground under the lake does not erode"
        );
    }

    #[test]
    fn rivers_into_the_sea_stop_at_sea_level() {
        let before = [20.0, 10.0, 2.0, -30.0, -40.0f32];
        let mut s = Strip::new(&before);
        s.sea.set(3);
        s.sea.set(4);
        s.erode();
        assert!(s.z[2] >= 0.0, "base 0 m at a mouth, got {}", s.z[2]);
        assert_eq!(&s.z[3..], &before[3..]);
    }

    #[test]
    fn diffusion_only_lowers_convex_cells() {
        // 5 × 5 with a 2 m bump at the centre and a 1 m pit beside it.
        let w = 5usize;
        let mut z = vec![10.0f32; 25];
        z[12] = 12.0;
        z[13] = 9.0;
        let mut s = Strip::new(&z);
        s.flow = vec![0; 25];
        let mut r = s.routing();
        r.world = World { w: w as u64, h: 5 };
        let mut out = z.clone();
        let mut all = Bits::new(25);
        (0..25).for_each(|i| all.set(i));
        let n = diffuse(&r, &mut out, 0.05, &all);
        assert!(out[12] < 12.0 && n >= 1, "the bump drops");
        assert_eq!(out[13], 9.0, "the pit is not filled");
        assert!(out.iter().zip(&z).all(|(a, b)| a <= b), "nothing rises");
        let mut far = z.clone();
        assert_eq!(
            diffuse(&r, &mut far, 0.05, &Bits::new(25)),
            0,
            "no cut nearby: the bump stays (B6)"
        );
    }

    #[test]
    fn a_tie_processes_the_receiver_first() {
        // Cell 3 equals its receiver 4 (a tie); 4 is cut, so 3 must see 4's new height.
        let before = [40.0, 30.0, 20.0, 10.0, 10.0, 5.0, 0.0f32];
        let mut s = Strip::new(&before);
        let f = s.erode();
        assert!(f.order_ties >= 1);
        assert!(
            s.z[3] < 10.0,
            "the tie cell erodes toward its lowered receiver, got {}",
            s.z[3]
        );
        for i in 0..before.len() - 1 {
            assert!(s.z[i] >= s.z[i + 1] - 1e-5, "cell {i} below its receiver");
        }
    }

    #[test]
    fn wall_rounds_never_leave_a_bed_below_its_receiver() {
        // A 9 × 9 trunk falling east on 4 m cells; its middle cut 6 m on a 36° raw side slope, so
        // the wall rounds halve some cuts; the bed grade must leave no thalweg cell below its
        // receiver (Plan A's 16b bed-rise bug).
        let (w, h) = (9usize, 9usize);
        let raw: Vec<f32> = (0..w * h)
            .map(|i| 100.0 - (i % w) as f32 * 0.5 + ((i / w) as f32 - 4.0).abs() * 2.9)
            .collect();
        let mut z = raw.clone();
        for x in 1..w - 1 {
            z[4 * w + x] -= 6.0;
        }
        let mut s = Strip::new(&raw);
        s.flow = (0..w * h)
            .map(|i| if i % w + 1 < w { 4 } else { 0 })
            .collect();
        s.ka = vec![0.0; w * h];
        let mut r = s.routing();
        r.world = World {
            w: w as u64,
            h: h as u64,
        };
        r.spacing_m = 4.0;
        let mut lower = vec![0.0f32; w * h];
        taper(&r, &mut z, &raw, &raw, 35.0, &mut lower);
        for x in 0..w - 1 {
            let c = 4 * w + x;
            assert!(
                z[c] >= z[c + 1] - 1e-5,
                "thalweg cell {x} below its receiver"
            );
        }
        assert!(
            z.iter().zip(&raw).all(|(a, b)| a <= b),
            "nothing rises above raw"
        );
    }

    #[test]
    fn a_deep_cut_is_tapered_three_cells_wide_and_under_the_wall_angle() {
        // 21 × 21 flat at 100 m, 4 m cells; the centre cell was cut 5 m.
        let (w, h) = (21usize, 21usize);
        let before = vec![100.0f32; w * h];
        let mut z = before.clone();
        z[10 * w + 10] = 95.0;
        let mut s = Strip::new(&before);
        s.flow = vec![0; w * h];
        s.ka = vec![0.0; w * h];
        let mut r = s.routing();
        r.world = World {
            w: w as u64,
            h: h as u64,
        };
        r.spacing_m = 4.0;
        let mut lower = vec![0.0f32; w * h];
        let (tapered, _, left) = taper(&r, &mut z, &before, &before, 35.0, &mut lower);
        assert!(tapered >= 8);
        assert!(
            z[10 * w + 11] < 99.0 && z[10 * w + 9] < 99.0,
            "both sides lowered: >= 3 cells wide"
        );
        assert_eq!(left, 0, "no wall left");
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                assert!(
                    horn_deg(&z, w, x, y, 4.0) <= 35.0 + 0.01,
                    "({x}, {y}) too steep"
                );
            }
        }
    }
}
