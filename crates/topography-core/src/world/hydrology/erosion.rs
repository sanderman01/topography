//! Shaping after breaching: breach trenches widened into small valleys, then (at 32 m) a few
//! iterations of the implicit stream-power law (Braun and Willett 2013, n = 1):
//! dz/dt = −K A^m S, solved from the outlets upstream so each cell moves towards its already
//! updated receiver. No uplift. Stream power grades collage seams and cliffs under rivers.

use super::flood::{self, Dims, SEA, receiver};
use super::lakes::Basin;
use serde_json::{Value, json};
use std::collections::HashMap;

/// Judgment constants, tuned on fujix-collage-55 (32 m) so real valley floors move a few
/// metres while steps across seams grade. A is discharge in m² (km² at mean rain × 10⁶), so
/// F = K_DT · A^M / L per iteration.
pub(super) const ITERATIONS: usize = 5;
pub(super) const M: f32 = 0.5;
pub(super) const K_DT: f32 = 7e-4;
/// Stream power runs on grids at least this coarse only.
pub(super) const MIN_SPACING_M: f64 = 32.0;
/// Judgment constant: trench walls are cut back to this side slope, over as many rings of
/// cells as the deepest cut needs (cut / (tan · spacing) + 1).
pub(super) const SIDE_SLOPE_DEG: f32 = 35.0;

pub(super) fn json() -> Value {
    json!({
        "law": "implicit stream power, n = 1 (Braun and Willett 2013), no uplift",
        "iterations": ITERATIONS,
        "m": M,
        "k_dt": K_DT,
        "area": "discharge in m² (km² at mean rain × 1e6)",
        "limits": "never below 0 m, a downstream basin level or, beside a basin, its level; \
            basins, the sea, glacier cells and ground draining onto glacier or into an ice \
            basin unchanged",
        "min_spacing_m": MIN_SPACING_M,
    })
}

pub(super) fn trench_json() -> Value {
    json!({
        "side_slope_deg": SIDE_SLOPE_DEG,
        "max_rings": "ceil(deepest cut / (tan(side slope) · spacing)) + 1",
        "rule": "ground beside a breach trench or a burned coarse river path (trench cells \
            too) is lowered to the trench side slope from the lowest neighbour already in the \
            valley, ring by ring, and drains into it; never below the trench, a kept basin \
            level beside it or 0 m; coarse lakes, basins on the coarse level's dry basins (an \
            ice caldera) and every basin on the trunk level, the sea and cells from the \
            previous band unchanged; on finer levels other dry and ice basin cells may be \
            lowered and leave their basin, but not a cell lower ground drains into (a basin \
            entry a trench ends at); glacier ground widens like any other (ice is a \
            surface layer); after channel carving, ground beside channel cells cut more than \
            one cell's rise widens the same way from the water surface, never below the \
            surface of a river it is a bank of (not ground ahead along the flow), more than \
            0.5 m below every river beside it, or a kept basin level beside it",
    })
}

/// Ice (glacier class), ice-basin cells and everything draining onto them. Not eroded by
/// stream power, which keeps the summit caldera and its walls as they are.
pub(super) fn ice_catchment(
    dims: Dims,
    dir: &[u8],
    body: &[u32],
    basins: &[Basin],
    ice: impl Fn(usize) -> bool,
) -> Vec<bool> {
    let n = dir.len();
    let mut frozen = vec![false; n];
    let mut stack: Vec<u32> = Vec::new();
    for (i, f) in frozen.iter_mut().enumerate() {
        if ice(i) || (body[i] != 0 && basins[body[i] as usize - 1].ice) {
            *f = true;
            stack.push(i as u32);
        }
    }
    while let Some(c) = stack.pop() {
        for (_, d) in dims.neighbours(c as usize) {
            if !frozen[d] && receiver(dims, dir, d) == Some(c as usize) {
                frozen[d] = true;
                stack.push(d as u32);
            }
        }
    }
    frozen
}

