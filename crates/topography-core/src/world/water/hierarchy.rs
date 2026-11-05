//! Global step 1: join the tiles' label graphs (pits that meet across a seam, edges across
//! seams) and build the depression hierarchy (Barnes, Callaghan and Wickert 2020) by merging
//! along spill edges from the lowest (Kruskal; equal saddles ordered by cell pair, so every
//! tiling builds the same tree). Leaves are pits; a meta-depression forms where two spill into
//! each other; a depression whose lowest edge leads to the ocean (the world edge, or anything
//! already joined to it) is top-level.

use super::flow::Next;
use super::geom::{UnionFind, World, tile_of, tiles};
use super::store::{Blob, Dec, Enc};
use super::watershed::{EDGE_KEY, TileGraph, ordered};
use crate::acquisition::{AcquisitionError, Result, ensure};

pub(crate) const NONE: u32 = u32::MAX;

/// A tile graph that does not fit the others (a stale or mismatched work file).
fn stale(msg: String) -> AcquisitionError {
    AcquisitionError::Invalid(format!("tile graphs disagree: {msg}"))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Hierarchy {
    pub leaves: usize,
    pub leaf_pit: Vec<u64>,
    pub low: Vec<f32>,
    pub spill: Vec<f32>,
    pub parent: Vec<u32>,
    pub children: Vec<[u32; 2]>,
    pub outlet: Vec<[u64; 2]>,
    pub geolink: Vec<u32>,
    pub ocean_order: Vec<u32>,
    pub supply: Vec<u64>,
}

impl Hierarchy {
    pub fn len(&self) -> usize {
        self.spill.len()
    }
    pub fn is_leaf(&self, d: u32) -> bool {
        (d as usize) < self.leaves
    }
}

impl Blob for Hierarchy {
    fn put(&self, e: &mut Enc) {
        (self.leaves as u64).put(e);
        self.leaf_pit.put(e);
        self.low.put(e);
        self.spill.put(e);
        self.parent.put(e);
        self.children.put(e);
        self.outlet.put(e);
        self.geolink.put(e);
        self.ocean_order.put(e);
        self.supply.put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let (leaves, leaf_pit, low, spill, parent): (u64, _, _, _, _) = Blob::get(d)?;
        let (children, outlet, geolink, ocean_order, supply) = Blob::get(d)?;
        Ok(Self {
            leaves: leaves as usize,
            leaf_pit,
            low,
            spill,
            parent,
            children,
            outlet,
            geolink,
            ocean_order,
            supply,
        })
    }
}

/// Maps the tiles' drainage ends to leaves. Sorted `Vec`s with binary search (review C3); each
/// ring cell's leaf is resolved once in `build`, so no lookup walks a path.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Resolver {
    /// Ring cell → the leaf its flow reaches (NONE: the world edge), sorted by world index.
    pub ring: Vec<(u64, u32)>,
    /// Pit key → its leaf (pits joined across seams share one), sorted by pit key.
    pub pit: Vec<(u64, u32)>,
}

impl Resolver {
    pub fn ring_leaf(&self, c: u64) -> Option<u32> {
        lookup(&self.ring, c)
    }
    pub fn pit_leaf(&self, p: u64) -> Option<u32> {
        lookup(&self.pit, p)
    }
    /// The leaf an in-tile end drains to (NONE: the world edge); None if no tile knows it.
    pub fn find(&self, t: Next) -> Option<u32> {
        match t {
            Next::Out(c) => self.ring_leaf(c),
            Next::Node(EDGE_KEY) => Some(NONE),
            Next::Node(p) => self.pit_leaf(p),
            Next::Cell(_) | Next::End => None,
        }
    }
    /// As `find`, for ends of the tile graphs `build` checked.
    pub fn leaf_of(&self, t: Next) -> u32 {
        self.find(t)
            .unwrap_or_else(|| panic!("{t:?} is not a known drainage end"))
    }
}

fn lookup(v: &[(u64, u32)], k: u64) -> Option<u32> {
    v.binary_search_by_key(&k, |e| e.0).ok().map(|i| v[i].1)
}

impl Blob for Resolver {
    fn put(&self, e: &mut Enc) {
        self.ring.put(e);
        self.pit.put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let (ring, pit) = Blob::get(d)?;
        Ok(Self { ring, pit })
    }
}

