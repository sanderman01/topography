//! Coarse-level erosion (spec "Coarse stages"; rulings B1–B7, B23, B29). Each step reads one
//! model run on the current ground and lowers it:
//! 1. implicit stream power (Braun and Willett 2013, n = 1, no uplift) in ascending ground
//!    order: a dry receiver is strictly lower (R12), so it is updated first. Routing is
//!    lake-aware: cells under a node's water do not erode, cells draining into a node take
//!    its level as their base, and a full node's sill cell erodes with the node's whole
//!    discharge while the level follows it (Cordonnier et al. 2019 with one outlet per node);
//! 2. each stream-power cut re-expressed as Plan A's parabolic gorge taper (valley-side
//!    response to incision);
//! 3. mass-conserving hillslope diffusion near cuts of at least DIFFUSE_MIN_M: convex cells
//!    give soil, concave ones take it (deposition).
//!
//! The owner rule (2026-11-08): ground changes only by erosion where water flows and
//! deposition where it pools; no pass undoes a cut to satisfy a check (`walls`,
//! `handoff_narrow` and `erosion_new_pits` are figures). Never below the floor (the breached
//! ground less the cell's cap). One global single-threaded pass in a fixed order:
//! deterministic.

use super::checks::CHANGED_M;
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
/// Stream-power cuts at least this deep open diffusion around them (ruling Q2).
pub(crate) const DIFFUSE_MIN_M: f32 = 0.25;
/// The narrowest taper (cells): a disc of radius 2.5 lowers a 5-cell-wide cut, so a channel
/// narrower than a cell does not cut a whole cell alone (preflight F1).
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
    #[cfg(test)]
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
    /// Cells diffusion lowered or raised.
    pub diffused_cells: u64,
    pub tapered_cells: u64,
    /// Strict local minima of the eroded ground that were none in the breached ground (B33).
    pub new_pits: u64,
    /// Lowering of full-K cells outside the cut footprint (B6), cm-binned p99 and max.
    pub hillslope_p99_m: f32,
    pub hillslope_max_m: f32,
    /// The deepest lowering and the highest rise (deposition) diffusion made.
    pub diffusion_max_m: f32,
    pub diffusion_max_rise_m: f32,
    /// The B6 figures outside the footprint widened by 3 cells (R3: past the taper discs).
    pub hillslope_wide_p99_m: f32,
    pub hillslope_wide_max_m: f32,
    /// Cells lowered by at least SHAPE_MIN_M by the whole routing (R4; `lowered_cells`
    /// counts stream power's).
    pub lowered_1cm: u64,
    /// Sills stream power cut by more than CHANGED_M.
    pub sills_cut: u64,
}