/// Widen trenches (`trench` cells, cut `cut(i)` m) into small valleys: `wall` ground beside
/// the valley above the side slope from its lowest valley neighbour (a trench cell's water
/// `surface` when wet, else its ground) is lowered to it, by no more than the trench cut
/// (less the slope's rise per further ring; a cell reached again keeps that bound from its
/// ground before widening), never below `floor(i)`, and then drains into that neighbour, ring
/// by ring. Trench cells can be walls too, so a shallow trench crossing a deep one leaves no
/// fin. Heights fall strictly towards the trench along the new receivers, so no pits or
/// cycles appear. Returns figures with side slopes before and after (the steepest wall of
/// each slot cell) and the lowered cells.
#[allow(clippy::too_many_arguments)]
pub(super) fn widen_trenches(
    dims: Dims,
    ground: &mut [f32],
    dir: &mut [u8],
    trench: impl Fn(usize) -> bool,
    wall: impl Fn(usize) -> bool,
    surface: impl Fn(usize) -> f32,
    floor: impl Fn(usize) -> f32,
    cut: impl Fn(usize) -> f32,
    spacing_m: f64,
    old_receiver: bool,
    outlet: impl Fn(usize) -> bool,
) -> (Value, Vec<u32>) {
    let n = ground.len();
    let tan = SIDE_SLOPE_DEG.to_radians().tan();
    let rise = |k: usize| tan * (Dims::step(Dims::back(k)) * spacing_m) as f32;
    let top = |ground: &[f32], t: usize| ground[t].max(surface(t));
    // Side slopes where a slot was cut: trench cells cut more than one cell's rise.
    let slot = |t: usize| trench(t) && cut(t) > tan * spacing_m as f32;
    let slopes = |ground: &[f32]| -> Value {
        let mut v: Vec<(f32, usize)> = (0..n)
            .filter(|&t| slot(t))
            .map(|t| {
                let deg = dims
                    .neighbours(t)
                    .filter(|&(_, k)| wall(k))
                    .map(|(d, k)| {
                        let run = Dims::step(Dims::back(d)) * spacing_m;
                        ((ground[k] - top(ground, t)) as f64 / run)
                            .atan()
                            .to_degrees() as f32
                    })
                    .fold(0f32, f32::max);
                (deg, t)
            })
            .collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        let q = |p: usize| v.get(v.len() * p / 100).map_or(0.0, |s| s.0);
        let last = v.last().map(|s| (s.0, [s.1 % dims.w, s.1 / dims.w]));
        json!({
            "median_deg": q(50),
            "p90_deg": q(90),
            "max_deg": last.map(|s| s.0),
            "max_at_cell": last.map(|s| s.1),
            "over_60_deg": v.iter().filter(|s| s.0 > 60.0).count(),
        })
    };
    let before = slopes(ground);

    // Trenches: 8-connected pieces of trench cells.
    let mut seen = vec![false; n];
    let (mut trenches, mut trench_cells) = (0u64, 0u64);
    let mut stack: Vec<u32> = Vec::new();
    for i in 0..n {
        if !trench(i) || seen[i] {
            continue;
        }
        trenches += 1;
        seen[i] = true;
        stack.push(i as u32);
        while let Some(c) = stack.pop() {
            trench_cells += 1;
            for (_, k) in dims.neighbours(c as usize) {
                if trench(k) && !seen[k] {
                    seen[k] = true;
                    stack.push(k as u32);
                }
            }
        }
    }
    drop(seen);

    // Each frontier cell carries its height and the cut its outer neighbours may take: the
    // trench's depth, less the side-slope rise per ring, so widening dies out. Enough rings
    // for the deepest cut to reach the side slope.
    let mut frontier: Vec<(u32, f32, f32)> = (0..n)
        .filter(|&i| trench(i))
        .map(|i| (i as u32, cut(i), top(ground, i)))
        .collect();
    let deepest = frontier.iter().map(|f| f.1).fold(0f32, f32::max);
    let max_rings = (deepest / (tan * spacing_m as f32)).ceil() as usize + 1;
    let (mut lowerings, mut rings) = (0u64, 0usize);
    // Ground before widening of each lowered cell: a cell reached again from another side
    // is bounded by the same total cut.
    let mut was: HashMap<u32, f32> = HashMap::new();
    let mut old_dir: HashMap<u32, u8> = HashMap::new();
    // Walls left unlowered: every lower neighbour drains through them.
    let mut refused = 0u64;
    let mut candidates: Vec<(u32, f32, u8, f32)> = Vec::new();
    while !frontier.is_empty() && rings < max_rings {
        candidates.clear();
        for &(j, left, z) in &frontier {
            let j = j as usize;
            for (k, c) in dims.neighbours(j) {
                if left <= 0.0 || !wall(c) {
                    continue;
                }
                let g0 = was.get(&(c as u32)).copied().unwrap_or(ground[c]);
                let limit = (z + rise(k)).max(g0 - left).max(floor(c));
                if ground[c] > limit {
                    candidates.push((c as u32, limit, Dims::back(k), left - rise(k)));
                }
            }
        }
        if candidates.is_empty() {
            break;
        }
        rings += 1;
        candidates.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        candidates.dedup_by_key(|c| c.0);
        frontier.clear();
        for &(c, limit, d, pass) in &candidates {
            // The lowered cell drains to the cell it was lowered from. Beside channels (no
            // re-routing follows), not where that cell's flow runs through it (a cycle): it
            // then takes its steepest lower neighbour whose flow does not, and with none (a
            // sill all its lower neighbours drain across) it is not lowered. Trench cycles
            // get their old receivers back below, and steepest descent follows.
            let i = c as usize;
            // A cell that lower dry ground drains into (a basin's entry, as a breach trench
            // ends at a sink) would leave its basin with that ground draining uphill: kept.
            // River cells may: their water surface carries the flow.
            if dims.neighbours(i).any(|(_, j)| {
                surface(j).is_nan() && ground[j] < limit && receiver(dims, dir, j) == Some(i)
            }) {
                refused += 1;
                continue;
            }
            let mut to = d;
            let into = dims.neighbour(i, d as usize - 1);
            if !old_receiver && into.is_some_and(|j| flows_into(dims, dir, j, i)) {
                let mut best: Option<(f32, u8)> = None;
                for (k, j) in dims.neighbours(i) {
                    let e = k as u8 + 1;
                    let slope = (limit - ground[j]) / Dims::step(e) as f32;
                    if slope > 0.0
                        && best.is_none_or(|(s, _)| slope > s)
                        && !flows_into(dims, dir, j, i)
                    {
                        best = Some((slope, e));
                    }
                }
                match best {
                    Some((_, e)) => to = e,
                    None => {
                        refused += 1;
                        continue;
                    }
                }
            }
            was.entry(c).or_insert(ground[i]);
            old_dir.entry(c).or_insert(dir[i]);
            lowerings += 1;
            ground[i] = limit;
            dir[i] = to;
            frontier.push((c, pass, limit));
        }
    }
    // A wall cell the trench drains into, pointed back at the trench, closes a cycle: it takes
    // its receiver before widening (`old_receiver`, trenches) or its steepest lower neighbour
    // that does not drain into it (channels, whose old receivers may now lie above it). A cell
    // lowered out of a sink basin had none, so it takes the steepest neighbour too, keeping
    // none only without one; an outlet keeps none.
    let ground_now: &[f32] = ground;
    flood::restore_on_cycles(dims, dir, |dir, c| {
        let old = *old_dir.get(&(c as u32))?;
        if old_receiver && (old != 0 || outlet(c)) {
            return Some(old);
        }
        let mut best: Option<(f32, u8)> = None;
        for (k, j) in dims.neighbours(c) {
            let d = k as u8 + 1;
            let slope = (ground_now[c] - ground_now[j]) / Dims::step(d) as f32;
            if slope > 0.0
                && receiver(dims, dir, j) != Some(c)
                && best.is_none_or(|(s, _)| slope > s)
            {
                best = Some((slope, d));
            }
        }
        best.map(|b| b.1).or(old_receiver.then_some(old))
    });
    drop(old_dir);
    let mut lowered: Vec<u32> = was.keys().copied().collect();
    lowered.sort_unstable();
    let (mut volume, mut max) = (0f64, 0f32);
    for &c in &lowered {
        let by = was[&c] - ground[c as usize];
        volume += by as f64 * spacing_m * spacing_m;
        max = max.max(by);
    }
    let figures = json!({
        "trenches": trenches,
        "trench_cells": trench_cells,
        "wall_cells_lowered": lowered.len(),
        "wall_lowerings": lowerings,
        "wall_cells_refused": refused,
        "wall_volume_m3": volume.round(),
        "wall_max_lowering_m": max,
        "rings_used": rings,
        "max_rings": max_rings,
        "slot_cells": (0..n).filter(|&t| slot(t)).count(),
        "slot_side_slope_before": before,
        "slot_side_slope_after": slopes(ground),
    });
    (figures, lowered)
}

