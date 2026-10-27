//! Rivers: cells with at least a threshold discharge (rain-weighted drainage area), one body
//! per river system (all river cells upstream of one mouth). The water surface never rises downstream and never stands
//! above a dry bank; the bed is cut below it instead.

use super::flood::{Dims, RIVER, SEA, receiver};
use super::lakes::{Basin, Kind};

/// Judgment constant: depth D = RIVER_DEPTH_M · A^0.4 with A the discharge in km² at mean
/// rain (1 m at 10 km², 6 m at 1000 km²). Channel shape is a later task.
const RIVER_DEPTH_M: f32 = 0.4;

pub(super) fn depth_m(discharge_km2: f32) -> f32 {
    RIVER_DEPTH_M * discharge_km2.powf(0.4)
}

/// Where a river system ends.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum End {
    Sea,
    Lake(u32),
    /// A closed dry basin (sink): an endorheic river.
    Sink(u32),
    /// A dry basin flow passes through (glacier ice): the river continues past its exit.
    Dry(u32),
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
                        Some(r)
                            if body[r] != 0 && basins[body[r] as usize - 1].kind == Kind::Sink =>
                        {
                            End::Sink(body[r])
                        }
                        Some(r) if body[r] != 0 => End::Dry(body[r]),
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
        surface[i] = match sill_level(dims, dir, flags, body, basins, i) {
            // A lake's outflow starts at its level; its banks may drop away (a waterfall lip).
            Some(level) => level,
            None => {
                let entered = entered_basin(dims, dir, flags, body, i);
                let mut s = ground[i] + depth_m(acc[i]);
                for (_, j) in dims.neighbours(i) {
                    if flags[j] & (SEA | RIVER) == 0
                        && !lake(flags, body, j)
                        && (body[j] == 0 || body[j] != entered)
                    {
                        s = s.min(ground[j]);
                    }
                }
                s
            }
        };
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
                    if sill_level(dims, dir, flags, body, basins, r).is_none() {
                        surface[r] = surface[r].min(surface[c]);
                    }
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

/// Ground raised beside lake exits.
#[derive(Default)]
pub(super) struct Lips {
    pub cells: u64,
    pub max_m: f32,
    pub volume_m3: f64,
}

/// Raise dry ground beside each lake exit that is below the lake level up to it (a rock sill,
/// the only raising), then every dry cell draining into a raised one that is now lower than it,
/// so no ground drains uphill. Basin cells are left as they are.
pub(super) fn raise_lips(
    dims: Dims,
    ground: &mut [f32],
    dir: &[u8],
    flags: &[u8],
    body: &[u32],
    basins: &[Basin],
    spacing_m: f64,
) -> Lips {
    let dry = |i: usize| flags[i] & (SEA | RIVER) == 0 && body[i] == 0;
    let mut lips = Lips::default();
    let mut stack: Vec<u32> = Vec::new();
    let mut raise = |ground: &mut [f32], i: usize, to: f32, stack: &mut Vec<u32>| {
        let by = to - ground[i];
        lips.cells += 1;
        lips.max_m = lips.max_m.max(by);
        lips.volume_m3 += by as f64 * spacing_m * spacing_m;
        ground[i] = to;
        stack.push(i as u32);
    };
    for x in 0..ground.len() {
        let Some(level) = sill_level(dims, dir, flags, body, basins, x) else {
            continue;
        };
        for (_, j) in dims.neighbours(x) {
            if dry(j) && ground[j] < level {
                raise(ground, j, level, &mut stack);
            }
        }
    }
    while let Some(r) = stack.pop() {
        let r = r as usize;
        for (_, d) in dims.neighbours(r) {
            if dry(d) && ground[d] < ground[r] && receiver(dims, dir, d) == Some(r) {
                let to = ground[r];
                raise(ground, d, to, &mut stack);
            }
        }
    }
    lips
}

/// A river cell some lake drains into (a lake exit): that lake's level.
pub(super) fn sill_level(
    dims: Dims,
    dir: &[u8],
    flags: &[u8],
    body: &[u32],
    basins: &[Basin],
    i: usize,
) -> Option<f32> {
    if flags[i] & RIVER == 0 {
        return None;
    }
    dims.neighbours(i)
        .filter(|&(_, j)| {
            flags[j] & (SEA | RIVER) == 0
                && body[j] != 0
                && basins[body[j] as usize - 1].kind == Kind::Lake
                && receiver(dims, dir, j) == Some(i)
        })
        .map(|(_, j)| basins[body[j] as usize - 1].level)
        .reduce(f32::min)
}

/// The dry basin a river cell drains straight into, or 0. Its cells are not banks.
pub(super) fn entered_basin(dims: Dims, dir: &[u8], flags: &[u8], body: &[u32], i: usize) -> u32 {
    match receiver(dims, dir, i) {
        Some(r) if flags[r] & (SEA | RIVER) == 0 => body[r],
        _ => 0,
    }
}
