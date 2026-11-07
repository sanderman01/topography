//! Coarse-level erosion (spec "Coarse stages"; rulings B1–B7, B23, B29). Each step reads one
//! model run on the current ground and lowers it:
//! 1. implicit stream power (Braun and Willett 2013, n = 1, no uplift) in ascending ground
//!    order: a dry receiver is strictly lower (R12), so it is updated first. Routing is
//!    lake-aware: cells under a node's water do not erode, cells draining into a node take
//!    its level as their base, and a full node's sill cell erodes with the node's whole
//!    discharge while the level follows it (Cordonnier et al. 2019 with one outlet per node);
//! 2. lowering-only hillslope diffusion (convex cells; no deposition, out of the spec's scope);
//! 3. each cut re-expressed as Plan A's parabolic gorge taper, then wall and narrow rounds
//!    that halve whole discs.
//!
//! Never below the floor (the breached ground less the cell's cap) and never raised. One
//! global single-threaded pass in a fixed order: deterministic.

use super::checks::{CHANGED_M, NARROW_M, NARROW_SHARE, horn_deg, is_wall};
use super::geom::{OFFSETS, Rect, World};
use super::layers::Layer;
use super::nodes::Nodes;
use super::pipeline::{self, Rasters, Setup};
use super::store::Work;
use super::table::Erosion;
use super::{INTERIOR, KIND_MASK};
use crate::acquisition::Result;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;

/// Glacier class: frozen like v2's ice catchments (B3: interior K).
const GLACIER: u8 = 10;