/// A spill edge between two leaves (`b` may be NONE: the ocean); `ca` lies in `a`.
#[derive(Clone, Copy)]
struct Link {
    a: u32,
    b: u32,
    saddle: f32,
    ca: u64,
    cb: u64,
}

impl Link {
    fn new(la: u32, lb: u32, saddle: f32, ca: u64, cb: u64) -> Self {
        if la < lb {
            Link {
                a: la,
                b: lb,
                saddle,
                ca,
                cb,
            }
        } else {
            Link {
                a: lb,
                b: la,
                saddle,
                ca: cb,
                cb: ca,
            }
        }
    }
    /// As `Edge::rank`: lowest saddle, then the cell pair.
    fn rank(&self) -> (u32, u64, u64) {
        (
            ordered(self.saddle),
            self.ca.min(self.cb),
            self.ca.max(self.cb),
        )
    }
}

/// Joins the tile graphs of every tile of `tile` cells (in `tiles` order) and builds the
/// hierarchy and the resolver of their ends.
pub(crate) fn build(
    world: World,
    tile: u64,
    graphs: &[TileGraph],
) -> Result<(Hierarchy, Resolver)> {
    ensure(
        graphs.len() == tiles(world, tile).len(),
        "the hierarchy needs every tile's graph",
    )?;
    ensure(
        graphs.iter().all(|g| g.cycles == 0),
        "a tile's drainage has cycles",
    )?;
    // Pit keys; every end the graphs name must be one of them, the edge or an exit (stale or
    // mismatched blobs are refused here, not by a panic later).
    let mut keys: Vec<u64> = graphs
        .iter()
        .flat_map(|g| g.pits.iter().map(|p| p.0))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    let known = |t: Next| match t {
        Next::Out(_) | Next::Node(EDGE_KEY) => true,
        Next::Node(k) => keys.binary_search(&k).is_ok(),
        Next::Cell(_) | Next::End => false,
    };
    for g in graphs {
        let ends = g.ring.iter().map(|r| r.end);
        let ends = ends.chain(g.edges.iter().flat_map(|e| [e.a, e.b]));
        if let Some(t) = ends.chain(g.supply.keys().copied()).find(|&t| !known(t)) {
            return Err(stale(format!("end {t:?} is no pit, exit or world edge")));
        }
    }
    // Ring cells: (cell, end, ground, pit), sorted by cell.
    let mut ring: Vec<(u64, Next, f32, bool)> = graphs
        .iter()
        .flat_map(|g| g.ring.iter().map(|r| (r.cell, r.end, r.z, r.pit)))
        .collect();
    ring.sort_unstable_by_key(|r| r.0);
    if let Some(w) = ring.windows(2).find(|w| w[0].0 == w[1].0) {
        return Err(stale(format!("ring cell {} is in two tiles", w[0].0)));
    }
    let ring_at = |c: u64| ring.binary_search_by_key(&c, |r| r.0).ok();
    // Equal pit cells that meet across a seam are one pit, named by its smallest key (R28).
    let slot = |k: u64| keys.binary_search(&k).expect("a checked pit key") as u32;
    let mut uf = UnionFind::new(keys.len());
    for &(c, end, z, _) in ring.iter().filter(|r| r.3) {
        let Next::Node(p) = end else {
            continue;
        };
        let (x, y) = world.xy(c);
        for j in 0..8 {
            let Some((nx, ny)) = world.neighbour(x, y, j) else {
                continue;
            };
            if let Some(i) = ring_at(world.index(nx, ny))
                && let (_, Next::Node(q), zq, true) = ring[i]
                && q != EDGE_KEY
                && zq == z
            {
                uf.union(slot(p), slot(q));
            }
        }
    }
    let roots: Vec<u64> = (0..keys.len() as u32)
        .map(|i| keys[uf.find(i) as usize])
        .collect();
    let mut leaf_pit = roots.clone();
    leaf_pit.sort_unstable();
    leaf_pit.dedup();
    let nl = leaf_pit.len();
    // Leaves and metas (fewer than the leaves) share u32 indices below NONE.
    ensure(2 * nl < NONE as usize, "too many leaves for u32 indices")?;
    let pit: Vec<(u64, u32)> = keys
        .iter()
        .zip(&roots)
        .map(|(&k, r)| (k, leaf_pit.binary_search(r).expect("a root") as u32))
        .collect();
    drop(roots);

    // Each ring cell's leaf, following exits from ring cell to ring cell once.
    const UNSEEN: u8 = 0;
    const OPEN: u8 = 1;
    let mut state = vec![UNSEEN; ring.len()];
    let mut leaf = vec![NONE; ring.len()];
    let mut stack: Vec<usize> = Vec::new();
    for start in 0..ring.len() {
        let mut i = start;
        let found = loop {
            if state[i] > OPEN {
                break leaf[i];
            }
            state[i] = OPEN;
            stack.push(i);
            match ring[i].1 {
                Next::Out(c) => {
                    let Some(k) = ring_at(c) else {
                        return Err(stale(format!("exit {c} enters no ring cell")));
                    };
                    ensure(state[k] != OPEN, "flow cycles across tiles")?;
                    i = k;
                }
                Next::Node(EDGE_KEY) => break NONE,
                Next::Node(p) => break lookup(&pit, p).expect("a checked pit key"),
                Next::Cell(_) | Next::End => unreachable!("ring ends are checked"),
            }
        };
        for i in stack.drain(..) {
            (leaf[i], state[i]) = (found, OPEN + 1);
        }
    }
    drop(state);
    let resolver = Resolver {
        ring: ring.iter().zip(&leaf).map(|(r, &l)| (r.0, l)).collect(),
        pit,
    };
    let leaf_of = |t: Next| {
        resolver
            .find(t)
            .ok_or_else(|| stale(format!("exit {t:?} enters no ring cell")))
    };

    let mut low = vec![f32::INFINITY; nl];
    let mut supply = vec![0u64; nl];
    for g in graphs {
        for &(p, z) in &g.pits {
            let l = leaf_of(Next::Node(p))? as usize;
            low[l] = low[l].min(z);
        }
        for (&t, &s) in &g.supply {
            let l = leaf_of(t)?;
            if l != NONE {
                supply[l as usize] += s;
            }
        }
    }

    // Candidate edges between leaves, then the lowest per pair by sorting (no HashMap).
    let mut links: Vec<Link> = Vec::new();
    let mut add = |la: u32, lb: u32, saddle: f32, ca: u64, cb: u64| {
        if la != lb {
            links.push(Link::new(la, lb, saddle, ca, cb));
        }
    };
    for g in graphs {
        for e in &g.edges {
            add(leaf_of(e.a)?, leaf_of(e.b)?, e.saddle, e.ca, e.cb);
        }
    }
    for (i, &(c, _, zc, _)) in ring.iter().enumerate() {
        let (x, y) = world.xy(c);
        for j in 0..8 {
            let Some((nx, ny)) = world.neighbour(x, y, j) else {
                continue;
            };
            let o = world.index(nx, ny);
            if o < c || tile_of(world, tile, x, y) == tile_of(world, tile, nx, ny) {
                continue;
            }
            let Some(k) = ring_at(o) else {
                return Err(stale(format!(
                    "cell {o} next to a seam is missing from its tile's ring"
                )));
            };
            add(leaf[i], leaf[k], zc.max(ring[k].2), c, o);
        }
    }
    drop((ring, leaf));
    links.sort_unstable_by_key(|l| (l.a, l.b, l.rank()));
    links.dedup_by_key(|l| (l.a, l.b));
    // Distinct pairs never share a cell pair, so the rank orders them totally.
    links.sort_unstable_by_key(|l| l.rank());

    // Kruskal from the lowest saddle; union-find slot `nl` is the ocean.
    let ocean = nl as u32;
    let mut uf = UnionFind::new(nl + 1);
    // The depression on top of each union-find root (NONE once joined to the ocean).
    let mut top: Vec<u32> = (0..nl as u32).chain([NONE]).collect();
    let mut h = Hierarchy {
        leaves: nl,
        leaf_pit,
        low,
        spill: vec![f32::INFINITY; nl],
        parent: vec![NONE; nl],
        children: vec![[NONE, NONE]; nl],
        outlet: vec![[0, 0]; nl],
        geolink: vec![NONE; nl],
        ocean_order: Vec::new(),
        supply,
    };
    let spill_out = |h: &mut Hierarchy, d: u32, l: &Link, from_a: bool| {
        let (inside, outside, beyond) = if from_a {
            (l.ca, l.cb, l.b)
        } else {
            (l.cb, l.ca, l.a)
        };
        h.spill[d as usize] = l.saddle;
        h.outlet[d as usize] = [inside, outside];
        h.geolink[d as usize] = beyond;
    };
    for l in links {
        let ia = if l.a == NONE { ocean } else { l.a };
        let ib = if l.b == NONE { ocean } else { l.b };
        let (ra, rb) = (uf.find(ia), uf.find(ib));
        if ra == rb {
            continue;
        }
        let ro = uf.find(ocean);
        if ra == ro || rb == ro {
            let (r, from_a) = if rb == ro { (ra, true) } else { (rb, false) };
            let d = top[r as usize];
            spill_out(&mut h, d, &l, from_a);
            h.ocean_order.push(d);
            let joined = uf.union(r, ro);
            top[joined as usize] = NONE;
        } else {
            let (da, db) = (top[ra as usize], top[rb as usize]);
            let m = h.len() as u32;
            h.low.push(h.low[da as usize].min(h.low[db as usize]));
            h.spill.push(f32::INFINITY);
            h.parent.push(NONE);
            h.children.push([da, db]);
            h.outlet.push([0, 0]);
            h.geolink.push(NONE);
            for (d, from_a) in [(da, true), (db, false)] {
                spill_out(&mut h, d, &l, from_a);
                h.parent[d as usize] = m;
            }
            let joined = uf.union(ra, rb);
            top[joined as usize] = m;
        }
    }
    let ro = uf.find(ocean);
    ensure(
        (0..nl as u32).all(|l| uf.find(l) == ro),
        "a basin has no way to the world edge",
    )?;
    Ok((h, resolver))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::world::water::geom::{Rect, tiles};
    use crate::world::water::watershed::{drain, tile_graph};

    /// Pass A on tiles of `tile` cells, then the hierarchy (r = 1 everywhere).
    pub(crate) fn graph_for(world: World, z: &[f32], tile: u64) -> (Hierarchy, Resolver) {
        let all = Rect::whole(world);
        let graphs: Vec<TileGraph> = tiles(world, tile)
            .into_iter()
            .map(|core| {
                let win = core.grow(1, world);
                let zw: Vec<f32> = (0..win.cells())
                    .map(|k| {
                        let (x, y) = win.world_xy(k);
                        z[all.local(x, y).expect("in")]
                    })
                    .collect();
                let zc: Vec<f32> = (0..core.cells())
                    .map(|k| {
                        let (x, y) = core.world_xy(k);
                        z[all.local(x, y).expect("in")]
                    })
                    .collect();
                let d = drain(world, win, &zw, core);
                tile_graph(world, core, &zc, &d, |_| 1000)
            })
            .collect();
        build(world, tile, &graphs).expect("hierarchy")
    }

    /// Rough terrain with many pits (deterministic).
    pub(crate) fn rough(world: World) -> Vec<f32> {
        (0..world.w * world.h)
            .map(|i| {
                let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
                20.0 + (x * 0.7).sin() * 3.0
                    + (y * 0.5).cos() * 3.0
                    + ((i * 7919) % 17) as f32 * 0.2
                    - x * 0.1
            })
            .collect()
    }

    #[test]
    fn two_pits_merge_below_their_way_out() {
        let (world, z) = crate::world::water::watershed::tests::two_pits();
        let (h, _) = graph_for(world, &z, 64);
        assert_eq!(h.leaves, 2);
        assert_eq!(h.leaf_pit, vec![15, 17]);
        assert_eq!(h.spill, vec![5.0, 5.0, 7.0]);
        assert_eq!(h.parent, vec![2, 2, NONE]);
        assert_eq!(h.children[2], [0, 1]);
        assert_eq!(h.low[2], 1.0);
        assert_eq!((h.geolink[0], h.geolink[1], h.geolink[2]), (1, 0, NONE));
        // A and B spill into each other across the 5 (16) next to B's floor (17).
        assert_eq!((h.outlet[0], h.outlet[1]), ([16, 17], [17, 16]));
        assert_eq!(h.ocean_order, vec![2]);
        let [inside, outside] = h.outlet[2];
        assert_eq!(z[inside as usize].max(z[outside as usize]), 7.0);
        assert_eq!(h.supply.len(), 2);
        assert!(
            h.supply.iter().all(|&s| s >= 1000),
            "each pit drains at least itself"
        );
    }

    #[test]
    fn tiles_build_the_same_hierarchy() {
        let world = World { w: 48, h: 40 };
        let z = rough(world);
        let (whole, _) = graph_for(world, &z, 64);
        assert!(whole.leaves > 5, "the case has pits");
        for size in [16, 7] {
            let (tiled, _) = graph_for(world, &z, size);
            assert_eq!(tiled, whole, "tile size {size}");
        }
    }

    /// A bowl whose flat floor (x 3..=6 on row 3, height 3) straddles tile seams.
    fn flat_bowl() -> (World, Vec<f32>) {
        let world = World { w: 10, h: 7 };
        let z = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = world.xy(i);
                let dx = (x as f32 - 4.5).abs();
                3.0 + (y as f32 - 3.0).abs() + 0.5 * (dx - 1.5).max(0.0)
            })
            .collect();
        (world, z)
    }

    #[test]
    fn a_flat_pit_across_seams_is_one_leaf_named_by_its_smallest_cell() {
        let (world, z) = flat_bowl();
        let floor = world.index(3, 3);
        let (whole, _) = graph_for(world, &z, 64);
        assert_eq!(whole.leaf_pit, vec![floor]);
        assert_eq!(whole.low, vec![3.0]);
        // Each tile names its piece by its own smallest cell; all resolve to the floor's leaf.
        for (size, pieces) in [(4, vec![3, 4]), (5, vec![3, 5]), (2, vec![3, 4, 6])] {
            let (tiled, r) = graph_for(world, &z, size);
            assert_eq!(tiled, whole, "tile size {size}");
            let keys: Vec<u64> = pieces.iter().map(|&x| world.index(x, 3)).collect();
            assert_eq!(r.pit, keys.iter().map(|&k| (k, 0)).collect::<Vec<_>>());
            assert!(keys.iter().all(|&k| r.leaf_of(Next::Node(k)) == 0));
        }
    }

    #[test]
    fn hierarchy_and_resolver_round_trip() {
        use crate::world::water::store::{decode, encode};
        let world = World { w: 24, h: 20 };
        let (h, r) = graph_for(world, &rough(world), 7);
        assert_eq!(decode::<Hierarchy>(&encode(&h)).expect("decode"), h);
        assert_eq!(decode::<Resolver>(&encode(&r)).expect("decode"), r);
    }

    #[test]
    fn a_missing_tile_is_refused() {
        let (world, z) = flat_bowl();
        let all = Rect::whole(world);
        let d = drain(world, all, &z, all);
        let g = tile_graph(world, all, &z, &d, |_| 1);
        assert!(build(world, 5, &[g]).is_err());
    }

    /// The whole-world hierarchy, checked against the same world cut into each tile size.
    fn alike_at(world: World, z: &[f32], sizes: &[u64]) -> Hierarchy {
        let (whole, _) = graph_for(world, z, world.w.max(world.h));
        for &size in sizes {
            let (tiled, _) = graph_for(world, z, size);
            assert_eq!(tiled, whole, "tile size {size}");
        }
        whole
    }

    /// Metas above leaf `d` up to its top-level depression.
    fn depth(h: &Hierarchy, mut d: u32) -> usize {
        let mut n = 0;
        while h.parent[d as usize] != NONE {
            (d, n) = (h.parent[d as usize], n + 1);
        }
        n
    }

    #[test]
    fn nested_bowls_build_alike_across_many_tiles() {
        // Eight basins: ridges of 3 pair them, of 5 join the pairs, of 7 the halves; the
        // edge wall is 9 with an outlet of 8.
        let world = World { w: 40, h: 40 };
        let z: Vec<f32> = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = world.xy(i);
                if world.is_edge(x, y) {
                    return if (x, y) == (39, 5) { 8.0 } else { 9.0 };
                }
                let ridge = match x {
                    10 | 30 => 3.0,
                    20 => 5.0,
                    _ => 0.0,
                };
                let ridge = if y == 20 { 7.0f32 } else { ridge };
                let (cx, cy) = ((x / 10) * 10 + 5, if y < 20 { 10 } else { 30 });
                let floor = 1.0 + 0.05 * (x.abs_diff(cx) + y.abs_diff(cy)) as f32;
                floor.max(ridge)
            })
            .collect();
        let h = alike_at(world, &z, &[4, 7]);
        assert_eq!(h.leaves, 8);
        assert_eq!(h.len(), 15, "seven metas");
        assert_eq!(h.ocean_order.len(), 1);
        assert!(
            (0..8).all(|l| depth(&h, l) == 3),
            "three levels above each basin"
        );
        assert_eq!(h.spill[14], 8.0);
    }

    #[test]
    fn terraces_with_equal_saddles_build_alike() {
        let world = World { w: 30, h: 27 };
        let z: Vec<f32> = rough(world).iter().map(|v| v.round()).collect();
        let h = alike_at(world, &z, &[3, 5]);
        assert!(h.leaves > 5, "the case has pits");
        let mut s = h.spill.clone();
        s.sort_by(f32::total_cmp);
        assert!(s.windows(2).any(|w| w[0] == w[1]), "equal saddles");
    }

    #[test]
    fn a_way_out_across_several_tiles_reaches_the_edge() {
        // A pit at x 4 behind a sill of 5 at x 8; beyond it a valley falls east to the edge.
        let world = World { w: 40, h: 9 };
        let z: Vec<f32> = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = world.xy(i);
                let wall = 9.0 + y.abs_diff(4) as f32;
                match (x, y) {
                    (39, 4) => 0.5,
                    _ if world.is_edge(x, y) || y.abs_diff(4) > 1 => wall,
                    (8, _) => 5.0,
                    (..8, _) => 2.0 + 0.1 * x.abs_diff(4) as f32 + 0.1 * y.abs_diff(4) as f32,
                    _ => 4.0 - 0.1 * (x - 8) as f32 + 0.1 * y.abs_diff(4) as f32,
                }
            })
            .collect();
        let h = alike_at(world, &z, &[4, 3]);
        assert_eq!(h.leaf_pit, vec![world.index(4, 4)]);
        assert_eq!((h.spill[0], h.geolink[0]), (5.0, NONE));
        assert_eq!(
            world.xy(h.outlet[0][1]).0,
            9,
            "the water leaves over the sill"
        );
        // A ring cell five tiles from the edge resolves to it.
        let (_, r) = graph_for(world, &z, 4);
        assert_eq!(r.ring_leaf(world.index(20, 4)), Some(NONE));
    }

    #[test]
    fn cycles_and_stale_graphs_are_refused() {
        let (world, z) = flat_bowl();
        let all = Rect::whole(world);
        let d = drain(world, all, &z, all);
        let good = tile_graph(world, all, &z, &d, |_| 1);
        assert!(build(world, 64, std::slice::from_ref(&good)).is_ok());
        let err = |g: TileGraph| build(world, 64, &[g]).expect_err("refused").to_string();
        let mut g = tile_graph(world, all, &z, &d, |_| 1);
        g.cycles = 1;
        assert!(err(g).contains("cycles"));
        let mut g = tile_graph(world, all, &z, &d, |_| 1);
        g.edges[0].a = Next::Node(1);
        assert!(err(g).contains("no pit"));
        let mut g = tile_graph(world, all, &z, &d, |_| 1);
        g.supply.insert(Next::Out(world.index(4, 3)), 1);
        assert!(err(g).contains("enters no ring cell"));
    }

    #[test]
    #[ignore]
    fn probe_hierarchy_at_the_dam_spot() {
        use crate::world::water::probe;
        let Some((win, z)) = probe::window(25645, 24400, 1024) else {
            return;
        };
        let world = World {
            w: win.w as u64,
            h: win.h as u64,
        };
        let (h, _) = graph_for(world, &z, 256);
        let deepest = (0..h.len())
            .max_by(|&a, &b| (h.spill[a] - h.low[a]).total_cmp(&(h.spill[b] - h.low[b])))
            .expect("depressions");
        eprintln!(
            "leaves {}, metas {}, top-level {}; deepest depression {} m (low {} spill {})",
            h.leaves,
            h.len() - h.leaves,
            h.ocean_order.len(),
            h.spill[deepest] - h.low[deepest],
            h.low[deepest],
            h.spill[deepest]
        );
        let (whole, _) = graph_for(world, &z, 1024);
        assert_eq!(h, whole, "256-cell tiles build the one-tile hierarchy");
        eprintln!("tiles of 256 and one tile of 1024 build the same hierarchy");
    }
}
