//! Sea, priority flood (Barnes et al. 2014), breaching by priority-flood carving and flow
//! accumulation. Receivers are stored as one D8 direction byte per cell.

use std::{cmp::Ordering, collections::BinaryHeap, collections::VecDeque};

/// Cell flags.
pub(super) const SEA: u8 = 1;
pub(super) const SEEN: u8 = 2;
pub(super) const RIVER: u8 = 4;

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
}

pub(super) fn receiver(dims: Dims, dir: &[u8], i: usize) -> Option<usize> {
    match dir[i] {
        0 => None,
        d => dims.neighbour(i, d as usize - 1),
    }
}

/// The largest f32 below `x` (`f32::next_down` needs Rust 1.86).
pub(super) fn next_down(x: f32) -> f32 {
    if x == 0.0 {
        -f32::from_bits(1)
    } else if x > 0.0 {
        f32::from_bits(x.to_bits() - 1)
    } else {
        f32::from_bits(x.to_bits() + 1)
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
        *f &= !SEEN;
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

/// Priority flood over ground in which kept basins (`basin` > 0) are flat at `levels[basin]`.
/// Every other depression is breached: when the flood reaches ground lower than the path it
/// came along, that path is lowered back towards the outlet until it descends (Soille 2004
/// carving; the cut runs from the pit through the spill to where ground falls away). Basin
/// cells are never lowered. Returns the number of path lowerings.
pub(super) fn breach(
    dims: Dims,
    ground: &mut [f32],
    basin: &[u32],
    levels: &[f32],
    flags: &mut [u8],
    dir: &mut [u8],
) -> u64 {
    let value = |g: &[f32], i: usize| match basin[i] {
        0 => g[i],
        b => levels[b as usize],
    };
    let mut heap = BinaryHeap::new();
    let mut pit = VecDeque::new();
    dir.fill(0);
    seeds(dims, |i| value(ground, i), flags, &mut heap);
    let mut lowered = 0;
    loop {
        let c = match pit.pop_front() {
            Some(c) => c,
            None => match heap.pop() {
                Some(item) => item.i as usize,
                None => break,
            },
        };
        let z = value(ground, c);
        for (k, n) in dims.neighbours(c) {
            if flags[n] & SEEN != 0 {
                continue;
            }
            flags[n] |= SEEN;
            dir[n] = OPPOSITE[k] + 1;
            if basin[n] != 0 {
                // Flat basin: breadth first from where the flood entered it.
                let level = levels[basin[n] as usize];
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
            let mut target = next_down(ground[n]);
            let mut at = Some(c);
            while let Some(a) = at {
                if basin[a] != 0 || ground[a] <= target {
                    break;
                }
                ground[a] = target;
                target = next_down(target);
                lowered += 1;
                at = receiver(dims, dir, a);
            }
            heap.push(Item {
                z: ground[n],
                i: n as u32,
            });
        }
    }
    lowered
}

/// Drainage area in km² per cell (itself included) along the receivers. Returns the number
/// of cells on cycles, which never resolve (0 for a flood tree).
pub(super) fn accumulate(dims: Dims, dir: &[u8], cell_km2: f32, acc: &mut [f32]) -> u64 {
    const DONE: u8 = u8::MAX;
    let n = dir.len();
    let mut donors = vec![0u8; n];
    for i in 0..n {
        if let Some(r) = receiver(dims, dir, i) {
            donors[r] += 1;
        }
    }
    acc.fill(cell_km2);
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
        breach(dims, &mut ground, &[0; 25], &[0.0], &mut flags, &mut dir);
        // Every land cell descends to the sea.
        for i in 0..20 {
            let mut c = i;
            while let Some(r) = receiver(dims, &dir, c) {
                assert!(ground[r] < ground[c], "cell {c} drains uphill");
                c = r;
            }
            assert!(flags[c] & SEA != 0 || dims.is_edge(c));
        }
        assert_eq!(ground[12], 1.0);
        let mut acc = vec![0f32; 25];
        assert_eq!(accumulate(dims, &dir, 1.0, &mut acc), 0);
        let below = receiver(dims, &dir, 12).expect("the pit drains");
        assert!(acc[below] >= 2.0);
    }
}
