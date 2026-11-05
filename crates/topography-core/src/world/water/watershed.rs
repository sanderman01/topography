//! Pass A: in each tile, the pits (equal neighbouring pit cells are one pit), where every
//! cell's flow ends inside the tile (the world edge, a pit, or the first cell outside), the r
//! supply by end, and the lowest spill edge between each pair of ends: Barnes 2016's label
//! graph, which the hierarchy (`hierarchy.rs`) joins across tiles.

use super::flow::{Next, ends};
use super::geom::{Rect, UnionFind, World};
use super::receivers::receivers;
use super::store::{Blob, Dec, Enc};
use crate::acquisition::Result;
use std::collections::{BTreeMap, HashMap};

/// Node key of the world edge: the ocean of the hierarchy.
pub(crate) const EDGE_KEY: u64 = u64::MAX;

/// An f32 as an unsigned key in total order.
pub(crate) fn ordered(z: f32) -> u32 {
    let b = z.to_bits();
    if b & 0x8000_0000 != 0 {
        !b
    } else {
        b | 0x8000_0000
    }
}

/// A spill edge: cell `ca` drains to end `a`, its neighbour `cb` to `b`; the water crosses at
/// the higher of the two.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Edge {
    pub a: Next,
    pub b: Next,
    pub saddle: f32,
    pub ca: u64,
    pub cb: u64,
}

impl Edge {
    /// Orders edges alike in every tiling: lowest saddle, then the cell pair.
    pub fn rank(&self) -> (u32, u64, u64) {
        (
            ordered(self.saddle),
            self.ca.min(self.cb),
            self.ca.max(self.cb),
        )
    }
}

/// A tile's drainage on the ground.
pub(crate) struct Drain {
    pub dir: Vec<u8>,
    /// Pit cells (local index) and their pit key.
    pub pit: HashMap<u32, u64>,
    pub end: Vec<Next>,
    pub cycles: u64,
}

pub(crate) fn drain(world: World, win: Rect, z: &[f32], core: Rect) -> Drain {
    let dir = receivers(world, win, z, core);
    let n = core.cells();
    assert!(
        n <= u32::MAX as usize,
        "a tile holds at most u32::MAX cells"
    );
    let zc = |k: usize| {
        let (x, y) = core.world_xy(k);
        z[win.local(x, y).expect("core inside window")]
    };
    let is_pit = |k: usize| {
        let (x, y) = core.world_xy(k);
        dir[k] == 0 && !world.is_edge(x, y)
    };
    let pits: Vec<u32> = (0..n as u32).filter(|&k| is_pit(k as usize)).collect();
    let slot: HashMap<u32, u32> = pits
        .iter()
        .enumerate()
        .map(|(i, &k)| (k, i as u32))
        .collect();
    let mut uf = UnionFind::new(pits.len());
    for (i, &k) in pits.iter().enumerate() {
        let (x, y) = core.world_xy(k as usize);
        for j in 0..8 {
            let Some((nx, ny)) = world.neighbour(x, y, j) else {
                continue;
            };
            let Some(m) = core.local(nx, ny) else {
                continue;
            };
            if let Some(&s) = slot.get(&(m as u32))
                && zc(m) == zc(k as usize)
            {
                uf.union(i as u32, s);
            }
        }
    }
    let mut name: HashMap<u32, u64> = HashMap::new();
    for (i, &k) in pits.iter().enumerate() {
        let (x, y) = core.world_xy(k as usize);
        let e = name.entry(uf.find(i as u32)).or_insert(u64::MAX);
        *e = (*e).min(world.index(x, y));
    }
    let pit: HashMap<u32, u64> = pits
        .iter()
        .enumerate()
        .map(|(i, &k)| (k, name[&uf.find(i as u32)]))
        .collect();
    let next = |k: usize| {
        let (x, y) = core.world_xy(k);
        if world.is_edge(x, y) {
            return Next::Node(EDGE_KEY);
        }
        match dir[k] {
            0 => Next::Node(pit[&(k as u32)]),
            d => {
                let (nx, ny) = world
                    .neighbour(x, y, d as usize - 1)
                    .expect("receiver inside");
                match core.local(nx, ny) {
                    Some(j) => Next::Cell(j as u32),
                    None => Next::Out(world.index(nx, ny)),
                }
            }
        }
    };
    let (end, cycles) = ends(n, next);
    Drain {
        dir,
        pit,
        end,
        cycles,
    }
}