/// Cuts shallower than this are not tapered.
pub(crate) const SHAPE_MIN_M: f32 = 0.01;
/// Wall rounds after the taper that halve this routing's lowering in the 3 × 3 of a wall;
/// later rounds remove it.
pub(crate) const WALL_ROUNDS: usize = 16;
/// At most this many removal rounds after them (as Plan A's channel `RESTORE_ROUNDS`); walls
/// still left are counted in `walls_left`.
pub(crate) const REMOVE_ROUNDS: usize = 128;
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
    /// The set bits, ascending.
    pub fn ones(&self) -> impl Iterator<Item = usize> + '_ {
        self.0.iter().enumerate().flat_map(|(k, &word)| {
            let mut v = word;
            std::iter::from_fn(move || {
                (v != 0).then(|| {
                    let b = v.trailing_zeros() as usize;
                    v &= v - 1;
                    k * 64 + b
                })
            })
        })
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
    /// Cells whose cut the first WALL_ROUNDS rounds halved (or reset when under 1 cm).
    pub wall_cells_restored: u64,
    pub walls_left: u64,
    /// Strict local minima of the eroded ground that were none in the breached ground (B33).
    pub new_pits: u64,
    /// Lowering of full-K cells outside the cut footprint (B6), cm-binned p99 and max.
    pub hillslope_p99_m: f32,
    pub hillslope_max_m: f32,
    /// Cells whose cut the removal rounds (after WALL_ROUNDS) reset to the routing's start.
    pub wall_cuts_removed: u64,
    /// Wall rounds used (the most over routings).
    pub wall_rounds: u64,
    /// Sills cut by stream power whose cut the wall rounds undid (that lake stays this routing).
    pub sills_reverted: u64,
    /// Narrow cuts (`handoff_narrow`'s rule on raw − z) the rounds found and halved, and those
    /// left on this routing's cells.
    pub narrow_found: u64,
    pub narrow_left: u64,
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
                self.wall_cuts_removed,
                self.wall_rounds,
                self.sills_reverted,
                self.narrow_found,
                self.narrow_left,
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
            wall_cuts_removed: g(11),
            wall_rounds: g(12),
            sills_reverted: g(13),
            narrow_found: g(14),
            narrow_left: g(15),
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
        self.wall_cuts_removed += o.wall_cuts_removed;
        self.wall_rounds = self.wall_rounds.max(o.wall_rounds);
        self.sills_reverted += o.sills_reverted;
        self.narrow_found += o.narrow_found;
        self.narrow_left = o.narrow_left; // the last routing's ground is what m2 reads
    }
    pub fn json(&self) -> Value {
        json!({
            "lowered_cells": self.lowered_cells, "max_lowering_m": self.max_lowering_m,
            "capped_cells": self.capped_cells, "sills_lowered": self.sills_lowered,
            "order_ties": self.order_ties, "diffused_cells": self.diffused_cells,
            "tapered_cells": self.tapered_cells, "wall_cells_restored": self.wall_cells_restored,
            "walls_left": self.walls_left, "erosion_new_pits": self.new_pits,
            "hillslope_lowering_p99_m": self.hillslope_p99_m, "hillslope_lowering_max_m": self.hillslope_max_m,
            "wall_cuts_removed": self.wall_cuts_removed, "wall_rounds": self.wall_rounds,
            "sills_reverted": self.sills_reverted,
            "narrow_found": self.narrow_found, "narrow_left": self.narrow_left,
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

fn sill_at(r: &Routing, c: usize) -> Option<usize> {
    r.sills.binary_search_by_key(&(c as u32), |s| s.cell).ok()
}

/// Under a node's water or sea, and not a sill: never eroded, tapered or graded.
fn wet(r: &Routing, c: usize) -> bool {
    r.sea.get(c) || (r.held.get(c) && sill_at(r, c).is_none())
}

/// The node of a wet held cell that some cell drains into (from `shore`).
fn lake_of(r: &Routing, t: usize) -> Option<usize> {
    if r.sea.get(t) || !r.held.get(t) || sill_at(r, t).is_some() {
        return None;
    }
    r.shore
        .binary_search_by_key(&(t as u32), |p| p.0)
        .ok()
        .map(|i| r.shore[i].1 as usize)
}

/// Where cell `c` drains this routing: a sill's down cell, else its D8 receiver.
fn target(r: &Routing, c: usize) -> Option<usize> {
    if let Some(i) = sill_at(r, c) {
        return Some(r.sills[i].down as usize);
    }
    let (w, h) = (r.world.w as usize, r.world.h as usize);
    receiver(w, h, c, r.flow[c] & 15)
}

/// The lowest a cell draining into water may go besides its floor: 0 m into the sea, the
/// routing's level into a lake (stream power may still lower it to a lowered level).
fn water_bound(r: &Routing, c: usize) -> f32 {
    match target(r, c) {
        Some(t) if r.sea.get(t) => 0.0,
        Some(t) => lake_of(r, t).map_or(f32::NEG_INFINITY, |o| r.nodes.0[o].level),
        None => f32::NEG_INFINITY,
    }
}

/// Implicit stream power, one step: z ← (z + F·base) / (1 + F), F = k·A^m / L, in ascending
/// ground order; a cell whose receiver (or, draining into a lake, the lake's sill) is not yet
/// updated has that chain processed first (review M3), so no base is stale. Never below the
/// floor or the base, never raised.
pub(crate) fn stream_power(r: &Routing, z: &mut [f32]) -> Figures {
    let erodible = |c: usize| !wet(r, c);
    // Each node's sill cell (first by cell), so a lake's drainers find it without a scan.
    let mut node_sill: Vec<Option<u32>> = vec![None; r.nodes.0.len()];
    for s in r.sills {
        node_sill[s.node as usize].get_or_insert(s.cell);
    }
    // The cell whose new height this cell's base needs: where it drains, or the sill of the
    // lake it drains into (also for a sill spilling straight into another lake).
    let needs = |c: usize| -> Option<usize> {
        let t = target(r, c)?;
        match lake_of(r, t) {
            Some(o) => node_sill[o].map(|s| s as usize),
            None => Some(t),
        }
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
            let Some(to) = target(r, c) else {
                continue;
            };
            let ka = sill_at(r, c).map_or(r.ka[c], |i| r.sills[i].ka);
            // Into a lake (from a shore cell or a sill): its current level, not its bed.
            let base = lake_of(r, to).map_or(z[to], |o| level[o]);
            // v2: rivers into the sea grade to 0 m, not to the seabed.
            let base = if r.sea.get(to) { base.max(0.0) } else { base };
            if ka <= 0.0 || zc <= base {
                continue;
            }
            let f = f64::from(ka) / (dist_cells(r.world.w as usize, c, to) * r.spacing_m);
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
            if let Some(i) = sill_at(r, c) {
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
/// neighbours' mean, never below the floor or its water bound; concave cells, interior, held,
/// sea and edge cells stay. Returns the cells lowered.
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
            let v = (row[x] + alpha * lap)
                .max(r.floor[c])
                .max(water_bound(r, c));
            if v < z[c] {
                z[c] = v;
                n += 1;
            }
        }
        std::mem::swap(&mut above, &mut row);
    }
    n
}

/// A cut's taper radius in cells (B7): max(2.5 cells, 2·d / (tan(wall)·spacing)); 0 (the cell
/// alone) under SHAPE_MIN_M.
fn disc_radius(d: f32, tan: f64, spacing_m: f64) -> f64 {
    if d < SHAPE_MIN_M {
        return 0.0;
    }
    (2.0 * f64::from(d) / (tan * spacing_m)).max(TAPER_MIN_CELLS)
}

/// A disc of depth `d` and radius `radius` at distance `rho` (cells): d·(1 − (ρ/R)²).
fn disc_at(d: f32, rho: f64, radius: f64) -> f32 {
    if rho == 0.0 {
        d
    } else if rho < radius {
        (f64::from(d) * (1.0 - (rho / radius).powi(2))) as f32
    } else {
        0.0
    }
}

/// Every cut (a source: depth d = before − z, not under water or sea; a sill counts as dry) as
/// a parabolic disc of `disc_radius`; a cell takes the deepest lowering over all discs, not
/// under water or sea, never below the floor or the water bound (its own cut excepted, which
/// stream power already bounded). Then rounds: each first grades beds (a changed cell below
/// where it drains is lifted to it, never above `before`; B33, Plan A 16b), then fixes the
/// 3 × 3 of every changed cell steeper than max(wall, raw) and every changed cell that
/// `handoff_narrow` fails on the whole cut (raw − z): the discs lowering such a cell by over
/// half this step's cut there are halved (dropped under 1 cm, and after WALL_ROUNDS rounds
/// every disc reaching it is dropped, as Plan A's channel restore). A held or reverted cut
/// thus takes its whole disc and leaves no uncut pillar or one-cell slot (ruling R3). Until
/// nothing changes or REMOVE_ROUNDS more rounds are spent; cells only rise in the rounds.
/// `depth` is scratch (one f32 a cell) for the sources' depths.
/// Returns the taper and wall figures (walls and narrow cuts left counted on the final ground).
pub(crate) fn taper(
    r: &Routing,
    z: &mut [f32],
    before: &[f32],
    raw: &[f32],
    wall_deg: f64,
    depth: &mut [f32],
) -> Figures {
    let (w, h) = (r.world.w as usize, r.world.h as usize);
    let n = z.len();
    let tan = wall_deg.to_radians().tan();
    let mut dmax = 0f32;
    for c in 0..n {
        depth[c] = if wet(r, c) {
            0.0
        } else {
            (before[c] - z[c]).max(0.0)
        };
        dmax = dmax.max(depth[c]);
    }
    let radius = |depth: &[f32], s: usize| disc_radius(depth[s], tan, r.spacing_m);
    let k = disc_radius(dmax, tan, r.spacing_m).ceil() as i64;
    // The window every disc fits in: offsets and their distances.
    let window: Vec<(i64, i64, f64)> = (-k..=k)
        .flat_map(|dy| (-k..=k).map(move |dx| (dx, dy)))
        .map(|(dx, dy)| (dx, dy, ((dx * dx + dy * dy) as f64).sqrt()))
        .filter(|&(_, _, rho)| rho <= k as f64)
        .collect();
    let at = |c: usize, dx: i64, dy: i64| {
        let (x, y) = ((c % w) as i64 + dx, (c / w) as i64 + dy);
        (x >= 0 && y >= 0 && x < w as i64 && y < h as i64).then(|| y as usize * w + x as usize)
    };
    // The ground a cell gets from the discs, clamped; its own cut is never clamped.
    let clamp = |depth: &[f32], j: usize, lw: f32| {
        (before[j] - lw)
            .max(r.floor[j])
            .max(water_bound(r, j))
            .min(before[j] - depth[j])
    };
    // The discs that lower cell `j` by more than `over`.
    let discs_at = |depth: &[f32], j: usize, over: f32, out: &mut Vec<u32>| {
        for &(dx, dy, rho) in &window {
            if let Some(s) = at(j, dx, dy) {
                let d = depth[s];
                if d > 0.0 && disc_at(d, rho, radius(depth, s)) > over {
                    out.push(s as u32);
                }
            }
        }
    };
    // The deepest lowering of cell `j` over all discs.
    let lower_at = |depth: &[f32], j: usize| {
        let mut lw = 0f32;
        for &(dx, dy, rho) in &window {
            if let Some(s) = at(j, dx, dy) {
                let d = depth[s];
                if d > lw {
                    lw = lw.max(disc_at(d, rho, radius(depth, s)));
                }
            }
        }
        lw
    };
    let mut fig = Figures::default();
    // The discs, scattered (z holds each source's own cut already).
    for c in 0..n {
        let d = depth[c];
        let rc = radius(depth, c);
        if rc == 0.0 {
            continue;
        }
        let kc = rc.ceil() as i64;
        for (dx, dy) in (-kc..=kc).flat_map(|dy| (-kc..=kc).map(move |dx| (dx, dy))) {
            let Some(j) = at(c, dx, dy) else { continue };
            let lw = disc_at(d, ((dx * dx + dy * dy) as f64).sqrt(), rc);
            if lw <= 0.0 || before[j] - lw >= z[j] || wet(r, j) {
                continue;
            }
            z[j] = z[j].min(clamp(depth, j, lw));
        }
    }
    fig.tapered_cells = (0..n).filter(|&j| z[j] < before[j] - depth[j]).count() as u64;
    // Every cell this step lowered, however little: on raw slopes at the wall angle a
    // sub-centimetre cut below an earlier routing's cut makes a wall too. Bits: 1 bit a cell.
    let mut changed = Bits::new(n);
    for j in 0..n {
        if z[j] < before[j] {
            changed.set(j);
        }
    }
    let cut_sills: Vec<usize> = r
        .sills
        .iter()
        .map(|s| s.cell as usize)
        .filter(|&c| before[c] - z[c] > CHANGED_M)
        .collect();
    let inner = |j: usize| {
        let (x, y) = (j % w, j / w);
        x > 0 && y > 0 && x + 1 < w && y + 1 < h
    };
    let around = |c: usize| {
        let (cx, cy) = ((c % w) as i64, (c / w) as i64);
        OFFSETS
            .iter()
            .copied()
            .chain([(0, 0)])
            .map(move |(dy, dx)| (cx + dx, cy + dy))
            .filter(move |&(x, y)| x >= 0 && y >= 0 && x < w as i64 && y < h as i64)
            .map(move |(x, y)| y as usize * w + x as usize)
    };
    // Walls in the 3 × 3 of `cells` (a wall elsewhere is as the routing found it).
    let find_walls = |z: &[f32], cells: &mut dyn Iterator<Item = usize>| {
        let mut walls: Vec<u32> = Vec::new();
        for c in cells {
            for j in around(c) {
                if inner(j)
                    && raw[j] - z[j] > CHANGED_M
                    && is_wall(
                        horn_deg(z, w, j % w, j / w, r.spacing_m),
                        horn_deg(raw, w, j % w, j / w, r.spacing_m),
                        wall_deg,
                    )
                {
                    walls.push(j as u32);
                }
            }
        }
        walls.sort_unstable();
        walls.dedup();
        walls
    };
    // Cells of `cells` (changed ones, two cells from the edge) that `handoff_narrow` fails on
    // the whole cut so far (raw − z): over its 3 × 3 grey opening by more than
    // max(NARROW_M, NARROW_SHARE × cut), on a trough of z (B9, ruling R3).
    let find_narrow = |z: &[f32], cells: &mut dyn Iterator<Item = usize>| {
        let cut = |x: i64, y: i64| {
            if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                return 0.0;
            }
            let j = y as usize * w + x as usize;
            (raw[j] - z[j]).max(0.0)
        };
        let mut out: Vec<u32> = Vec::new();
        for c in cells {
            let (x, y) = ((c % w) as i64, (c / w) as i64);
            if !changed.get(c) || x < 2 || y < 2 || x + 2 >= w as i64 || y + 2 >= h as i64 {
                continue;
            }
            let cc = cut(x, y);
            if cc <= CHANGED_M {
                continue;
            }
            let zc = |dx: i64, dy: i64| z[(y + dy) as usize * w + (x + dx) as usize];
            if ![(1, 0), (0, 1), (1, 1), (1, -1)]
                .iter()
                .any(|&(dx, dy)| zc(dx, dy).min(zc(-dx, -dy)) > zc(0, 0) + NARROW_M)
            {
                continue;
            }
            let mut open = f32::NEG_INFINITY;
            for (dx, dy) in (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| (dx, dy))) {
                let mut m = f32::INFINITY;
                for (ex, ey) in (-1..=1).flat_map(|ey| (-1..=1).map(move |ex| (ex, ey))) {
                    m = m.min(cut(x + dx + ex, y + dy + ey));
                }
                open = open.max(m);
            }
            if cc - open > NARROW_M.max(NARROW_SHARE * cc) {
                out.push(c as u32);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    };
    // The 5 × 5 of cells in `a` or `b`: where a change can make or end a narrow cut.
    let near5 = |a: &Bits, b: &Bits| {
        let mut out: Vec<usize> = a
            .ones()
            .chain(b.ones())
            .flat_map(|c| {
                let (cx, cy) = ((c % w) as i64, (c / w) as i64);
                (-2..=2)
                    .flat_map(move |dy| (-2..=2).map(move |dx| (cx + dx, cy + dy)))
                    .filter(|&(x, y)| x >= 0 && y >= 0 && x < w as i64 && y < h as i64)
                    .map(|(x, y)| y as usize * w + x as usize)
            })
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    };
    // The footprints of the discs `hit` rise to what the discs now give (lifts kept); returns
    // the cells raised.
    let rerender = |depth: &[f32], z: &mut [f32], hit: &[u32]| {
        let mut dirty: Vec<u32> = hit
            .iter()
            .flat_map(|&c| {
                window
                    .iter()
                    .filter_map(move |&(dx, dy, _)| at(c as usize, dx, dy))
            })
            .filter(|&j| !wet(r, j))
            .map(|j| j as u32)
            .collect();
        dirty.sort_unstable();
        dirty.dedup();
        dirty.retain(|&j| {
            let ju = j as usize;
            let v = clamp(depth, ju, lower_at(depth, ju));
            let up = v > z[ju];
            if up {
                z[ju] = v;
            }
            up
        });
        dirty
    };
    // Cells raised in the last round: a new wall lies in their 3 × 3 (round 0: every cut).
    let mut touched: Option<Bits> = None;
    for round in 0..WALL_ROUNDS + REMOVE_ROUNDS {
        let walls = match &touched {
            None => find_walls(z, &mut changed.ones()),
            Some(t) => find_walls(z, &mut t.ones()),
        };
        let (mut raised, mut any) = (Bits::new(n), false);
        // Bed grade (to its fixed point inside the round): no changed cell below where it
        // drains (a sill to its down cell).
        loop {
            let mut moved = false;
            for c in changed.ones() {
                let Some(t) = target(r, c) else {
                    continue;
                };
                if wet(r, t) {
                    continue;
                }
                let v = before[c].min(z[c].max(z[t]));
                if v > z[c] {
                    z[c] = v;
                    raised.set(c);
                    any = true;
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
        // Narrow cuts after the bed grade (it lifts cells too).
        let narrow = match &touched {
            None => find_narrow(z, &mut changed.ones()),
            Some(t) => find_narrow(z, &mut near5(t, &raised).into_iter()),
        };
        fig.narrow_found += narrow.len() as u64;
        if walls.is_empty() && narrow.is_empty() && !any {
            break;
        }
        // A wall's 3 × 3 and each narrow cell: their discs are halved alike.
        let mut fix: Vec<u32> = walls
            .iter()
            .flat_map(|&j| around(j as usize).map(|c| c as u32))
            .chain(narrow.iter().copied())
            .collect();
        fix.sort_unstable();
        fix.dedup();
        // The discs to halve (or drop): those lowering a fixed cell by over half its cut,
        // every disc reaching it in the removal rounds.
        let remove = round >= WALL_ROUNDS;
        let mut hit: Vec<u32> = Vec::new();
        for &j in &fix {
            let cut = before[j as usize] - z[j as usize];
            if cut > 0.0 {
                discs_at(
                    depth,
                    j as usize,
                    if remove { 0.0 } else { cut / 2.0 },
                    &mut hit,
                );
            }
        }
        hit.sort_unstable();
        hit.dedup();
        for &c in &hit {
            let d = &mut depth[c as usize];
            *d = if remove || *d / 2.0 < SHAPE_MIN_M {
                0.0
            } else {
                *d / 2.0
            };
        }
        for j in rerender(depth, z, &hit) {
            raised.set(j as usize);
            any = true;
            if remove {
                fig.wall_cuts_removed += 1;
            } else {
                fig.wall_cells_restored += 1;
            }
        }
        fig.wall_rounds = round as u64 + 1;
        // Nothing raised: walls this routing did not cut stay (counted below), cells only
        // rise, so further rounds change nothing.
        if !any {
            break;
        }
        touched = Some(raised);
    }
    fig.sills_reverted = cut_sills
        .into_iter()
        .filter(|&c| before[c] - z[c] <= CHANGED_M)
        .count() as u64;
    // Counted on the final ground (after the last bed grade), over every cut.
    fig.walls_left = find_walls(z, &mut changed.ones()).len() as u64;
    fig.narrow_left = find_narrow(z, &mut changed.ones()).len() as u64;
    fig
}

/// Strict local minima (8 neighbours) of `z` that are none in `breached`, off water and sea
/// and off the map edge (B33; Task 7 makes the same test a hard check on the published ground).
pub(crate) fn new_pits(world: World, z: &[f32], breached: &[f32], held: &Bits, sea: &Bits) -> u64 {
    let (w, h) = (world.w as usize, world.h as usize);
    let strict_min = |v: &[f32], c: usize| {
        let (x, y) = ((c % w) as i64, (c / w) as i64);
        OFFSETS.iter().all(|&(dy, dx)| {
            let (nx, ny) = (x + dx, y + dy);
            nx < 0
                || ny < 0
                || nx >= w as i64
                || ny >= h as i64
                || v[ny as usize * w + nx as usize] > v[c]
        })
    };
    (0..z.len())
        .filter(|&c| {
            let (x, y) = (c % w, c / w);
            x > 0 && y > 0 && x + 1 < w && y + 1 < h && !held.get(c) && !sea.get(c)
        })
        .filter(|&c| strict_min(z, c) && !strict_min(breached, c))
        .count() as u64
}

/// Held cells a dry cell drains into, and held sill down cells, with their node ordinal
/// (node raster − 2), sorted by cell. A sill spilling straight into another lake grades to
/// that lake's level, not its bed (Task 5 I1), so its down cell is a shore cell too.
pub(crate) fn shore(world: World, node: &[u32], flow: &[u8], sills: &[Sill]) -> Vec<(u32, u32)> {
    let (w, h) = (world.w as usize, world.h as usize);
    let mut shore: Vec<(u32, u32)> = (0..node.len())
        .filter(|&c| node[c] < 2)
        .filter_map(|c| receiver(w, h, c, flow[c] & 15))
        .chain(sills.iter().map(|s| s.down as usize))
        .filter(|&r| node[r] >= 2)
        .map(|r| (r as u32, node[r] - 2))
        .collect();
    shore.sort_unstable();
    shore.dedup();
    shore
}

/// The erodibility k·dt of a cell (B3): interior and glacier cells take `interior_k` of it.
fn kfor(e: &Erosion, interior: bool) -> f64 {
    if interior {
        e.k_dt * e.interior_k
    } else {
        e.k_dt
    }
}

/// The `e` stage (B23): `e.routings` × [model on the current ground, stream power, diffusion,
/// taper and wall rounds] from the breached ground into `work/e<k>.f32`. Each routing is
/// cached; returns the last ground's path, the summed figures and each routing's.
pub(crate) fn erode(
    s: &Setup,
    breached: &Layer<f32>,
    raw: &Layer<f32>,
    work: &Work,
    tag: &str,
    e: &Erosion,
) -> Result<(PathBuf, Figures, Vec<Figures>)> {
    let world = s.world;
    let n = (world.w * world.h) as usize;
    let (w, h) = (world.w as usize, world.h as usize);
    let whole = Rect::whole(world);
    let mut per = Vec::new();
    let mut total = Figures::default();
    for k in 0..e.routings as usize {
        let ground = if k == 0 {
            breached.clone()
        } else {
            Layer::open(world, &work.path(&format!("e{}.f32", k - 1)))?
        };
        let out = Layer::open_or_create(world, &work.path(&format!("e{k}.f32")), 0.0)?;
        // Written after the closure returns: a kill mid-routing reruns it from e<k−1>.
        let blob = work.cached(&format!("{tag}/r{k}"), || {
            let rasters = Rasters::files(world, &work.path(""), &format!("e{k}"))?;
            let m = pipeline::model(
                s,
                &ground,
                Some(work),
                &format!("{tag}/m{k}"),
                Some(&rasters),
            )?;
            let started = std::time::Instant::now();
            // Interior bits, then the u8 rasters go (preflight F11: memory).
            let mut interior = Bits::new(n);
            {
                let flags = s.flags(whole)?;
                let classes = match &s.classes {
                    Some(c) => c.read(whole)?,
                    None => vec![0; n],
                };
                for c in 0..n {
                    if flags[c] & KIND_MASK == INTERIOR || classes[c] == GLACIER {
                        interior.set(c);
                    }
                }
            }
            // Floor: the breached ground less the cap (B4); breach cuts are not counted.
            let floor: Vec<f32> = {
                let bz = breached.to_vec()?;
                (0..n)
                    .map(|c| {
                        bz[c]
                            - if interior.get(c) {
                                e.interior_cap_m
                            } else {
                                e.cap_m
                            }
                    })
                    .collect()
            };
            let node = rasters.node.to_vec()?;
            let flow = rasters.flow.to_vec()?;
            let (mut held, mut sea) = (Bits::new(n), Bits::new(n));
            for (c, &v) in node.iter().enumerate() {
                if v >= 2 {
                    held.set(c);
                }
                if v == 1 {
                    sea.set(c);
                }
            }
            let mut z = ground.to_vec()?;
            // k·A^m per cell, in place of the discharge (A in m²).
            let mut ka = rasters.discharge.to_vec()?;
            let nodes = &m.water.nodes;
            let q_at: HashMap<usize, f32> = nodes
                .0
                .iter()
                .filter(|nd| nd.full)
                .map(|nd| (nd.outlet[1] as usize, ka[nd.outlet[1] as usize]))
                .collect();
            for (c, v) in ka.iter_mut().enumerate() {
                *v = if held.get(c) || sea.get(c) {
                    0.0
                } else {
                    (kfor(e, interior.get(c)) * (f64::from(*v) * 1e6).powf(e.m)) as f32
                };
            }
            let mut sills: Vec<Sill> = nodes
                .0
                .iter()
                .enumerate()
                .filter(|(_, nd)| nd.full)
                .filter_map(|(o, nd)| {
                    let [a, b] = [nd.outlet[0] as usize, nd.outlet[1] as usize];
                    let (cell, other) = if z[a] >= z[b] { (a, b) } else { (b, a) };
                    let down = if cell == a {
                        Some(b)
                    } else {
                        receiver(w, h, b, flow[b] & 15)
                    }?;
                    let q = f64::from(q_at[&b]);
                    Some(Sill {
                        cell: cell as u32,
                        down: down as u32,
                        node: o as u32,
                        other_z: z[other],
                        ka: (kfor(e, interior.get(cell)) * (q * 1e6).powf(e.m)) as f32,
                    })
                })
                .collect();
            sills.sort_unstable_by_key(|s| s.cell);
            sills.dedup_by_key(|s| s.cell);
            let shore = shore(world, &node, &flow, &sills);
            drop(node);
            let before = z.clone();
            let r = Routing {
                world,
                spacing_m: s.spacing_m,
                floor: &floor,
                ka: &ka,
                flow: &flow,
                held: &held,
                sea: &sea,
                interior: &interior,
                shore: &shore,
                sills: &sills,
                nodes,
            };
            let mut fig = stream_power(&r, &mut z);
            let near = footprint(&r, &before, &z, s.params.wall_deg);
            fig.diffused_cells = diffuse(&r, &mut z, e.diffusion, &near);
            // The taper does not read ka: its buffer is reused for the lowering.
            let r = Routing {
                world,
                spacing_m: s.spacing_m,
                floor: &floor,
                ka: &[],
                flow: &flow,
                held: &held,
                sea: &sea,
                interior: &interior,
                shore: &shore,
                sills: &sills,
                nodes,
            };
            let mut lower = ka;
            let rawv = raw.to_vec()?;
            let t = taper(&r, &mut z, &before, &rawv, s.params.wall_deg, &mut lower);
            drop((rawv, lower));
            fig.tapered_cells = t.tapered_cells;
            fig.wall_cells_restored = t.wall_cells_restored;
            fig.wall_cuts_removed = t.wall_cuts_removed;
            fig.wall_rounds = t.wall_rounds;
            fig.sills_reverted = t.sills_reverted;
            fig.walls_left = t.walls_left;
            fig.narrow_found = t.narrow_found;
            fig.narrow_left = t.narrow_left;
            let left = t.walls_left;
            // B6 figure: full-K land outside the river cuts' footprint should not move.
            let mut hist = vec![0u64; super::checks::BINS];
            let mut hmax = 0f32;
            for c in 0..n {
                if !near.get(c) && !interior.get(c) && !held.get(c) && !sea.get(c) {
                    let d = (before[c] - z[c]).max(0.0);
                    hist[((d * 100.0) as usize).min(super::checks::BINS - 1)] += 1;
                    hmax = hmax.max(d);
                }
            }
            fig.hillslope_p99_m = super::checks::p99_m(&hist) as f32;
            fig.hillslope_max_m = hmax;
            let bz = breached.to_vec()?;
            fig.new_pits = new_pits(world, &z, &bz, &held, &sea);
            out.write(whole, &z)?;
            eprintln!(
                "erosion: routing {}/{} lowered {} cells (max {:.2} m), {} sills, {} new pits, \
                 {} walls left; passes {:.0} s{}",
                k + 1,
                e.routings,
                fig.lowered_cells,
                fig.max_lowering_m,
                fig.sills_lowered,
                fig.new_pits,
                left,
                started.elapsed().as_secs_f64(),
                super::peak()
            );
            Ok(fig.to_blob())
        })?;
        let fig = Figures::from_blob(blob);
        total.add(fig);
        per.push(fig);
    }
    Ok((work.path(&format!("e{}.f32", e.routings - 1)), total, per))
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
        let t = taper(&r, &mut z, &before, &before, 35.0, &mut lower);
        let (tapered, left) = (t.tapered_cells, t.walls_left);
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

    #[test]
    fn a_lake_spilling_into_another_lake_grades_to_its_level_not_its_bed() {
        // Lake A (cells 1–2, level 30) spills over sill cell 3 straight into lake B (cells 4–5,
        // level 20, bed 14). A strong sill grades toward B's level 20, never toward B's bed.
        let before = [45.0, 25.0, 24.0, 30.0, 15.0, 14.0, 22.0, 10.0, 0.0f32];
        let mut s = Strip::new(&before);
        for c in [1, 2, 4, 5] {
            s.held.set(c);
        }
        s.nodes = Nodes(vec![
            Node {
                key: 2,
                level: 30.0,
                low: 24.0,
                full: true,
                lake: true,
                outlet: [2, 3],
                cells: 2,
                ..Default::default()
            },
            Node {
                key: 5,
                level: 20.0,
                low: 14.0,
                lake: true,
                cells: 2,
                ..Default::default()
            },
        ]);
        s.shore = vec![(1, 0), (4, 1)];
        s.sills = vec![Sill {
            cell: 3,
            down: 4,
            node: 0,
            other_z: 24.0,
            ka: 1000.0,
        }];
        let f = s.erode();
        assert!(
            s.z[3] >= 20.0 - 1e-4,
            "the sill stays above B's level, got {}",
            s.z[3]
        );
        assert!(s.z[3] < 25.0, "the sill incises, got {}", s.z[3]);
        assert_eq!(f.sills_lowered, 1);
        assert!(
            s.z[0] >= 24.0 - 1e-4,
            "A's drainer stays above A's new level"
        );
        assert_eq!(&s.z[4..6], &before[4..6], "B's bed does not erode");
    }

    /// An 11 × 11 flat at 100 m on 4 m cells flowing east, columns 0–`lake_x` held (node 0).
    fn lake_west(lake_x: usize, level: f32) -> Strip {
        let (w, h) = (11usize, 11usize);
        let mut s = Strip::new(&vec![100.0f32; w * h]);
        s.flow = (0..w * h)
            .map(|i| if i % w + 1 < w { 4 } else { 0 })
            .collect();
        s.ka = vec![0.0; w * h];
        for i in (0..w * h).filter(|i| i % w <= lake_x) {
            s.held.set(i);
        }
        s.nodes = Nodes(vec![Node {
            key: 0,
            level,
            low: 90.0,
            lake: true,
            ..Default::default()
        }]);
        s
    }

    fn routing_11(s: &Strip) -> Routing<'_> {
        let mut r = s.routing();
        r.world = World { w: 11, h: 11 };
        r.spacing_m = 4.0;
        r
    }

    #[test]
    fn a_held_inside_sill_is_tapered_and_graded() {
        // Lake in columns 0–4 at 100 m over a 95 m bed; its sill is the held inside cell (4, 5)
        // at 99.8 m, cut 3.8 m by stream power; it drains to the dry (5, 5). Dry ground falls
        // east 0.5 m a cell, so the cut has somewhere to grade to.
        let w = 11usize;
        let mut s = lake_west(4, 100.0);
        let (sill, down) = (5 * w + 4, 5 * w + 5);
        s.sills = vec![Sill {
            cell: sill as u32,
            down: down as u32,
            node: 0,
            other_z: 100.0,
            ka: 0.0,
        }];
        let mut before: Vec<f32> = (0..w * w)
            .map(|i| match i % w {
                x if x <= 4 => 95.0,
                x => 100.0 - 0.5 * (x - 5) as f32,
            })
            .collect();
        before[sill] = 99.8;
        let mut z = before.clone();
        z[sill] = 96.0;
        let mut lower = vec![0.0f32; w * w];
        taper(&routing_11(&s), &mut z, &before, &before, 35.0, &mut lower);
        for c in [4 * w + 5, down, 6 * w + 5] {
            assert!(
                before[c] - z[c] > 0.5,
                "the outlet cut is 3 cells wide, cell {c} at {}",
                z[c]
            );
        }
        assert!(
            before[sill] - z[sill] > 0.5,
            "the sill stays cut, got {}",
            z[sill]
        );
        assert!(
            z[sill] > 96.0,
            "the sill is lifted to its graded outlet, got {}",
            z[sill]
        );
        assert!(
            z[sill] >= z[down] - 1e-5,
            "the sill is graded to its down cell"
        );
        assert!(z.iter().zip(&before).all(|(a, b)| a <= b), "nothing rises");
    }

    #[test]
    fn a_disc_does_not_cut_a_shore_cell_below_its_lake() {
        // Lake in columns 0–3 at 99.9 m (bed 98 m); the shore drains west into it, row 5 is a
        // river flowing east past it on ground falling 0.6 m a cell, cut 2 m at (6, 5): its disc
        // (R 2.5) reaches (4, 4) and (4, 6), which it would take to 99.6 m.
        let w = 11usize;
        let mut s = lake_west(3, 99.9);
        s.flow = (0..w * w)
            .map(|i| match (i % w, i / w) {
                (x, 5) if x + 1 < w => 4,
                (x, y) if x > 0 && y != 5 => 3,
                _ => 0,
            })
            .collect();
        s.shore = (0..w).map(|y| ((y * w + 3) as u32, 0)).collect();
        let before: Vec<f32> = (0..w * w)
            .map(|i| match i % w {
                x if x <= 3 => 98.0,
                x => 100.0 - 0.6 * (x - 4) as f32,
            })
            .collect();
        let mut z = before.clone();
        z[5 * w + 6] -= 2.0;
        let mut lower = vec![0.0f32; w * w];
        taper(&routing_11(&s), &mut z, &before, &before, 35.0, &mut lower);
        assert!(z[5 * w + 5] < 99.0, "the river's disc lowers its bed");
        for y in [4, 6] {
            let c = y * w + 4;
            assert!(
                z[c] >= 99.9 - 1e-4,
                "shore cell (4, {y}) below the lake level: {}",
                z[c]
            );
        }
    }

    #[test]
    fn a_held_sill_spilling_into_another_lake_grades_to_its_level() {
        // Lake A (cells 1–3, level 30) spills from its own held cell 3 straight into lake B
        // (cells 4–5, level 20, bed 14): no dry cell drains into B, so only the sill's down
        // cell makes cell 4 a shore cell of B; without it the sill grades to B's bed.
        let before = [45.0, 25.0, 24.0, 26.0, 15.0, 14.0, 22.0, 10.0, 0.0f32];
        let mut s = Strip::new(&before);
        let mut node = vec![0u32; before.len()];
        for c in [1, 2, 3] {
            s.held.set(c);
            node[c] = 2;
        }
        for c in [4, 5] {
            s.held.set(c);
            node[c] = 3;
        }
        s.nodes = Nodes(vec![
            Node {
                key: 2,
                level: 30.0,
                low: 24.0,
                full: true,
                lake: true,
                outlet: [3, 4],
                cells: 3,
                ..Default::default()
            },
            Node {
                key: 5,
                level: 20.0,
                low: 14.0,
                lake: true,
                cells: 2,
                ..Default::default()
            },
        ]);
        s.sills = vec![Sill {
            cell: 3,
            down: 4,
            node: 0,
            other_z: 15.0,
            ka: 1000.0,
        }];
        let world = World { w: 9, h: 1 };
        s.shore = shore(world, &node, &s.flow, &s.sills);
        assert_eq!(s.shore, [(1, 0), (4, 1)]);
        s.erode();
        assert!(
            s.z[3] >= 20.0 - 1e-4 && s.z[3] < 25.0,
            "the sill incises to B's level, not its bed: {}",
            s.z[3]
        );
    }

    #[test]
    fn a_sub_centimetre_cut_on_a_steep_raw_slope_leaves_no_wall() {
        // A 40° raw plane rising east on 4 m cells, lowered 1 m from column 2 on by earlier
        // routings (no wall). This routing cuts (3, 3) 9 mm, under the taper's 1 cm: Horn at
        // (4, 3) steepens about 0.02°, a wall the rounds must find and undo.
        let (w, h) = (8usize, 8usize);
        let rise = (4.0 * 40f64.to_radians().tan()) as f32;
        let raw: Vec<f32> = (0..w * h).map(|i| (i % w) as f32 * rise).collect();
        let before: Vec<f32> = (0..w * h)
            .map(|i| raw[i] - if i % w >= 2 { 1.0 } else { 0.0 })
            .collect();
        let mut s = Strip::new(&before);
        s.flow = vec![0; w * h];
        let mut z = before.clone();
        z[3 * w + 3] -= 0.009;
        let mut r = s.routing();
        r.world = World {
            w: w as u64,
            h: h as u64,
        };
        r.spacing_m = 4.0;
        let wall_at = |z: &[f32], x: usize, y: usize| {
            is_wall(
                horn_deg(z, w, x, y, 4.0),
                horn_deg(&raw, w, x, y, 4.0),
                35.0,
            )
        };
        assert!(wall_at(&z, 4, 3), "the setup makes a wall");
        let mut lower = vec![0.0f32; w * h];
        let t = taper(&r, &mut z, &before, &raw, 35.0, &mut lower);
        assert_eq!(
            t.wall_cells_restored, 1,
            "the 9 mm cut is reset within the halving rounds"
        );
        assert_eq!((t.wall_cuts_removed, t.wall_rounds), (0, 1));
        let left = t.walls_left;
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                assert!(
                    raw[y * w + x] - z[y * w + x] <= CHANGED_M || !wall_at(&z, x, y),
                    "wall at ({x}, {y})"
                );
            }
        }
        assert_eq!(left, 0);
    }

    /// Cells failing `handoff_narrow`'s rule (B9, ruling R3) for the cut `before − z`: the cut
    /// exceeds its 3 × 3 grey opening by over max(0.5, 0.5 × cut) on a trough of `z`.
    fn slots(z: &[f32], before: &[f32], w: usize, h: usize) -> Vec<(usize, usize)> {
        let cut = |x: i64, y: i64| {
            if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                return 0.0;
            }
            let j = y as usize * w + x as usize;
            (before[j] - z[j]).max(0.0)
        };
        let mut out = Vec::new();
        for y in 2..h - 2 {
            for x in 2..w - 2 {
                let (xi, yi) = (x as i64, y as i64);
                let c = cut(xi, yi);
                if c <= CHANGED_M {
                    continue;
                }
                let mut open = f32::NEG_INFINITY;
                for (dx, dy) in (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| (dx, dy))) {
                    let mut m = f32::INFINITY;
                    for (ex, ey) in (-1..=1).flat_map(|ey| (-1..=1).map(move |ex| (ex, ey))) {
                        m = m.min(cut(xi + dx + ex, yi + dy + ey));
                    }
                    open = open.max(m);
                }
                let zc = |dx: i64, dy: i64| z[(yi + dy) as usize * w + (xi + dx) as usize];
                let trough = [(1, 0), (0, 1), (1, 1), (1, -1)]
                    .iter()
                    .any(|&(dx, dy)| zc(dx, dy).min(zc(-dx, -dy)) > zc(0, 0) + 0.5);
                if trough && c - open > 0.5f32.max(0.5 * c) {
                    out.push((x, y));
                }
            }
        }
        out
    }

    /// Uncut cells standing above a cut on both sides of one axis (both neighbours cut over
    /// 0.5 m): a pillar left inside a cut reach.
    fn pillars(z: &[f32], before: &[f32], w: usize, h: usize) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let c = y * w + x;
                if before[c] - z[c] > CHANGED_M {
                    continue;
                }
                let cut = |dx: i64, dy: i64| {
                    let j = (y as i64 + dy) as usize * w + (x as i64 + dx) as usize;
                    before[j] - z[j]
                };
                if [(1, 0), (0, 1), (1, 1), (1, -1)]
                    .iter()
                    .any(|&(dx, dy)| cut(dx, dy).min(cut(-dx, -dy)) > 0.5)
                {
                    out.push((x, y));
                }
            }
        }
        out
    }

    #[test]
    fn a_trunk_cut_in_a_narrow_valley_floor_is_no_slot_and_still_drains() {
        // 32 m cells; a trunk (row 7) falling east 0.5 m a cell on a 3-cell valley floor whose
        // sides steepen to about 35° two cells out and 44° beyond. Stream power cuts the trunk
        // alone; the side cells' discs make walls on the steep sides, whose wall rounds must not
        // leave the trunk cut deeper than a 3-cell-wide cut.
        let (w, h) = (21usize, 15usize);
        let side = [0.0f32, 2.0, 6.0, 46.5, 67.5, 88.5, 109.5, 130.5];
        let before: Vec<f32> = (0..w * h)
            .map(|i| 100.0 - 0.5 * (i % w) as f32 + side[(i / w).abs_diff(7)])
            .collect();
        let mut s = Strip::new(&before);
        s.flow = (0..w * h)
            .map(|i| match (i % w, i / w) {
                (x, 7) if x + 1 < w => 4,
                (_, 7) => 0,
                (_, y) if y < 7 => 2,
                _ => 1,
            })
            .collect();
        s.ka = (0..w * h)
            .map(|i| if i / w == 7 { 200.0 } else { 0.0 })
            .collect();
        let mut r = s.routing();
        r.world = World {
            w: w as u64,
            h: h as u64,
        };
        let mut z = before.clone();
        stream_power(&r, &mut z);
        let deepest = (0..w)
            .map(|x| before[7 * w + x] - z[7 * w + x])
            .fold(0.0, f32::max);
        assert!(deepest > 2.0, "stream power cuts the trunk, got {deepest}");
        let mut lower = vec![0.0f32; w * h];
        let t = taper(&r, &mut z, &before, &before, 35.0, &mut lower);
        assert!(t.wall_cells_restored > 0, "the setup makes walls");
        assert_eq!(slots(&z, &before, w, h), [], "one-cell slots");
        assert_eq!(pillars(&z, &before, w, h), [], "pillars");
        for x in 0..w - 1 {
            let c = 7 * w + x;
            assert!(z[c] >= z[c + 1] - 1e-5, "trunk cell {x} below its receiver");
        }
        let none = Bits::new(w * h);
        assert_eq!(new_pits(r.world, &z, &before, &none, &none), 0);
        assert!(z.iter().zip(&before).all(|(a, b)| a <= b), "nothing rises");
    }

    /// A 25 × 25 grid of 32 m cells: a trunk (row 12) falling east 1 m a cell between sides
    /// rising `rise` m a cell with `amp` m of hashed roughness (seed `seed`), cut by stream
    /// power (trunk k·A^m `ka`) and tapered. Returns before, the result and the taper figures.
    fn rough_trunk(seed: u64, amp: f32, rise: f32, ka: f32) -> (Vec<f32>, Vec<f32>, Figures) {
        let (w, h) = (25usize, 25usize);
        let noise = |i: usize| {
            let v = (i as u64 ^ seed.rotate_left(32))
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .rotate_left(17);
            (v % 1000) as f32 / 1000.0 - 0.5
        };
        let before: Vec<f32> = (0..w * h)
            .map(|i| {
                let k = (i / w).abs_diff(12) as f32;
                200.0 - (i % w) as f32 + rise * k + amp * noise(i) * k.min(1.0)
            })
            .collect();
        let mut s = Strip::new(&before);
        // The trunk flows east; other cells by steepest descent (0 in a pit).
        s.flow = (0..w * h)
            .map(|i| match (i % w, i / w) {
                (x, 12) if x + 1 < w => 4,
                (_, 12) => 0,
                _ => (1..=8u8)
                    .filter_map(|d| receiver(w, h, i, d).map(|t| (d, t)))
                    .map(|(d, t)| (d, (before[i] - before[t]) as f64 / dist_cells(w, i, t)))
                    .filter(|&(_, g)| g > 0.0)
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .map_or(0, |(d, _)| d),
            })
            .collect();
        s.ka = (0..w * h)
            .map(|i| if i / w == 12 { ka } else { 0.0 })
            .collect();
        let mut r = s.routing();
        r.world = World {
            w: w as u64,
            h: h as u64,
        };
        let mut z = before.clone();
        stream_power(&r, &mut z);
        let mut lower = vec![0.0f32; w * h];
        let t = taper(&r, &mut z, &before, &before, 35.0, &mut lower);
        (before, z, t)
    }

    #[test]
    fn wall_rounds_on_rough_steep_ground_leave_no_pillar_or_slot() {
        // Rough ground whose sides rise near the wall angle, as GLO-30 canyons do. Wall rounds
        // hold and revert cuts here; a cell they leave uncut must not stand between cut cells,
        // and no cut may be left a one-cell slot.
        let (before, z, t) = rough_trunk(55, 45.0, 10.0, 1000.0);
        assert!(t.wall_cells_restored > 0, "the setup makes walls");
        assert_eq!(pillars(&z, &before, 25, 25), [], "pillars");
        assert_eq!(slots(&z, &before, 25, 25), [], "one-cell slots");
        assert_eq!(t.walls_left, 0);
        assert!(z.iter().zip(&before).all(|(a, b)| a <= b), "nothing rises");
    }

    #[test]
    fn bits_list_their_ones_in_order() {
        let mut b = Bits::new(200);
        for i in [0, 63, 64, 130, 199] {
            b.set(i);
        }
        assert_eq!(b.ones().collect::<Vec<_>>(), [0, 63, 64, 130, 199]);
        assert_eq!(Bits::new(70).ones().count(), 0);
    }
}
