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
use crate::acquisition::{Result, ensure};

pub(crate) const NONE: u32 = u32::MAX;

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
        (self.children.len() as u64).put(e);
        for c in &self.children {
            (c[0], c[1]).put(e);
        }
        self.outlet.put(e);
        self.geolink.put(e);
        self.ocean_order.put(e);
        self.supply.put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let (leaves, leaf_pit, low, spill, parent): (u64, _, _, _, _) = Blob::get(d)?;
        let (children, outlet, geolink, ocean_order, supply): (Vec<(u32, u32)>, _, _, _, _) =
            Blob::get(d)?;
        Ok(Self {
            leaves: leaves as usize,
            leaf_pit,
            low,
            spill,
            parent,
            children: children.into_iter().map(|(a, b)| [a, b]).collect(),
            outlet,
            geolink,
            ocean_order,
            supply,
        })
    }
}

/// Maps a tile's drainage ends to leaves. Sorted `Vec`s with binary search (review C3): about
/// a third of the memory of HashMaps at 10⁷ leaves.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Resolver {
    /// Ring cells' in-tile ends, sorted by world index.
    pub ring: Vec<(u64, Next)>,
    /// Pit key → the key of the pit region it belongs to across seams, sorted by pit key.
    pub root: Vec<(u64, u64)>,
    /// Leaf keys (pit regions and contracted super-leaves), sorted: a leaf is its position.
    pub leaf_pit: Vec<u64>,
}

impl Resolver {
    pub fn ring_end(&self, c: u64) -> Option<Next> {
        self.ring
            .binary_search_by_key(&c, |e| e.0)
            .ok()
            .map(|i| self.ring[i].1)
    }
    pub fn root_of(&self, p: u64) -> u64 {
        self.root
            .binary_search_by_key(&p, |e| e.0)
            .map_or(p, |i| self.root[i].1)
    }
    pub fn leaf_index(&self, key: u64) -> u32 {
        self.leaf_pit.binary_search(&key).map_or(NONE, |i| i as u32)
    }
    /// The leaf an in-tile end drains to; NONE for the world edge.
    pub fn leaf_of(&self, mut t: Next) -> u32 {
        // Each step enters another ring cell, so a longer walk is a cycle across tiles.
        for _ in 0..=self.ring.len() {
            match t {
                Next::Out(c) => t = self.ring_end(c).expect("an exit enters a ring cell"),
                Next::Node(EDGE_KEY) => return NONE,
                Next::Node(p) => {
                    let l = self.leaf_index(self.root_of(p));
                    assert!(l != NONE, "pit {p} is not a leaf");
                    return l;
                }
                Next::Cell(_) | Next::End => panic!("{t:?} is not a drainage end"),
            }
        }
        panic!("flow cycles across tiles")
    }
}

impl Blob for Resolver {
    fn put(&self, e: &mut Enc) {
        self.ring.put(e);
        self.root.put(e);
        self.leaf_pit.put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let (ring, root, leaf_pit) = Blob::get(d)?;
        Ok(Self {
            ring,
            root,
            leaf_pit,
        })
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
    // Ring cells: (cell, end, ground, pit), sorted by cell.
    let mut ring: Vec<(u64, Next, f32, bool)> = graphs
        .iter()
        .flat_map(|g| g.ring.iter().map(|r| (r.cell, r.end, r.z, r.pit)))
        .collect();
    ring.sort_unstable_by_key(|r| r.0);
    let ring_at = |c: u64| ring.binary_search_by_key(&c, |r| r.0).ok().map(|i| ring[i]);
    // Equal pit cells that meet across a seam are one pit, named by its smallest key (R28).
    let mut keys: Vec<u64> = graphs
        .iter()
        .flat_map(|g| g.pits.iter().map(|p| p.0))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    let slot = |k: u64| keys.binary_search(&k).expect("a pit key") as u32;
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
            if let Some((_, Next::Node(q), zq, true)) = ring_at(world.index(nx, ny))
                && zq == z
            {
                uf.union(slot(p), slot(q));
            }
        }
    }
    let root: Vec<(u64, u64)> = keys
        .iter()
        .enumerate()
        .map(|(i, &k)| (k, keys[uf.find(i as u32) as usize]))
        .collect();
    let mut leaf_pit: Vec<u64> = root.iter().map(|r| r.1).collect();
    leaf_pit.sort_unstable();
    leaf_pit.dedup();
    let resolver = Resolver {
        ring: ring.iter().map(|r| (r.0, r.1)).collect(),
        root,
        leaf_pit: leaf_pit.clone(),
    };
    let nl = leaf_pit.len();
    ensure(nl < NONE as usize, "too many leaves for u32 indices")?;

    let mut low = vec![f32::INFINITY; nl];
    let mut supply = vec![0u64; nl];
    for g in graphs {
        for &(p, z) in &g.pits {
            let l = resolver.leaf_index(resolver.root_of(p)) as usize;
            low[l] = low[l].min(z);
        }
        for (&t, &s) in &g.supply {
            let l = resolver.leaf_of(t);
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
            add(
                resolver.leaf_of(e.a),
                resolver.leaf_of(e.b),
                e.saddle,
                e.ca,
                e.cb,
            );
        }
    }
    for &(c, end_c, zc, _) in &ring {
        let (x, y) = world.xy(c);
        for j in 0..8 {
            let Some((nx, ny)) = world.neighbour(x, y, j) else {
                continue;
            };
            let o = world.index(nx, ny);
            if o < c || tile_of(world, tile, x, y) == tile_of(world, tile, nx, ny) {
                continue;
            }
            let Some((_, end_o, zo, _)) = ring_at(o) else {
                return Err(crate::acquisition::AcquisitionError::Invalid(format!(
                    "cell {o} next to a seam is missing from its tile's ring"
                )));
            };
            add(
                resolver.leaf_of(end_c),
                resolver.leaf_of(end_o),
                zc.max(zo),
                c,
                o,
            );
        }
    }
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
        assert_eq!(h.geolink[0], 1);
        assert_eq!(h.geolink[2], NONE);
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
            assert_eq!(r.root, keys.iter().map(|&k| (k, floor)).collect::<Vec<_>>());
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
            .filter(|&d| h.spill[d].is_finite())
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