/// A cell of the tile's outer ring, for joins across seams.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Ring {
    pub cell: u64,
    pub z: f32,
    pub end: Next,
    pub pit: bool,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct TileGraph {
    pub ring: Vec<Ring>,
    /// Pit keys and their ground.
    pub pits: Vec<(u64, f32)>,
    /// r supply (thousandths of a cell) by end.
    pub supply: BTreeMap<Next, u64>,
    /// The lowest edge per pair of ends inside the tile.
    pub edges: Vec<Edge>,
    pub cycles: u64,
}

/// Pass A's result for one tile; `zc` holds the core's ground, `supply` each core cell's r.
pub(crate) fn tile_graph(
    world: World,
    core: Rect,
    zc: &[f32],
    d: &Drain,
    supply: impl Fn(usize) -> u64,
) -> TileGraph {
    let n = core.cells();
    let mut sup = BTreeMap::new();
    for k in 0..n {
        *sup.entry(d.end[k]).or_insert(0) += supply(k);
    }
    let mut best: HashMap<(Next, Next), Edge> = HashMap::new();
    for k in 0..n {
        let (x, y) = core.world_xy(k);
        // Forward neighbours (S, E, SW, SE) see each pair once.
        for j in [1usize, 3, 6, 7] {
            let Some((nx, ny)) = world.neighbour(x, y, j) else {
                continue;
            };
            let Some(m) = core.local(nx, ny) else {
                continue;
            };
            let (ea, eb) = (d.end[k], d.end[m]);
            if ea == eb {
                continue;
            }
            let (ck, cm, saddle) = (world.index(x, y), world.index(nx, ny), zc[k].max(zc[m]));
            let e = if ea < eb {
                Edge {
                    a: ea,
                    b: eb,
                    saddle,
                    ca: ck,
                    cb: cm,
                }
            } else {
                Edge {
                    a: eb,
                    b: ea,
                    saddle,
                    ca: cm,
                    cb: ck,
                }
            };
            best.entry((e.a, e.b))
                .and_modify(|o| {
                    if e.rank() < o.rank() {
                        *o = e
                    }
                })
                .or_insert(e);
        }
    }
    let mut edges: Vec<Edge> = best.into_values().collect();
    edges.sort_by_key(|e| (e.a, e.b));
    let ring = (0..n)
        .filter_map(|k| {
            let (x, y) = core.world_xy(k);
            core.on_ring(x, y).then(|| Ring {
                cell: world.index(x, y),
                z: zc[k],
                end: d.end[k],
                pit: d.pit.contains_key(&(k as u32)),
            })
        })
        .collect();
    let mut pits: BTreeMap<u64, f32> = BTreeMap::new();
    for (&k, &key) in &d.pit {
        pits.insert(key, zc[k as usize]);
    }
    TileGraph {
        ring,
        pits: pits.into_iter().collect(),
        supply: sup,
        edges,
        cycles: d.cycles,
    }
}

impl Blob for Ring {
    fn put(&self, e: &mut Enc) {
        (self.cell, self.z, self.end, self.pit).put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let (cell, z, end, pit) = Blob::get(d)?;
        Ok(Self { cell, z, end, pit })
    }
}

impl Blob for Edge {
    fn put(&self, e: &mut Enc) {
        (self.a, self.b, self.saddle, self.ca, self.cb).put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let (a, b, saddle, ca, cb) = Blob::get(d)?;
        Ok(Self {
            a,
            b,
            saddle,
            ca,
            cb,
        })
    }
}

