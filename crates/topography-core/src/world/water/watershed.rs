//! Pass A: in each tile, the pits (equal neighbouring pit cells are one pit), where every
//! cell's flow ends inside the tile (the world edge, a pit, or the first cell outside), the r
//! supply by end, and the lowest spill edge between each pair of ends: Barnes 2016's label
//! graph, which the hierarchy (`hierarchy.rs`) joins across tiles.

use super::flow::{Next, ends};
use super::geom::{Rect, UnionFind, World};
use super::store::{Blob, Dec, Enc};
use crate::acquisition::Result;
use std::collections::{BTreeMap, HashMap};

/// Node key of the world edge: the ocean of the hierarchy.
pub(crate) const EDGE_KEY: u64 = u64::MAX;

/// `z` with -0.0 as +0.0, so equal heights encode alike in every tiling.
fn norm(z: f32) -> f32 {
    z + 0.0
}

/// A finite f32 as an unsigned key in total order (-0.0 and +0.0 alike).
pub(crate) fn ordered(z: f32) -> u32 {
    assert!(z.is_finite(), "height {z} is not finite");
    let b = norm(z).to_bits();
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

/// Drainage of `core`; `z` covers `win` (`core` grown by one cell) and must be finite, or a
/// NaN cell would pass for a pit.
#[cfg(test)]
pub(crate) fn drain(world: World, win: Rect, z: &[f32], core: Rect) -> Drain {
    let dir = super::receivers::receivers(world, win, z, core);
    drain_with(world, win, z, core, dir)
}

/// `drain` with the receivers `dir` of `core` given (each strictly lower on `z`, or a kept
/// prior receiver at equal height; or none). An equal-height cycle is counted in `cycles`.
pub(crate) fn drain_with(world: World, win: Rect, z: &[f32], core: Rect, dir: Vec<u8>) -> Drain {
    if let Some(k) = z.iter().position(|v| !v.is_finite()) {
        let (x, y) = win.world_xy(k);
        panic!("ground at ({x}, {y}) is {}, not finite", z[k]);
    }
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
    assert_eq!(zc.len(), n, "zc covers the core");
    assert!(
        zc.iter().all(|v| v.is_finite()),
        "core ground is not finite"
    );
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
            let (ck, cm) = (world.index(x, y), world.index(nx, ny));
            let saddle = norm(zc[k].max(zc[m]));
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
                z: norm(zc[k]),
                end: d.end[k],
                pit: d.pit.contains_key(&(k as u32)),
            })
        })
        .collect();
    // A pit's height is its key cell's (always in this tile).
    let mut pits: BTreeMap<u64, f32> = BTreeMap::new();
    for (&k, &key) in &d.pit {
        let (x, y) = core.world_xy(k as usize);
        if world.index(x, y) == key {
            pits.insert(key, norm(zc[k as usize]));
        }
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
        // A: the 1, the 5 and the 8s at x 1-2 plus (3, 3); B: the 2, the 6 and the other 8s
        // at x 3-4; the edge: its 20 cells plus the column of x 5 above the outlet.
        let supply: Vec<(Next, u64)> = g.supply.iter().map(|(&e, &s)| (e, s)).collect();
        assert_eq!(
            supply,
            vec![
                (Next::Node(15), 7 * 1000),
                (Next::Node(17), 5 * 1000),
                (Next::Node(EDGE_KEY), 23 * 1000)
            ]
        );
        assert_eq!(g.pits, vec![(15, 1.0), (17, 2.0)]);
    }

    /// The `w` x `h` world `z` cut to `win`.
    fn cut(world: World, z: &[f32], win: Rect) -> Vec<f32> {
        (0..win.cells())
            .map(|k| {
                let (x, y) = win.world_xy(k);
                z[world.index(x, y) as usize]
            })
            .collect()
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
        let d = drain(world, win, &cut(world, &z, win), core);
        assert_eq!(
            d.pit.values().copied().collect::<Vec<_>>(),
            vec![8],
            "only (2, 1) is in this tile"
        );
        let left = drain(world, Rect::whole(world), &z, Rect::whole(world));
        assert_eq!(left.end[9], Next::Node(8), "(3, 1) joins the pit named 8");

        // A plane falling east: inner core cells leave the tile at the first cell outside.
        let plane: Vec<f32> = (0..24).map(|i| 10.0 - (i % 6) as f32).collect();
        let d = drain(world, win, &cut(world, &plane, win), core);
        let at = |x, y| d.end[core.local(x, y).expect("in core")];
        assert_eq!(at(2, 1), Next::Out(world.index(3, 1)));
        assert_eq!(at(1, 1), Next::Out(world.index(3, 1)));
        assert_eq!(at(1, 2), Next::Out(world.index(3, 2)));
        assert_eq!(
            at(0, 1),
            Next::Node(EDGE_KEY),
            "world edge cells end at the edge"
        );
        assert!(d.pit.is_empty());
    }

    #[test]
    fn signed_zeros_in_a_flat_pit_are_one_height() {
        let world = World { w: 6, h: 4 };
        let mut z = vec![5.0f32; 24];
        (z[8], z[9]) = (0.0, -0.0); // one flat pit at sea level
        let all = Rect::whole(world);
        let d = drain(world, all, &z, all);
        assert_eq!(d.end[8], Next::Node(8));
        assert_eq!(d.end[9], Next::Node(8), "-0.0 and +0.0 are one pit");
        assert_eq!(ordered(-0.0), ordered(0.0));
        assert!(ordered(-0.5) < ordered(-0.0) && ordered(0.0) < ordered(f32::MIN_POSITIVE));
        // The key cell (8) names the pit's height; -0.0 there is stored as +0.0.
        for (a, b) in [(0.0, -0.0), (-0.0, 0.0)] {
            (z[8], z[9]) = (a, b);
            let d = drain(world, all, &z, all);
            let g = tile_graph(world, all, &z, &d, |_| 1);
            assert_eq!(g.pits.len(), 1);
            assert_eq!(g.pits[0].1.to_bits(), 0.0f32.to_bits());
        }
    }

    #[test]
    #[should_panic(expected = "not finite")]
    fn a_nan_cell_is_refused() {
        let (world, mut z) = two_pits();
        z[2 * 7 + 2] = f32::NAN;
        let all = Rect::whole(world);
        drain(world, all, &z, all);
    }

    #[test]
    fn tile_graphs_round_trip() {
        use super::super::store::{decode, encode};
        let (world, z) = two_pits();
        // The inner 5 x 3: its ring holds pit A and the 7 leaving for the outlet.
        let core = Rect {
            x0: 1,
            y0: 1,
            w: 5,
            h: 3,
        };
        let d = drain(world, Rect::whole(world), &z, core);
        let g = tile_graph(world, core, &cut(world, &z, core), &d, |k| k as u64);
        let cells: Vec<u64> = g.ring.iter().map(|r| r.cell).collect();
        assert_eq!(cells, [8, 9, 10, 11, 12, 15, 19, 22, 23, 24, 25, 26]);
        let at = |c: u64| *g.ring.iter().find(|r| r.cell == c).expect("on the ring");
        assert_eq!(
            at(15),
            Ring {
                cell: 15,
                z: 1.0,
                end: Next::Node(15),
                pit: true
            }
        );
        assert_eq!(
            at(19),
            Ring {
                cell: 19,
                z: 7.0,
                end: Next::Out(20),
                pit: false
            }
        );
        assert_eq!(g.ring.iter().filter(|r| r.pit).count(), 1);
        assert!(!g.edges.is_empty());
        let back = decode::<TileGraph>(&encode(&g)).expect("decode");
        assert_eq!(back.ring, g.ring);
        assert_eq!(back, g);
    }

    #[test]
    #[ignore]
    fn probe_pits_in_the_canyon() {
        pit_probe("canyon", 27169, 44546);
        contraction_probe("canyon", 27169, 44546);
    }

    /// Plains (class 18/0 on 253 of 256 class cells, 74 m p5-p95 relief): the lowest-relief
    /// class-dominated 2 km block of the layout's class map.
    #[test]
    #[ignore]
    fn probe_pits_on_the_plains() {
        pit_probe("plains", 18944, 36863);
        contraction_probe("plains", 18944, 36863);
    }

    /// Task 8b: the 2048² window as its own world, cut into tiles and contracted. Prints the
    /// pits, the settled trees, the global leaves left and the contraction ratio, and checks
    /// that contraction keeps every depression (`contract::tests::signature`).
    fn contraction_probe(name: &str, east: u64, north: u64) {
        use crate::world::water::contract::contract;
        use crate::world::water::contract::tests::signature;
        use crate::world::water::geom::tiles;
        use crate::world::water::hierarchy::{NONE, build};
        use crate::world::water::probe;
        let Some((win, z)) = probe::window(east, north, 2048) else {
            return;
        };
        let world = World {
            w: win.w as u64,
            h: win.h as u64,
        };
        for tile in [512u64, 1024, 2048] {
            let graphs: Vec<TileGraph> = tiles(world, tile)
                .into_iter()
                .map(|core| {
                    let w = core.grow(1, world);
                    let d = drain(world, w, &cut(world, &z, w), core);
                    tile_graph(world, core, &cut(world, &z, core), &d, |_| 1000)
                })
                .collect();
            let pits: usize = graphs.iter().map(|g| g.pits.len()).sum();
            let edges = |gs: &[TileGraph]| gs.iter().map(|g| g.edges.len()).sum::<usize>();
            let edges_before = edges(&graphs);
            let (whole, _) = build(world, tile, &graphs).expect("hierarchy");
            let (graphs, frags): (Vec<TileGraph>, Vec<_>) =
                graphs.into_iter().map(contract).unzip();
            let (part, _) = build(world, tile, &graphs).expect("contracted hierarchy");
            let candidates: usize = frags.iter().map(|f| f.h.leaves).sum();
            let tops = |f: &crate::world::water::contract::Frag| {
                (0..f.h.len()).filter(|&d| f.h.parent[d] == NONE).count()
            };
            let trees: usize = frags.iter().map(tops).sum();
            let multi: usize = frags
                .iter()
                .map(|f| {
                    (0..f.h.len())
                        .filter(|&d| f.h.parent[d] == NONE && f.size[d] > 1)
                        .count()
                })
                .sum();
            let want = signature(&whole, false);
            let mut got = signature(&part, false);
            for f in &frags {
                got.extend(signature(&f.h, true));
            }
            got.sort_unstable();
            assert_eq!(got, want, "{name} tiles {tile}: contraction is not exact");
            eprintln!(
                "{name} E{east} N{north} tiles {tile}²: pits {pits}, global leaves {} -> {} \
                 (ratio {:.2}); candidates {candidates} settle into {trees} super-leaves \
                 ({multi} of more than one pit); in-tile edges {edges_before} -> {}; exact",
                whole.leaves,
                part.leaves,
                whole.leaves as f64 / part.leaves.max(1) as f64,
                edges(&graphs)
            );
        }
    }

    /// One window per class, each a 2 km block the class owns on all 256 class cells of
    /// fujix-collage-55/class.u8 (the dominated block nearest the centroid of all such).
    #[test]
    #[ignore]
    fn probe_pits_per_class() {
        for (name, east, north) in [
            ("star dunes (8)", 57216, 47231),
            ("crest-train dunes (16)", 54784, 57343),
            ("badlands (7)", 27264, 59391),
            ("braided (9)", 51072, 7551),
            ("tower karst (14)", 33280, 19327),
            ("alpine (6)", 41600, 20095),
            ("stratovolcano (11)", 33280, 31487),
        ] {
            pit_probe(name, east, north);
        }
    }

    /// Pit and in-tile edge density of a 2048² window, extrapolated to the world.
    fn pit_probe(name: &str, east: u64, north: u64) {
        use crate::world::water::probe;
        let Some((win, z)) = probe::window(east, north, 2048) else {
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
            "{name} E{east} N{north}: pits {} (one per {} cells), edges {}, cycles {}",
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
        let (pits_km2, edges_km2) = (g.pits.len() as f64 / km2, g.edges.len() as f64 / km2);
        eprintln!(
            "{name}: core {km2:.3} km², pits/km² {pits_km2:.0}, edges/km² {edges_km2:.0}, \
             zero-depth pits {flat} ({:.0}/km²)",
            flat as f64 / km2
        );
        // The world at this density: struct sizes only, no map overhead or seam edges.
        let world_km2 = (probe::WORLD.w * probe::WORLD.h) as f64 / 1e6;
        let (pits, edges) = (pits_km2 * world_km2, edges_km2 * world_km2);
        let bytes = pits * std::mem::size_of::<(u64, f32)>() as f64
            + edges * std::mem::size_of::<Edge>() as f64;
        eprintln!(
            "{name}: world {world_km2:.0} km² -> pits {:.2} M, edges {:.2} M, {:.2} GB \
             ({} B/edge, {} B/pit)",
            pits / 1e6,
            edges / 1e6,
            bytes / 1e9,
            std::mem::size_of::<Edge>(),
            std::mem::size_of::<(u64, f32)>()
        );
    }
}
