//! A few iterations of the implicit stream-power law (Braun and Willett 2013, n = 1):
//! dz/dt = −K A^m S, solved from the outlets upstream so each cell moves towards its already
//! updated receiver. No uplift. Grades collage seams and breach trenches into valley profiles.

use super::flood::{Dims, SEA, receiver};
use super::lakes::Basin;
use serde_json::{Value, json};

/// Judgment constants, tuned on fujix-collage-55 (32 m) so real valley floors move a few
/// metres while steps across seams and breach trenches grade. A is discharge in m² (km² at
/// mean rain × 10⁶), so F = K_DT · A^M / L per iteration.
pub(super) const ITERATIONS: usize = 5;
pub(super) const M: f32 = 0.5;
pub(super) const K_DT: f32 = 7e-4;
/// Stream power runs on grids at least this coarse only.
pub(super) const MIN_SPACING_M: f64 = 32.0;

pub(super) fn json() -> Value {
    json!({
        "law": "implicit stream power, n = 1 (Braun and Willett 2013), no uplift",
        "iterations": ITERATIONS,
        "m": M,
        "k_dt": K_DT,
        "area": "discharge in m² (km² at mean rain × 1e6)",
        "limits": "never below 0 m, a downstream basin level or, beside a basin, its level; glacier cells and ground draining onto them or into ice basins unchanged; \
            basins and the sea unchanged",
        "min_spacing_m": MIN_SPACING_M,
    })
}

/// Lowering figures: total and largest, and on river cells (`channel`) split into those
/// breaching cut (trenches) and the rest (real valley floors).
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
        "river_cells_unbreached": summary(&mut real),
        "river_cells_breached": summary(&mut trench),
    })
}

/// Lower land (not sea, basin or glacier cells) by stream power along the receivers. `acc` is the
/// discharge (km² at mean rain). Kept basins act as base level at their level, the sea at
/// 0 m; cells beside a basin stay at or above its level, so its spill and shore hold.
#[allow(clippy::too_many_arguments)]
pub(super) fn stream_power(
    dims: Dims,
    ground: &mut [f32],
    dir: &[u8],
    acc: &[f32],
    flags: &[u8],
    body: &[u32],
    basins: &[Basin],
    ice: impl Fn(usize) -> bool,
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
    // Per cell: never below 0 m or the highest basin level beside it (never raised). Ice
    // (glacier class) and all ground draining onto it or into an ice basin is not eroded,
    // which keeps the summit caldera and its walls as they are.
    let mut frozen = vec![false; n];
    let floor: Vec<f32> = order
        .iter()
        .map(|&i| {
            let i = i as usize;
            frozen[i] = ice(i)
                || receiver(dims, dir, i).is_some_and(|r| {
                    frozen[r] || (body[r] != 0 && basins[body[r] as usize - 1].ice)
                });
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
