//! Receivers across a full pond. A pond's cells stand under one level (the routing surface is
//! flat there) and drain to one outlet cell. A priority flood from the outlet over the pond's
//! own ground sets the order: lowest first; ties by the Barnes, Lehman and Mulla 2014 mask
//! (2 · steps from the outlet side + reversed steps from the banks), then by a smooth noise
//! field of the world position, then by cell index. Each cell drains to a neighbour popped
//! before it (or the outlet), picked on the bed's fall line by the dithered Horn rule of dry
//! land (`receivers::fall_pick`). Where the bed is exactly flat the fall line is the mask's,
//! swung by a second noise field, so paths meander instead of running straight or diagonal
//! (review I2). Pop order falls strictly along every path, so there are no loops. The result
//! depends only on the cells, their ground and the outlet, so a pond solved in its tile or
//! gathered across tiles comes out alike.

use super::geom::{OFFSETS, World};
use super::receivers::{fall_pick, fall_turns};
use super::watershed::ordered;
use crate::acquisition::{Result, ensure};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};

/// Noise wavelengths in cells (judgment values: bends every 10–30 cells).
const NOISE_WAVES: [f64; 2] = [24.0, 11.0];
/// On an exactly flat bed the mask's fall line swings by `SWAY · sin(2π · CYCLES · (n − ½))`
/// eighths of a turn, n a second noise field (judgment values: up to ±45°; value noise stays
/// near ½, so several cycles keep the swing from resting at one angle).
const SWAY: f64 = 1.0;
const CYCLES: f64 = 3.0;

