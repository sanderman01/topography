//! Exact per-tile contraction (review C3, R25). A tile's candidate pits are its pits off the
//! ring (ring pits may join pits across a seam). Each candidate's frontier is the lowest saddle
//! by which its watershed touches anything else: a non-candidate end (a ring pit, a cell
//! draining out of the tile or to the world edge) or one of its own ring cells (any seam edge
//! sits there). All are at or above the ring's lowest ground. A local Kruskal merges two
//! components only below both frontiers; a component that meets an edge it may not take is
//! frozen. Every merge it makes is then one the world's Kruskal makes in the same way, because
//! no edge of lower rank touches either component outside the tile. Settled trees go to the
//! global step as one super-leaf named by their smallest pit (R28); `solve_local` solves their
//! inside later.

use super::balance::{Balance, Counts, solve};
use super::flow::Next;
use super::geom::UnionFind;
use super::hierarchy::{Hierarchy, NONE};
use super::store::{Blob, Dec, Enc};
use super::watershed::{Below0, EDGE_KEY, Edge, Ring, TileGraph, add_below0};
use crate::acquisition::{Result, ensure};
use std::collections::BTreeMap;

/// A tile's settled trees. Leaves are its candidate pits (keys in `h.leaf_pit`), metas their
/// settled merges; a tree's top has `parent == NONE` and spill +∞ until the global step.
/// `h.ocean_order` is empty and `h.supply` holds this tile's cells only: only `solve_local`
/// completes them, so `h` never goes to `solve` directly.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Frag {
    pub h: Hierarchy,
    /// Each depression's tree key: the tree's smallest pit.
    pub tree: Vec<u64>,
    /// Each depression's tree size in pits.
    pub size: Vec<u32>,
    /// Ring cells draining to a candidate pit, with that pit as it was before the remap
    /// (sorted by cell): a path entering the tile there reaches that pit's leaf (B1).
    pub ring_pit: Vec<(u64, u64)>,
}

impl Blob for Frag {
    fn put(&self, e: &mut Enc) {
        self.h.put(e);
        self.tree.put(e);
        self.size.put(e);
        self.ring_pit.put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let (h, tree, size, ring_pit) = Blob::get(d)?;
        Ok(Self {
            h,
            tree,
            size,
            ring_pit,
        })
    }
}

impl Frag {
    /// The local leaf of a pit key, if it is a candidate of this tile.
    pub fn leaf(&self, pit: u64) -> Option<u32> {
        self.h.leaf_pit.binary_search(&pit).ok().map(|i| i as u32)
    }
    /// The local leaf a ring cell of this tile drains to, if a candidate pit.
    pub fn ring_leaf(&self, cell: u64) -> Option<u32> {
        let i = self.ring_pit.binary_search_by_key(&cell, |r| r.0).ok()?;
        self.leaf(self.ring_pit[i].1)
    }
}

/// The smallest pit key under each depression: its node key (R28). Children come before
/// parents.
pub(crate) fn min_pits(h: &Hierarchy) -> Vec<u64> {
    let mut out = vec![u64::MAX; h.len()];
    for d in 0..h.len() {
        out[d] = if h.is_leaf(d as u32) {
            h.leaf_pit[d]
        } else {
            h.children[d]
                .iter()
                .map(|&c| out[c as usize])
                .min()
                .unwrap_or(u64::MAX)
        };
    }
    out
}