/// Whether the flow from `from` reaches `to` within 4,096 cells. A loop through a longer
/// chain is left to the cycle spill after widening and to the cycle guard; refusing every long
/// chain left 1 m slots unwidened.
fn flows_into(dims: Dims, dir: &[u8], from: usize, to: usize) -> bool {
    let mut c = from;
    for _ in 0..4096 {
        if c == to {
            return true;
        }
        match receiver(dims, dir, c) {
            Some(r) => c = r,
            None => return false,
        }
    }
    false
}

/// Lowering figures: total and largest, and on river cells (`channel`) split into those
/// breaching or trench widening cut and the rest (real valley floors).
pub(super) fn figures(
    dims: Dims,
    before: &[f32],
    after: &[f32],
    breached: &[bool],
    channel: impl Fn(usize) -> bool,
    spacing_m: f64,
) -> Value {
    let (mut cells, mut volume, mut max, mut at) = (0u64, 0f64, 0f32, 0usize);
    let (mut real, mut trench) = (Vec::new(), Vec::new());
    for i in 0..before.len() {
        let dz = before[i] - after[i];
        if dz > 0.0 {
            cells += 1;
            volume += dz as f64 * spacing_m * spacing_m;
            if dz > max {
                (max, at) = (dz, i);
            }
        }
        if channel(i) {
            if breached[i] { &mut trench } else { &mut real }.push(dz);
        }
    }
    let summary = |v: &mut Vec<f32>| {
        v.sort_by(f32::total_cmp);
        let q = |p: usize| v.get(v.len() * p / 100).copied().unwrap_or(0.0);
        json!({ "cells": v.len(), "median_m": q(50), "p90_m": q(90), "max_m": v.last() })
    };
    json!({
        "cells_lowered": cells,
        "volume_m3": volume.round(),
        "max_m": max,
        "max_at_cell": [at % dims.w, at / dims.w],
        "river_cells_uncut": summary(&mut real),
        "river_cells_breached_or_widened": summary(&mut trench),
    })
}

