//! Fill-Spill-Merge (Barnes, Callaghan and Wickert 2020) with a water balance by area. A
//! depression's catchment supplies Σ r over its cells (thousandths of a cell); a lake of that
//! many cells evaporates it. A depression that cannot hold its supply below its spill fills,
//! and the rest overflows: into its sibling, into its parent when both are full, or (top-level)
//! across its outlet into the tree beyond. Trees are taken in reverse of the order they joined
//! the ocean, so overflow always runs into a tree not yet taken. All amounts are integers.

use super::LAKE_DEPTH_M;
use super::hierarchy::{Hierarchy, NONE};
use super::store::{Blob, Dec, Enc};
use super::table::MILLI;
use super::watershed::add_below0;
use crate::acquisition::{Result, ensure};
use std::collections::BTreeMap;

/// Metres as integer millimetres.
pub(crate) fn mm(z: f32) -> i64 {
    (f64::from(z) * 1000.0).round() as i64
}

/// The spill of each leaf's top-level ancestor (cells above it are in no depression).
pub(crate) fn top_spill(h: &Hierarchy) -> Vec<f32> {
    let mut top = vec![0.0f32; h.len()];
    for d in (0..h.len()).rev() {
        let p = h.parent[d];
        top[d] = if p == NONE {
            h.spill[d]
        } else {
            top[p as usize]
        };
    }
    top.truncate(h.leaves);
    top
}

/// The lowest depression on `leaf`'s chain whose spill is at or above `z`; NONE above all.
/// A NaN `z` is held by none, so a counting caller skips the cell.
pub(crate) fn holding(h: &Hierarchy, top: &[f32], leaf: u32, z: f32) -> u32 {
    if leaf == NONE || z.is_nan() || z > top[leaf as usize] {
        return NONE;
    }
    let mut d = leaf;
    while h.spill[d as usize] < z {
        d = h.parent[d as usize];
    }
    d
}

/// A tile's cells by holding depression: count, Σ ground (mm), bbox [x0, y0, x1, y1].
pub(crate) type TileCounts = BTreeMap<u32, (u64, i64, [u64; 4])>;

pub(crate) fn count(t: &mut TileCounts, d: u32, z: f32, x: u64, y: u64) {
    let e = t.entry(d).or_insert((0, 0, [u64::MAX, u64::MAX, 0, 0]));
    e.0 += 1;
    e.1 += mm(z);
    e.2 = [e.2[0].min(x), e.2[1].min(y), e.2[2].max(x), e.2[3].max(y)];
}

