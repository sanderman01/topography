//! Sea, priority flood (Barnes et al. 2014), breaching by priority-flood carving and flow
//! accumulation. Receivers are stored as one D8 direction byte per cell.

use std::{cmp::Ordering, collections::BinaryHeap, collections::VecDeque};

use super::lakes::{Basin, Kind};

/// Cell flags.
pub(super) const SEA: u8 = 1;
pub(super) const SEEN: u8 = 2;
pub(super) const RIVER: u8 = 4;
/// Inside a depression already checked by `breach`.
const CHECKED: u8 = 8;
/// A river centreline cell (also `RIVER`); other river cells are channel width beside it.
pub(super) const CENTRE: u8 = 16;

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
    /// Offset of direction byte `d` (row, column).
    pub fn offset(d: u8) -> (isize, isize) {
        OFFSETS[d as usize - 1]
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

/// Mark ground below 0 connected (8-way) to the grid edge as sea, not crossing `blocked`.
pub(super) fn open_sea(
    dims: Dims,
    ground: &[f32],
    blocked: impl Fn(usize) -> bool,
    flags: &mut [u8],
) {
    let mut stack: Vec<u32> = Vec::new();
    for i in 0..ground.len() {
        flags[i] &= !SEA;
        if dims.is_edge(i) && ground[i] < 0.0 && !blocked(i) {
            flags[i] |= SEA;
            stack.push(i as u32);
        }
    }
    while let Some(i) = stack.pop() {
        for (_, j) in dims.neighbours(i as usize) {
            if flags[j] & SEA == 0 && ground[j] < 0.0 && !blocked(j) {
                flags[j] |= SEA;
                stack.push(j as u32);
            }
        }
    }
}

/// Seed the flood: sea cells are seen, those on the coast and non-sea edge cells are queued.
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
        } else if dims.is_edge(i) {
            flags[i] |= SEEN;
            heap.push(Item {
                z: value(i),
                i: i as u32,
            });
        }
    }
}

/// Depression-filled surface (no gradient across flats) and receivers along the flood.
/// Outlets are the sea and the grid edge.
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

/// Lower the path from `from` towards the outlet to at most `target`, stopping at basin cells
/// and at ground already low enough. Returns the cells lowered.
fn lower(
    dims: Dims,
    ground: &mut [f32],
    body: &[u32],
    dir: &[u8],
    from: usize,
    target: f32,
) -> u64 {
    let (mut at, mut lowered) = (Some(from), 0);
    while let Some(a) = at {
        if body[a] != 0 || ground[a] <= target {
            break;
        }
        ground[a] = target;
        lowered += 1;
        at = receiver(dims, dir, a);
    }
    lowered
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
/// A depression deeper than `cap_m` below its spill is kept instead, as `deep_kind(pit)` says:
/// a sink (dry, flow ends there) or lakes at the spill lowered by `cap_m` (not below 0 m), one
/// per piece of ground below that level. A shallower one with ground below 0 m becomes lakes at
/// 0 m (lagoons), so cuts never go below sea level. Returns cells lowered.
#[allow(clippy::too_many_arguments)]
pub(super) fn breach(
    dims: Dims,
    ground: &mut [f32],
    body: &mut [u32],
    basins: &mut Vec<Basin>,
    flags: &mut [u8],
    dir: &mut [u8],
    cap_m: f32,
    deep_kind: impl Fn(usize) -> Kind,
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
                if z - p <= cap_m && p >= 0.0 {
                    lowered += lower(dims, ground, body, dir, c, p);
                } else if z - p > cap_m && deep_kind(low) == Kind::Sink {
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
                    lowered += lower(dims, ground, body, dir, c, level);
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
                lowered += lower(dims, ground, body, dir, c, ground[n]);
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
        open_sea(dims, &ground, |_| false, &mut flags);
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
            |_| Kind::Lake,
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
            open_sea(dims, &ground, |_| false, &mut flags);
            breach(
                dims,
                &mut ground,
                &mut body,
                &mut basins,
                &mut flags,
                &mut dir,
                2.0,
                |_| kind,
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
}