/// Lower land (not sea, basin or `frozen` cells) by stream power along the receivers. `acc`
/// is the discharge (km² at mean rain). Kept basins act as base level at their level, the
/// sea at 0 m; cells beside a basin stay at or above its level, so its spill and shore hold.
#[allow(clippy::too_many_arguments)]
pub(super) fn stream_power(
    dims: Dims,
    ground: &mut [f32],
    dir: &[u8],
    acc: &[f32],
    flags: &[u8],
    body: &[u32],
    basins: &[Basin],
    frozen: &[bool],
    spacing_m: f64,
) {
    let n = ground.len();
    let level = |i: usize| (body[i] != 0).then(|| basins[body[i] as usize - 1].level);
    let land = |i: usize| flags[i] & SEA == 0 && body[i] == 0;
    // Receivers before donors: breadth first from cells that drain off land.
    let mut order: Vec<u32> = (0..n)
        .filter(|&i| land(i) && receiver(dims, dir, i).is_none_or(|r| !land(r)))
        .map(|i| i as u32)
        .collect();
    let mut k = 0;
    while k < order.len() {
        let c = order[k] as usize;
        k += 1;
        for (_, d) in dims.neighbours(c) {
            if land(d) && receiver(dims, dir, d) == Some(c) {
                order.push(d as u32);
            }
        }
    }
    // Per cell: never below 0 m or the highest basin level beside it (never raised).
    let floor: Vec<f32> = order
        .iter()
        .map(|&i| {
            let i = i as usize;
            if frozen[i] {
                return ground[i];
            }
            let beside = dims
                .neighbours(i)
                .filter_map(|(_, j)| level(j))
                .fold(0f32, f32::max);
            ground[i].min(beside)
        })
        .collect();
    for _ in 0..ITERATIONS {
        for (&i, &low) in order.iter().zip(&floor) {
            let i = i as usize;
            let Some(r) = receiver(dims, dir, i) else {
                continue;
            };
            let base = if flags[r] & SEA != 0 {
                0.0
            } else {
                level(r).unwrap_or(ground[r])
            }
            .min(ground[i]);
            let length = Dims::step(dir[i]) * spacing_m;
            let f = K_DT * (acc[i] * 1e6).powf(M) / length as f32;
            // Rounding must neither raise a cell nor put it below its receiver.
            let z = ((ground[i] + f * base) / (1.0 + f)).clamp(base, ground[i]);
            ground[i] = z.max(low);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trench along row 2 (cut 5 m) whose end (2, 2) drains east into (2, 3), the entry of
    /// a sink basin above it (no receiver), as a breach trench ends at a sink.
    fn trench_end_scene() -> (Dims, Vec<f32>, Vec<u8>) {
        let dims = Dims { w: 6, h: 5 };
        let mut ground = vec![10.0f32; 30];
        for x in 0..3 {
            ground[12 + x] = 0.0;
        }
        let mut dir = vec![0u8; 30];
        for (i, d) in dir.iter_mut().enumerate() {
            if !dims.is_edge(i) {
                *d = 1;
            }
        }
        (dir[12], dir[13], dir[14], dir[15], dir[16]) = (0, 3, 4, 0, 4);
        (dims, ground, dir)
    }

    #[test]
    fn widening_keeps_the_cell_a_trench_end_drains_into() {
        for old_receiver in [true, false] {
            let (dims, mut ground, mut dir) = trench_end_scene();
            let trench = |i: usize| (12..15).contains(&i);
            let (_, lowered) = widen_trenches(
                dims,
                &mut ground,
                &mut dir,
                trench,
                |i| !trench(i),
                |_| f32::NAN,
                |_| 0.0,
                |i| if trench(i) { 5.0 } else { 0.0 },
                4.0,
                old_receiver,
                |_| false,
            );
            let mode = format!("old_receiver {old_receiver}");
            assert_eq!(flood::cycle_cells(dims, &dir), 0, "{mode}");
            assert!(
                ground[8] < 10.0,
                "a wall beside the trench is lowered, {mode}"
            );
            // Lowered, the sink entry would leave its basin with the trench draining uphill.
            assert_eq!(ground[15], 10.0, "{mode}");
            assert_eq!(receiver(dims, &dir, 14), Some(15), "{mode}");
            assert_eq!(dir[15], 0, "{mode}");
            for &c in &lowered {
                let c = c as usize;
                assert!(receiver(dims, &dir, c).is_some(), "{c} drains, {mode}");
                for (_, j) in dims.neighbours(c) {
                    let uphill = receiver(dims, &dir, j) == Some(c) && ground[j] < ground[c];
                    assert!(!uphill, "{j} drains uphill into {c}, {mode}");
                }
            }
        }
    }
}
