//! Sea, priority flood (Barnes et al. 2014), breaching by priority-flood carving and flow
//! accumulation. Receivers are stored as one D8 direction byte per cell.

use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, HashSet, VecDeque},
};

use super::lakes::{Basin, Kind};

/// Cell flags.
pub(super) const SEA: u8 = 1;
pub(super) const SEEN: u8 = 2;
pub(super) const RIVER: u8 = 4;
/// Inside a depression already checked by `breach`.
const CHECKED: u8 = 8;
/// A river centreline cell (also `RIVER`); other river cells are channel width beside it.
pub(super) const CENTRE: u8 = 16;
/// Lowered by breaching (a trench cell).
pub(super) const BREACHED: u8 = 32;
/// Copied from the previous band (`--north`): never changed here.
pub(super) const FIXED: u8 = 64;
/// A flood outlet that is not sea: a grid edge the water leaves by, or a fixed cell beside
/// the free ones.
pub(super) const OUTLET: u8 = 128;

/// `flow.u8` bits above the D8 direction (low 4 bits): centreline, dry basin, endorheic.
pub(super) const FLOW_CENTRE: u8 = 16;
pub(super) const FLOW_DRY: u8 = 32;
pub(super) const FLOW_SINK: u8 = 64;

/// D8 neighbour offsets (row, column); a direction byte is an index here plus one, 0 none.
const OFFSETS: [(isize, isize); 8] = [
    (-1, 0),
    (1, 0),
    (0, -1),
    (0, 1),
    (-1, -1),
    (-1, 1),
    (1, -1),
    (1, 1),
];
const OPPOSITE: [u8; 8] = [1, 0, 3, 2, 7, 6, 5, 4];

#[derive(Clone, Copy)]
pub(super) struct Dims {
    pub w: usize,
    pub h: usize,
}

impl Dims {
    pub fn neighbour(self, i: usize, k: usize) -> Option<usize> {
        let (dy, dx) = OFFSETS[k];
        let y = (i / self.w).checked_add_signed(dy)?;
        let x = (i % self.w).checked_add_signed(dx)?;
        (y < self.h && x < self.w).then_some(y * self.w + x)
    }
    pub fn neighbours(self, i: usize) -> impl Iterator<Item = (usize, usize)> {
        (0..8).filter_map(move |k| self.neighbour(i, k).map(|j| (k, j)))
    }
    pub fn is_edge(self, i: usize) -> bool {
        let (y, x) = (i / self.w, i % self.w);
        y == 0 || x == 0 || y + 1 == self.h || x + 1 == self.w
    }
    /// Step length in cells for a direction byte.
    pub fn step(d: u8) -> f64 {
        if d > 4 { std::f64::consts::SQRT_2 } else { 1.0 }
    }
    /// Neighbour `k` of `j` lies ahead along direction byte `d` (positive projection): it is
    /// the next cross-section's bank, not this one's.
    pub fn ahead(self, d: u8, j: usize, k: usize) -> bool {
        if d == 0 {
            return false;
        }
        let (dy, dx) = OFFSETS[d as usize - 1];
        let oy = (k / self.w) as isize - (j / self.w) as isize;
        let ox = (k % self.w) as isize - (j % self.w) as isize;
        oy * dy + ox * dx > 0
    }
    /// Direction byte from neighbour `k` of a cell (as in `neighbours`) back to the cell.
    pub fn back(k: usize) -> u8 {
        OPPOSITE[k] + 1
    }
    /// Offset of direction byte `d` (row, column).
    pub fn offset(d: u8) -> (isize, isize) {
        OFFSETS[d as usize - 1]
    }
    /// Direction byte from cell `a` to its neighbour `b`.
    pub fn towards(self, a: usize, b: usize) -> u8 {
        let off = (
            (b / self.w) as isize - (a / self.w) as isize,
            (b % self.w) as isize - (a % self.w) as isize,
        );
        OFFSETS
            .iter()
            .position(|&o| o == off)
            .map_or(0, |k| k as u8 + 1)
    }
}

pub(super) fn receiver(dims: Dims, dir: &[u8], i: usize) -> Option<usize> {
    match dir[i] {
        0 => None,
        d => dims.neighbour(i, d as usize - 1),
    }
}