fn union(a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Counts {
    pub cells: Vec<u64>,
    pub sum_mm: Vec<i64>,
    pub bbox: Vec<[u64; 4]>,
}

impl Counts {
    pub fn dense(nd: usize, tiles: &[TileCounts]) -> Self {
        let mut c = Counts {
            cells: vec![0; nd],
            sum_mm: vec![0; nd],
            bbox: vec![[u64::MAX, u64::MAX, 0, 0]; nd],
        };
        for t in tiles {
            for (&d, &(n, s, b)) in t {
                let d = d as usize;
                c.cells[d] += n;
                c.sum_mm[d] += s;
                c.bbox[d] = union(c.bbox[d], b);
            }
        }
        c
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Balance {
    /// In a tree that joins the ocean at or below 0 m.
    pub sea: Vec<bool>,
    /// Cells at or below the spill, over the subtree.
    pub n: Vec<u64>,
    /// Volume to the spill, mm × cells.
    pub vfill_mm: Vec<i64>,
    pub bbox: Vec<[u64; 4]>,
    pub full: Vec<bool>,
    /// Water held at this depression's own level (beyond its children's capacities).
    pub own: Vec<u64>,
    /// The depression whose water covers this one's cells (itself or its top wet ancestor).
    pub node: Vec<u32>,
    /// Water level of node depressions (NaN elsewhere, and for closed nodes until set).
    pub level: Vec<f32>,
    /// Closed wet nodes: how many cells their level covers.
    pub need: Vec<u64>,
    /// Supply that reached the sea or left the map.
    pub to_sea: u64,
    /// Overflow poured into a leaf from outside it: (leaf, the outlet cell it entered by,
    /// amount), sorted. A contracted super-leaf (Task 8b) re-solves its inside from these.
    /// Amounts into a target that ends full depend on the leaf order (canonical, so they are
    /// deterministic); only pours into targets that end not full are order-free.
    pub pours: Vec<(u32, u64, u64)>,
}

struct Fsm<'a> {
    h: &'a Hierarchy,
    cap: Vec<u64>,
    sea: Vec<bool>,
    top_of: Vec<u32>,
    taken: Vec<bool>,
    full: Vec<bool>,
    own: Vec<u64>,
    pending: Vec<u64>,
    to_sea: u64,
    pours: Vec<(u32, u64, u64)>,
}

impl Fsm<'_> {
    fn pour(&mut self, mut d: u32, mut w: u64) -> Result<()> {
        let h = self.h;
        while w > 0 {
            let du = d as usize;
            if !self.full[du] {
                let kids: u64 = if h.is_leaf(d) {
                    0
                } else {
                    h.children[du].iter().map(|&c| self.cap[c as usize]).sum()
                };
                let room = self.cap[du] - kids - self.own[du];
                if w < room {
                    self.own[du] += w;
                    return Ok(());
                }
                self.own[du] += room;
                w -= room;
                self.full[du] = true;
                // An exact fill passes nothing on.
                if w == 0 {
                    return Ok(());
                }
            }
            let p = h.parent[du];
            if p == NONE {
                let g = h.geolink[du];
                if g == NONE || self.sea[self.top_of[g as usize] as usize] {
                    self.to_sea += w;
                } else {
                    ensure(
                        !self.taken[self.top_of[g as usize] as usize],
                        format!("depression {d} overflows into a tree already taken"),
                    )?;
                    self.pending[g as usize] += w;
                    self.pours.push((g, h.outlet[du][1], w));
                }
                return Ok(());
            }
            let [a, b] = h.children[p as usize];
            let sibling = if a == d { b } else { a };
            d = if self.full[sibling as usize] {
                p
            } else {
                self.pours.push((h.geolink[du], h.outlet[du][1], w));
                h.geolink[du]
            };
        }
        Ok(())
    }
}

/// The coastal water table (B13, review I4): a depression holding a sub-zero cell that may
/// not keep a lake below sea level stands at least at its lowest ground at or above 0 m: one
/// cell more than its sub-zero cells (all of which are its lowest). Never adds water: a
/// node whose k reaches its capacity stands at its spill and passes nothing on.
pub(crate) fn coastal_floor(k_balance: u64, sub_zero: u64, bad: u64) -> u64 {
    if bad > 0 {
        k_balance.max(sub_zero + 1)
    } else {
        k_balance
    }
}

/// `sea_at_zero`: top-level trees spilling at or below 0 m are sea (the world's hierarchy);
/// false for a tile's settled fragment, whose tops are not top-level in the world (Task 8b).
pub(crate) fn solve(h: &Hierarchy, c: &Counts, sea_at_zero: bool) -> Result<Balance> {
    let nd = h.len();
    let (mut n, mut sum, mut bbox) = (c.cells.clone(), c.sum_mm.clone(), c.bbox.clone());
    ensure(
        h.below0.len() == h.leaves,
        "below-zero counts do not match the leaves",
    )?;
    // Below-zero counts over each depression's own leaves and children, never through
    // overflow (B13, preflight F19).
    let mut below0 = h.below0.clone();
    below0.resize(nd, (0, 0));
    // Children come before their parents.
    for d in 0..nd {
        let p = h.parent[d];
        if p != NONE {
            let p = p as usize;
            n[p] += n[d];
            sum[p] += sum[d];
            bbox[p] = union(bbox[p], bbox[d]);
            let b = below0[d];
            add_below0(&mut below0[p], b);
        }
    }
    let kids_of = |d: usize, n: &[u64]| -> u64 {
        if h.is_leaf(d as u32) {
            0
        } else {
            h.children[d].iter().map(|&k| n[k as usize]).sum()
        }
    };
    let vfill_mm = (0..nd)
        .map(|d| mm(h.spill[d]) * n[d] as i64 - sum[d])
        .collect();
    let mut sea = vec![false; nd];
    let mut top_of = vec![0u32; nd];
    for d in (0..nd).rev() {
        let p = h.parent[d];
        (sea[d], top_of[d]) = if p == NONE {
            (sea_at_zero && h.spill[d] <= 0.0, d as u32)
        } else {
            (sea[p as usize], top_of[p as usize])
        };
    }
    let mut by_top: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for l in 0..h.leaves as u32 {
        by_top.entry(top_of[l as usize]).or_default().push(l);
    }
    let mut f = Fsm {
        h,
        cap: n.iter().map(|&k| MILLI * k).collect(),
        sea: sea.clone(),
        top_of,
        taken: vec![false; nd],
        // A depression of zero depth (a flat with an outlet) holds no water: it is full from
        // the start and drains on, whatever its r (R32, review N15).
        full: (0..nd).map(|d| !sea[d] && h.spill[d] <= h.low[d]).collect(),
        own: vec![0; nd],
        pending: h.supply.clone(),
        to_sea: 0,
        pours: Vec::new(),
    };
    for &t in h.ocean_order.iter().rev() {
        f.taken[t as usize] = true;
        for l in by_top.remove(&t).unwrap_or_default() {
            let w = std::mem::take(&mut f.pending[l as usize]);
            if sea[t as usize] {
                f.to_sea += w;
            } else {
                f.pour(l, w)?;
            }
        }
    }
    let both_full =
        |d: usize| !h.is_leaf(d as u32) && h.children[d].iter().all(|&k| f.full[k as usize]);
    // The coastal water table makes a depression wet when its floor covers more than its
    // children (a dry basin too); a floor within one child is that child's.
    let floored = |d: usize| {
        let kids = kids_of(d, &n);
        coastal_floor(kids, below0[d].0, below0[d].1) > kids
    };
    let wet = |d: usize| !sea[d] && (f.full[d] || f.own[d] > 0 || both_full(d) || floored(d));
    let mut node = vec![NONE; nd];
    for d in (0..nd).rev() {
        let p = h.parent[d];
        let up = if p == NONE { NONE } else { node[p as usize] };
        node[d] = if up != NONE {
            up
        } else if wet(d) {
            d as u32
        } else {
            NONE
        };
    }
    let mut level = vec![f32::NAN; nd];
    let mut need = vec![0u64; nd];
    for d in 0..nd {
        if node[d] != d as u32 {
            continue;
        }
        if f.full[d] {
            level[d] = h.spill[d];
            continue;
        }
        let kids = kids_of(d, &n);
        let more = f.own[d].div_ceil(MILLI);
        // At most every held cell: a floor above the spill stands at it and spills nothing.
        let k = coastal_floor(kids + more, below0[d].0, below0[d].1).min(n[d]);
        if more == 0 && k == kids {
            level[d] = h.spill[h.children[d][0] as usize];
        } else {
            need[d] = k;
        }
    }
    let supplied: u64 = h.supply.iter().sum();
    ensure(
        supplied == f.own.iter().sum::<u64>() + f.to_sea,
        "the water balance does not close",
    )?;
    let mut pours = f.pours;
    pours.sort_unstable();
    Ok(Balance {
        sea,
        n,
        vfill_mm,
        bbox,
        full: f.full,
        own: f.own,
        node,
        level,
        need,
        to_sea: f.to_sea,
        pours,
    })
}

impl Balance {
    /// Levels of closed wet nodes from their cells' ground (gathered from the tiles): the
    /// `need`-th lowest.
    pub fn set_closed_levels(&mut self, gathered: &[(u32, f32)]) -> Result<()> {
        let mut by: BTreeMap<u32, Vec<f32>> = BTreeMap::new();
        for &(d, z) in gathered {
            by.entry(d).or_default().push(z);
        }
        for (d, mut zs) in by {
            let k = self.need[d as usize] as usize;
            ensure(
                k >= 1 && k <= zs.len(),
                format!("closed basin {d} has {} cells, needs {k}", zs.len()),
            )?;
            zs.sort_unstable_by(f32::total_cmp);
            self.level[d as usize] = zs[k - 1];
        }
        ensure(
            self.need
                .iter()
                .zip(&self.level)
                .all(|(&k, l)| k == 0 || l.is_finite()),
            "a closed basin got no level",
        )
    }

    /// Cells a node's water covers: all held cells when full, the needed cells when closed,
    /// both children's when it stands exactly at their merge saddle.
    pub fn area_cells(&self, h: &Hierarchy, d: u32) -> u64 {
        let d = d as usize;
        if self.full[d] {
            self.n[d]
        } else if self.need[d] > 0 {
            self.need[d]
        } else if !h.is_leaf(d as u32) {
            h.children[d].iter().map(|&k| self.n[k as usize]).sum()
        } else {
            0
        }
    }

    /// A node labelled a lake: at least 0.3 m deep (spec) and at least `min_cells` in area
    /// (`lake_min_m2`, review C2); other nodes are ponds.
    pub fn is_lake(&self, h: &Hierarchy, d: u32, min_cells: u64) -> bool {
        self.level[d as usize] - h.low[d as usize] >= LAKE_DEPTH_M
            && self.area_cells(h, d) >= min_cells
    }
}

impl Blob for Balance {
    fn put(&self, e: &mut Enc) {
        self.sea.put(e);
        self.n.put(e);
        self.vfill_mm.put(e);
        self.bbox.put(e);
        self.full.put(e);
        self.own.put(e);
        self.node.put(e);
        self.level.put(e);
        self.need.put(e);
        self.to_sea.put(e);
        self.pours.put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let (sea, n, vfill_mm, bbox, full) = Blob::get(d)?;
        let (own, node, level, need, to_sea) = Blob::get(d)?;
        let pours = Blob::get(d)?;
        Ok(Self {
            sea,
            n,
            vfill_mm,
            bbox,
            full,
            own,
            node,
            level,
            need,
            to_sea,
            pours,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Leaves 0 and 1 merge at 5 m into meta 2, which spills to the ocean at 7 m. Cells at or
    /// below each spill (by first holding depression): leaf 0: 3, leaf 1: 2, meta 2: 4 more.
    fn handmade(supply: [u64; 2], ocean_spill: f32) -> (Hierarchy, Counts) {
        let h = Hierarchy {
            leaves: 2,
            leaf_pit: vec![10, 20],
            low: vec![1.0, 2.0, 1.0],
            spill: vec![5.0, 5.0, ocean_spill],
            parent: vec![2, 2, NONE],
            children: vec![[NONE, NONE], [NONE, NONE], [0, 1]],
            outlet: vec![[11, 12], [12, 11], [13, 14]],
            geolink: vec![1, 0, NONE],
            ocean_order: vec![2],
            supply: supply.to_vec(),
            below0: vec![(0, 0); 2],
        };
        let c = Counts {
            cells: vec![3, 2, 4],
            sum_mm: vec![6000, 5000, 24000],
            bbox: vec![[0, 0, 1, 1]; 3],
        };
        (h, c)
    }

    #[test]
    fn the_coastal_floor_needs_one_bad_cell_and_never_lowers() {
        assert_eq!(
            coastal_floor(3, 10, 0),
            3,
            "all sub-zero cells hot-dry and far: balance"
        );
        assert_eq!(
            coastal_floor(3, 10, 1),
            11,
            "one cell near the coast floors the basin"
        );
        assert_eq!(coastal_floor(40, 10, 5), 40, "a fuller balance stays");
    }

    /// Leaves 0 and 1 merge at −1 m (all their cells sub-zero) under meta 2, spilling at 3 m;
    /// with no supply the floor makes meta 2 the node, covering one cell more than the six
    /// sub-zero ones (at most every held cell); hot-dry and far, it stays dry.
    #[test]
    fn the_coastal_floor_rises_to_the_depression_that_crosses_sea_level() {
        let (mut h, mut c) = handmade([0, 0], 3.0);
        (h.low, h.spill) = (vec![-4.0, -3.0, -4.0], vec![-1.0, -1.0, 3.0]);
        h.below0 = vec![(4, 4), (2, 1)];
        let b = solve(&h, &c, true).expect("solve");
        assert_eq!(b.node, vec![2, 2, 2]);
        assert_eq!(b.need[2], 7);
        assert_eq!(b.full, vec![false; 3]);
        assert_eq!(b.to_sea, 0, "no water is added");
        c.cells[2] = 1;
        assert_eq!(solve(&h, &c, true).expect("solve").need[2], 6);
        h.below0 = vec![(4, 0), (2, 0)];
        assert_eq!(solve(&h, &c, true).expect("solve").node, vec![NONE; 3]);
    }

    #[test]
    fn a_humid_pair_fills_and_spills_to_the_sea() {
        let (h, c) = handmade([10_000, 10_000], 7.0);
        let b = solve(&h, &c, true).expect("solve");
        assert_eq!(b.n, vec![3, 2, 9]);
        assert_eq!(b.full, vec![true, true, true]);
        assert_eq!(b.node, vec![2, 2, 2]);
        assert_eq!(b.level[2], 7.0);
        assert_eq!(b.to_sea, 20_000 - 9_000);
        assert_eq!(b.vfill_mm[0], 5000 * 3 - 6000);
    }

    #[test]
    fn overflow_runs_into_the_sibling_first() {
        // Leaf 0 holds 3 cells; its extra 2 cells' worth fills leaf 1 exactly: one lake at 5 m.
        let (h, c) = handmade([5_000, 0], 7.0);
        let b = solve(&h, &c, true).expect("solve");
        assert_eq!(b.full, vec![true, true, false]);
        assert_eq!(b.node, vec![2, 2, 2], "both full: one lake at their saddle");
        assert_eq!(b.level[2], 5.0);
        assert_eq!(b.to_sea, 0);
    }

    #[test]
    fn an_arid_basin_stands_where_its_area_meets_its_supply() {
        let (h, c) = handmade([2_500, 0], 7.0);
        let mut b = solve(&h, &c, true).expect("solve");
        assert_eq!(b.full, vec![false, false, false]);
        assert_eq!(b.node, vec![0, NONE, NONE]);
        assert_eq!(b.need[0], 3, "2.5 cells of supply cover 3 cells");
        assert!(b.level[0].is_nan());
        b.set_closed_levels(&[(0, 4.0), (0, 1.0), (0, 2.5)])
            .expect("levels");
        assert_eq!(b.level[0], 4.0);
        assert!(!b.is_lake(&h, 1, 1));
        assert_eq!(b.area_cells(&h, 0), 3);
        assert!(b.is_lake(&h, 0, 3), "3 m deep over 3 cells");
        assert!(
            !b.is_lake(&h, 0, 4),
            "smaller than the lake minimum: a pond"
        );
    }

    #[test]
    fn a_flat_with_an_outlet_is_never_a_closed_basin() {
        // Leaf 1 is a flat at 5 m whose spill is its own level: zero depth, no supply (r = 0).
        let (mut h, mut c) = handmade([0, 0], 7.0);
        h.low[1] = 5.0;
        c.sum_mm[1] = 10_000;
        let b = solve(&h, &c, true).expect("solve");
        assert!(b.full[1], "zero depth: full by definition (R32)");
        assert_eq!(b.node[1], 1, "a node with an outlet, so flow crosses it");
        assert_eq!(b.level[1], 5.0);
        assert!(!b.full[0], "leaf 0 has depth and no water: dry");
    }

    #[test]
    fn overflow_into_a_leaf_is_recorded_with_its_entry_cell() {
        let (h, c) = handmade([5_000, 0], 7.0);
        let b = solve(&h, &c, true).expect("solve");
        assert_eq!(
            b.pours,
            vec![(1, 12, 2_000)],
            "leaf 0's extra entered leaf 1 by cell 12"
        );
    }

    #[test]
    fn no_supply_leaves_a_dry_basin_and_sea_trees_take_everything() {
        let (h, c) = handmade([0, 0], 7.0);
        assert_eq!(solve(&h, &c, true).expect("solve").node, vec![NONE; 3]);
        let (h, c) = handmade([10_000, 1], -1.0);
        let b = solve(&h, &c, true).expect("solve");
        assert_eq!(b.sea, vec![true; 3]);
        assert_eq!(b.node, vec![NONE; 3]);
        assert_eq!(b.to_sea, 10_001);
    }

    #[test]
    fn overflow_crosses_into_a_tree_that_joined_the_ocean_earlier() {
        // Leaf 1 joined the ocean first (spill 2 m); leaf 0 spills at 3 m into leaf 1.
        let h = Hierarchy {
            leaves: 2,
            leaf_pit: vec![10, 20],
            low: vec![1.0, 1.0],
            spill: vec![3.0, 2.0],
            parent: vec![NONE, NONE],
            children: vec![[NONE, NONE]; 2],
            outlet: vec![[11, 21], [22, 23]],
            geolink: vec![1, NONE],
            ocean_order: vec![1, 0],
            supply: vec![5_000, 0],
            below0: vec![(0, 0); 2],
        };
        let c = Counts {
            cells: vec![2, 4],
            sum_mm: vec![0, 0],
            bbox: vec![[0, 0, 0, 0]; 2],
        };
        let b = solve(&h, &c, true).expect("solve");
        assert!(b.full[0]);
        assert!(!b.full[1], "3 cells of overflow do not fill 4");
        assert_eq!(b.need[1], 3);
        assert_eq!(b.to_sea, 0);
    }

    #[test]
    fn an_exact_fill_pours_nothing_on() {
        let (h, c) = handmade([3_000, 0], 7.0);
        let b = solve(&h, &c, true).expect("solve");
        assert_eq!(b.full, vec![true, false, false]);
        assert!(b.pours.is_empty(), "no zero-amount pours");
        assert_eq!((b.node[0], b.level[0], b.to_sea), (0, 5.0, 0));
    }

    #[test]
    fn a_closed_meta_stands_above_its_full_children() {
        // Both leaves fill (5 cells); 1.5 cells' worth stands on the meta: 5 + 2 cells.
        let (h, c) = handmade([6_500, 0], 7.0);
        let mut b = solve(&h, &c, true).expect("solve");
        assert_eq!(b.full, vec![true, true, false]);
        assert_eq!(b.own[2], 1_500);
        assert_eq!(b.node, vec![2, 2, 2]);
        assert_eq!(b.need[2], 7);
        // The subtree's cells: leaf 0, leaf 1, then the meta's own 4.
        let zs = [1.0, 2.0, 3.0, 2.0, 3.0, 7.0, 5.5, 6.5, 6.0];
        let gathered: Vec<(u32, f32)> = zs.iter().map(|&z| (2, z)).collect();
        b.set_closed_levels(&gathered).expect("levels");
        assert_eq!(b.level[2], 6.0, "the 7th lowest of the subtree");
        assert_eq!(b.area_cells(&h, 2), 7);
        assert!(b.is_lake(&h, 2, 7));
        assert!(
            b.set_closed_levels(&gathered[..6]).is_err(),
            "too few cells"
        );
    }

    #[test]
    fn overflow_climbs_from_a_full_leaf_deep_in_the_sibling() {
        // Leaves 0 and 1 merge at 4 m (meta 3), which merges with leaf 2 at 6 m (meta 4,
        // to the ocean at 8 m). Leaf 2 spills into leaf 0, already full: the water climbs to
        // leaf 0's sibling, leaf 1.
        let h = Hierarchy {
            leaves: 3,
            leaf_pit: vec![10, 20, 30],
            low: vec![1.0; 5],
            spill: vec![4.0, 4.0, 6.0, 6.0, 8.0],
            parent: vec![3, 3, 4, 4, NONE],
            children: vec![[NONE, NONE], [NONE, NONE], [NONE, NONE], [0, 1], [3, 2]],
            outlet: vec![[100, 101], [101, 100], [200, 201], [201, 200], [300, 301]],
            geolink: vec![1, 0, 0, 2, NONE],
            ocean_order: vec![4],
            supply: vec![1_000, 0, 2_500],
            below0: vec![(0, 0); 3],
        };
        let c = Counts {
            cells: vec![1, 2, 1, 1, 2],
            sum_mm: vec![1_000, 3_000, 1_000, 5_000, 14_000],
            bbox: vec![[0, 0, 1, 1]; 5],
        };
        let b = solve(&h, &c, true).expect("solve");
        assert_eq!(b.full, vec![true, false, true, false, false]);
        assert_eq!(b.own[1], 1_500);
        assert_eq!(b.pours, vec![(0, 201, 1_500), (1, 101, 1_500)]);
        assert_eq!(b.node, vec![0, 1, 2, NONE, NONE]);
        assert_eq!(b.need[1], 2);
        assert_eq!((b.level[0], b.level[2]), (4.0, 6.0));
        assert_eq!(b.to_sea, 0);
    }

    #[test]
    fn a_flat_passes_its_inflow_on_to_its_sibling() {
        let (mut h, mut c) = handmade([0, 1_500], 7.0);
        h.low[1] = 5.0;
        c.sum_mm[1] = 10_000;
        let b = solve(&h, &c, true).expect("solve");
        assert_eq!(b.full, vec![false, true, false]);
        assert_eq!(b.pours, vec![(0, 11, 1_500)]);
        assert_eq!((b.own[0], b.own[1]), (1_500, 0));
        assert_eq!((b.node[0], b.need[0]), (0, 2));
        assert_eq!((b.node[1], b.level[1]), (1, 5.0));
    }

    #[test]
    fn without_sea_at_zero_a_low_tree_holds_water() {
        // The handmade pair 6 m lower: the whole tree lies below 0 m.
        let (mut h, c) = handmade([10_000, 10_000], -1.0);
        (h.low, h.spill) = (vec![-5.0, -4.0, -5.0], vec![-3.0, -3.0, -1.0]);
        let b = solve(&h, &c, false).expect("solve");
        assert_eq!(b.sea, vec![false; 3]);
        assert_eq!(b.full, vec![true; 3]);
        assert_eq!((b.node[2], b.level[2]), (2, -1.0));
        assert_eq!(b.to_sea, 11_000);
        assert_eq!(solve(&h, &c, true).expect("solve").node, vec![NONE; 3]);
    }

    #[test]
    fn overflow_into_a_taken_tree_is_refused() {
        // The ocean order says leaf 0 was taken first, so leaf 0's overflow meets a taken tree.
        let h = Hierarchy {
            leaves: 2,
            leaf_pit: vec![10, 20],
            low: vec![1.0, 1.0],
            spill: vec![3.0, 2.0],
            parent: vec![NONE, NONE],
            children: vec![[NONE, NONE]; 2],
            outlet: vec![[11, 21], [22, 23]],
            geolink: vec![1, NONE],
            ocean_order: vec![0, 1],
            supply: vec![5_000, 0],
            below0: vec![(0, 0); 2],
        };
        let c = Counts {
            cells: vec![2, 4],
            sum_mm: vec![0, 0],
            bbox: vec![[0, 0, 0, 0]; 2],
        };
        assert!(solve(&h, &c, true).is_err());
    }

    #[test]
    fn balance_round_trips() {
        use crate::world::water::store::{decode, encode};
        let (h, c) = handmade([5_000, 0], 7.0);
        let b = solve(&h, &c, true).expect("solve");
        // Levels hold NaN, so compare the encodings.
        let back: Balance = decode(&encode(&b)).expect("decode");
        assert_eq!(encode(&back), encode(&b));
        assert_eq!(back.pours, b.pours);
    }

    #[test]
    fn holding_finds_the_first_spill_above() {
        let (h, _) = handmade([0, 0], 7.0);
        let top = top_spill(&h);
        assert_eq!(top, vec![7.0, 7.0]);
        assert_eq!(holding(&h, &top, 0, 4.0), 0);
        assert_eq!(holding(&h, &top, 0, 6.0), 2);
        assert_eq!(holding(&h, &top, 0, 7.5), NONE);
        assert_eq!(holding(&h, &top, NONE, 0.0), NONE);
        assert_eq!(holding(&h, &top, 0, f32::NAN), NONE);
    }
}