impl Blob for TileGraph {
    fn put(&self, e: &mut Enc) {
        self.ring.put(e);
        self.pits.put(e);
        self.supply.put(e);
        self.edges.put(e);
        self.cycles.put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        Ok(Self {
            ring: Blob::get(d)?,
            pits: Blob::get(d)?,
            supply: Blob::get(d)?,
            edges: Blob::get(d)?,
            cycles: Blob::get(d)?,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 7 x 5: an edge of 9 with an outlet of 4 at (6, 2); inside, pit A (1 at x 1) and pit B
    /// (2 at x 3) separated by a 5, B draining east over a 7 to the outlet.
    pub(crate) fn two_pits() -> (World, Vec<f32>) {
        let rows = [
            [9., 9., 9., 9., 9., 9., 9.],
            [9., 8., 8., 8., 8., 8., 9.],
            [9., 1., 5., 2., 6., 7., 4.],
            [9., 8., 8., 8., 8., 8., 9.],
            [9., 9., 9., 9., 9., 9., 9.],
        ];
        (
            World { w: 7, h: 5 },
            rows.iter().flatten().copied().collect(),
        )
    }

    #[test]
    fn cells_end_at_pits_or_the_edge() {
        let (world, z) = two_pits();
        let all = Rect::whole(world);
        let d = drain(world, all, &z, all);
        assert_eq!(d.cycles, 0);
        assert_eq!(d.end[2 * 7 + 1], Next::Node(15), "pit A is its own key");
        assert_eq!(d.end[2 * 7 + 3], Next::Node(17));
        assert_eq!(d.end[2 * 7 + 4], Next::Node(17), "the 6 drains to B");
        assert_eq!(d.end[0], Next::Node(EDGE_KEY));
    }

    #[test]
    fn the_lowest_edge_per_pair_is_kept() {
        let (world, z) = two_pits();
        let all = Rect::whole(world);
        let d = drain(world, all, &z, all);
        let g = tile_graph(world, all, &z, &d, |_| 1000);
        let between = |a: u64, b: u64| {
            g.edges.iter().find(|e| {
                (e.a, e.b) == (Next::Node(a), Next::Node(b))
                    || (e.a, e.b) == (Next::Node(b), Next::Node(a))
            })
        };
        assert_eq!(between(15, 17).expect("A-B").saddle, 5.0);
        assert_eq!(between(17, EDGE_KEY).expect("B-edge").saddle, 7.0);
        assert_eq!(g.supply.values().sum::<u64>(), 35 * 1000);
        assert_eq!(g.pits.len(), 2);
    }

    #[test]
    fn equal_neighbouring_pits_are_one_pit_and_exits_name_the_outside_cell() {
        let world = World { w: 6, h: 4 };
        let mut z = vec![5.0f32; 24];
        (z[8], z[9]) = (1.0, 1.0); // (2, 1) and (3, 1): one flat pit
        let core = Rect {
            x0: 0,
            y0: 0,
            w: 3,
            h: 4,
        };
        let win = core.grow(1, world);
        let zw: Vec<f32> = (0..win.cells())
            .map(|k| {
                let (x, y) = win.world_xy(k);
                z[(y * 6 + x) as usize]
            })
            .collect();
        let d = drain(world, win, &zw, core);
        assert_eq!(
            d.pit.values().copied().collect::<Vec<_>>(),
            vec![8],
            "only (2, 1) is in this tile"
        );
        let left = drain(world, Rect::whole(world), &z, Rect::whole(world));
        assert_eq!(left.end[9], Next::Node(8), "(3, 1) joins the pit named 8");
    }

    #[test]
    fn tile_graphs_round_trip() {
        use super::super::store::{decode, encode};
        let (world, z) = two_pits();
        let all = Rect::whole(world);
        let d = drain(world, all, &z, all);
        let g = tile_graph(world, all, &z, &d, |k| k as u64);
        assert!(!g.ring.is_empty() && !g.edges.is_empty());
        assert_eq!(decode::<TileGraph>(&encode(&g)).expect("decode"), g);
    }

    #[test]
    #[ignore]
    fn probe_pits_in_the_canyon() {
        use crate::world::water::probe;
        let Some((win, z)) = probe::window(27169, 44546, 2048) else {
            return;
        };
        let core = Rect {
            x0: win.x0 + 1,
            y0: win.y0 + 1,
            w: win.w - 2,
            h: win.h - 2,
        };
        let d = drain(probe::WORLD, win, &z, core);
        let zc: Vec<f32> = (0..core.cells())
            .map(|k| {
                let (x, y) = core.world_xy(k);
                z[win.local(x, y).expect("in")]
            })
            .collect();
        let g = tile_graph(probe::WORLD, core, &zc, &d, |_| 1000);
        eprintln!(
            "pits {} (one per {} cells), edges {}, cycles {}",
            g.pits.len(),
            core.cells() / g.pits.len().max(1),
            g.edges.len(),
            g.cycles
        );
        // Pits whose lowest in-tile spill is at their own floor (flats, no depth).
        let mut spill: HashMap<Next, f32> = HashMap::new();
        for e in &g.edges {
            for n in [e.a, e.b] {
                let s = spill.entry(n).or_insert(f32::INFINITY);
                *s = s.min(e.saddle);
            }
        }
        let flat = g
            .pits
            .iter()
            .filter(|&&(k, z)| spill.get(&Next::Node(k)).is_some_and(|&s| s <= z))
            .count();
        let km2 = core.cells() as f64 / 1e6;
        eprintln!(
            "core {:.3} km², pits/km² {:.0}, zero-depth pits {} ({:.0}/km²)",
            km2,
            g.pits.len() as f64 / km2,
            flat,
            flat as f64 / km2
        );
    }
}
