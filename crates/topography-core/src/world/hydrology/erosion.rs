//! Shaping after breaching: breach trenches widened into small valleys, then (at 32 m) a few
//! iterations of the implicit stream-power law (Braun and Willett 2013, n = 1):
//! dz/dt = −K A^m S, solved from the outlets upstream so each cell moves towards its already
//! updated receiver. No uplift. Stream power grades collage seams and cliffs under rivers.

use super::flood::{BREACHED, Dims, SEA, receiver};
use super::lakes::Basin;
use serde_json::{Value, json};

/// Judgment constants, tuned on fujix-collage-55 (32 m) so real valley floors move a few
/// metres while steps across seams grade. A is discharge in m² (km² at mean rain × 10⁶), so
/// F = K_DT · A^M / L per iteration.
pub(super) const ITERATIONS: usize = 5;
pub(super) const M: f32 = 0.5;
pub(super) const K_DT: f32 = 7e-4;
/// Stream power runs on grids at least this coarse only.
pub(super) const MIN_SPACING_M: f64 = 32.0;
/// Judgment constants: trench walls are cut back to this side slope, over at most this many
/// rings of cells beside the trench.
pub(super) const SIDE_SLOPE_DEG: f32 = 35.0;
pub(super) const WIDEN_RINGS: usize = 16;

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
        "max_rings": WIDEN_RINGS,
        "rule": "dry ground beside a breach trench is lowered to the trench side slope from \
            the lowest neighbour already in the valley, ring by ring, and drains into it; \
            never below the trench, a basin level beside it or 0 m; basins, the sea, glacier \
            and ground draining onto glacier or into an ice basin unchanged",
    })
}

/// Ice (glacier class), ice-basin cells and everything draining onto them. Not eroded or
/// widened, which keeps the summit caldera and its walls as they are.
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

/// Widen breach trenches (cells flagged `BREACHED`, cut `cut(i)` m by breaching) into small
/// valleys: dry ground beside a trench above the side slope from its lowest valley neighbour
/// is lowered to it, by no more than the trench cut (less the slope's rise per further
/// ring), and then drains into that neighbour, ring by ring. Heights fall strictly towards
/// the trench along the new receivers, so no pits or cycles appear. Returns figures with
/// side slopes before and after (the steepest wall of each slot cell).
#[allow(clippy::too_many_arguments)]
pub(super) fn widen_trenches(
    dims: Dims,
    ground: &mut [f32],
    dir: &mut [u8],
    flags: &[u8],
    body: &[u32],
    basins: &[Basin],
    frozen: &[bool],
    cut: impl Fn(usize) -> f32,
    spacing_m: f64,
) -> Value {
    let n = ground.len();
    let tan = SIDE_SLOPE_DEG.to_radians().tan();
    let trench = |i: usize| flags[i] & BREACHED != 0 && flags[i] & SEA == 0 && body[i] == 0;
    let wall = |i: usize| flags[i] & (SEA | BREACHED) == 0 && body[i] == 0 && !frozen[i];
    let level = |i: usize| (body[i] != 0).then(|| basins[body[i] as usize - 1].level);
    let rise = |k: usize| tan * (Dims::step(Dims::back(k)) * spacing_m) as f32;
    // Side slopes where breaching cut a slot: trench cells cut more than one cell's rise.
    let slot = |t: usize| trench(t) && cut(t) > tan * spacing_m as f32;
    let slopes = |ground: &[f32]| -> Value {
        let mut v: Vec<f32> = (0..n)
            .filter(|&t| slot(t))
            .map(|t| {
                dims.neighbours(t)
                    .filter(|&(_, k)| wall(k))
                    .map(|(d, k)| {
                        let run = Dims::step(Dims::back(d)) * spacing_m;
                        ((ground[k] - ground[t]) as f64 / run).atan().to_degrees() as f32
                    })
                    .fold(0f32, f32::max)
            })
            .collect();
        v.sort_by(f32::total_cmp);
        let q = |p: usize| v.get(v.len() * p / 100).copied().unwrap_or(0.0);
        json!({ "median_deg": q(50), "p90_deg": q(90), "max_deg": v.last() })
    };
    let before = slopes(ground);

    // Trenches: 8-connected pieces of breached cells.
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

    // Each frontier cell carries the cut its outer neighbours may take: the trench's breach
    // depth, less the side-slope rise per ring, so widening dies out within a few cells.
    let mut frontier: Vec<(u32, f32)> = (0..n)
        .filter(|&i| trench(i))
        .map(|i| (i as u32, cut(i)))
        .collect();
    let (mut lowered, mut volume, mut max, mut rings) = (0u64, 0f64, 0f32, 0usize);
    let mut candidates: Vec<(u32, f32, u8, f32)> = Vec::new();
    while !frontier.is_empty() && rings < WIDEN_RINGS {
        candidates.clear();
        for &(j, left) in &frontier {
            let j = j as usize;
            for (k, c) in dims.neighbours(j) {
                if !wall(c) || left <= 0.0 {
                    continue;
                }
                let beside = dims
                    .neighbours(c)
                    .filter_map(|(_, b)| level(b))
                    .fold(0f32, f32::max);
                let limit = (ground[j] + rise(k)).max(ground[c] - left).max(beside);
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
            let c = c as usize;
            let by = ground[c] - limit;
            lowered += 1;
            volume += by as f64 * spacing_m * spacing_m;
            max = max.max(by);
            ground[c] = limit;
            dir[c] = d;
            frontier.push((c as u32, pass));
        }
    }
    json!({
        "trenches": trenches,
        "trench_cells": trench_cells,
        "wall_cells_lowered": lowered,
        "wall_volume_m3": volume.round(),
        "wall_max_lowering_m": max,
        "rings_used": rings,
        "slot_cells": (0..n).filter(|&t| slot(t)).count(),
        "slot_side_slope_before": before,
        "slot_side_slope_after": slopes(ground),
    })
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
