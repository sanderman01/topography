//! Receivers across a full pond. A pond's cells stand under one level (the routing surface is
//! flat there) and drain to one outlet cell. A priority flood from the outlet over the pond's
//! own ground (lowest first; ties by the Barnes, Lehman and Mulla 2014 mask, 2 · steps from
//! the outlet side + reversed steps from the banks, then by cell index) gives each cell the
//! cell that flooded it as receiver: paths follow the pond's lowest ground, its natural
//! thalweg, instead of BFS geodesics that run straight or diagonal (review I2). The result
//! depends only on the cells, their ground and the outlet, so a pond solved in its tile or
//! gathered across tiles comes out alike.

use super::geom::{OFFSETS, World};
use super::watershed::ordered;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};

/// A direction byte for every pond cell (`cells`: world index and ground, sorted by index;
/// same order out). `outlet` is the cell outside the pond its water leaves by.
pub(crate) fn resolve(world: World, cells: &[(u64, f32)], outlet: u64) -> Vec<(u64, u8)> {
    debug_assert!(
        cells.windows(2).all(|p| p[0].0 < p[1].0),
        "cells not sorted"
    );
    let n = cells.len();
    let index = |c: u64| cells.binary_search_by_key(&c, |p| p.0).ok();
    debug_assert!(index(outlet).is_none(), "the outlet is a pond cell");
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
    // Direction byte from cell i to its neighbour `to`.
    let towards = |i: usize, to: u64| {
        let ((x, y), (tx, ty)) = (world.xy(cells[i].0), world.xy(to));
        let off = (ty as i64 - y as i64, tx as i64 - x as i64);
        OFFSETS
            .iter()
            .position(|&o| o == off)
            .map_or(0, |k| k as u8 + 1)
    };
    let key = |i: usize| Reverse((ordered(cells[i].1), mask[i], cells[i].0, i));
    let mut dir = vec![0u8; n];
    let mut heap = BinaryHeap::new();
    for (i, d) in dir.iter_mut().enumerate() {
        if around(i).any(|c| c == outlet) {
            *d = towards(i, outlet);
            heap.push(key(i));
        }
    }
    while let Some(Reverse((_, _, _, i))) = heap.pop() {
        for c in around(i) {
            if let Some(j) = index(c) {
                if dir[j] == 0 {
                    dir[j] = towards(j, cells[i].0);
                    heap.push(key(j));
                }
            }
        }
    }
    cells.iter().zip(dir).map(|(&(c, _), d)| (c, d)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::water::geom::OFFSETS;
    use std::collections::HashMap;

    fn follow(world: World, dirs: &HashMap<u64, u8>, mut c: u64, outlet: u64) -> Vec<u64> {
        let mut path = vec![c];
        while c != outlet {
            let d = dirs[&c];
            assert_ne!(d, 0, "cell {c} has no receiver");
            let (dy, dx) = OFFSETS[d as usize - 1];
            let (x, y) = world.xy(c);
            c = world.index((x as i64 + dx) as u64, (y as i64 + dy) as u64);
            path.push(c);
            assert!(path.len() <= 400, "a loop");
        }
        path
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
        let dirs: HashMap<u64, u8> = resolve(world, &cells, outlet).into_iter().collect();
        assert_eq!(dirs.len(), 21);
        for &(start, _) in &cells {
            let (mut c, mut steps) = (start, 0);
            while c != outlet {
                let d = dirs[&c];
                assert_ne!(d, 0, "cell {c} has no receiver");
                let (dy, dx) = OFFSETS[d as usize - 1];
                let (x, y) = world.xy(c);
                c = world.index((x as i64 + dx) as u64, (y as i64 + dy) as u64);
                steps += 1;
                assert!(steps <= 21, "a loop from {start}");
            }
        }
        // From the far end of the low middle row the path keeps to it.
        for c in follow(world, &dirs, 21, outlet) {
            assert!(c == outlet || c / 10 == 2, "left the low row at {c}");
        }
    }

    #[test]
    fn a_path_follows_a_bent_low_line_not_a_straight_one() {
        // A 12 x 12 pond with a low L-shaped line: down column 2, then along row 10 to the
        // outlet at (12, 10). A BFS geodesic would cut the corner diagonally.
        let world = World { w: 14, h: 14 };
        let low = |x: u64, y: u64| (x == 2 && y <= 10) || (y == 10 && x >= 2);
        let cells: Vec<(u64, f32)> = (1..13)
            .flat_map(|y| (1..12).map(move |x| (y * 14 + x, if low(x, y) { 4.0 } else { 5.0 })))
            .collect();
        let outlet = 10 * 14 + 12;
        let dirs: HashMap<u64, u8> = resolve(world, &cells, outlet).into_iter().collect();
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
        let dirs: HashMap<u64, u8> = resolve(world, &cells, outlet).into_iter().collect();
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
}