/// Settles the tile's trees and returns the graph the global step sees (each tree one
/// super-leaf: supplies summed, edges re-minimised, ring ends remapped) and the fragment.
pub(crate) fn contract(g: TileGraph) -> (TileGraph, Frag) {
    let mut ring_pits: Vec<u64> = g
        .ring
        .iter()
        .filter_map(|r| match (r.pit, r.end) {
            (true, Next::Node(p)) => Some(p),
            _ => None,
        })
        .collect();
    ring_pits.sort_unstable();
    ring_pits.dedup();
    let pits: Vec<(u64, f32)> = g
        .pits
        .iter()
        .copied()
        .filter(|p| ring_pits.binary_search(&p.0).is_err())
        .collect();
    drop(ring_pits);
    let n = pits.len();
    let slot = |t: Next| match t {
        Next::Node(p) if p != EDGE_KEY => pits.binary_search_by_key(&p, |q| q.0).ok(),
        _ => None,
    };
    let mut frontier = vec![f32::INFINITY; n];
    for r in &g.ring {
        if let Some(i) = slot(r.end) {
            frontier[i] = frontier[i].min(r.z);
        }
    }
    let mut inner: Vec<(usize, usize, Edge)> = Vec::new();
    for e in &g.edges {
        match (slot(e.a), slot(e.b)) {
            (Some(i), Some(j)) => inner.push((i, j, *e)),
            (Some(i), None) => frontier[i] = frontier[i].min(e.saddle),
            (None, Some(j)) => frontier[j] = frontier[j].min(e.saddle),
            (None, None) => {}
        }
    }
    inner.sort_unstable_by_key(|x| x.2.rank());

    let mut h = Hierarchy {
        leaves: n,
        leaf_pit: pits.iter().map(|p| p.0).collect(),
        low: pits.iter().map(|p| p.1).collect(),
        spill: vec![f32::INFINITY; n],
        parent: vec![NONE; n],
        children: vec![[NONE, NONE]; n],
        outlet: vec![[0, 0]; n],
        geolink: vec![NONE; n],
        ocean_order: Vec::new(),
        supply: pits
            .iter()
            .map(|p| g.supply.get(&Next::Node(p.0)).copied().unwrap_or(0))
            .collect(),
        below0: pits
            .iter()
            .map(|p| g.below0.get(&Next::Node(p.0)).copied().unwrap_or_default())
            .collect(),
    };
    let mut uf = UnionFind::new(n);
    // Per union-find root: the depression on top, and whether it may merge no more.
    let mut top: Vec<u32> = (0..n as u32).collect();
    let mut frozen = vec![false; n];
    for (i, j, e) in inner {
        let (ri, rj) = (uf.find(i as u32) as usize, uf.find(j as u32) as usize);
        if ri == rj {
            continue;
        }
        let (da, db) = (top[ri], top[rj]);
        // Ties stay unsettled: the global Kruskal orders them (conservative, still exact). A
        // zero-depth depression (R32) starts full with its capacity counted in its parent's,
        // so it stays a global leaf, as it would uncontracted.
        if frozen[ri]
            || frozen[rj]
            || e.saddle >= frontier[ri]
            || e.saddle >= frontier[rj]
            || e.saddle <= h.low[da as usize]
            || e.saddle <= h.low[db as usize]
        {
            (frozen[ri], frozen[rj]) = (true, true);
            continue;
        }
        let m = h.len() as u32;
        h.low.push(h.low[da as usize].min(h.low[db as usize]));
        h.spill.push(f32::INFINITY);
        h.parent.push(NONE);
        h.children.push([da, db]);
        h.outlet.push([0, 0]);
        h.geolink.push(NONE);
        // `e.ca` drains to pit i (under `da`), `e.cb` to pit j (under `db`).
        for (d, inside, outside, beyond) in [(da, e.ca, e.cb, j as u32), (db, e.cb, e.ca, i as u32)]
        {
            h.spill[d as usize] = e.saddle;
            h.outlet[d as usize] = [inside, outside];
            h.geolink[d as usize] = beyond;
            h.parent[d as usize] = m;
        }
        let r = uf.union(ri as u32, rj as u32) as usize;
        (top[r], frontier[r], frozen[r]) = (m, frontier[ri].min(frontier[rj]), false);
    }
    // Each depression's union-find root: the smallest index, so the smallest pit key.
    let mut root: Vec<u32> = (0..n as u32).map(|i| uf.find(i)).collect();
    for d in n..h.len() {
        root.push(root[h.children[d][0] as usize]);
    }
    let mut members = vec![0u32; n];
    for &r in &root[..n] {
        members[r as usize] += 1;
    }
    let tree: Vec<u64> = root.iter().map(|&r| pits[r as usize].0).collect();
    let size = root.iter().map(|&r| members[r as usize]).collect();
    drop((members, top, frozen, frontier));

    // The remapped graph: each tree is one leaf named by its key.
    let remap = |t: Next| match slot(t) {
        Some(i) => Next::Node(tree[i]),
        None => t,
    };
    let mut supply = BTreeMap::new();
    for (&t, &s) in &g.supply {
        *supply.entry(remap(t)).or_insert(0) += s;
    }
    let mut below0 = BTreeMap::new();
    for (&t, &b) in &g.below0 {
        add_below0(below0.entry(remap(t)).or_default(), b);
    }
    let mut edges: Vec<Edge> = g
        .edges
        .iter()
        .filter_map(|e| {
            let (a, b) = (remap(e.a), remap(e.b));
            (a != b).then(|| {
                if a < b {
                    Edge { a, b, ..*e }
                } else {
                    Edge {
                        a: b,
                        b: a,
                        ca: e.cb,
                        cb: e.ca,
                        ..*e
                    }
                }
            })
        })
        .collect();
    edges.sort_unstable_by_key(|e| (e.a, e.b, e.rank()));
    edges.dedup_by_key(|e| (e.a, e.b));
    let ring = g
        .ring
        .iter()
        .map(|r| Ring {
            end: remap(r.end),
            ..*r
        })
        .collect();
    // A super-leaf's ground is its lowest pit's (build starts every leaf at +∞).
    let mut low: Vec<(u64, f32)> = g
        .pits
        .iter()
        .map(|&(p, z)| match slot(Next::Node(p)) {
            Some(i) => (tree[i], z),
            None => (p, z),
        })
        .collect();
    low.sort_unstable_by(|x, y| x.0.cmp(&y.0).then(x.1.total_cmp(&y.1)));
    low.dedup_by_key(|p| p.0);
    let mut ring_pit: Vec<(u64, u64)> = g
        .ring
        .iter()
        .filter_map(|r| match r.end {
            Next::Node(p) if slot(r.end).is_some() => Some((r.cell, p)),
            _ => None,
        })
        .collect();
    ring_pit.sort_unstable();
    let graph = TileGraph {
        ring,
        pits: low,
        supply,
        below0,
        edges,
        cycles: g.cycles,
    };
    (
        graph,
        Frag {
            h,
            tree,
            size,
            ring_pit,
        },
    )
}