/// Min-heap entry: lowest value first, then lowest index.
#[derive(PartialEq)]
struct Item {
    z: f32,
    i: u32,
}
impl Eq for Item {}
impl Ord for Item {
    fn cmp(&self, other: &Self) -> Ordering {
        other.z.total_cmp(&self.z).then(other.i.cmp(&self.i))
    }
}
impl PartialOrd for Item {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Mark ground below 0 connected (8-way) to a `seed` cell below 0 (the world edge, coarse
/// sea) or to fixed sea as sea, never into a `barrier` cell (a coarse lake's: a lagoon the
/// coarse level kept apart from the sea stays apart). Fixed cells keep their flags.
pub(super) fn open_sea(
    dims: Dims,
    ground: &[f32],
    seed: impl Fn(usize) -> bool,
    barrier: impl Fn(usize) -> bool,
    flags: &mut [u8],
) {
    let mut stack: Vec<u32> = Vec::new();
    for i in 0..ground.len() {
        if flags[i] & FIXED != 0 {
            if flags[i] & SEA != 0 {
                stack.push(i as u32);
            }
            continue;
        }
        flags[i] &= !SEA;
        if ground[i] < 0.0 && seed(i) && !barrier(i) {
            flags[i] |= SEA;
            stack.push(i as u32);
        }
    }
    while let Some(i) = stack.pop() {
        for (_, j) in dims.neighbours(i as usize) {
            if flags[j] & (SEA | FIXED) == 0 && ground[j] < 0.0 && !barrier(j) {
                flags[j] |= SEA;
                stack.push(j as u32);
            }
        }
    }
}

/// Seed the flood: sea and fixed cells are seen; coast cells and outlets (`OUTLET`) are queued.
fn seeds(dims: Dims, value: impl Fn(usize) -> f32, flags: &mut [u8], heap: &mut BinaryHeap<Item>) {
    for f in flags.iter_mut() {
        *f &= !(SEEN | CHECKED);
    }
    for i in 0..flags.len() {
        if flags[i] & SEA != 0 {
            flags[i] |= SEEN;
            if dims.neighbours(i).any(|(_, j)| flags[j] & SEA == 0) {
                heap.push(Item {
                    z: value(i),
                    i: i as u32,
                });
            }
        } else if flags[i] & (FIXED | OUTLET) != 0 {
            flags[i] |= SEEN;
            if flags[i] & OUTLET != 0 {
                heap.push(Item {
                    z: value(i),
                    i: i as u32,
                });
            }
        }
    }
}

/// Every edge cell an outlet (a whole world, no coarse level).
pub(super) fn edge_outlets(dims: Dims, flags: &mut [u8]) {
    for (i, f) in flags.iter_mut().enumerate() {
        if dims.is_edge(i) {
            *f |= OUTLET;
        }
    }
}

/// Depression-filled surface (no gradient across flats) and receivers along the flood.
/// Outlets are the sea and `OUTLET` cells.
pub(super) fn fill(dims: Dims, ground: &[f32], flags: &mut [u8], dir: &mut [u8]) -> Vec<f32> {
    let mut filled = ground.to_vec();
    let mut heap = BinaryHeap::new();
    let mut pit = VecDeque::new();
    seeds(dims, |i| ground[i], flags, &mut heap);
    loop {
        let c = match pit.pop_front() {
            Some(c) => c,
            None => match heap.pop() {
                Some(item) => item.i as usize,
                None => break,
            },
        };
        let z = filled[c];
        for (k, n) in dims.neighbours(c) {
            if flags[n] & SEEN != 0 {
                continue;
            }
            flags[n] |= SEEN;
            dir[n] = OPPOSITE[k] + 1;
            if ground[n] <= z {
                filled[n] = z;
                pit.push_back(n);
            } else {
                heap.push(Item {
                    z: ground[n],
                    i: n as u32,
                });
            }
        }
    }
    filled
}

/// Lower the path from `from` towards the outlet to at most `target`, stopping at basin cells,
/// at ground already low enough and at a cell whose carve `floor` lies above `target`; lowered
/// cells get `BREACHED`. Returns the cells lowered.
#[allow(clippy::too_many_arguments)]
fn lower(
    dims: Dims,
    ground: &mut [f32],
    flags: &mut [u8],
    body: &[u32],
    dir: &[u8],
    floor: &[f32],
    from: usize,
    target: f32,
) -> u64 {
    let (mut at, mut lowered) = (Some(from), 0);
    while let Some(a) = at {
        if body[a] != 0 || ground[a] <= target || flags[a] & FIXED != 0 || floor[a] > target {
            break;
        }
        ground[a] = target;
        flags[a] |= BREACHED;
        lowered += 1;
        at = receiver(dims, dir, a);
    }
    lowered
}

/// `lower` from `from` to `target` would stop at a fixed cell or a carve `floor` above
/// `target` (a cut too deep, or one leaving a step at the seam).
#[allow(clippy::too_many_arguments)]
fn blocked(
    dims: Dims,
    ground: &[f32],
    flags: &[u8],
    body: &[u32],
    dir: &[u8],
    floor: &[f32],
    from: usize,
    target: f32,
) -> bool {
    let mut at = Some(from);
    while let Some(a) = at {
        if body[a] != 0 || ground[a] <= target {
            return false;
        }
        if flags[a] & FIXED != 0 || floor[a] > target {
            return true;
        }
        at = receiver(dims, dir, a);
    }
    false
}

/// Level of the lake that flow from `from` reaches first along the receivers, if it reaches a
/// kept basin that is a lake; else -inf.
fn lake_downstream(dims: Dims, body: &[u32], basins: &[Basin], dir: &[u8], from: usize) -> f32 {
    let mut at = Some(from);
    while let Some(a) = at {
        if body[a] != 0 {
            let basin = &basins[body[a] as usize - 1];
            return if basin.kind == Kind::Lake {
                basin.level
            } else {
                f32::NEG_INFINITY
            };
        }
        at = receiver(dims, dir, a);
    }
    f32::NEG_INFINITY
}

/// Priority flood over ground in which kept basins (`body` > 0) are flat at their level.
/// Every other depression is breached: when the flood first reaches it, the path it came along
/// is lowered once to the depression's lowest ground, and cells inside lower than their path
/// lower it again on the way in (Soille 2004 carving, cutting from the pit through the spill to
/// where the ground falls away). Equal heights are a flat, crossed without carving, so the
/// result descends or stays level along every receiver. Basin cells are never lowered.
///
/// A depression deeper than `cap_m` below its spill is kept instead, as `keep_kind(pit, true)`
/// says: a sink (dry, flow ends there) or lakes at the spill lowered by `cap_m` (not below
/// 0 m), one per piece of ground below that level. A shallower one with ground below 0 m is
/// kept as `keep_kind(pit, false)` says, lakes at 0 m (lagoons) or a sink, so cuts never go
/// below sea level. One whose path out starts at a fixed cell (from the previous band) above
/// the pit is a sink. On a finer level (`fine`), a depression the flood reaches from a lake
/// joins it, and one beside a lake is breached past the cap (the coarse lake must drain). A
/// path out that would cut a cell below its carve `floor` counts as blocked, as a fixed cell
/// does, and no cell is lowered below it.
/// Returns cells lowered.
#[allow(clippy::too_many_arguments)]
pub(super) fn breach(
    dims: Dims,
    ground: &mut [f32],
    body: &mut [u32],
    basins: &mut Vec<Basin>,
    flags: &mut [u8],
    dir: &mut [u8],
    cap_m: f32,
    floor: &[f32],
    fine: bool,
    keep_kind: impl Fn(usize, bool) -> Kind,
) -> u64 {
    let value = |g: &[f32], body: &[u32], basins: &[Basin], i: usize| match body[i] {
        0 => g[i],
        b => basins[b as usize - 1].level,
    };
    let mut heap = BinaryHeap::new();
    let mut pit = VecDeque::new();
    let (mut region, mut stack): (Vec<u32>, Vec<u32>) = (Vec::new(), Vec::new());
    dir.fill(0);
    seeds(dims, |i| value(ground, body, basins, i), flags, &mut heap);
    let mut lowered = 0;
    loop {
        let c = match pit.pop_front() {
            Some(c) => c,
            None => match heap.pop() {
                Some(item) => item.i as usize,
                None => break,
            },
        };
        let z = value(ground, body, basins, c);
        for (k, n) in dims.neighbours(c) {
            if flags[n] & SEEN != 0 {
                continue;
            }
            if body[n] == 0 && flags[n] & CHECKED == 0 && ground[n] < z {
                // A depression not seen before: everything unseen below the path, connected.
                region.clear();
                flags[n] |= CHECKED;
                stack.push(n as u32);
                let mut low = n;
                while let Some(a) = stack.pop() {
                    let a = a as usize;
                    region.push(a as u32);
                    if ground[a] < ground[low] {
                        low = a;
                    }
                    for (_, m) in dims.neighbours(a) {
                        if flags[m] & (SEEN | CHECKED | SEA) == 0 && body[m] == 0 && ground[m] < z {
                            flags[m] |= CHECKED;
                            stack.push(m as u32);
                        }
                    }
                }
                let p = ground[low];
                let deep = z - p > cap_m;
                let blocked = blocked(dims, ground, flags, body, dir, floor, c, p);
                let lake =
                    |i: usize| body[i] != 0 && basins[body[i] as usize - 1].kind == Kind::Lake;
                if fine && lake(c) {
                    // Below the lake's level beside it: it joins the lake (`lakes::extend`).
                } else if !deep && p >= 0.0 && !blocked {
                    lowered += lower(dims, ground, flags, body, dir, floor, c, p);
                } else if let Some(beside) = region
                    .iter()
                    .flat_map(|&a| dims.neighbours(a as usize))
                    .find(|&(_, m)| lake(m))
                    .map(|(_, m)| body[m])
                    .filter(|_| fine && !blocked)
                {
                    // Around a lake that must drain: cut past the cap, not below 0 m; ground
                    // below the lake's level connected to it joins the lake.
                    let level = basins[beside as usize - 1].level;
                    if p < level {
                        for &a in &region {
                            let a = a as usize;
                            if ground[a] < level
                                && dims.neighbours(a).any(|(_, m)| body[m] == beside)
                            {
                                body[a] = beside;
                                stack.push(a as u32);
                            }
                        }
                        while let Some(b) = stack.pop() {
                            for (_, m) in dims.neighbours(b as usize) {
                                if flags[m] & (SEEN | CHECKED) == CHECKED
                                    && body[m] == 0
                                    && ground[m] < level
                                {
                                    body[m] = beside;
                                    stack.push(m as u32);
                                }
                            }
                        }
                    }
                    // Pockets below 0 m it did not take stay below the cut, as a shallow
                    // depression below 0 m would: kept, never breached below sea level.
                    for &a in &region {
                        let a = a as usize;
                        if body[a] != 0 || ground[a] >= 0.0 {
                            continue;
                        }
                        let kind = keep_kind(a, false);
                        let level = if kind == Kind::Sink { z } else { 0.0 };
                        basins.push(Basin::kept(kind, level, c as u32));
                        let id = basins.len() as u32;
                        body[a] = id;
                        stack.push(a as u32);
                        while let Some(b) = stack.pop() {
                            for (_, m) in dims.neighbours(b as usize) {
                                if flags[m] & (SEEN | CHECKED) == CHECKED
                                    && body[m] == 0
                                    && ground[m] < 0.0
                                {
                                    body[m] = id;
                                    stack.push(m as u32);
                                }
                            }
                        }
                    }
                    lowered += lower(dims, ground, flags, body, dir, floor, c, p.max(0.0));
                } else if blocked || keep_kind(low, deep) == Kind::Sink {
                    // (A shallow one below 0 m becomes a lagoon at 0 m in any class.)
                    basins.push(Basin::kept(Kind::Sink, z, c as u32));
                    let id = basins.len() as u32;
                    for &a in &region {
                        body[a as usize] = id;
                    }
                } else {
                    // Not below a lake the outlet drains into: no dry ground below its level.
                    let level = (z - cap_m)
                        .max(0.0)
                        .max(lake_downstream(dims, body, basins, dir, c));
                    for &a in &region {
                        let a = a as usize;
                        if body[a] != 0 || ground[a] >= level {
                            continue;
                        }
                        basins.push(Basin::kept(Kind::Lake, level, c as u32));
                        let id = basins.len() as u32;
                        body[a] = id;
                        stack.push(a as u32);
                        while let Some(b) = stack.pop() {
                            for (_, m) in dims.neighbours(b as usize) {
                                if flags[m] & (SEEN | CHECKED) == CHECKED
                                    && body[m] == 0
                                    && ground[m] < level
                                {
                                    body[m] = id;
                                    stack.push(m as u32);
                                }
                            }
                        }
                    }
                }
            }
            flags[n] |= SEEN;
            dir[n] = OPPOSITE[k] + 1;
            if body[n] != 0 {
                // Basin: breadth first from where the flood entered it, cut to its level there.
                let basin = &basins[body[n] as usize - 1];
                let level = basin.level;
                if basin.kind == Kind::Sink {
                    dir[n] = 0;
                } else {
                    lowered += lower(dims, ground, flags, body, dir, floor, c, level);
                }
                if level <= z {
                    pit.push_back(n);
                } else {
                    heap.push(Item {
                        z: level,
                        i: n as u32,
                    });
                }
                continue;
            }
            if ground[n] < z {
                if blocked(dims, ground, flags, body, dir, floor, c, ground[n]) {
                    // The way out cannot be cut that low: the cell is filled to it instead.
                    ground[n] = z;
                } else {
                    lowered += lower(dims, ground, flags, body, dir, floor, c, ground[n]);
                }
            }
            if ground[n] == z {
                pit.push_back(n);
            } else {
                heap.push(Item {
                    z: ground[n],
                    i: n as u32,
                });
            }
        }
    }
    lowered
}

/// Receivers along the fall line for every land cell outside basins, sea, outlets and fixed
/// cells, on the final ground. The flood's discovery order picks the lowest neighbour, which is
/// nearly always a diagonal and draws 45° streams across slopes, and plain steepest descent
/// draws parallel straight lines on smooth slopes whose fall line lies between two of the
/// eight directions. So each cell takes the two neighbours bracketing its gradient direction
/// (Horn's 3 x 3 slope) and picks one with a share equal to the angle between them, against a
/// low-discrepancy threshold of its position: paths follow the fall line on average, and the
/// result is deterministic. Where the picked neighbour is not lower (or at an edge) it takes
/// the steepest descent (drop over distance, a diagonal √2 long). Cells with no strictly
/// lower neighbour (flats) keep the flood receiver, so every cell still drains. Strictly lower
/// steps alone cannot form a cycle, but a flood step left uphill (a pit lowered after the
/// flood) can close one: its cells get their flood receivers back, and a cycle left after that
/// spills (`spill_cycles`). A fixed cell is a receiver only as an outlet. Returns the cells the
/// fall-line step changed and the cells the spill redirected.
#[allow(clippy::too_many_arguments)]
pub(super) fn steepest(
    dims: Dims,
    origin: [i64; 2],
    ground: &[f32],
    flags: &[u8],
    body: &[u32],
    basins: &[Basin],
    dir: &mut [u8],
) -> (u64, u64) {
    // Directions (index + 1 is the byte) at 0°, 45°, ... from east turning south.
    const AROUND: [usize; 8] = [3, 7, 1, 6, 2, 4, 0, 5];
    let mut changed = 0u64;
    let flood_dir = dir.to_vec();
    let receives = |j: usize| flags[j] & FIXED == 0 || flags[j] & OUTLET != 0;
    // A basin counts at its level, as in the flood (its cells route to its exit).
    let height = |j: usize| match body[j] {
        0 => ground[j],
        b => basins[b as usize - 1].level,
    };
    for i in 0..ground.len() {
        if flags[i] & (SEA | FIXED | OUTLET) != 0 || body[i] != 0 || dir[i] == 0 {
            continue;
        }
        let mut best: Option<(f32, u8)> = None;
        for (k, j) in dims.neighbours(i) {
            if !receives(j) {
                continue;
            }
            let drop = ground[i] - height(j);
            if drop <= 0.0 {
                continue;
            }
            let slope = drop / Dims::step(k as u8 + 1) as f32;
            if best.is_none_or(|(s, _)| slope > s) {
                best = Some((slope, k as u8 + 1));
            }
        }
        let Some((_, mut d)) = best else { continue };
        if let Some(k) = fall_line(dims, origin, ground, i, &AROUND) {
            let ok = dims
                .neighbour(i, k)
                .is_some_and(|j| receives(j) && height(j) < ground[i]);
            if ok {
                d = k as u8 + 1;
            }
        }
        if d != dir[i] {
            dir[i] = d;
            changed += 1;
        }
    }
    restore_on_cycles(dims, dir, |_, i| Some(flood_dir[i]));
    let spilled = spill_cycles(dims, ground, flags, dir, |_| true);
    (changed, spilled)
}

/// The neighbour (index) on the fall line of cell `i`, dithered between the two directions
/// bracketing its gradient; None at the grid edge or on a flat. `origin` is the world column
/// and row of the grid's first cell, so overlapping bands and windows pick alike.
fn fall_line(
    dims: Dims,
    origin: [i64; 2],
    ground: &[f32],
    i: usize,
    around: &[usize; 8],
) -> Option<usize> {
    let (y, x) = (i / dims.w, i % dims.w);
    if y == 0 || x == 0 || y + 1 == dims.h || x + 1 == dims.w {
        return None;
    }
    let z = |dy: isize, dx: isize| {
        f64::from(ground[(y as isize + dy) as usize * dims.w + (x as isize + dx) as usize])
    };
    // Horn: east and south rises; the water goes the other way.
    let east = (z(-1, 1) + 2.0 * z(0, 1) + z(1, 1)) - (z(-1, -1) + 2.0 * z(0, -1) + z(1, -1));
    let south = (z(1, -1) + 2.0 * z(1, 0) + z(1, 1)) - (z(-1, -1) + 2.0 * z(-1, 0) + z(-1, 1));
    if east == 0.0 && south == 0.0 {
        return None;
    }
    let turns =
        (-south).atan2(-east).rem_euclid(std::f64::consts::TAU) / std::f64::consts::FRAC_PI_4;
    let sector = turns.floor() as usize % 8;
    // R2 low-discrepancy threshold: spread along rows, columns and diagonals.
    let (wx, wy) = (origin[0] + x as i64, origin[1] + y as i64);
    let t = (wx as f64 * 0.754_877_666_2 + wy as f64 * 0.569_840_290_9).rem_euclid(1.0);
    let k = if turns.fract() > t {
        (sector + 1) % 8
    } else {
        sector
    };
    Some(around[k])
}

/// Cells on receiver cycles, counted (0 for a proper drainage tree).
pub(super) fn cycle_cells(dims: Dims, dir: &[u8]) -> u64 {
    on_cycles(dims, dir).len() as u64
}

/// A pit lowered after the flood (a trench end) drains uphill, so a neighbour stepping down
/// into it closes a cycle. Each cycle then spills like a depression: the member beside the
/// lowest cell outside it whose flow does not reach a cycle (and that may receive: no fixed
/// cell but an outlet) drains there; members `may` allows go first. Returns the cells
/// redirected.
pub(super) fn spill_cycles(
    dims: Dims,
    ground: &[f32],
    flags: &[u8],
    dir: &mut [u8],
    may: impl Fn(usize) -> bool,
) -> u64 {
    let mut redirected = 0;
    for _ in 0..32 {
        let cycles = on_cycles(dims, dir);
        if cycles.is_empty() {
            break;
        }
        let mut on = vec![false; dir.len()];
        for &c in &cycles {
            on[c] = true;
        }
        let mut seen = vec![false; dir.len()];
        for &start in &cycles {
            if seen[start] {
                continue;
            }
            let mut members = Vec::new();
            let mut c = start;
            while !seen[c] {
                seen[c] = true;
                members.push(c);
                c = receiver(dims, dir, c).unwrap_or(start);
            }
            // A neighbour whose flow reaches a cycle would only lengthen it.
            let reaches_cycle = |mut j: usize| loop {
                if on[j] {
                    return true;
                }
                match receiver(dims, dir, j) {
                    Some(r) => j = r,
                    None => return false,
                }
            };
            let receives = |j: usize| flags[j] & FIXED == 0 || flags[j] & OUTLET != 0;
            let spill = |allowed: &dyn Fn(usize) -> bool| {
                let mut best: Option<(f32, usize, u8)> = None;
                for &m in members.iter().filter(|&&m| allowed(m)) {
                    for (k, j) in dims.neighbours(m) {
                        if best.is_none_or(|b| ground[j] < b.0) && receives(j) && !reaches_cycle(j)
                        {
                            best = Some((ground[j], m, k as u8 + 1));
                        }
                    }
                }
                best
            };
            let best = spill(&may).or_else(|| spill(&|_| true));
            if let Some((_, m, d)) = best {
                dir[m] = d;
                redirected += 1;
                continue;
            }
            // Every neighbour drains into the cycle: spill the whole region draining into it
            // over its lowest receiving neighbour, reversing the flow from there to the cycle.
            let mut region: HashSet<usize> = members.iter().copied().collect();
            // Cells in visiting order, so ties break the same way every run.
            let mut order = members.clone();
            let mut k = 0;
            while k < order.len() {
                let c = order[k];
                k += 1;
                for (_, j) in dims.neighbours(c) {
                    if !region.contains(&j) && receiver(dims, dir, j) == Some(c) {
                        region.insert(j);
                        order.push(j);
                    }
                }
            }
            // Only cells `may` allows are reversed, never fixed ones: else the guard sees it. A
            // cell whose flow to the cycle crosses another is no spill point (a coarse path
            // into a lake whose spill drains back into it: the lake spills elsewhere).
            let allowed = |c: usize| may(c) && flags[c] & FIXED == 0;
            let mut blocked: HashMap<usize, bool> = HashMap::new();
            for &c in &order {
                let b = !allowed(c)
                    || (!on[c]
                        && receiver(dims, dir, c)
                            .is_none_or(|r| blocked.get(&r).copied().unwrap_or(true)));
                blocked.insert(c, b);
            }
            let mut out: Option<(f32, usize, usize)> = None;
            for &u in order.iter().filter(|&&u| !blocked[&u]) {
                for (_, v) in dims.neighbours(u) {
                    if !region.contains(&v) && receives(v) && out.is_none_or(|b| ground[v] < b.0) {
                        out = Some((ground[v], u, v));
                    }
                }
            }
            let Some((_, u, v)) = out else { continue };
            let mut path = vec![u];
            while !on[*path.last().unwrap_or(&u)] {
                match receiver(dims, dir, path[path.len() - 1]) {
                    Some(r) => path.push(r),
                    None => break,
                }
            }
            if path.iter().any(|&c| !allowed(c)) {
                continue;
            }
            for k in (1..path.len()).rev() {
                dir[path[k]] = dims.towards(path[k], path[k - 1]);
            }
            dir[u] = dims.towards(u, v);
            redirected += path.len() as u64;
        }
    }
    redirected
}

/// Dry land cells (not fixed, wet or in a basin) that drain uphill onto plain land take their
/// steepest lower neighbour whose flow does not come back to them: a receiver restored on a
/// cycle can lie above a cell lowered after the flood. Returns the cells changed and their
/// new direction bytes.
pub(super) fn unclimb(
    dims: Dims,
    ground: &[f32],
    flags: &[u8],
    body: &[u32],
    dir: &mut [u8],
) -> Vec<(usize, u8)> {
    let plain = |j: usize| flags[j] & SEA == 0 && (flags[j] & RIVER != 0 || body[j] == 0);
    let returns = |dir: &[u8], from: usize, to: usize| {
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
        true
    };
    let mut changed = Vec::new();
    for i in 0..ground.len() {
        if flags[i] & (FIXED | SEA | RIVER) != 0 || body[i] != 0 {
            continue;
        }
        if !receiver(dims, dir, i).is_some_and(|r| plain(r) && ground[r] > ground[i]) {
            continue;
        }
        let mut best: Option<(f32, u8)> = None;
        for (k, j) in dims.neighbours(i) {
            let d = k as u8 + 1;
            let slope = (ground[i] - ground[j]) / Dims::step(d) as f32;
            let receives = flags[j] & FIXED == 0 || flags[j] & OUTLET != 0;
            if receives && slope > 0.0 && best.is_none_or(|(s, _)| slope > s) && !returns(dir, j, i)
            {
                best = Some((slope, d));
            }
        }
        if let Some((_, d)) = best {
            dir[i] = d;
            changed.push((i, d));
        }
    }
    changed
}

/// Whether the routing from `from` reaches `to` (within 4,096 cells; longer counts as yes).
pub(super) fn reaches(dims: Dims, dir: &[u8], from: usize, to: usize) -> bool {
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
    true
}

/// Cells on receiver cycles take the receiver `pick` gives (None: leave the cell), until no
/// cycle is left, no cell changes or after 32 rounds. Returns the cells changed.
pub(super) fn restore_on_cycles(
    dims: Dims,
    dir: &mut [u8],
    pick: impl Fn(&[u8], usize) -> Option<u8>,
) -> u64 {
    let mut restored = 0;
    for _ in 0..32 {
        let mut any = false;
        for i in on_cycles(dims, dir) {
            if let Some(d) = pick(dir, i).filter(|&d| d != dir[i]) {
                dir[i] = d;
                restored += 1;
                any = true;
            }
        }
        if !any {
            break;
        }
    }
    restored
}

/// Cells on receiver cycles (a cycle has no exit, so nothing else stays unpeeled).
fn on_cycles(dims: Dims, dir: &[u8]) -> Vec<usize> {
    let n = dir.len();
    let mut donors = vec![0u32; n];
    for i in 0..n {
        if let Some(r) = receiver(dims, dir, i) {
            donors[r] += 1;
        }
    }
    let mut stack: Vec<usize> = (0..n).filter(|&i| donors[i] == 0).collect();
    let mut done = vec![false; n];
    while let Some(c) = stack.pop() {
        done[c] = true;
        if let Some(r) = receiver(dims, dir, c) {
            donors[r] -= 1;
            if donors[r] == 0 {
                stack.push(r);
            }
        }
    }
    (0..n).filter(|&i| !done[i]).collect()
}

/// Sum of `weight` per cell (itself included) along the receivers: drainage area when the
/// weight is the cell area. Returns the number of cells on cycles (0 for a flood tree).
pub(super) fn accumulate(
    dims: Dims,
    dir: &[u8],
    weight: impl Fn(usize) -> f32,
    acc: &mut [f32],
) -> u64 {
    const DONE: u8 = u8::MAX;
    let n = dir.len();
    let mut donors = vec![0u8; n];
    for i in 0..n {
        if let Some(r) = receiver(dims, dir, i) {
            donors[r] += 1;
        }
    }
    for (i, a) in acc.iter_mut().enumerate() {
        *a = weight(i);
    }
    for i in 0..n {
        let mut c = i;
        while donors[c] == 0 {
            donors[c] = DONE;
            let Some(r) = receiver(dims, dir, c) else {
                break;
            };
            acc[r] += acc[c];
            donors[r] -= 1;
            c = r;
        }
    }
    donors.iter().filter(|&&d| d != DONE).count() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breach_drains_a_pit_and_keeps_a_basin_flat() {
        // 5 x 5 ridge with a pit at the centre and the sea along the bottom row.
        let dims = Dims { w: 5, h: 5 };
        #[rustfmt::skip]
        let mut ground = vec![
            9.0, 9.0, 9.0, 9.0, 9.0,
            9.0, 8.0, 8.0, 8.0, 9.0,
            9.0, 8.0, 1.0, 8.0, 9.0,
            9.0, 5.0, 5.0, 5.0, 9.0,
            -1.0, -1.0, -1.0, -1.0, -1.0,
        ];
        let mut flags = vec![0u8; 25];
        let mut dir = vec![0u8; 25];
        edge_outlets(dims, &mut flags);
        open_sea(dims, &ground, |i| dims.is_edge(i), |_| false, &mut flags);
        let filled = fill(dims, &ground, &mut flags, &mut dir);
        assert_eq!(filled[12], 5.0);
        let (mut body, mut basins) = (vec![0u32; 25], Vec::new());
        breach(
            dims,
            &mut ground,
            &mut body,
            &mut basins,
            &mut flags,
            &mut dir,
            30.0,
            &[f32::NEG_INFINITY; 64],
            false,
            |_, _| Kind::Lake,
        );
        assert!(basins.is_empty(), "a 4 m deep pit is breached");
        // Every land cell descends to the sea.
        for i in 0..20 {
            let mut c = i;
            while let Some(r) = receiver(dims, &dir, c) {
                assert!(ground[r] <= ground[c], "cell {c} drains uphill");
                c = r;
            }
            assert!(flags[c] & SEA != 0 || dims.is_edge(c));
        }
        assert_eq!(ground[12], 1.0);
        let mut acc = vec![0f32; 25];
        assert_eq!(accumulate(dims, &dir, |_| 1.0, &mut acc), 0);
        let below = receiver(dims, &dir, 12).expect("the pit drains");
        assert!(acc[below] >= 2.0);
    }

    #[test]
    fn a_deep_pit_is_kept_with_its_outlet_lowered_by_the_cap() {
        let dims = Dims { w: 5, h: 5 };
        #[rustfmt::skip]
        let start = vec![
            9.0, 9.0, 9.0, 9.0, 9.0,
            9.0, 8.0, 8.0, 8.0, 9.0,
            9.0, 8.0, 1.0, 8.0, 9.0,
            9.0, 5.0, 5.0, 5.0, 9.0,
            -1.0, -1.0, -1.0, -1.0, -1.0,
        ];
        for kind in [Kind::Lake, Kind::Sink] {
            let mut ground = start.clone();
            let (mut flags, mut dir) = (vec![0u8; 25], vec![0u8; 25]);
            let (mut body, mut basins) = (vec![0u32; 25], Vec::new());
            edge_outlets(dims, &mut flags);
            open_sea(dims, &ground, |i| dims.is_edge(i), |_| false, &mut flags);
            breach(
                dims,
                &mut ground,
                &mut body,
                &mut basins,
                &mut flags,
                &mut dir,
                2.0,
                &[f32::NEG_INFINITY; 64],
                false,
                |_, deep| if deep { kind } else { Kind::Lake },
            );
            assert_eq!(basins.len(), 1);
            assert_eq!(body[12], 1);
            assert_eq!(ground[12], 1.0);
            let exit = receiver(dims, &dir, 12);
            if kind == Kind::Lake {
                assert_eq!(basins[0].level, 3.0);
                assert_eq!(exit.map(|e| ground[e]), Some(3.0), "outlet cut by the cap");
            } else {
                assert_eq!(exit, None, "flow ends in a sink");
                assert_eq!(ground.iter().filter(|&&g| g == 3.0).count(), 0);
            }
        }
    }

    /// Every cell's receivers reach a cell without one (no cycle).
    fn drains(dims: Dims, dir: &[u8]) -> bool {
        cycle_cells(dims, dir) == 0
    }

    #[test]
    fn steepest_gives_a_pit_its_flood_receiver_back() {
        // A trench end (2, 2) lowered after the flood drains uphill east into (2, 3), whose
        // steepest step is back into the pit: the cycle goes, the flood receiver returns.
        let dims = Dims { w: 5, h: 5 };
        let mut ground = vec![10.0f32; 25];
        ground[12] = 1.0;
        ground[13] = 5.0;
        let mut flags = vec![0u8; 25];
        edge_outlets(dims, &mut flags);
        // North, towards the edge.
        let mut dir: Vec<u8> = (0..25).map(|i| u8::from(!dims.is_edge(i))).collect();
        dir[12] = 4; // east, uphill
        dir[13] = 4;
        let basins: Vec<Basin> = Vec::new();
        steepest(dims, [0, 0], &ground, &flags, &[0; 25], &basins, &mut dir);
        assert!(drains(dims, &dir));
        assert_eq!(receiver(dims, &dir, 12), Some(13));
    }

    #[test]
    fn a_cycle_spills_to_its_lowest_receiving_neighbour() {
        // Cells 6 and 7 point at each other. The lowest neighbour (2, fixed, not an outlet)
        // may not receive; the next lowest (11) does.
        let dims = Dims { w: 4, h: 4 };
        let mut ground = vec![9.0f32; 16];
        (ground[6], ground[7], ground[2], ground[11]) = (5.0, 5.0, 0.0, 3.0);
        let mut flags = vec![0u8; 16];
        flags[2] = FIXED;
        let mut dir = vec![0u8; 16];
        (dir[6], dir[7]) = (4, 3);
        assert_eq!(cycle_cells(dims, &dir), 2);
        assert_eq!(spill_cycles(dims, &ground, &flags, &mut dir, |_| true), 1);
        assert!(drains(dims, &dir));
        assert!([6, 7].iter().any(|&m| receiver(dims, &dir, m) == Some(11)));
    }

    #[test]
    fn routing_follows_the_fall_line_on_a_plane() {
        // A plane falling 20° south of east: paths alternate east and south-east steps
        // instead of running straight east.
        let dims = Dims { w: 60, h: 60 };
        let a = 20f32.to_radians();
        let ground: Vec<f32> = (0..3600)
            .map(|i| {
                let (y, x) = ((i / 60) as f32, (i % 60) as f32);
                100.0 - x * a.cos() - y * a.sin()
            })
            .collect();
        let mut flags = vec![0u8; 3600];
        edge_outlets(dims, &mut flags);
        let mut dir: Vec<u8> = (0..3600)
            .map(|i| if dims.is_edge(i) { 0 } else { 4 })
            .collect();
        let basins: Vec<Basin> = Vec::new();
        steepest(dims, [0, 0], &ground, &flags, &[0; 3600], &basins, &mut dir);
        let (mut c, mut steps) = (10 * 60 + 1, 0);
        while let Some(r) = receiver(dims, &dir, c) {
            (c, steps) = (r, steps + 1);
        }
        let (dy, dx) = ((c / 60) as f32 - 10.0, (c % 60) as f32 - 1.0);
        assert!(steps > 20);
        assert!((dy / dx - a.tan()).abs() < 0.1, "path slope {}", dy / dx);
    }

    #[test]
    fn overlapping_bands_route_shared_rows_alike() {
        // One rough slope; band B starts 20 rows below band A. The rows both hold (away from
        // their grid edges) get the same receivers.
        let height = |x: i64, y: i64| {
            100.0 - x as f32 * 0.9 - y as f32 * 0.4 + ((x * 7 + y * 13) % 5) as f32 * 0.05
        };
        let band = |row0: i64| {
            let dims = Dims { w: 40, h: 40 };
            let ground: Vec<f32> = (0..1600)
                .map(|i| height((i % 40) as i64, row0 + (i / 40) as i64))
                .collect();
            let mut flags = vec![0u8; 1600];
            edge_outlets(dims, &mut flags);
            let mut dir: Vec<u8> = (0..1600)
                .map(|i| if dims.is_edge(i) { 0 } else { 4 })
                .collect();
            let basins: Vec<Basin> = Vec::new();
            steepest(
                dims,
                [0, row0],
                &ground,
                &flags,
                &[0; 1600],
                &basins,
                &mut dir,
            );
            dir
        };
        let (a, b) = (band(0), band(20));
        for y in 21..39 {
            for x in 1..39 {
                assert_eq!(a[y * 40 + x], b[(y - 20) * 40 + x], "cell ({x}, {y})");
            }
        }
    }

    #[test]
    fn a_cycle_every_neighbour_drains_into_spills_the_whole_region() {
        // A 2-cycle (6, 7) that every other cell drains into, but the corner 15 (no
        // receiver): no cycle cell has a usable neighbour, so the whole region spills over 15
        // and the flow from there to the cycle is reversed.
        let dims = Dims { w: 4, h: 4 };
        let ground = vec![9.0f32; 16];
        let flags = vec![0u8; 16];
        let mut dir = vec![0u8; 16];
        for (i, d) in dir.iter_mut().enumerate() {
            let (y, x) = (i / 4, i % 4);
            let target = if x < 2 { 6 } else { 7 };
            let (ty, tx) = (target / 4, target % 4);
            let (dy, dx) = (ty as isize - y as isize, tx as isize - x as isize);
            if (dy, dx) != (0, 0) && dy.abs() <= 1 && dx.abs() <= 1 {
                *d = OFFSETS
                    .iter()
                    .position(|&o| o == (dy, dx))
                    .map_or(0, |k| k as u8 + 1);
            }
        }
        (dir[6], dir[7]) = (4, 3);
        dir[14] = 1; // to 10, which drains into the cycle
        dir[15] = 0;
        let before = cycle_cells(dims, &dir);
        assert_eq!(before, 2);
        assert!(spill_cycles(dims, &ground, &flags, &mut dir, |_| true) > 0);
        assert!(drains(dims, &dir));
    }

    #[test]
    fn a_region_spills_where_its_way_back_is_free() {
        // One row: the 2-cycle (3, 4) with donors 1 -> 2 -> 3 and 5 -> 4. The lowest outlet
        // (0, beside 1) is reached back only through 2, which may not change (a coarse path):
        // the region spills over 6 instead.
        let dims = Dims { w: 7, h: 1 };
        let mut ground = vec![5.0f32; 7];
        (ground[0], ground[6]) = (1.0, 2.0);
        let flags = vec![0u8; 7];
        let (e, w) = (4u8, 3u8);
        let mut dir = vec![0, e, e, e, w, w, 0];
        assert_eq!(cycle_cells(dims, &dir), 2);
        spill_cycles(dims, &ground, &flags, &mut dir, |i| i != 2);
        assert_eq!(cycle_cells(dims, &dir), 0);
        assert_eq!((dir[2], dir[4], dir[5]), (e, e, e));
    }

    #[test]
    fn the_sea_does_not_flood_a_coarse_lake() {
        // Ground below 0 m from the edge (column 0) to column 4; columns 2 and 3 are a coarse
        // lake's cells, so the sea stops at column 1.
        let dims = Dims { w: 5, h: 3 };
        let ground = vec![-1.0f32; 15];
        let mut flags = vec![0u8; 15];
        open_sea(
            dims,
            &ground,
            |i| i % 5 == 0,
            |i| (2..4).contains(&(i % 5)),
            &mut flags,
        );
        let sea: Vec<usize> = (0..15)
            .filter(|&i| flags[i] & SEA != 0)
            .map(|i| i % 5)
            .collect();
        assert!(sea.iter().all(|&x| x < 2), "sea in columns {sea:?}");
    }

    #[test]
    fn breaching_beside_a_lake_never_cuts_below_sea_level() {
        // A lake cell (1, 1) at level 1.5 m, a hollow behind a 2 m rim above the sea row, and
        // a pocket at -0.5 m inside it that the lake does not reach: the pocket is kept, and
        // no breached cell ends up below 0 m.
        let dims = Dims { w: 6, h: 6 };
        #[rustfmt::skip]
        let mut ground = vec![
            3.0, 3.0, 3.0, 3.0, 3.0, 3.0,
            3.0, 0.5, 1.8, 1.8, 1.8, 3.0,
            3.0, 1.8, 1.8, 1.8, 1.8, 3.0,
            3.0, 1.8, 1.8, -0.5, 1.8, 3.0,
            3.0, 3.0, 2.0, 3.0, 3.0, 3.0,
            -1.0, -1.0, -1.0, -1.0, -1.0, -1.0,
        ];
        let mut flags = vec![0u8; 36];
        let mut dir = vec![0u8; 36];
        edge_outlets(dims, &mut flags);
        open_sea(dims, &ground, |i| dims.is_edge(i), |_| false, &mut flags);
        let mut body = vec![0u32; 36];
        body[7] = 1;
        let mut basins = vec![Basin::kept(Kind::Lake, 1.5, 0)];
        basins[0].id = 2;
        breach(
            dims,
            &mut ground,
            &mut body,
            &mut basins,
            &mut flags,
            &mut dir,
            30.0,
            &[f32::NEG_INFINITY; 64],
            true,
            |_, _| Kind::Sink,
        );
        for i in 0..36 {
            if flags[i] & SEA == 0 && body[i] == 0 {
                assert!(ground[i] >= 0.0, "cell {i} at {} m", ground[i]);
            }
        }
        assert_ne!(body[21], 0, "the pocket is kept");
    }
}