/// Smooth value noise in [0, 1) keyed on the world position and a seed: hashed lattice
/// octaves.
fn noise(x: u64, y: u64, seed: u64) -> f64 {
    let hash = |ix: i64, iy: i64, o: u64| {
        // splitmix64 of the lattice point and octave.
        let mut h = (ix as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add((iy as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F))
            .wrapping_add(o.wrapping_mul(0x1656_67B1_9E37_79F9));
        h = (h ^ (h >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h = (h ^ (h >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        h ^= h >> 31;
        (h >> 11) as f64 / (1u64 << 53) as f64
    };
    let smooth = |t: f64| t * t * (3.0 - 2.0 * t);
    let (mut sum, mut weight) = (0.0, 0.0);
    for (o, &wave) in NOISE_WAVES.iter().enumerate() {
        let (fx, fy) = (x as f64 / wave, y as f64 / wave);
        let (ix, iy) = (fx.floor() as i64, fy.floor() as i64);
        let (tx, ty) = (smooth(fx - ix as f64), smooth(fy - iy as f64));
        let o = 2 * seed + o as u64;
        let top = hash(ix, iy, o) + tx * (hash(ix + 1, iy, o) - hash(ix, iy, o));
        let bottom = hash(ix, iy + 1, o) + tx * (hash(ix + 1, iy + 1, o) - hash(ix, iy + 1, o));
        let w = wave / NOISE_WAVES[0];
        sum += w * (top + ty * (bottom - top));
        weight += w;
    }
    (sum / weight).min(1.0 - f64::EPSILON)
}

/// A direction byte for every pond cell (`cells`: world index and ground, sorted by index;
/// same order out). `outlet` is the cell outside the pond its water leaves by. A cell the
/// flood does not reach is an error.
pub(crate) fn resolve(world: World, cells: &[(u64, f32)], outlet: u64) -> Result<Vec<(u64, u8)>> {
    ensure(
        cells.windows(2).all(|p| p[0].0 < p[1].0),
        "pond cells are not sorted",
    )?;
    let n = cells.len();
    let index = |c: u64| cells.binary_search_by_key(&c, |p| p.0).ok();
    ensure(
        index(outlet).is_none(),
        "a pond's outlet is one of its cells",
    )?;
    let around = |i: usize| {
        let (x, y) = world.xy(cells[i].0);
        (0..8).filter_map(move |k| world.neighbour(x, y, k).map(|(nx, ny)| world.index(nx, ny)))
    };
    let bfs = |seed: &dyn Fn(usize) -> bool| {
        let mut dist = vec![0u32; n];
        let mut queue = VecDeque::new();
        for (i, d) in dist.iter_mut().enumerate() {
            if seed(i) {
                *d = 1;
                queue.push_back(i);
            }
        }
        while let Some(i) = queue.pop_front() {
            for c in around(i) {
                if let Some(j) = index(c) {
                    if dist[j] == 0 {
                        dist[j] = dist[i] + 1;
                        queue.push_back(j);
                    }
                }
            }
        }
        dist
    };
    let toward = bfs(&|i| around(i).any(|c| c == outlet));
    let away = bfs(&|i| around(i).any(|c| c != outlet && index(c).is_none()));
    let high = away.iter().copied().max().unwrap_or(0);
    let mask: Vec<u32> = (0..n)
        .map(|i| 2 * toward[i] + if away[i] > 0 { high + 1 - away[i] } else { 0 })
        .collect();
    let tie: Vec<f64> = cells
        .iter()
        .map(|&(c, _)| {
            let (x, y) = world.xy(c);
            noise(x, y, 0)
        })
        .collect();

    // The flood's pop order (u64::MAX: not reached).
    let key = |i: usize| {
        let q = (tie[i] * f64::from(u32::MAX)) as u32;
        Reverse((ordered(cells[i].1), mask[i], q, cells[i].0, i))
    };
    let mut rank = vec![u64::MAX; n];
    let mut seen = vec![false; n];
    let mut heap = BinaryHeap::new();
    for (i, s) in seen.iter_mut().enumerate() {
        if around(i).any(|c| c == outlet) {
            *s = true;
            heap.push(key(i));
        }
    }
    let mut next = 0u64;
    while let Some(Reverse((.., i))) = heap.pop() {
        rank[i] = next;
        next += 1;
        for c in around(i) {
            if let Some(j) = index(c) {
                if !seen[j] {
                    seen[j] = true;
                    heap.push(key(j));
                }
            }
        }
    }
    ensure(
        rank.iter().all(|&r| r != u64::MAX),
        "a pond cell is not reached from its outlet",
    )?;

    // Receivers: the dithered fall line of the bed among the neighbours popped earlier;
    // on an exactly flat bed, that of the mask swung by noise.
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (x, y) = world.xy(cells[i].0);
        let at = |dy: i64, dx: i64| {
            let (nx, ny) = (x.checked_add_signed(dx)?, y.checked_add_signed(dy)?);
            (nx < world.w && ny < world.h).then(|| world.index(nx, ny))
        };
        let bed = |dy, dx| match at(dy, dx).and_then(index) {
            Some(j) => f64::from(cells[j].1),
            None => f64::from(cells[i].1),
        };
        let flat = |dy, dx| match at(dy, dx) {
            Some(c) if c == outlet => 0.0,
            Some(c) => f64::from(mask[index(c).unwrap_or(i)]),
            None => f64::from(mask[i]),
        };
        let earlier = |k: usize| {
            let (dy, dx) = OFFSETS[k];
            at(dy, dx).is_some_and(|c| c == outlet || index(c).is_some_and(|j| rank[j] < rank[i]))
        };
        let turns = fall_turns(bed)
            .or_else(|| {
                let n = noise(x, y, 1) - 0.5;
                let sway = SWAY * (std::f64::consts::TAU * CYCLES * n).sin();
                Some(fall_turns(flat)? + sway)
            })
            .map_or(0.0, |t| t.rem_euclid(8.0));
        let k = fall_pick(x, y, turns, earlier);
        ensure(k.is_some(), "a pond cell has no earlier neighbour")?;
        out.push((cells[i].0, k.map_or(0, |k| k as u8 + 1)));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    fn solve(world: World, cells: &[(u64, f32)], outlet: u64) -> HashMap<u64, u8> {
        resolve(world, cells, outlet)
            .expect("resolve")
            .into_iter()
            .collect()
    }

    fn follow(world: World, dirs: &HashMap<u64, u8>, mut c: u64, outlet: u64) -> Vec<u64> {
        let mut path = vec![c];
        while c != outlet {
            let d = dirs[&c];
            assert_ne!(d, 0, "cell {c} has no receiver");
            let (dy, dx) = OFFSETS[d as usize - 1];
            let (x, y) = world.xy(c);
            c = world.index((x as i64 + dx) as u64, (y as i64 + dy) as u64);
            path.push(c);
            assert!(path.len() <= 10_000, "a loop");
        }
        path
    }

    /// The longest run of same-direction steps from cells at least 3 cells inside the pond
    /// (water may follow a straight bank).
    fn longest_inner_run(world: World, cells: &[(u64, f32)], dirs: &HashMap<u64, u8>) -> usize {
        let pond: HashSet<u64> = cells.iter().map(|c| c.0).collect();
        let inner = |c: u64| {
            let (x, y) = world.xy(c);
            (-3i64..=3).all(|dy| {
                (-3i64..=3).all(|dx| {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    nx >= 0 && ny >= 0 && pond.contains(&world.index(nx as u64, ny as u64))
                })
            })
        };
        let mut longest = 0;
        for &(start, _) in cells {
            let (mut c, mut run) = (start, 0);
            while inner(c) {
                let d = dirs[&c];
                if run > 0 && d != dirs[&start] {
                    break;
                }
                let (dy, dx) = OFFSETS[d as usize - 1];
                let (x, y) = world.xy(c);
                c = world.index((x as i64 + dx) as u64, (y as i64 + dy) as u64);
                run += 1;
            }
            longest = longest.max(run);
        }
        longest
    }

    #[test]
    fn a_pond_drains_along_its_lowest_ground_to_the_outlet() {
        // A 7 x 3 pond (x 1..8, y 1..4) in a 10 x 5 world, outlet (8, 2); the middle row is
        // 10 cm lower than the rows beside it.
        let world = World { w: 10, h: 5 };
        let cells: Vec<(u64, f32)> = (1..4)
            .flat_map(|y| (1..8).map(move |x| (y * 10 + x, if y == 2 { 9.9 } else { 10.0 })))
            .collect();
        let outlet = 28;
        let dirs = solve(world, &cells, outlet);
        assert_eq!(dirs.len(), 21);
        for &(start, _) in &cells {
            assert!(
                follow(world, &dirs, start, outlet).len() <= 22,
                "a loop from {start}"
            );
        }
        // From the far end of the low middle row the path keeps to it.
        for c in follow(world, &dirs, 21, outlet) {
            assert!(c == outlet || c / 10 == 2, "left the low row at {c}");
        }
    }

    #[test]
    fn a_path_follows_a_bent_low_line_not_a_straight_one() {
        // An 11 x 12 pond (x 1..12, y 1..13) with a low L-shaped line: down column 2, then
        // along row 10 to the outlet at (12, 10). A BFS geodesic would cut the corner.
        let world = World { w: 14, h: 14 };
        let low = |x: u64, y: u64| (x == 2 && y <= 10) || (y == 10 && x >= 2);
        let cells: Vec<(u64, f32)> = (1..13)
            .flat_map(|y| (1..12).map(move |x| (y * 14 + x, if low(x, y) { 4.0 } else { 5.0 })))
            .collect();
        let outlet = 10 * 14 + 12;
        let dirs = solve(world, &cells, outlet);
        let path = follow(world, &dirs, 14 + 2, outlet);
        for c in &path[..path.len() - 1] {
            assert!(low(c % 14, c / 14), "left the low line at {c}");
        }
    }

    #[test]
    fn every_path_keeps_to_a_meandering_low_line() {
        // A 13 x 13 pond (x, y 1..14) with an S-shaped low line from (1, 12) to the outlet at
        // (14, 2): east along row 12, north up column 12, west along row 7, north up column 3,
        // east along row 2. The banks slope down toward the outlet and the line's own ground
        // rises eastward, so neither a straight, a diagonal nor a steepest-ground path keeps to it.
        let world = World { w: 16, h: 16 };
        let low = |x: u64, y: u64| {
            (y == 12 && x <= 12)
                || (x == 12 && (7..=12).contains(&y))
                || (y == 7 && (3..=12).contains(&x))
                || (x == 3 && (2..=7).contains(&y))
                || (y == 2 && x >= 3)
        };
        let (ox, oy) = (14u64, 2u64);
        let ground = |x: u64, y: u64| {
            if low(x, y) {
                4.0 + 0.01 * x as f32
            } else {
                5.0 + 0.01 * x.abs_diff(ox).max(y.abs_diff(oy)) as f32
            }
        };
        let cells: Vec<(u64, f32)> = (1..14)
            .flat_map(|y| (1..14).map(move |x| (y * 16 + x, ground(x, y))))
            .collect();
        let outlet = world.index(ox, oy);
        let dirs = solve(world, &cells, outlet);
        assert_eq!(dirs.len(), cells.len());
        let line: Vec<u64> = cells
            .iter()
            .map(|c| c.0)
            .filter(|&c| low(c % 16, c / 16))
            .collect();
        for &start in &line {
            let path = follow(world, &dirs, start, outlet);
            for c in &path[..path.len() - 1] {
                assert!(
                    low(c % 16, c / 16),
                    "the path from {start} left the low line at {c}"
                );
            }
        }
        // The far end walks the whole line: about 12 + 5 + 9 + 5 + 11 steps.
        assert!(follow(world, &dirs, world.index(1, 12), outlet).len() > 35);
        // The banks hold no pits, so every path only descends their ground until it meets the line.
        let z: HashMap<u64, f32> = cells.iter().copied().collect();
        for &(start, _) in &cells {
            let path = follow(world, &dirs, start, outlet);
            for p in path.windows(2) {
                if p[1] == outlet || low(p[0] % 16, p[0] / 16) {
                    break;
                }
                assert!(z[&p[1]] <= z[&p[0]], "{start} climbs its bank: {path:?}");
            }
        }
    }

    #[test]
    fn paths_on_a_tilted_bed_follow_its_fall_line() {
        // A 60 x 60 pond whose bed falls 1:3 (east 3, south 1), outlet past its low corner.
        let world = World { w: 64, h: 64 };
        let cells: Vec<(u64, f32)> = (1..61)
            .flat_map(|y| {
                (1..61).map(move |x| (y * 64 + x, 50.0 - 0.03 * x as f32 - 0.01 * y as f32))
            })
            .collect();
        let outlet = world.index(61, 61);
        let dirs = solve(world, &cells, outlet);
        let run = longest_inner_run(world, &cells, &dirs);
        assert!(run <= 15, "a straight run of {run} cells");
        // Paths keep the fall line's slope (1/3) away from the banks.
        let path = follow(world, &dirs, world.index(4, 10), outlet);
        let (x, y) = world.xy(path[30]);
        let slope = (y as f64 - 10.0) / (x as f64 - 4.0);
        assert!((slope - 1.0 / 3.0).abs() < 0.15, "path slope {slope}");
        // Shuffled, then sorted, input gives the same receivers.
        let mut shuffled = cells.clone();
        shuffled.sort_by_key(|c| c.0.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        shuffled.sort_by_key(|c| c.0);
        assert_eq!(
            resolve(world, &shuffled, outlet).expect("resolve"),
            resolve(world, &cells, outlet).expect("resolve")
        );
    }

    #[test]
    fn paths_on_an_exactly_flat_pond_meander() {
        // A 60 x 60 pond of one ground height, at two world positions, with its outlet on the
        // east, north, west or south side.
        let world = World { w: 300, h: 300 };
        for (x0, y0) in [(1, 1), (137, 211)] {
            let cells: Vec<(u64, f32)> = (y0..y0 + 60)
                .flat_map(|y| (x0..x0 + 60).map(move |x| (world.index(x, y), 7.0)))
                .collect();
            for (ox, oy) in [
                (x0 + 60, y0 + 29),
                (x0 + 13, y0 - 1),
                (x0 - 1, y0 + 44),
                (x0 + 30, y0 + 60),
            ] {
                let outlet = world.index(ox, oy);
                let dirs = solve(world, &cells, outlet);
                for &(start, _) in &cells {
                    follow(world, &dirs, start, outlet);
                }
                // 20, not 15: the R2 dither picks the rarer direction in long gaps when the
                // line is within a few degrees of an axis or diagonal (see the task report).
                let run = longest_inner_run(world, &cells, &dirs);
                assert!(
                    run <= 20,
                    "a straight run of {run} cells, outlet ({ox}, {oy})"
                );
            }
        }
    }

    #[test]
    fn every_path_keeps_to_a_low_arc() {
        // A 30 x 30 pond with a low quarter circle (radius 29 about (1, 1)) from (30, 1) to
        // (1, 30); the outlet (0, 30) is just past its end.
        let world = World { w: 34, h: 34 };
        let mut arc = HashSet::new();
        for k in 0..=2000 {
            let a = f64::from(k) / 2000.0 * std::f64::consts::FRAC_PI_2;
            arc.insert(world.index(
                (1.0 + 29.0 * a.cos()).round() as u64,
                (1.0 + 29.0 * a.sin()).round() as u64,
            ));
        }
        let cells: Vec<(u64, f32)> = (1..31)
            .flat_map(|y| (1..31).map(move |x| y * 34 + x))
            .map(|c| (c, if arc.contains(&c) { 4.0 } else { 5.0 }))
            .collect();
        let outlet = world.index(0, 30);
        let dirs = solve(world, &cells, outlet);
        for &start in &arc {
            let path = follow(world, &dirs, start, outlet);
            for c in &path[..path.len() - 1] {
                assert!(arc.contains(c), "the path from {start} left the arc at {c}");
            }
        }
        assert!(follow(world, &dirs, world.index(30, 1), outlet).len() > 40);
    }

    #[test]
    fn a_cell_the_outlet_cannot_reach_is_an_error() {
        let world = World { w: 10, h: 10 };
        let cells = vec![(11, 1.0), (12, 1.0), (77, 1.0)];
        assert!(resolve(world, &cells, 13).is_err());
    }
}
