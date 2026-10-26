//! Rivers: cells draining at least a threshold area, one body per river system (all river
//! cells upstream of one mouth). The water surface never rises downstream and never stands
//! above a dry bank; the bed is cut below it instead.

use super::flood::{Dims, RIVER, SEA, receiver};
use super::lakes::{Basin, Kind};

/// Judgment constant: depth D = RIVER_DEPTH_M · A^0.4 with A the drainage area in km² standing
/// in for discharge (1 m at 10 km², 6 m at 1000 km²). Channel shape is a later task.
const RIVER_DEPTH_M: f32 = 0.4;

pub(super) fn depth_m(area_km2: f32) -> f32 {
    RIVER_DEPTH_M * area_km2.powf(0.4)
}

/// Where a river system ends.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum End {
    Sea,
    Lake(u32),
    /// A closed dry basin (sink): an endorheic river.
    Sink(u32),
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

/// Bed cut below the ground to keep river surfaces under their banks.
#[derive(Default)]
pub(super) struct BedCut {
    pub cells: u64,
    pub max_m: f32,
}

/// Mark river cells (flag `RIVER`), store river ids (index + 1) in `body`, set their water
/// surface and lower their bed (`ground`) to the surface minus the depth where needed.
#[allow(clippy::too_many_arguments)]
pub(super) fn extract(
    dims: Dims,
    ground: &mut [f32],
    dir: &[u8],
    acc: &[f32],
    flags: &mut [u8],
    body: &mut [u32],
    basins: &[Basin],
    river_km2: f32,
    spacing_m: f64,
    surface: &mut [f32],
) -> (Vec<River>, BedCut) {
    let n = ground.len();
    for i in 0..n {
        if flags[i] & SEA == 0 && body[i] == 0 && acc[i] >= river_km2 {
            flags[i] |= RIVER;
        }
    }
    let river = |flags: &[u8], i: usize| flags[i] & RIVER != 0;
    let lake = |flags: &[u8], body: &[u32], i: usize| {
        flags[i] & (SEA | RIVER) == 0
            && body[i] != 0
            && basins[body[i] as usize - 1].kind == Kind::Lake
    };

    // Ids: walk down to a river cell with an id or the river's end, then fill in from below.
    let mut rivers: Vec<River> = Vec::new();
    let mut chain: Vec<u32> = Vec::new();
    for i in 0..n {
        if !river(flags, i) || body[i] != 0 {
            continue;
        }
        chain.clear();
        let mut c = i;
        loop {
            chain.push(c as u32);
            match receiver(dims, dir, c) {
                Some(r) if river(flags, r) && body[r] == 0 => c = r,
                _ => break,
            }
        }
        for &x in chain.iter().rev() {
            let x = x as usize;
            let id = match receiver(dims, dir, x) {
                Some(r) if river(flags, r) => body[r],
                r => {
                    let end = match r {
                        None => End::Edge,
                        Some(r) if flags[r] & SEA != 0 => End::Sea,
                        Some(r) if lake(flags, body, r) => End::Lake(body[r]),
                        Some(r) if body[r] != 0 => End::Sink(body[r]),
                        Some(_) => End::Land,
                    };
                    rivers.push(River {
                        mouth: x as u32,
                        end,
                        cells: 0,
                        length_m: 0.0,
                        drainage_km2: acc[x],
                    });
                    rivers.len() as u32
                }
            };
            body[x] = id;
            let river = &mut rivers[id as usize - 1];
            river.cells += 1;
            river.length_m += Dims::step(dir[x]) * spacing_m;
        }
    }

    // Surfaces: ground plus depth, at most the lowest dry bank and the level of any lake
    // draining in, and at most every river cell upstream (upstream first, by donor count).
    let mut donors = vec![0u8; n];
    for i in 0..n {
        if !river(flags, i) {
            continue;
        }
        let mut s = ground[i] + depth_m(acc[i]);
        for (_, j) in dims.neighbours(i) {
            if lake(flags, body, j) {
                if receiver(dims, dir, j) == Some(i) {
                    s = s.min(basins[body[j] as usize - 1].level);
                }
            } else if flags[j] & (SEA | RIVER) == 0 {
                s = s.min(ground[j]);
            }
        }
        surface[i] = s;
        if let Some(r) = receiver(dims, dir, i).filter(|&r| river(flags, r)) {
            donors[r] += 1;
        }
    }
    for i in 0..n {
        let mut c = i;
        while river(flags, c) && donors[c] == 0 {
            donors[c] = u8::MAX;
            match receiver(dims, dir, c).filter(|&r| river(flags, r)) {
                Some(r) => {
                    surface[r] = surface[r].min(surface[c]);
                    donors[r] -= 1;
                    c = r;
                }
                None => break,
            }
        }
    }
    let mut cut = BedCut::default();
    for i in 0..n {
        if river(flags, i) {
            let bed = surface[i] - depth_m(acc[i]);
            if bed < ground[i] {
                cut.cells += 1;
                cut.max_m = cut.max_m.max(ground[i] - bed);
                ground[i] = bed;
            }
        }
    }
    (rivers, cut)
}