/// A tree's global facts, by tree key: its super-leaf's spill and spill edge.
pub(crate) type Tops = BTreeMap<u64, (f32, [u64; 2])>;

/// The fragment's hierarchy with each listed tree's top given its super-leaf's global spill
/// and outlet (N7: a full top's node then matches the uncontracted run's), unlisted tops −∞ so
/// `holding` finds nothing in them.
pub(crate) fn with_spills(frag: &Frag, tops: &Tops) -> Hierarchy {
    let mut h = frag.h.clone();
    for d in 0..h.len() {
        if h.parent[d] == NONE {
            match tops.get(&frag.tree[d]) {
                Some(&(spill, outlet)) => (h.spill[d], h.outlet[d]) = (spill, outlet),
                None => h.spill[d] = f32::NEG_INFINITY,
            }
        }
    }
    h
}

/// Solves the inside of the trees in `tops`. Their leaves get their own supply plus `extra`
/// (local leaf, water from outside: the global pours into the tree and the r of other tiles'
/// cells draining in across the ring, B1); other trees are left dry, and water poured
/// into them is an error. A listed top's overflow is `to_sea`. `counts` holds every cell the
/// listed trees hold (from every tile) by local holding depression (B2). `extra_below0` adds
/// the below-zero counts of other tiles' cells draining in (never pours: B13).
pub(crate) fn solve_local(
    frag: &Frag,
    tops: &Tops,
    extra: &[(u32, u64)],
    extra_below0: &[(u32, Below0)],
    counts: &Counts,
) -> Result<Balance> {
    let mut h = with_spills(frag, tops);
    let listed = |d: usize| tops.contains_key(&frag.tree[d]);
    // Unlisted spills are −∞: a cell counted there would overflow `solve`'s volumes.
    ensure(
        counts.cells.len() == h.len() && (0..h.len()).all(|d| listed(d) || counts.cells[d] == 0),
        "counts hold cells of a tree not being solved",
    )?;
    // Every listed top is taken, so all its water is accounted for (`solve` checks).
    h.ocean_order = (0..h.len())
        .filter(|&d| h.parent[d] == NONE && listed(d))
        .map(|d| d as u32)
        .collect();
    for l in 0..h.leaves {
        if !listed(l) {
            h.supply[l] = 0;
            h.below0[l] = (0, 0);
        }
    }
    for &(l, w) in extra {
        h.supply[l as usize] += w;
    }
    for &(l, b) in extra_below0 {
        add_below0(&mut h.below0[l as usize], b);
    }
    solve(&h, counts, false)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::world::water::balance::{TileCounts, count, holding, top_spill};
    use crate::world::water::geom::{Rect, World, tiles};
    use crate::world::water::hierarchy::build;
    use crate::world::water::hierarchy::tests::{graph_for, rough};
    use crate::world::water::watershed::{drain, ordered, tile_graph};

    /// Pass A on `world` cut into tiles of `tile` cells, r = 1 everywhere.
    fn graphs_for(world: World, z: &[f32], tile: u64) -> Vec<TileGraph> {
        graphs_with(world, z, tile, |_| 1000)
    }

    /// Pass A on `world` cut into tiles of `tile` cells, with r by world cell.
    fn graphs_with(
        world: World,
        z: &[f32],
        tile: u64,
        r: impl Fn(u64) -> u64 + Copy,
    ) -> Vec<TileGraph> {
        let all = Rect::whole(world);
        let cut = |r: Rect| -> Vec<f32> {
            (0..r.cells())
                .map(|k| {
                    let (x, y) = r.world_xy(k);
                    z[all.local(x, y).expect("in")]
                })
                .collect()
        };
        tiles(world, tile)
            .into_iter()
            .map(|core| {
                let win = core.grow(1, world);
                let d = drain(world, win, &cut(win), core);
                tile_graph(world, core, &cut(core), &d, |k| {
                    let (x, y) = core.world_xy(k);
                    r(world.index(x, y))
                })
            })
            .collect()
    }

    /// Pass A with contraction on tiles of `tile` cells: the fragments and the global build.
    fn contracted_for(world: World, z: &[f32], tile: u64) -> (Vec<Frag>, Hierarchy) {
        let (graphs, frags): (Vec<TileGraph>, Vec<Frag>) =
            graphs_for(world, z, tile).into_iter().map(contract).unzip();
        let (h, _) = build(world, tile, &graphs).expect("hierarchy");
        (frags, h)
    }

    /// A depression as contraction must keep it: smallest pit, spill and low bits, the
    /// parent's smallest pit (MAX: none) and the spill edge (which names the geolink's leaf).
    pub(crate) type Sig = (u64, u32, u32, u64, [u64; 2]);

    /// The signature of every depression (`skip_tops`: only those with a parent), sorted.
    pub(crate) fn signature(h: &Hierarchy, skip_tops: bool) -> Vec<Sig> {
        let keys = min_pits(h);
        let mut v: Vec<Sig> = (0..h.len())
            .filter(|&d| !(skip_tops && h.parent[d] == NONE))
            .map(|d| {
                let p = h.parent[d];
                let up = if p == NONE {
                    u64::MAX
                } else {
                    keys[p as usize]
                };
                let s = (keys[d], ordered(h.spill[d]), ordered(h.low[d]));
                (s.0, s.1, s.2, up, h.outlet[d])
            })
            .collect();
        v.sort_unstable();
        v
    }

    /// Pits under each depression, given each leaf's.
    fn pits_under(h: &Hierarchy, leaf: impl Fn(usize) -> u64) -> Vec<u64> {
        let mut out = vec![0u64; h.len()];
        for d in 0..h.len() {
            out[d] = if h.is_leaf(d as u32) {
                leaf(d)
            } else {
                h.children[d].iter().map(|&c| out[c as usize]).sum()
            };
        }
        out
    }

    /// Cells held by each depression of `h`, given each cell's leaf.
    fn counts_of(world: World, h: &Hierarchy, z: &[f32], leaf: impl Fn(usize) -> u32) -> Counts {
        let top = top_spill(h);
        let mut t = TileCounts::new();
        for (k, &zk) in z.iter().enumerate() {
            let l = leaf(k);
            if l == NONE {
                continue;
            }
            let d = holding(h, &top, l, zk);
            if d != NONE {
                let (x, y) = world.xy(k as u64);
                count(&mut t, d, zk, x, y);
            }
        }
        Counts::dense(h.len(), &[t])
    }

    /// A depression's balance by (smallest pit, pits under it): full, own, its node's
    /// (smallest pit, pits under) ((MAX, 0): none) and outlet.
    type Facts = BTreeMap<(u64, u64), (bool, u64, (u64, u64), [u64; 2])>;

    /// The balance of the uncontracted world (one tile) and of the contracted one on tiles of
    /// `tile` (global solve, then each fragment's listed trees solved with their pours), with
    /// their water sent to the sea.
    fn balances(
        world: World,
        z: &[f32],
        tile: u64,
        r: impl Fn(u64) -> u64 + Copy,
    ) -> [(Facts, u64); 2] {
        let all = Rect::whole(world);
        let dw = drain(world, all, z, all);
        let pit_of = |k: usize| match dw.end[k] {
            Next::Node(p) if p != EDGE_KEY => Some(p),
            _ => None,
        };
        let key = |keys: &[u64], under: &[u64], d: u32| {
            if d == NONE {
                (u64::MAX, 0)
            } else {
                (keys[d as usize], under[d as usize])
            }
        };

        let big = world.w.max(world.h);
        let (hf, rf) = build(world, big, &graphs_with(world, z, big, r)).expect("hierarchy");
        let cf = counts_of(world, &hf, z, |k| {
            pit_of(k).map_or(NONE, |p| rf.leaf_of(Next::Node(p)))
        });
        let bf = solve(&hf, &cf, true).expect("balance");
        let (kf, uf) = (min_pits(&hf), pits_under(&hf, |_| 1));
        let whole: Facts = (0..hf.len())
            .map(|d| {
                let v = (
                    bf.full[d],
                    bf.own[d],
                    key(&kf, &uf, bf.node[d]),
                    hf.outlet[d],
                );
                (key(&kf, &uf, d as u32), v)
            })
            .collect();

        let (graphs, frags): (Vec<TileGraph>, Vec<Frag>) = graphs_with(world, z, tile, r)
            .into_iter()
            .map(contract)
            .unzip();
        let (hp, rp) = build(world, tile, &graphs).expect("contracted hierarchy");
        // Candidate pit → (fragment, local leaf).
        let cand: BTreeMap<u64, (usize, u32)> = frags
            .iter()
            .enumerate()
            .flat_map(|(i, f)| {
                f.h.leaf_pit
                    .iter()
                    .enumerate()
                    .map(move |(l, &p)| (p, (i, l as u32)))
            })
            .collect();
        let global = |p: u64| match cand.get(&p) {
            Some(&(i, l)) => rp.leaf_of(Next::Node(frags[i].tree[l as usize])),
            None => rp.leaf_of(Next::Node(p)),
        };
        let cp = counts_of(world, &hp, z, |k| pit_of(k).map_or(NONE, global));
        let bp = solve(&hp, &cp, true).expect("contracted balance");
        // A global leaf that is a settled tree: its fragment and local top.
        let tree_of = |g: usize| {
            let &(i, l) = cand.get(&hp.leaf_pit[g])?;
            let f = &frags[i];
            let t =
                (0..f.h.len()).find(|&d| f.h.parent[d] == NONE && f.tree[d] == f.tree[l as usize]);
            Some((i, t.expect("a top")))
        };
        let size = |g: usize| tree_of(g).map_or(1, |(i, t)| u64::from(frags[i].size[t]));
        let (kp, up) = (min_pits(&hp), pits_under(&hp, size));
        let mut part: Facts = (0..hp.len())
            .filter(|&d| !(hp.is_leaf(d as u32) && tree_of(d).is_some()))
            .map(|d| {
                let v = (
                    bp.full[d],
                    bp.own[d],
                    key(&kp, &up, bp.node[d]),
                    hp.outlet[d],
                );
                (key(&kp, &up, d as u32), v)
            })
            .collect();

        // Pass L: each fragment's trees that are not sea, with the pours into them.
        for (i, f) in frags.iter().enumerate() {
            let sup = |d: usize| rp.leaf_of(Next::Node(f.tree[d])) as usize;
            let tops: Tops = (0..f.h.len())
                .filter(|&d| f.h.parent[d] == NONE && !bp.sea[sup(d)])
                .map(|d| (f.tree[d], (hp.spill[sup(d)], hp.outlet[sup(d)])))
                .collect();
            let extra: Vec<(u32, u64)> = bp
                .pours
                .iter()
                .filter_map(|&(_, c, w)| match cand.get(&pit_of(c as usize)?) {
                    Some(&(j, l)) if j == i => Some((l, w)),
                    _ => None,
                })
                .collect();
            let hs = with_spills(f, &tops);
            let ts = top_spill(&hs);
            let mut t = TileCounts::new();
            for (k, &zk) in z.iter().enumerate() {
                let Some(&(j, l)) = pit_of(k).and_then(|p| cand.get(&p)) else {
                    continue;
                };
                if j != i {
                    continue;
                }
                let d = holding(&hs, &ts, l, zk);
                if d != NONE {
                    let (x, y) = world.xy(k as u64);
                    count(&mut t, d, zk, x, y);
                }
            }
            // `f.h.supply` holds this tile's cells only; cells of other tiles drain in across
            // the ring (B1).
            let mut extra = extra;
            for (l, &p) in f.h.leaf_pit.iter().enumerate() {
                if !tops.contains_key(&f.tree[l]) {
                    continue;
                }
                let w: u64 = (0..z.len())
                    .filter(|&k| pit_of(k) == Some(p))
                    .map(|k| r(k as u64))
                    .sum();
                extra.push((l as u32, w - f.h.supply[l]));
            }
            let b = solve_local(f, &tops, &extra, &[], &Counts::dense(f.h.len(), &[t]))
                .expect("pass L");
            let (kl, ul) = (min_pits(&f.h), pits_under(&f.h, |_| 1));
            for d in 0..f.h.len() {
                let g = sup(d);
                let gn = bp.node[g];
                let node = if gn != NONE && gn as usize != g {
                    key(&kp, &up, gn)
                } else if tops.contains_key(&f.tree[d]) {
                    key(&kl, &ul, b.node[d])
                } else {
                    (u64::MAX, 0)
                };
                let v = (b.full[d], b.own[d], node, hs.outlet[d]);
                part.insert(key(&kl, &ul, d as u32), v);
            }
        }
        [(whole, bf.to_sea), (part, bp.to_sea)]
    }

    #[test]
    fn contraction_keeps_the_balance() {
        // Rounded heights: flats and zero-depth pits; r from dry to wet, so trees from
        // nearly empty to just full.
        let world = World { w: 30, h: 27 };
        let z: Vec<f32> = rough(world).iter().map(|v| v.round()).collect();
        for scale in [3, 40, 150, 400, 1000] {
            let r = move |c: u64| scale * (1 + (c * 2_654_435_761) % 7);
            for tile in [9, 5, 3] {
                let [whole, part] = balances(world, &z, tile, r);
                assert_eq!(whole.1, part.1, "to_sea, r × {scale}, tiles {tile}");
                for (k, v) in &whole.0 {
                    assert_eq!(
                        part.0.get(k),
                        Some(v),
                        "depression {k:?}, r × {scale}, tiles {tile}"
                    );
                }
                assert_eq!(whole.0.len(), part.0.len());
            }
        }
    }

    /// The global part plus every fragment's settled (non-top) depressions is the whole
    /// hierarchy: same smallest pits, spills and lows.
    fn assert_contraction_exact(world: World, z: &[f32], sizes: &[u64]) -> (usize, usize) {
        let (full, _) = graph_for(world, z, world.w.max(world.h));
        let mut fewest = full.leaves;
        for &size in sizes {
            let (frags, part) = contracted_for(world, z, size);
            let mut got = signature(&part, false);
            for f in &frags {
                got.extend(signature(&f.h, true));
                assert_eq!(f.tree.len(), f.h.len());
                assert_eq!(f.size.len(), f.h.len());
            }
            got.sort_unstable();
            assert_eq!(got, signature(&full, false), "tile size {size}");
            assert!(part.leaves <= full.leaves);
            assert_eq!(
                part.supply.iter().sum::<u64>(),
                full.supply.iter().sum::<u64>(),
                "tile size {size}: supply kept"
            );
            fewest = fewest.min(part.leaves);
        }
        (full.leaves, fewest)
    }

    #[test]
    fn contraction_keeps_every_depression_and_spill() {
        let world = World { w: 48, h: 40 };
        let (all, fewest) = assert_contraction_exact(world, &rough(world), &[64, 16, 7]);
        assert!(fewest < all, "something settles ({fewest} of {all})");
    }

    #[test]
    fn terraces_with_equal_saddles_contract_exactly() {
        // Rounded heights: many equal saddles and frontiers (ties stay global).
        let world = World { w: 30, h: 27 };
        let z: Vec<f32> = rough(world).iter().map(|v| v.round()).collect();
        assert_contraction_exact(world, &z, &[30, 9, 5, 3]);
    }

    #[test]
    fn two_pits_below_their_frontiers_settle_into_one_tree() {
        // 7 x 5, pits A (1 m) and B (2 m) behind a 5 m saddle, B leaves east over a 7 m rim.
        let (world, z) = crate::world::water::watershed::tests::two_pits();
        let all = Rect::whole(world);
        let d = drain(world, all, &z, all);
        let (g, frag) = contract(tile_graph(world, all, &z, &d, |_| 1000));
        // A and B merge at 5 m, below both frontiers (A's 8 m rim, B's 7 m exit): one tree.
        assert_eq!(frag.h.leaves, 2);
        assert_eq!(frag.h.len(), 3);
        assert_eq!(frag.tree, vec![15, 15, 15]);
        assert_eq!(frag.size, vec![2, 2, 2]);
        assert_eq!(frag.h.spill[..2], [5.0, 5.0]);
        assert_eq!(
            frag.h.spill[2],
            f32::INFINITY,
            "the top waits for the global step"
        );
        assert_eq!(min_pits(&frag.h), vec![15, 17, 15]);
        assert_eq!(
            g.pits,
            vec![(15, 1.0)],
            "one super-leaf named by the smaller pit"
        );
        assert_eq!(g.supply[&Next::Node(15)], frag.h.supply.iter().sum::<u64>());
        assert!(g.edges.iter().all(|e| e.a != e.b));
        let out = g
            .edges
            .iter()
            .find(|e| e.b == Next::Node(EDGE_KEY))
            .expect("the tree's edge to the world edge");
        assert_eq!((out.a, out.saddle), (Next::Node(15), 7.0));
    }

    #[test]
    fn a_pit_draining_to_the_ring_stays_global_and_ring_cells_find_their_leaf() {
        let (world, z) = crate::world::water::watershed::tests::two_pits();
        // The inner 5 x 3: pit A (15) sits on its ring; B (17) is a candidate whose ring
        // cells (the 8s above and below it) drain into it.
        let core = Rect {
            x0: 1,
            y0: 1,
            w: 5,
            h: 3,
        };
        let d = drain(world, Rect::whole(world), &z, core);
        let zc: Vec<f32> = (0..core.cells())
            .map(|k| {
                let (x, y) = core.world_xy(k);
                z[world.index(x, y) as usize]
            })
            .collect();
        let (g, frag) = contract(tile_graph(world, core, &zc, &d, |_| 1000));
        assert_eq!(frag.h.leaf_pit, vec![17], "A is a ring pit");
        assert_eq!(frag.h.len(), 1);
        assert_eq!(g.pits, vec![(15, 1.0), (17, 2.0)]);
        assert!(!frag.ring_pit.is_empty());
        assert!(frag.ring_pit.iter().all(|&(_, p)| p == 17));
        let (c, _) = frag.ring_pit[0];
        assert_eq!(frag.ring_leaf(c), Some(0));
        assert_eq!(frag.ring_leaf(15), None, "A's own cell is no candidate's");
        assert_eq!(frag.leaf(15), None);
    }

    #[test]
    fn fragments_round_trip() {
        use crate::world::water::store::{decode, encode};
        let world = World { w: 24, h: 20 };
        for g in graphs_for(world, &rough(world), 12) {
            let (g, f) = contract(g);
            assert_eq!(decode::<Frag>(&encode(&f)).expect("decode"), f);
            assert_eq!(decode::<TileGraph>(&encode(&g)).expect("decode"), g);
        }
    }

    #[test]
    fn solve_local_takes_listed_trees_and_their_pours() {
        let (world, z) = crate::world::water::watershed::tests::two_pits();
        let all = Rect::whole(world);
        let d = drain(world, all, &z, all);
        let (_, frag) = contract(tile_graph(world, all, &z, &d, |_| 1000));
        // The tree's top spills at 7 into the edge, as the global hierarchy has it.
        let tops: Tops = [(15, (7.0, [18, 19]))].into();
        let h = with_spills(&frag, &tops);
        assert_eq!((h.spill[2], h.outlet[2]), (7.0, [18, 19]));
        let counts = Counts {
            cells: vec![1, 1, 2],
            sum_mm: vec![1000, 2000, 10_000],
            bbox: vec![[0, 0, 6, 4]; 3],
        };
        let b = solve_local(&frag, &tops, &[(1, 500)], &[], &counts).expect("balance");
        // 12,500 in: A and B hold 1,000 each, the merge 2,000 more, 8,500 leaves.
        assert_eq!(b.own.iter().sum::<u64>() + b.to_sea, 12_500);
        assert!(b.full[2]);
        assert_eq!(b.to_sea, 8_500);
        // Unlisted: its tops are −∞ (`holding` counts no cell there) and its water is refused.
        let none = Tops::new();
        assert_eq!(with_spills(&frag, &none).spill[2], f32::NEG_INFINITY);
        let empty = Counts {
            cells: vec![0; 3],
            sum_mm: vec![0; 3],
            bbox: vec![[u64::MAX, u64::MAX, 0, 0]; 3],
        };
        assert!(solve_local(&frag, &none, &[(0, 1)], &[], &empty).is_err());
        let err =
            solve_local(&frag, &none, &[], &[], &counts).expect_err("cells of an unlisted tree");
        assert!(err.to_string().contains("not being solved"));
        let dry = solve_local(&frag, &none, &[], &[], &empty).expect("balance");
        assert_eq!(dry.own.iter().sum::<u64>() + dry.to_sea, 0);
    }
}