impl Figures {
    pub fn to_blob(self) -> (Vec<u64>, f32) {
        let b = |v: f32| u64::from(v.to_bits());
        (
            vec![
                self.lowered_cells,
                self.capped_cells,
                self.sills_lowered,
                self.order_ties,
                self.diffused_cells,
                self.tapered_cells,
                self.new_pits,
                b(self.hillslope_p99_m),
                b(self.hillslope_max_m),
                b(self.diffusion_max_m),
                b(self.diffusion_max_rise_m),
                b(self.hillslope_wide_p99_m),
                b(self.hillslope_wide_max_m),
                self.lowered_1cm,
                self.sills_cut,
            ],
            self.max_lowering_m,
        )
    }
    pub fn from_blob((v, max): (Vec<u64>, f32)) -> Self {
        let g = |i: usize| v.get(i).copied().unwrap_or(0);
        let f = |i: usize| f32::from_bits(g(i) as u32);
        Self {
            lowered_cells: g(0),
            capped_cells: g(1),
            sills_lowered: g(2),
            order_ties: g(3),
            diffused_cells: g(4),
            tapered_cells: g(5),
            new_pits: g(6),
            hillslope_p99_m: f(7),
            hillslope_max_m: f(8),
            diffusion_max_m: f(9),
            diffusion_max_rise_m: f(10),
            hillslope_wide_p99_m: f(11),
            hillslope_wide_max_m: f(12),
            lowered_1cm: g(13),
            sills_cut: g(14),
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
        self.new_pits = o.new_pits; // the last routing's ground is what m2 reads
        self.hillslope_p99_m = self.hillslope_p99_m.max(o.hillslope_p99_m);
        self.hillslope_max_m = self.hillslope_max_m.max(o.hillslope_max_m);
        self.diffusion_max_m = self.diffusion_max_m.max(o.diffusion_max_m);
        self.diffusion_max_rise_m = self.diffusion_max_rise_m.max(o.diffusion_max_rise_m);
        self.hillslope_wide_p99_m = self.hillslope_wide_p99_m.max(o.hillslope_wide_p99_m);
        self.hillslope_wide_max_m = self.hillslope_wide_max_m.max(o.hillslope_wide_max_m);
        self.lowered_1cm += o.lowered_1cm;
        self.sills_cut += o.sills_cut;
    }
    pub fn json(&self) -> Value {
        json!({
            "lowered_cells": self.lowered_cells, "max_lowering_m": self.max_lowering_m,
            "capped_cells": self.capped_cells, "sills_lowered": self.sills_lowered,
            "order_ties": self.order_ties, "diffused_cells": self.diffused_cells,
            "tapered_cells": self.tapered_cells, "erosion_new_pits": self.new_pits,
            "hillslope_lowering_p99_m": self.hillslope_p99_m, "hillslope_lowering_max_m": self.hillslope_max_m,
            "sills_cut": self.sills_cut,
            "diffusion_max_m": self.diffusion_max_m,
            "diffusion_max_rise_m": self.diffusion_max_rise_m,
            "hillslope_lowering_wide_p99_m": self.hillslope_wide_p99_m,
            "hillslope_lowering_wide_max_m": self.hillslope_wide_max_m,
            "lowered_1cm_cells": self.lowered_1cm,
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

/// Cells within R_taper + 1 (+ `extra`) of a cell this routing's stream power lowered by at
/// least `min_cut`: DIFFUSE_MIN_M where diffusion may act (B6, review I3, ruling Q2),
/// SHAPE_MIN_M for the B6 figures.
pub(crate) fn footprint(
    r: &Routing,
    before: &[f32],
    z: &[f32],
    wall_deg: f64,
    min_cut: f32,
    extra: f64,
) -> Bits {
    let (w, h) = (r.world.w as usize, r.world.h as usize);
    let tan = wall_deg.to_radians().tan();
    let mut near = Bits::new(z.len());
    for c in 0..z.len() {
        let d = before[c] - z[c];
        if d < min_cut {
            continue;
        }
        let radius = (2.0 * f64::from(d) / (tan * r.spacing_m)).max(TAPER_MIN_CELLS) + 1.0 + extra;
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

/// What one diffusion step moved.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Diffused {
    /// Cells lowered or raised.
    pub cells: u64,
    pub max_lowering_m: f32,
    pub max_rise_m: f32,
}

/// Mass-conserving hillslope diffusion, one explicit step (alpha = D·dt/dx² <= 0.25) in flux
/// form on the 4-neighbour edges between `near` cells (the river cuts' footprint, B6): convex
/// cells give soil, concave ones (footslopes, valley floors) take it as deposition. Interior,
/// held, sea and edge cells take no part (no-flux). A cell's outflows are scaled so it never
/// goes below its floor or water bound, so every flux lands somewhere: mass is conserved.
pub(crate) fn diffuse(r: &Routing, z: &mut [f32], alpha: f32, near: &Bits) -> Diffused {
    let (w, h) = (r.world.w as usize, r.world.h as usize);
    let mut out = Diffused::default();
    if alpha <= 0.0 || w < 3 || h < 3 {
        return out;
    }
    let active = |c: usize| {
        let (x, y) = (c % w, c / w);
        x > 0
            && y > 0
            && x + 1 < w
            && y + 1 < h
            && near.get(c)
            && !r.held.get(c)
            && !r.sea.get(c)
            && !r.interior.get(c)
    };
    let nbrs = |c: usize| [c - w, c + w, c - 1, c + 1];
    // Rows y − 2 and y − 1 before this step (rows y and below are not yet written).
    let (mut prev2, mut prev1) = (vec![0f32; w], z[..w].to_vec());
    let mut row = vec![0f32; w];
    for y in 1..h - 1 {
        {
            let old = |j: usize| match (j / w + 2).checked_sub(y) {
                Some(0) => prev2[j % w],
                Some(1) => prev1[j % w],
                _ => z[j],
            };
            // The share of its outflows a cell may give without going below its bounds.
            let give = |c: usize| {
                let zc = old(c);
                let o: f32 = nbrs(c)
                    .into_iter()
                    .filter(|&j| active(j))
                    .map(|j| (zc - old(j)).max(0.0))
                    .sum::<f32>()
                    * alpha;
                let room = zc - r.floor[c].max(water_bound(r, c));
                if o <= 0.0 {
                    0.0
                } else {
                    (room / o).clamp(0.0, 1.0)
                }
            };
            for (x, v) in row.iter_mut().enumerate() {
                let c = y * w + x;
                let zc = old(c);
                *v = zc;
                if !active(c) {
                    continue;
                }
                let (gc, mut dz) = (give(c), 0f32);
                for j in nbrs(c).into_iter().filter(|&j| active(j)) {
                    let d = old(j) - zc;
                    dz += alpha * d * if d > 0.0 { give(j) } else { gc };
                }
                *v = zc + dz;
            }
        }
        std::mem::swap(&mut prev2, &mut prev1);
        prev1.copy_from_slice(&z[y * w..(y + 1) * w]);
        for (x, &v) in row.iter().enumerate() {
            let c = y * w + x;
            let d = v - prev1[x];
            if d != 0.0 {
                out.cells += 1;
                out.max_lowering_m = out.max_lowering_m.max(-d);
                out.max_rise_m = out.max_rise_m.max(d);
            }
            z[c] = v;
        }
    }
    out
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
/// stream power already bounded). `depth` is scratch (one f32 a cell) for the sources' depths.
/// Returns the cells the discs lowered past their own cut.
pub(crate) fn taper(
    r: &Routing,
    z: &mut [f32],
    before: &[f32],
    wall_deg: f64,
    depth: &mut [f32],
) -> u64 {
    let (w, h) = (r.world.w as usize, r.world.h as usize);
    let n = z.len();
    let tan = wall_deg.to_radians().tan();
    for c in 0..n {
        depth[c] = if wet(r, c) {
            0.0
        } else {
            (before[c] - z[c]).max(0.0)
        };
    }
    let at = |c: usize, dx: i64, dy: i64| {
        let (x, y) = ((c % w) as i64 + dx, (c / w) as i64 + dy);
        (x >= 0 && y >= 0 && x < w as i64 && y < h as i64).then(|| y as usize * w + x as usize)
    };
    // The discs, scattered (z holds each source's own cut already).
    for c in 0..n {
        let d = depth[c];
        let rc = disc_radius(d, tan, r.spacing_m);
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
            let v = (before[j] - lw)
                .max(r.floor[j])
                .max(water_bound(r, j))
                .min(before[j] - depth[j]);
            z[j] = z[j].min(v);
        }
    }
    (0..n).filter(|&j| z[j] < before[j] - depth[j]).count() as u64
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

/// One routing's passes and their figures (no `new_pits`; the stage counts them).
pub(crate) struct Step {
    pub fig: Figures,
    pub stream_s: f64,
    pub taper_s: f64,
    pub diffuse_s: f64,
}

/// One routing on the model's routing `r` (its `ka` ignored; `ka` given here): stream power,
/// then the taper on the stream-power cuts only, then mass-conserving diffusion around cuts
/// of at least DIFFUSE_MIN_M (ruling Q2: diffusion is smooth and needs no disc). `snap` sees
/// the ground after each pass ("sp", "tp", "df").
pub(crate) fn step(
    r: &Routing,
    ka: Vec<f32>,
    z: &mut [f32],
    wall_deg: f64,
    alpha: f32,
    snap: &mut dyn FnMut(&str, &[f32]) -> Result<()>,
) -> Result<Step> {
    let n = z.len();
    let started = std::time::Instant::now();
    let before = z.to_vec();
    let mut fig = stream_power(&Routing { ka: &ka, ..*r }, z);
    let stream_s = started.elapsed().as_secs_f64();
    snap("sp", z)?;
    fig.sills_cut = r
        .sills
        .iter()
        .filter(|s| before[s.cell as usize] - z[s.cell as usize] > CHANGED_M)
        .count() as u64;
    let near = footprint(r, &before, z, wall_deg, SHAPE_MIN_M, 0.0);
    let wide = footprint(r, &before, z, wall_deg, SHAPE_MIN_M, 3.0);
    let diffusing = footprint(r, &before, z, wall_deg, DIFFUSE_MIN_M, 0.0);
    // The taper does not read ka: its buffer is reused for the lowering.
    let mut lower = ka;
    let taper_start = std::time::Instant::now();
    fig.tapered_cells = taper(r, z, &before, wall_deg, &mut lower);
    let taper_s = taper_start.elapsed().as_secs_f64();
    drop(lower);
    snap("tp", z)?;
    let diffuse_start = std::time::Instant::now();
    let d = diffuse(r, z, alpha, &diffusing);
    (
        fig.diffused_cells,
        fig.diffusion_max_m,
        fig.diffusion_max_rise_m,
    ) = (d.cells, d.max_lowering_m, d.max_rise_m);
    let diffuse_s = diffuse_start.elapsed().as_secs_f64();
    drop(diffusing);
    snap("df", z)?;
    // B6 figure: full-K land outside the river cuts' footprint should not move.
    let bins = super::checks::BINS;
    let (mut hist, mut hist_wide) = (vec![0u64; bins], vec![0u64; bins]);
    let (mut hmax, mut hmax_wide) = (0f32, 0f32);
    for c in 0..n {
        let d = (before[c] - z[c]).max(0.0);
        if d >= SHAPE_MIN_M {
            fig.lowered_1cm += 1;
        }
        if near.get(c) || r.interior.get(c) || r.held.get(c) || r.sea.get(c) {
            continue;
        }
        let bin = ((d * 100.0) as usize).min(bins - 1);
        hist[bin] += 1;
        hmax = hmax.max(d);
        if !wide.get(c) {
            hist_wide[bin] += 1;
            hmax_wide = hmax_wide.max(d);
        }
    }
    fig.hillslope_p99_m = super::checks::p99_m(&hist) as f32;
    fig.hillslope_max_m = hmax;
    fig.hillslope_wide_p99_m = super::checks::p99_m(&hist_wide) as f32;
    fig.hillslope_wide_max_m = hmax_wide;
    Ok(Step {
        fig,
        stream_s,
        taper_s,
        diffuse_s,
    })
}

/// The `e` stage (B23): `e.routings` × [model on the current ground, stream power, taper,
/// diffusion] from the breached ground into `work/e<k>.f32`. Each routing is cached; returns
/// the last ground's path, the summed figures and each routing's.
pub(crate) fn erode(
    s: &Setup,
    breached: &Layer<f32>,
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
            // Diagnostic (calibration): the ground after stream power, taper and diffusion.
            let trace = std::env::var_os("TOPOGRAPHY_EROSION_TRACE").is_some();
            let mut snap = |name: &str, z: &[f32]| -> Result<()> {
                if !trace {
                    return Ok(());
                }
                Layer::open_or_create(world, &work.path(&format!("e{k}-{name}.f32")), 0.0)?
                    .write(whole, z)
            };
            let st = step(&r, ka, &mut z, s.params.wall_deg, e.diffusion, &mut snap)?;
            let (mut fig, stream_s, taper_s, diffuse_s) = (st.fig, st.stream_s, st.taper_s, st.diffuse_s);
            let bz = breached.to_vec()?;
            fig.new_pits = new_pits(world, &z, &bz, &held, &sea);
            out.write(whole, &z)?;
            eprintln!(
                "erosion: routing {}/{} lowered {} cells (max {:.2} m), {} sills, {} new pits, \
                 diffusion max rise {:.2} m; passes {:.0} s (stream power {:.0} s, taper {:.0} s, diffusion {:.0} s){}",
                k + 1,
                e.routings,
                fig.lowered_cells,
                fig.max_lowering_m,
                fig.sills_lowered,
                fig.new_pits,
                fig.diffusion_max_rise_m,
                started.elapsed().as_secs_f64(),
                stream_s,
                taper_s,
                diffuse_s,
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
    use crate::world::water::checks::horn_deg;
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

    /// A w × h routing with no flow (tests of the diffusion and footprint passes).
    fn grid(z: &[f32], w: usize, h: usize) -> Strip {
        let mut s = Strip::new(z);
        s.flow = vec![0; w * h];
        s
    }

    fn all(n: usize) -> Bits {
        let mut b = Bits::new(n);
        (0..n).for_each(|i| b.set(i));
        b
    }

    #[test]
    fn diffusion_never_notches_a_knife_ridge() {
        // Crest line N–S at x = 3, 100 m stepping 1 m a cell, sides at 0 m (ruling Q2).
        let (w, h) = (7usize, 7usize);
        let z: Vec<f32> = (0..w * h)
            .map(|i| {
                if i % w == 3 {
                    100.0 - (i / w) as f32
                } else {
                    0.0
                }
            })
            .collect();
        let s = grid(&z, w, h);
        let mut r = s.routing();
        r.world = World { w: 7, h: 7 };
        let mut out = z.clone();
        diffuse(&r, &mut out, 0.05, &all(w * h));
        // Rows beside the map edge are skipped: edge cells take no part in diffusion.
        for y in 2..h - 2 {
            let c = y * w + 3;
            assert!(
                out[c] >= out[c - w].min(out[c + w]),
                "crest ({y}) notched: {} under {} / {}",
                out[c],
                out[c - w],
                out[c + w]
            );
        }
    }

    #[test]
    fn the_taper_ignores_diffusion_lowering() {
        // An 11 × 11 plane falling east 1 m a cell (32 m cells); stream power cuts only (3, 5),
        // ~1 m, which opens diffusion within 3.5 cells. A 5 m bump at (6, 5), 3 cells away,
        // diffuses; it must not become a taper disc (ruling Q2), so its far side stays.
        let (w, h) = (11usize, 11usize);
        let mut z: Vec<f32> = (0..w * h).map(|i| 100.0 - (i % w) as f32).collect();
        z[5 * w + 6] += 5.0;
        let mut s = Strip::new(&z);
        s.ka = vec![0.0; w * h];
        s.ka[5 * w + 3] = 1000.0;
        let mut r = s.routing();
        r.world = World {
            w: w as u64,
            h: h as u64,
        };
        let before = z.clone();
        let st = step(&r, s.ka.clone(), &mut z, 35.0, 0.05, &mut |_, _| Ok(())).expect("step");
        assert!(before[5 * w + 3] - z[5 * w + 3] > 0.25, "the river cut");
        assert!(z[5 * w + 6] < before[5 * w + 6], "the bump diffuses");
        assert!(st.fig.diffused_cells >= 1);
        for (x, y) in [(7, 5), (7, 4), (7, 6)] {
            assert_eq!(
                z[y * w + x],
                before[y * w + x],
                "({x}, {y}) beside the bump moved"
            );
        }
    }

    #[test]
    fn a_shallow_cut_does_not_enable_diffusion() {
        let before = vec![10.0f32; 25];
        let s = grid(&before, 5, 5);
        let mut r = s.routing();
        r.world = World { w: 5, h: 5 };
        let mut z = before.clone();
        z[12] -= 0.2;
        let near = footprint(&r, &before, &z, 35.0, DIFFUSE_MIN_M, 0.0);
        assert_eq!(near.ones().count(), 0, "a 0.2 m cut");
        z[12] = before[12] - 0.3;
        let near = footprint(&r, &before, &z, 35.0, DIFFUSE_MIN_M, 0.0);
        assert!(near.get(12), "a 0.3 m cut");
    }

    #[test]
    fn diffusion_conserves_mass_and_deposits_in_hollows() {
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
        let d = diffuse(&r, &mut out, 0.05, &all(25));
        assert!(out[12] < 12.0 && d.cells >= 2, "the bump drops");
        assert!(out[13] > 9.0, "the pit fills (deposition)");
        assert!(d.max_rise_m > 0.0 && d.max_lowering_m > 0.0);
        let sum = |v: &[f32]| v.iter().map(|&x| f64::from(x)).sum::<f64>();
        assert!((sum(&out) - sum(&z)).abs() < 1e-3, "mass is conserved");
        let mut far = z.clone();
        assert_eq!(
            diffuse(&r, &mut far, 0.05, &Bits::new(25)).cells,
            0,
            "no cut nearby: the bump stays (B6)"
        );
    }

    #[test]
    fn diffusion_never_goes_below_the_floor_and_still_conserves_mass() {
        // A 4 m bump whose floor is 1 m below it: it gives at most 1 m, all kept nearby.
        let mut z = vec![10.0f32; 25];
        z[12] = 14.0;
        let mut s = grid(&z, 5, 5);
        s.floor = z.iter().map(|v| v - 1.0).collect();
        let mut r = s.routing();
        r.world = World { w: 5, h: 5 };
        let mut out = z.clone();
        diffuse(&r, &mut out, 0.25, &all(25));
        assert!(out[12] >= 13.0 - 1e-5, "floor held, got {}", out[12]);
        let sum = |v: &[f32]| v.iter().map(|&x| f64::from(x)).sum::<f64>();
        assert!((sum(&out) - sum(&z)).abs() < 1e-3, "mass is conserved");
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
        let tapered = taper(&r, &mut z, &before, 35.0, &mut lower);
        assert!(tapered >= 8);
        assert!(
            z[10 * w + 11] < 99.0 && z[10 * w + 9] < 99.0,
            "both sides lowered: >= 3 cells wide"
        );
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
    fn a_held_inside_sill_is_tapered() {
        // Lake in columns 0–4 at 100 m over a 95 m bed; its sill is the held inside cell (4, 5)
        // at 99.8 m, cut 3.8 m by stream power; it drains to the dry (5, 5). Dry ground falls
        // east 0.5 m a cell.
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
        taper(&routing_11(&s), &mut z, &before, 35.0, &mut lower);
        for c in [4 * w + 5, down, 6 * w + 5] {
            assert!(
                before[c] - z[c] > 0.5,
                "the outlet cut is 3 cells wide, cell {c} at {}",
                z[c]
            );
        }
        assert_eq!(z[sill], 96.0, "the sill keeps its own cut");
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
        taper(&routing_11(&s), &mut z, &before, 35.0, &mut lower);
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
    fn bits_list_their_ones_in_order() {
        let mut b = Bits::new(200);
        for i in [0, 63, 64, 130, 199] {
            b.set(i);
        }
        assert_eq!(b.ones().collect::<Vec<_>>(), [0, 63, 64, 130, 199]);
        assert_eq!(Bits::new(70).ones().count(), 0);
    }
}
