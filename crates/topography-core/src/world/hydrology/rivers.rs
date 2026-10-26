//! Rivers: cells draining at least a threshold area, one body per river system (all river
//! cells upstream of one mouth), with a water surface that never rises downstream.

use super::flood::{Dims, RIVER, SEA, receiver};
use super::lakes::{Basin, Kind};

/// Judgment constant: depth D = RIVER_DEPTH_M · A^0.4 with A the drainage area in km² standing
/// in for discharge (1 m at 10 km², 6 m at 1000 km²). Channels are carved in a later task.
const RIVER_DEPTH_M: f32 = 0.4;

pub(super) fn depth_m(area_km2: f32) -> f32 {
    RIVER_DEPTH_M * area_km2.powf(0.4)
}

/// Where a river system ends.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum End {
    Sea,
    Lake(u32),
    Edge,
    /// Dry land: never expected (rivers grow downstream), reported as a failure.
    Land,
}

pub(super) struct River {
    pub mouth: u32,
    pub end: End,
    pub cells: u32,
    pub length_m: f64,
    pub drainage_km2: f32,
}

/// Mark river cells (flag `RIVER`), store river ids (index + 1) in `body` and set their water
/// surface: ground plus depth, raised where needed so it never rises downstream, and at least
/// the level of the water the river ends in.
#[allow(clippy::too_many_arguments)]
pub(super) fn extract(
    dims: Dims,
    ground: &[f32],
    dir: &[u8],
    acc: &[f32],
    flags: &mut [u8],
    body: &mut [u32],
    basins: &[Basin],
    river_km2: f32,
    spacing_m: f64,
    surface: &mut [f32],
) -> Vec<River> {
    for i in 0..ground.len() {
        if flags[i] & SEA == 0 && body[i] == 0 && acc[i] >= river_km2 {
            flags[i] |= RIVER;
        }
    }
    let river = |flags: &[u8], i: usize| flags[i] & RIVER != 0;
    let mut rivers: Vec<River> = Vec::new();
    let mut chain: Vec<u32> = Vec::new();
    for i in 0..ground.len() {
        if !river(flags, i) || !surface[i].is_nan() {
            continue;
        }
        // Walk down to a known river cell or the river's end, then fill in from below.
        chain.clear();
        let mut c = i;
        loop {
            chain.push(c as u32);
            match receiver(dims, dir, c) {
                Some(r) if river(flags, r) && surface[r].is_nan() => c = r,
                _ => break,
            }
        }
        for &x in chain.iter().rev() {
            let x = x as usize;
            let r = receiver(dims, dir, x);
            let (below, id) = match r {
                Some(r) if river(flags, r) => (surface[r], body[r]),
                _ => {
                    let (end, below) = match r {
                        None => (End::Edge, f32::NEG_INFINITY),
                        Some(r) if flags[r] & SEA != 0 => (End::Sea, 0.0),
                        Some(r)
                            if body[r] != 0 && basins[body[r] as usize - 1].kind == Kind::Lake =>
                        {
                            (End::Lake(body[r]), basins[body[r] as usize - 1].level)
                        }
                        Some(_) => (End::Land, f32::NEG_INFINITY),
                    };
                    rivers.push(River {
                        mouth: x as u32,
                        end,
                        cells: 0,
                        length_m: 0.0,
                        drainage_km2: acc[x],
                    });
                    (below, rivers.len() as u32)
                }
            };
            surface[x] = (ground[x] + depth_m(acc[x])).max(below);
            body[x] = id;
            let river = &mut rivers[id as usize - 1];
            river.cells += 1;
            river.length_m += Dims::step(dir[x]) * spacing_m;
        }
    }
    rivers
}
