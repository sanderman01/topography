//! One run of the water model over tiles: pass A (label graphs, settled trees contracted) →
//! global hierarchy → pass B (cells by holding depression) → Fill-Spill-Merge → pass L (the
//! inside of settled trees, solved in their tile) → pass L2 (the other settled trees, from
//! the cells and pours of every tile) → pass C (ground of closed basins) → levels and the
//! node table → pass D (ponds that cross tiles) → their receivers → pass E (flow reports) →
//! the exact port solve → pass F (write the rasters). A window is the same run with one tile;
//! water nodes are named by their smallest pit (R28), so nothing depends on the split.

use super::Params;
use super::balance::{Balance, Counts, TileCounts, count, holding, solve, top_spill};
use super::contract::{Frag, Tops, contract, min_pits, solve_local, with_spills};
use super::flats;
use super::flow::{Next, Report, Solved, accumulate, add_inflows, ends, solve as solve_flow};
use super::geom::{Rect, World, tiles};
use super::hierarchy::{Hierarchy, NONE, Resolver, build};
use super::layers::{Classes, Layer};
use super::nodes::{Node, Nodes};
use super::rain::{Q_PER_KM2, Rain};
use super::store::{Blob, Dec, Enc, Work, cached, for_tiles};
use super::table::Table;
use super::watershed::{Drain, TileGraph, drain, tile_graph};
use crate::acquisition::{AcquisitionError, Result, ensure};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

fn invalid(msg: impl Into<String>) -> AcquisitionError {
    AcquisitionError::Invalid(msg.into())
}

pub(crate) struct Setup {
    pub world: World,
    /// North-west corner of the grid in the layout plane (x east, y north).
    pub nw_m: [f64; 2],
    pub spacing_m: f64,
    pub tile: u64,
    pub threads: usize,
    pub classes: Option<Classes>,
    pub table: Table,
    pub rain: Rain,
    pub params: Params,
    /// Settle depression trees inside tiles (review C3); exact, so off only in its test.
    pub contract: bool,
}

impl Setup {
    pub fn tiles(&self) -> Vec<Rect> {
        tiles(self.world, self.tile)
    }
    pub fn cell_km2(&self) -> f64 {
        self.spacing_m * self.spacing_m / 1e6
    }
    /// The smallest labelled lake in cells (`lake_min_m2`).
    pub fn lake_min_cells(&self) -> u64 {
        (self.table.lake_min_m2 / (self.spacing_m * self.spacing_m)).ceil() as u64
    }
    pub fn centre(&self, x: u64, y: u64) -> (f64, f64) {
        (
            self.nw_m[0] + (x as f64 + 0.5) * self.spacing_m,
            self.nw_m[1] - (y as f64 + 0.5) * self.spacing_m,
        )
    }
    pub fn rain_units(&self, core: Rect) -> Result<Vec<u64>> {
        (0..core.cells())
            .map(|k| {
                let (x, y) = core.world_xy(k);
                let (cx, cy) = self.centre(x, y);
                self.rain.units(cx, cy, self.cell_km2())
            })
            .collect()
    }
    /// Each cell's r in thousandths (class 0 without classes).
    pub fn supply(&self, core: Rect) -> Result<Vec<u64>> {
        Ok(match &self.classes {
            Some(c) => c
                .read(core)?
                .into_iter()
                .map(|k| u64::from(self.table.r_milli(k)))
                .collect(),
            None => vec![u64::from(self.table.r_milli(0)); core.cells()],
        })
    }
    /// River start in km² as f32, compared with `discharge` everywhere.
    pub fn river_km2(&self) -> f32 {
        self.params.river_km2 as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Wet {
    Dry,
    Sea,
    Lake(u32),
    Pond(u32),
}

/// Ring cells whose in-tile path leaves their tile, with the cell it enters (sorted by ring
/// cell): a path is followed across tiles ring cell by ring cell (B1).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Exits(pub Vec<(u64, u64)>);

impl Exits {
    fn new(graphs: &[TileGraph]) -> Self {
        let mut v: Vec<(u64, u64)> = graphs
            .iter()
            .flat_map(|g| {
                g.ring.iter().filter_map(|r| match r.end {
                    Next::Out(c) => Some((r.cell, c)),
                    _ => None,
                })
            })
            .collect();
        v.sort_unstable();
        Self(v)
    }
    /// The ring cell where a path that enters a tile at `c` stops leaving tiles.
    /// Errs on a cycle (possible only with stale cached graphs).
    pub fn entry(&self, mut c: u64) -> Result<u64> {
        for _ in 0..=self.0.len() {
            match self.0.binary_search_by_key(&c, |e| e.0) {
                Ok(i) => c = self.0[i].1,
                Err(_) => return Ok(c),
            }
        }
        Err(invalid(format!("flow cycles across tiles at cell {c}")))
    }
}

/// The r supply of paths that leave their tile, by the ring cell where they stop leaving
/// tiles (sorted, summed): a settled tree's water from cells outside its tile's own (B1).
fn foreign_supply(graphs: &[TileGraph], exits: &Exits) -> Result<Vec<(u64, u64)>> {
    let mut v: Vec<(u64, u64)> = graphs
        .iter()
        .flat_map(|g| {
            g.supply.iter().filter_map(|(&t, &s)| match t {
                Next::Out(c) => Some(exits.entry(c).map(|e| (e, s))),
                _ => None,
            })
        })
        .collect::<Result<_>>()?;
    v.sort_unstable_by_key(|e| e.0);
    let mut out: Vec<(u64, u64)> = Vec::with_capacity(v.len());
    for (c, s) in v {
        match out.last_mut() {
            Some(last) if last.0 == c => last.1 += s,
            _ => out.push((c, s)),
        }
    }
    Ok(out)
}

/// The water of a run (global).
pub(crate) struct Water {
    pub h: Hierarchy,
    pub resolver: Resolver,
    pub top: Vec<f32>,
    pub bal: Balance,
    /// Each global depression's smallest pit: its node key.
    pub minpit: Vec<u64>,
    /// Every node, global and settled in tiles, by key.
    pub nodes: Nodes,
    /// Keys of super-leaves whose inside was solved apart, in their tile or in pass L2
    /// (sorted); they are no global nodes.
    pub solved: Vec<u64>,
    /// Trees solved in pass L2 from cells of every tile (B2), and their index by tree key.
    pub remote: Vec<Local>,
    pub remote_of: BTreeMap<u64, u32>,
    pub exits: Exits,
}

/// A tile's settled trees (pass A) and the inside of those it solves (pass L).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Local {
    pub frag: Frag,
    /// `frag.h` with the solved trees' top spills set (others −∞).
    pub h: Hierarchy,
    pub top: Vec<f32>,
    pub bal: Balance,
    pub minpit: Vec<u64>,
    /// Trees solved in this `Local`: tree key → the super-leaf's global spill and spill edge.
    pub solved: Tops,
}

impl Local {
    /// A drainage end with settled pits replaced by their tree's key (what the resolver knows).
    pub fn remap(&self, t: Next) -> Next {
        match t {
            Next::Node(p) => self
                .frag
                .leaf(p)
                .map_or(t, |l| Next::Node(self.frag.tree[l as usize])),
            other => other,
        }
    }
    /// The local leaf a cell's flow reaches in this fragment, from the cell's own in-tile end
    /// (pre-contraction), in any tile: an `Out` path is followed ring cell by ring cell until
    /// it stops leaving tiles; if that ring cell drains to a pit of this fragment, its leaf
    /// (B1). None when the path ends elsewhere.
    pub fn leaf_of_end(&self, exits: &Exits, end: Next) -> Result<Option<u32>> {
        Ok(match end {
            Next::Node(p) => self.frag.leaf(p),
            Next::Out(c) => self.frag.ring_leaf(exits.entry(c)?),
            Next::Cell(_) | Next::End => None,
        })
    }
    /// (held, node key and level) of a cell at `z` on local leaf `l` of a tree solved here.
    fn water_at(&self, l: u32, z: f32) -> (bool, Option<(u64, f32)>) {
        water_in(&self.h, &self.top, &self.bal, &self.minpit, l, z)
    }
}

/// (held, node key and level) of a cell at `z` on `leaf` of a hierarchy (N16: one helper for
/// the global, in-tile and remote cases).
fn water_in(
    h: &Hierarchy,
    top: &[f32],
    bal: &Balance,
    minpit: &[u64],
    leaf: u32,
    z: f32,
) -> (bool, Option<(u64, f32)>) {
    let h0 = holding(h, top, leaf, z);
    let nd = if h0 == NONE {
        NONE
    } else {
        bal.node[h0 as usize]
    };
    (
        h0 != NONE,
        (nd != NONE).then(|| (minpit[nd as usize], bal.level[nd as usize])),
    )
}

impl Blob for Local {
    fn put(&self, e: &mut Enc) {
        self.frag.put(e);
        self.h.put(e);
        self.top.put(e);
        self.bal.put(e);
        self.minpit.put(e);
        self.solved.put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let (frag, h, top, bal, minpit) = Blob::get(d)?;
        Ok(Self {
            frag,
            h,
            top,
            bal,
            minpit,
            solved: Blob::get(d)?,
        })
    }
}

pub(crate) struct Model {
    pub water: Water,
    /// Receivers of full ponds that cross tiles, by world cell.
    pub ponds: HashMap<u64, u8>,
    pub flow: Solved,
    /// Each tile's `Local` (runs without a work directory; with one, they are read back).
    pub locals: Vec<Local>,
}

impl Model {
    pub fn local(&self, work: Option<&Work>, tag: &str, i: usize) -> Result<Local> {
        match work {
            Some(w) => Ok(w
                .cached::<(Local, Nodes, Vec<u64>)>(&format!("{tag}/l/{i:05}"), || {
                    Err(invalid("pass L result missing from the work directory"))
                })?
                .0),
            None => Ok(self.locals[i].clone()),
        }
    }
}

pub(crate) struct Rasters {
    pub level: Layer<f32>,
    pub node: Layer<u32>,
    pub discharge: Layer<f32>,
    pub flow: Layer<u8>,
}

impl Rasters {
    pub fn memory(world: World) -> Self {
        Self {
            level: Layer::filled(world, f32::NAN),
            node: Layer::filled(world, 0),
            discharge: Layer::filled(world, 0.0),
            flow: Layer::filled(world, 0),
        }
    }
    pub fn files(world: World, dir: &Path, tag: &str) -> Result<Self> {
        Ok(Self {
            level: Layer::open_or_create(world, &dir.join(format!("{tag}-level.f32")), f32::NAN)?,
            node: Layer::open_or_create(world, &dir.join(format!("{tag}-node.u32")), 0)?,
            discharge: Layer::open_or_create(
                world,
                &dir.join(format!("{tag}-discharge.f32")),
                0.0,
            )?,
            flow: Layer::open_or_create(world, &dir.join(format!("{tag}-flow.u8")), 0)?,
        })
    }
}

pub(crate) struct TileState {
    pub core: Rect,
    pub z: Vec<f32>,
    pub dir: Vec<u8>,
    pub wet: Vec<Wet>,
    /// Inside a depression (held), wet or not.
    pub held: Vec<bool>,
    pub level: Vec<f32>,
}

/// The `core` part of a window's values (shared by every pass that reads a halo, N16).
pub(crate) fn crop<T: Copy>(win: Rect, zw: &[T], core: Rect) -> Vec<T> {
    (0..core.cells())
        .map(|k| {
            let (x, y) = core.world_xy(k);
            zw[win.local(x, y).expect("core in window")]
        })
        .collect()
}

fn inside(b: [u64; 4], core: Rect) -> bool {
    core.contains(b[0], b[1]) && core.contains(b[2], b[3])
}

/// Every core cell's water: sea (fill level at or below 0 m from the world edge), a node's
/// water (lake or pond) where the ground is at or below its level, else dry. A cell held by a
/// super-leaf solved apart (in its tile or in pass L2) uses that tree's local hierarchy; all
/// others the global one.
pub(crate) fn state(
    s: &Setup,
    w: &Water,
    local: &Local,
    core: Rect,
    win: Rect,
    zw: &[f32],
) -> Result<TileState> {
    let d = drain(s.world, win, zw, core);
    let z = crop(win, zw, core);
    let n = core.cells();
    let (mut wet, mut held, mut level) = (vec![Wet::Dry; n], vec![false; n], vec![f32::NAN; n]);
    for k in 0..n {
        let end = d.end[k];
        let leaf = w.resolver.leaf_of(local.remap(end));
        if z[k] <= 0.0 && (leaf == NONE || w.top[leaf as usize] <= 0.0) {
            (wet[k], level[k]) = (Wet::Sea, 0.0);
            continue;
        }
        let h0 = holding(&w.h, &w.top, leaf, z[k]);
        let key = (h0 != NONE && h0 == leaf).then(|| w.h.leaf_pit[leaf as usize]);
        let (is_held, water) = match key.filter(|k| w.solved.binary_search(k).is_ok()) {
            Some(key) => {
                let t = match w.remote_of.get(&key) {
                    Some(&r) => &w.remote[r as usize],
                    None if local.solved.contains_key(&key) => local,
                    None => {
                        return Err(invalid(format!(
                            "a cell held by tree {key} lies outside the tile that solved it"
                        )));
                    }
                };
                let l = t.leaf_of_end(&w.exits, end)?.ok_or_else(|| {
                    invalid(format!(
                        "a cell held by tree {key} does not reach its fragment"
                    ))
                })?;
                t.water_at(l, z[k])
            }
            None => water_in(&w.h, &w.top, &w.bal, &w.minpit, leaf, z[k]),
        };
        held[k] = is_held;
        if let Some((key, lv)) = water
            && z[k] <= lv
        {
            let o = w
                .nodes
                .ordinal(key)
                .ok_or_else(|| invalid(format!("node {key} is missing from the node table")))?;
            level[k] = lv;
            wet[k] = if w.nodes.at(o).lake {
                Wet::Lake(o)
            } else {
                Wet::Pond(o)
            };
        }
    }
    Ok(TileState {
        core,
        z,
        dir: d.dir,
        wet,
        held,
        level,
    })
}

/// Receivers (local index → direction byte) of the full ponds in a tile: from the gathered
/// ones, else solved here when the whole pond lies in the tile. Every full pond cell gets one.
fn flats_for(
    s: &Setup,
    w: &Water,
    st: &TileState,
    gathered: &HashMap<u64, u8>,
) -> Result<HashMap<u32, u8>> {
    let mut out = HashMap::new();
    let mut local: BTreeMap<u32, Vec<(u64, f32)>> = BTreeMap::new();
    let mut want = 0usize;
    for k in 0..st.core.cells() {
        let Wet::Pond(o) = st.wet[k] else { continue };
        let node = w.nodes.at(o);
        if !node.full {
            continue;
        }
        want += 1;
        let (x, y) = st.core.world_xy(k);
        let gi = s.world.index(x, y);
        if let Some(&d) = gathered.get(&gi) {
            out.insert(k as u32, d);
        } else if inside(node.bbox, st.core) {
            local.entry(o).or_default().push((gi, st.z[k]));
        }
    }
    for (o, cells) in local {
        for (c, d) in flats::resolve(s.world, &cells, w.nodes.at(o).outlet[1])? {
            let (x, y) = s.world.xy(c);
            out.insert(st.core.local(x, y).expect("pond in tile") as u32, d);
        }
    }
    ensure(
        out.len() == want && out.values().all(|&d| d != 0),
        "a full pond cell has no receiver",
    )?;
    Ok(out)
}

/// The receiver byte a cell's flow takes (0 where it ends in a node, the sea or a pit).
fn byte_of(w: &Water, st: &TileState, flat: &HashMap<u32, u8>, k: usize) -> u8 {
    match st.wet[k] {
        Wet::Dry => st.dir[k],
        Wet::Pond(o) if w.nodes.at(o).full => flat[&(k as u32)],
        _ => 0,
    }
}

/// What the global step tells pass L and L2 about the settled trees.
struct Trees<'a> {
    h: &'a Hierarchy,
    resolver: &'a Resolver,
    bal: &'a Balance,
    exits: &'a Exits,
    foreign: &'a [(u64, u64)],
    min_cells: u64,
}

impl Trees<'_> {
    fn leaf(&self, key: u64) -> Result<usize> {
        self.resolver
            .pit_leaf(key)
            .map(|l| l as usize)
            .ok_or_else(|| invalid(format!("tree {key} is no global leaf")))
    }
    /// Overflow poured into global leaf `sl`: (the cell it enters by, amount).
    fn pours_into(&self, sl: u32) -> impl Iterator<Item = (u64, u64)> + '_ {
        let p = &self.bal.pours;
        let from = p.partition_point(|x| x.0 < sl);
        p[from..]
            .iter()
            .take_while(move |x| x.0 == sl)
            .map(|x| (x.1, x.2))
    }
    /// The trees of `frag` whose inside must be solved apart: more than one pit, and a
    /// super-leaf that is its own node without being full (otherwise its water is the
    /// super-leaf's, or an ancestor's, as uncontracted). Sea trees are never nodes.
    fn wanted(&self, frag: &Frag) -> Result<Tops> {
        let mut out = Tops::new();
        for d in 0..frag.h.len() {
            if frag.h.parent[d] != NONE || frag.size[d] < 2 {
                continue;
            }
            let sl = self.leaf(frag.tree[d])?;
            if self.bal.node[sl] == sl as u32 && !self.bal.full[sl] {
                out.insert(frag.tree[d], (self.h.spill[sl], self.h.outlet[sl]));
            }
        }
        Ok(out)
    }
    /// Cells of a tile held by the trees `pick` names (by key), at or below their
    /// super-leaf's spill: (key, local leaf, z, x, y).
    #[allow(clippy::type_complexity)]
    fn cells<'b>(
        &self,
        own: &Local,
        core: Rect,
        win: Rect,
        zw: &[f32],
        d: &Drain,
        pick: impl Fn(u64) -> Option<&'b Local>,
    ) -> Result<Vec<(u64, u32, f32, u64, u64)>> {
        let mut out = Vec::new();
        for k in 0..core.cells() {
            let sl = self.resolver.leaf_of(own.remap(d.end[k]));
            if sl == NONE {
                continue;
            }
            let key = self.h.leaf_pit[sl as usize];
            let Some(t) = pick(key) else { continue };
            let (x, y) = core.world_xy(k);
            let z = zw[win.local(x, y).expect("core in window")];
            if z <= self.h.spill[sl as usize] {
                let l = t.leaf_of_end(self.exits, d.end[k])?.ok_or_else(|| {
                    invalid(format!(
                        "a cell held by tree {key} does not reach its fragment"
                    ))
                })?;
                out.push((key, l, z, x, y));
            }
        }
        Ok(out)
    }

    /// Solves the listed trees of one fragment from the cells they hold (local leaf, z, x, y;
    /// from every tile) and the water poured in (local leaf, amount). Each leaf's supply is its
    /// own tile's cells plus the paths of other cells that enter the tile through its ring
    /// (B1); per tree that must equal the super-leaf's global supply. Returns the `Local` and
    /// the nodes of the listed trees.
    fn solve(
        &self,
        frag: Frag,
        tops: Tops,
        cells: &[(u32, f32, u64, u64)],
        pours: &[(u32, u64)],
    ) -> Result<(Local, Nodes)> {
        let mut extra: Vec<(u32, u64)> = Vec::new();
        let mut supply: BTreeMap<u64, u64> = BTreeMap::new();
        for l in 0..frag.h.leaves {
            if tops.contains_key(&frag.tree[l]) {
                *supply.entry(frag.tree[l]).or_insert(0) += frag.h.supply[l];
            }
        }
        for &(cell, pit) in &frag.ring_pit {
            let l = frag
                .leaf(pit)
                .ok_or_else(|| invalid(format!("ring pit {pit} is no candidate of its tile")))?;
            let key = frag.tree[l as usize];
            if !tops.contains_key(&key) {
                continue;
            }
            if let Ok(i) = self.foreign.binary_search_by_key(&cell, |f| f.0) {
                extra.push((l, self.foreign[i].1));
                *supply.entry(key).or_insert(0) += self.foreign[i].1;
            }
        }
        for (&key, &got) in &supply {
            let want = self.h.supply[self.leaf(key)?];
            ensure(
                got == want,
                format!("tree {key} gets supply {got} through its tile, {want} globally"),
            )?;
        }
        extra.extend_from_slice(pours);

        let lh = with_spills(&frag, &tops);
        let ltop = top_spill(&lh);
        let mut t = TileCounts::new();
        let mut held = Vec::with_capacity(cells.len());
        for &(l, z, x, y) in cells {
            let h0 = holding(&lh, &ltop, l, z);
            ensure(h0 != NONE, "a gathered cell is held by no depression")?;
            count(&mut t, h0, z, x, y);
            held.push((h0, z));
        }
        let mut lbal = solve_local(&frag, &tops, &extra, &Counts::dense(lh.len(), &[t]))?;
        let closed: Vec<(u32, f32)> = held
            .iter()
            .filter_map(|&(h0, z)| {
                let nd = lbal.node[h0 as usize];
                (nd != NONE && lbal.need[nd as usize] > 0).then_some((nd, z))
            })
            .collect();
        lbal.set_closed_levels(&closed)?;
        let minpit = min_pits(&lh);
        let nodes = (0..lh.len() as u32)
            .filter(|&d| lbal.node[d as usize] == d && tops.contains_key(&frag.tree[d as usize]))
            .map(|d| node_of(&lh, &lbal, &minpit, d, self.min_cells))
            .collect();
        let local = Local {
            frag,
            h: lh,
            top: ltop,
            bal: lbal,
            minpit,
            solved: tops,
        };
        Ok((local, Nodes(nodes)))
    }
}

/// A node depression's row in the node table.
fn node_of(h: &Hierarchy, b: &Balance, minpit: &[u64], d: u32, min_cells: u64) -> Node {
    let du = d as usize;
    Node {
        key: minpit[du],
        level: b.level[du],
        low: h.low[du],
        full: b.full[du],
        lake: b.is_lake(h, d, min_cells),
        outlet: h.outlet[du],
        cells: b.area_cells(h, d),
        vfill_mm: b.vfill_mm[du],
        need: b.need[du],
        bbox: b.bbox[du],
    }
}

fn next_of(s: &Setup, w: &Water, st: &TileState, flat: &HashMap<u32, u8>, k: usize) -> Next {
    let (x, y) = st.core.world_xy(k);
    match (byte_of(w, st, flat, k), st.wet[k]) {
        (0, Wet::Lake(o) | Wet::Pond(o)) => Next::Node(w.nodes.at(o).key),
        (0, _) => Next::End,
        (d, _) => {
            let (nx, ny) = s
                .world
                .neighbour(x, y, d as usize - 1)
                .expect("receiver inside");
            match st.core.local(nx, ny) {
                Some(j) => Next::Cell(j as u32),
                None => Next::Out(s.world.index(nx, ny)),
            }
        }
    }
}

/// What pass L keeps per tile: the tree keys solved here, those left to pass L2, the nodes
/// and the `Local` (default with a work directory, which holds it).
type PartL = (Vec<u64>, Vec<u64>, Vec<Node>, Local);
/// What pass L2 gathers in one tile: cells (key, local leaf, z, x, y) and pours (key, local
/// leaf, amount) of the trees left to it.
type Gathered = (Vec<(u64, u32, f32, u64, u64)>, Vec<(u64, u32, u64)>);

pub(crate) fn model(
    s: &Setup,
    ground: &Layer<f32>,
    work: Option<&Work>,
    tag: &str,
    out: Option<&Rasters>,
) -> Result<Model> {
    let tiles = s.tiles();
    let world = s.world;
    ensure(ground.world() == world, "the ground is not the run's world")?;
    let read = |core: Rect| -> Result<(Rect, Vec<f32>)> {
        let win = core.grow(1, world);
        Ok((win, ground.read(win)?))
    };
    let missing = |what: &str| invalid(format!("{what} missing from the work directory"));

    // A: label graphs; trees settled in the tile become super-leaves (Task 8b).
    let parts: Vec<(TileGraph, Frag)> =
        for_tiles(&tiles, s.threads, &format!("{tag} A"), |i, core| {
            let (g, frag) = cached(work, &format!("{tag}/a/{i:05}"), || {
                let (win, zw) = read(core)?;
                let supply = s.supply(core)?;
                let d = drain(world, win, &zw, core);
                let g = tile_graph(world, core, &crop(win, &zw, core), &d, |k| supply[k]);
                Ok(if s.contract {
                    contract(g)
                } else {
                    (g, Frag::default())
                })
            })?;
            // With a work directory fragments are read back when needed; keep memory small.
            Ok((
                g,
                if work.is_some() {
                    Frag::default()
                } else {
                    frag
                },
            ))
        })?;
    let (graphs, frags): (Vec<TileGraph>, Vec<Frag>) = parts.into_iter().unzip();
    // `build` refuses tile graphs with cycles.
    let (h, resolver) = cached(work, &format!("{tag}/g1"), || build(world, s.tile, &graphs))?;
    let exits = Exits::new(&graphs);
    let foreign = foreign_supply(&graphs, &exits)?;
    drop(graphs);
    let top = top_spill(&h);
    let frag_at = |i: usize| -> Result<Frag> {
        match work {
            Some(wk) => Ok(wk
                .cached::<(TileGraph, Frag)>(&format!("{tag}/a/{i:05}"), || Err(missing("pass A")))?
                .1),
            None => Ok(frags[i].clone()),
        }
    };

    // B: cells by their holding depression in the global hierarchy.
    let counts: Vec<TileCounts> = for_tiles(&tiles, s.threads, &format!("{tag} B"), |i, core| {
        cached(work, &format!("{tag}/b/{i:05}"), || {
            let (win, zw) = read(core)?;
            let d = drain(world, win, &zw, core);
            let local = Local {
                frag: frag_at(i)?,
                ..Default::default()
            };
            let mut t = TileCounts::new();
            for k in 0..core.cells() {
                let (x, y) = core.world_xy(k);
                let z = zw[win.local(x, y).expect("in")];
                let h0 = holding(&h, &top, resolver.leaf_of(local.remap(d.end[k])), z);
                if h0 != NONE {
                    count(&mut t, h0, z, x, y);
                }
            }
            Ok(t)
        })
    })?;
    let mut bal = cached(work, &format!("{tag}/g2"), || {
        solve(&h, &Counts::dense(h.len(), &counts), true)
    })?;
    drop(counts);

    // L: a settled tree whose super-leaf holds water without being full keeps that water
    // inside, and its inside must be solved apart (globally it was one leaf). A tile solves
    // such a tree itself when every cell the tree holds lies in the tile (its count equals
    // the global one) and every pour into it enters there; every other such tree goes to pass
    // L2, which gathers its cells and pours from all tiles (B2). Supply from other tiles'
    // cells is known globally (`foreign`). Exact either way.
    let trees = Trees {
        h: &h,
        resolver: &resolver,
        bal: &bal,
        exits: &exits,
        foreign: &foreign,
        min_cells: s.lake_min_cells(),
    };
    let part_l: Vec<PartL> = for_tiles(&tiles, s.threads, &format!("{tag} L"), |i, core| {
        let (local, nodes, remote) = cached(work, &format!("{tag}/l/{i:05}"), || {
            let frag = frag_at(i)?;
            let want = trees.wanted(&frag)?;
            if want.is_empty() {
                let local = Local {
                    frag,
                    ..Default::default()
                };
                return Ok((local, Nodes::default(), Vec::new()));
            }
            let (win, zw) = read(core)?;
            let d = drain(world, win, &zw, core);
            let probe = Local {
                frag,
                ..Default::default()
            };
            let pick = |key: u64| want.contains_key(&key).then_some(&probe);
            let mut cells: BTreeMap<u64, Vec<(u32, f32, u64, u64)>> = BTreeMap::new();
            for (key, l, z, x, y) in trees.cells(&probe, core, win, &zw, &d, pick)? {
                cells.entry(key).or_default().push((l, z, x, y));
            }
            let (mut here, mut remote, mut pours) = (Tops::new(), Vec::new(), Vec::new());
            for (&key, &spill) in &want {
                let sl = trees.leaf(key)?;
                let n = cells.get(&key).map_or(0, Vec::len) as u64;
                ensure(
                    n <= bal.n[sl],
                    "a tile holds more cells of a tree than the world",
                )?;
                let mut inside = n == bal.n[sl];
                let mut entries = Vec::new();
                for (cell, amount) in trees.pours_into(sl as u32) {
                    let (cx, cy) = world.xy(cell);
                    match core.local(cx, cy) {
                        Some(k) => {
                            let l = probe.leaf_of_end(&exits, d.end[k])?.ok_or_else(|| {
                                invalid(format!("a pour into tree {key} does not reach it"))
                            })?;
                            entries.push((l, amount));
                        }
                        None => inside = false,
                    }
                }
                if inside {
                    here.insert(key, spill);
                    pours.extend(entries);
                } else {
                    remote.push(key);
                }
            }
            let frag = probe.frag;
            if here.is_empty() {
                let local = Local {
                    frag,
                    ..Default::default()
                };
                return Ok((local, Nodes::default(), remote));
            }
            let mine: Vec<(u32, f32, u64, u64)> = here
                .keys()
                .flat_map(|k| cells.remove(k).unwrap_or_default())
                .collect();
            let (local, nodes) = trees.solve(frag, here, &mine, &pours)?;
            Ok((local, nodes, remote))
        })?;
        let keys = local.solved.keys().copied().collect();
        let keep = if work.is_some() {
            Local::default()
        } else {
            local
        };
        Ok((keys, remote, nodes.0, keep))
    })?;

    // L2: the remaining trees, solved from the cells and pours of every tile.
    let mut by_tile: BTreeMap<usize, Tops> = BTreeMap::new();
    for (i, p) in part_l.iter().enumerate() {
        for &key in &p.1 {
            let sl = trees.leaf(key)?;
            by_tile
                .entry(i)
                .or_default()
                .insert(key, (h.spill[sl], h.outlet[sl]));
        }
    }
    let mut remote: Vec<Local> = Vec::new();
    let mut remote_of: BTreeMap<u64, u32> = BTreeMap::new();
    let mut remote_nodes: Vec<Node> = Vec::new();
    if !by_tile.is_empty() {
        let probes: Vec<Local> = by_tile
            .iter()
            .map(|(&i, tops)| {
                Ok(Local {
                    frag: frag_at(i)?,
                    solved: tops.clone(),
                    ..Default::default()
                })
            })
            .collect::<Result<_>>()?;
        for (r, p) in probes.iter().enumerate() {
            for &key in p.solved.keys() {
                remote_of.insert(key, r as u32);
            }
        }
        let pick = |key: u64| remote_of.get(&key).map(|&r| &probes[r as usize]);
        let gathered: Vec<Gathered> =
            for_tiles(&tiles, s.threads, &format!("{tag} L2"), |i, core| {
                cached(work, &format!("{tag}/l2/{i:05}"), || {
                    let (win, zw) = read(core)?;
                    let d = drain(world, win, &zw, core);
                    let own = Local {
                        frag: frag_at(i)?,
                        ..Default::default()
                    };
                    let cells = trees.cells(&own, core, win, &zw, &d, pick)?;
                    let mut pours = Vec::new();
                    for (&key, &r) in &remote_of {
                        for (cell, amount) in trees.pours_into(trees.leaf(key)? as u32) {
                            let (cx, cy) = world.xy(cell);
                            let Some(k) = core.local(cx, cy) else {
                                continue;
                            };
                            let l = probes[r as usize]
                                .leaf_of_end(&exits, d.end[k])?
                                .ok_or_else(|| {
                                    invalid(format!("a pour into tree {key} does not reach it"))
                                })?;
                            pours.push((key, l, amount));
                        }
                    }
                    Ok((cells, pours))
                })
            })?;
        // Grouped by tree once; the solve only sums and sorts them, so order does not matter.
        let mut cells_by: BTreeMap<u64, Vec<(u32, f32, u64, u64)>> = BTreeMap::new();
        let mut pours_by: BTreeMap<u64, Vec<(u32, u64)>> = BTreeMap::new();
        for (cells, pours) in gathered {
            for (key, l, z, x, y) in cells {
                cells_by.entry(key).or_default().push((l, z, x, y));
            }
            for (key, l, amount) in pours {
                pours_by.entry(key).or_default().push((l, amount));
            }
        }
        for p in probes {
            let (mut mine, mut pours) = (Vec::new(), Vec::new());
            for &key in p.solved.keys() {
                let cells = cells_by.remove(&key).unwrap_or_default();
                let (n, want) = (cells.len() as u64, bal.n[trees.leaf(key)?]);
                ensure(
                    n == want,
                    format!("tree {key}: {n} cells gathered, {want} held globally"),
                )?;
                mine.extend(cells);
                pours.extend(pours_by.remove(&key).unwrap_or_default());
            }
            let (local, nodes) = trees.solve(p.frag, p.solved, &mine, &pours)?;
            remote_nodes.extend(nodes.0);
            remote.push(local);
        }
    }

    // A super-leaf solved apart is no global node: its water is its local nodes'.
    let mut solved: Vec<u64> = part_l
        .iter()
        .flat_map(|p| p.0.iter().chain(&p.1).copied())
        .collect();
    solved.sort_unstable();
    let leaves: Vec<usize> = solved
        .iter()
        .map(|&k| trees.leaf(k))
        .collect::<Result<_>>()?;
    for sl in leaves {
        (bal.node[sl], bal.need[sl]) = (NONE, 0);
    }
    let mut table: Vec<Node> = remote_nodes;
    let mut mem_locals: Vec<Local> = Vec::new();
    for (_, _, nodes, local) in part_l {
        table.extend(nodes);
        mem_locals.push(local);
    }
    let local_of = |i: usize| -> Result<Local> {
        match work {
            Some(wk) => Ok(wk
                .cached::<(Local, Nodes, Vec<u64>)>(&format!("{tag}/l/{i:05}"), || {
                    Err(missing("pass L"))
                })?
                .0),
            None => Ok(mem_locals[i].clone()),
        }
    };

    // C: the ground of closed wet basins of the global hierarchy, for their levels. Cells of
    // solved super-leaves have no global node now and are skipped.
    if bal.need.iter().any(|&k| k > 0) {
        let gathered: Vec<Vec<(u32, f32)>> =
            for_tiles(&tiles, s.threads, &format!("{tag} C"), |i, core| {
                cached(work, &format!("{tag}/c/{i:05}"), || {
                    let (win, zw) = read(core)?;
                    let d = drain(world, win, &zw, core);
                    let local = Local {
                        frag: frag_at(i)?,
                        ..Default::default()
                    };
                    let mut v = Vec::new();
                    for k in 0..core.cells() {
                        let (x, y) = core.world_xy(k);
                        let z = zw[win.local(x, y).expect("in")];
                        let h0 = holding(&h, &top, resolver.leaf_of(local.remap(d.end[k])), z);
                        if h0 != NONE {
                            let nd = bal.node[h0 as usize];
                            if nd != NONE && bal.need[nd as usize] > 0 {
                                v.push((nd, z));
                            }
                        }
                    }
                    Ok(v)
                })
            })?;
        bal = cached(work, &format!("{tag}/g3"), || {
            let mut b = bal.clone();
            b.set_closed_levels(&gathered.concat())?;
            Ok(b)
        })?;
    }
    // The node table: global nodes and every tree solved apart, by key (R28).
    let minpit = min_pits(&h);
    table.extend(
        (0..h.len() as u32)
            .filter(|&d| bal.node[d as usize] == d)
            .map(|d| node_of(&h, &bal, &minpit, d, s.lake_min_cells())),
    );
    table.sort_unstable_by_key(|n| n.key);
    ensure(
        table.windows(2).all(|p| p[0].key < p[1].key),
        "two water nodes share a key",
    )?;
    let water = Water {
        h,
        resolver,
        top,
        bal,
        minpit,
        nodes: Nodes(table),
        solved,
        remote,
        remote_of,
        exits,
    };
    let w = &water;

    // D: full ponds that cross tiles, solved once.
    let spanning: Vec<Vec<(u32, u64, f32)>> =
        for_tiles(&tiles, s.threads, &format!("{tag} D"), |i, core| {
            cached(work, &format!("{tag}/d/{i:05}"), || {
                let (win, zw) = read(core)?;
                let st = state(s, w, &local_of(i)?, core, win, &zw)?;
                Ok((0..core.cells())
                    .filter_map(|k| match st.wet[k] {
                        Wet::Pond(o) if w.nodes.at(o).full && !inside(w.nodes.at(o).bbox, core) => {
                            let (x, y) = core.world_xy(k);
                            Some((o, world.index(x, y), st.z[k]))
                        }
                        _ => None,
                    })
                    .collect())
            })
        })?;
    let ponds: Vec<(u64, u8)> = cached(work, &format!("{tag}/gp"), || {
        let mut by: BTreeMap<u32, Vec<(u64, f32)>> = BTreeMap::new();
        for (o, c, z) in spanning.into_iter().flatten() {
            by.entry(o).or_default().push((c, z));
        }
        let mut out = Vec::new();
        for (o, mut cells) in by {
            cells.sort_unstable_by_key(|c| c.0);
            out.extend(flats::resolve(world, &cells, w.nodes.at(o).outlet[1])?);
        }
        Ok(out)
    })?;
    let ponds: HashMap<u64, u8> = ponds.into_iter().collect();

    // E: flow reports; full nodes pass their water on at their outlet.
    let outlets: BTreeMap<u64, Option<u64>> = w
        .nodes
        .0
        .iter()
        .map(|n| (n.key, n.full.then_some(n.outlet[1])))
        .collect();
    let outlet_cells: HashSet<u64> = outlets.values().flatten().copied().collect();
    let reports: Vec<Report> = for_tiles(&tiles, s.threads, &format!("{tag} E"), |i, core| {
        cached(work, &format!("{tag}/e/{i:05}"), || {
            let (win, zw) = read(core)?;
            let st = state(s, w, &local_of(i)?, core, win, &zw)?;
            let flat = flats_for(s, w, &st, &ponds)?;
            let rain = s.rain_units(core)?;
            let next = |k: usize| next_of(s, w, &st, &flat, k);
            let local = accumulate(core.cells(), next, |k| rain[k]);
            let (end, looped) = ends(core.cells(), next);
            ensure(
                local.cycles + looped == 0,
                format!("flow in tile {i} has a cycle"),
            )?;
            let mut port_end = BTreeMap::new();
            for (k, &e) in end.iter().enumerate() {
                let (x, y) = core.world_xy(k);
                let gi = world.index(x, y);
                if core.on_ring(x, y) || outlet_cells.contains(&gi) {
                    port_end.insert(gi, e);
                }
            }
            Ok(Report {
                out: local.out,
                node_in: local.node_in,
                port_end,
            })
        })
    })?;
    let flow = cached(work, &format!("{tag}/g4"), || {
        solve_flow(&reports, &outlets)
    })?;
    drop(reports);

    // F: the rasters.
    if let Some(r) = out {
        let thr = s.river_km2();
        for_tiles(&tiles, s.threads, &format!("{tag} F"), |i, core| {
            let name = format!("{tag}/f/{i:05}");
            if work.is_some_and(|wk| wk.is_done(&name)) {
                return Ok(());
            }
            let (win, zw) = read(core)?;
            let st = state(s, w, &local_of(i)?, core, win, &zw)?;
            let flat = flats_for(s, w, &st, &ponds)?;
            let rain = s.rain_units(core)?;
            let next = |k: usize| next_of(s, w, &st, &flat, k);
            let acc = accumulate(core.cells(), next, |k| rain[k]);
            ensure(acc.cycles == 0, format!("flow in tile {i} has a cycle"))?;
            let mut q = acc.q;
            let ports: Vec<(usize, u64)> = (0..core.cells())
                .filter_map(|k| {
                    let (x, y) = core.world_xy(k);
                    flow.inflow.get(&world.index(x, y)).map(|&a| (k, a))
                })
                .collect();
            add_inflows(&mut q, &ports, next)?;
            let n = core.cells();
            let (mut node, mut disch, mut fl) = (vec![0u32; n], vec![0f32; n], vec![0u8; n]);
            for k in 0..n {
                let qk = match next(k) {
                    Next::Node(nd) => *flow
                        .node_total
                        .get(&nd)
                        .ok_or_else(|| invalid(format!("node {nd} has no total")))?,
                    _ => q[k],
                };
                disch[k] = (qk as f64 / Q_PER_KM2) as f32;
                node[k] = match st.wet[k] {
                    Wet::Dry => 0,
                    Wet::Sea => 1,
                    Wet::Lake(o) | Wet::Pond(o) => o + 2,
                };
                let b = byte_of(w, &st, &flat, k);
                let mut f = b;
                if b != 0 && disch[k] >= thr {
                    f |= 16;
                }
                match st.wet[k] {
                    Wet::Dry if st.held[k] => f |= 32 | 64,
                    Wet::Lake(o) | Wet::Pond(o) if !w.nodes.at(o).full => f |= 64,
                    _ => {}
                }
                fl[k] = f;
            }
            r.level.write(core, &st.level)?;
            r.node.write(core, &node)?;
            r.discharge.write(core, &disch)?;
            r.flow.write(core, &fl)?;
            match work {
                Some(wk) => wk.mark_done(&name),
                None => Ok(()),
            }
        })?;
    }
    Ok(Model {
        water,
        ponds,
        flow,
        locals: mem_locals,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::water::geom::tile_of;
    use crate::world::water::testing::{setup, terrain};

    type Out = (Model, Vec<u32>, Vec<u32>, Vec<u32>, Vec<u8>);

    fn bits(v: Vec<f32>) -> Vec<u32> {
        v.iter().map(|x| x.to_bits()).collect()
    }

    /// A run of `s` on `ground`: the model and its rasters (level, discharge as bits).
    fn run_on(s: &Setup, ground: &Layer<f32>) -> Out {
        let r = Rasters::memory(s.world);
        let m = model(s, ground, None, "m", Some(&r)).expect("model");
        (
            m,
            bits(r.level.to_vec().unwrap()),
            r.node.to_vec().unwrap(),
            bits(r.discharge.to_vec().unwrap()),
            r.flow.to_vec().unwrap(),
        )
    }

    fn assert_same(a: &Out, b: &Out, at: &str) {
        assert_eq!(a.1, b.1, "levels, {at}");
        assert_eq!(a.2, b.2, "labels, {at}");
        assert_eq!(a.3, b.3, "discharge, {at}");
        assert_eq!(a.4, b.4, "flow, {at}");
        assert_eq!(a.0.water.nodes, b.0.water.nodes, "nodes, {at}");
    }

    fn run_with(world: World, z: &[f32], tile: u64, contract: bool) -> Out {
        let mut s = setup(world, tile, 1e-5);
        (s.tile, s.contract) = (tile, contract);
        run_on(&s, &Layer::memory(world, z.to_vec()))
    }

    /// Whole vs tiles, and contraction on vs off (review C3): every raster and the node table
    /// bit-identical; also on terraced ground (rounded heights: flats and zero-depth pits).
    #[test]
    fn tilings_and_contraction_give_bit_identical_results() {
        let world = World { w: 64, h: 64 };
        let smooth = terrain(world);
        let terraced: Vec<f32> = smooth.iter().map(|z| (z * 10.0).round() / 10.0).collect();
        for (name, z) in [("smooth", &smooth), ("terraced", &terraced)] {
            let whole = run_with(world, z, 64, false);
            for (tile, contract) in [(64, true), (16, false), (16, true), (13, true), (7, true)] {
                let at = format!("{name}, tile {tile}, contraction {contract}");
                let got = run_with(world, z, tile, contract);
                assert_same(&got, &whole, &at);
                if (tile, contract) == (16, true) {
                    let w = &got.0.water;
                    assert!(!w.solved.is_empty(), "a tree solved apart, {at}");
                    if name == "terraced" {
                        assert!(!w.remote.is_empty(), "a tree solved across tiles, {at}");
                    }
                }
            }
        }
        // The terraced ground has zero-depth pits (R32): with contraction on, some lie in a
        // tile whose fragment settled a tree of several pits, and they stay global leaves.
        let s = setup(world, 16, 1e-5);
        let m = run_on(&s, &Layer::memory(world, terraced)).0;
        let h = &m.water.h;
        let contracted = |l: usize| {
            let (x, y) = world.xy(h.leaf_pit[l]);
            m.locals[tile_of(world, 16, x, y)]
                .frag
                .size
                .iter()
                .any(|&n| n >= 2)
        };
        assert!(
            (0..h.leaves).any(|l| h.spill[l] <= h.low[l] && contracted(l)),
            "a zero-depth pit stays a global leaf in a contracted tile"
        );
    }

    /// An arid (r = 0.1) closed basin behind a rim, with two pits inside tile x 32..48, y 16..32
    /// of a 64 x 48 world, spilling east through a notch. `elongated`: its floor ramps west
    /// across the seam at x = 32, so cells in the neighbouring tile are held by the settled
    /// tree (pass L2); otherwise every held cell lies in the tile (pass L). `dimple`: a humid
    /// dimple (r = 1) on the elongated floor at (29, 23) fills and pours into the tree from
    /// the other tile.
    fn basin(elongated: bool, dimple: bool) -> (Setup, Layer<f32>) {
        let world = World { w: 64, h: 48 };
        let z: Vec<f32> = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
                let mut z = 25.0 + 0.01 * (50.0 - x) + ((i * 7919) % 13) as f32 * 0.001;
                if x > 50.0 {
                    z -= 2.0 * (x - 50.0);
                }
                let rx = if x < 40.0 && elongated { 14.0 } else { 6.0 };
                let d2 = ((x - 40.0) / rx).powi(2) + ((y - 24.0) / 5.0).powi(2);
                if d2 < 1.0 {
                    z -= 5.0 * (1.0 - d2);
                    for px in [38.0, 42.0] {
                        z -= (-((x - px).powi(2) + (y - 24.0).powi(2)) / 2.0).exp();
                    }
                } else if d2 < 2.5 {
                    z += 1.5;
                }
                if dimple {
                    let r = ((x - 29.0).powi(2) + (y - 23.0).powi(2)).sqrt();
                    z -= 2.0 * (1.0 - r / 2.5).max(0.0);
                }
                if y == 24.0 && x >= 44.0 {
                    z = z.min(23.0 - 0.1 * (x - 44.0));
                }
                // A walled gully from the tile to the north through the rim: water from other
                // tiles enters the tree above its spill, and nothing else from the north.
                if (39.0..=41.0).contains(&x) && (11.0..20.0).contains(&y) {
                    z = if x == 40.0 && y >= 12.0 {
                        z.min(24.9 - 0.05 * (y - 12.0))
                    } else {
                        z.max(28.0)
                    };
                }
                z
            })
            .collect();
        let mut s = setup(world, 16, 1e-5);
        // Karst (r = 0.02) north of the rim keeps the gully's catchment from filling the basin.
        let class: Vec<u8> = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
                if dimple && (x - 29.0).powi(2) + (y - 23.0).powi(2) <= 6.25 {
                    18
                } else if y < 20.0 {
                    1
                } else {
                    2
                }
            })
            .collect();
        s.classes = Some(Classes::new(Layer::memory(world, class), 1, world).expect("classes"));
        (s, Layer::memory(world, z))
    }

    /// Pass L and L2 against the uncontracted whole (B2): a closed lake inside a settled tree,
    /// wholly in its tile; holding cells across a seam; and that with a pour from the other
    /// tile entering at an interior cell (mapped through the ring, B1).
    #[test]
    fn settled_trees_solved_in_their_tile_or_across_seams_match_the_whole() {
        for (elongated, dimple) in [(false, false), (true, false), (true, true)] {
            let run = |tile: u64, contract: bool| {
                let (mut s, ground) = basin(elongated, dimple);
                (s.tile, s.contract) = (tile, contract);
                run_on(&s, &ground)
            };
            let case = format!("elongated {elongated}, dimple {dimple}");
            let whole = run(64, false);
            for (tile, contract) in [(64, true), (16, false)] {
                let at = format!("{case}, tile {tile}, contraction {contract}");
                assert_same(&run(tile, contract), &whole, &at);
            }
            let tiled = run(16, true);
            assert_same(&tiled, &whole, &case);
            let w = &whole.0.water;
            assert!(
                w.nodes.0.iter().any(|n| !n.full && n.need > 0),
                "the basin holds a closed lake"
            );
            let t = &tiled.0.water;
            assert_eq!(t.solved.len(), 1, "one tree solved apart");
            let key = t.solved[0];
            let sl = t.resolver.pit_leaf(key).expect("a global leaf") as usize;
            let (world, tile_at) = (World { w: 64, h: 48 }, |c: u64| {
                tile_of(World { w: 64, h: 48 }, 16, c % 64, c / 64)
            });
            let local = if elongated {
                assert_eq!(
                    t.remote.len(),
                    1,
                    "the tree holds cells across the seam: pass L2"
                );
                &t.remote[0]
            } else {
                assert!(t.remote.is_empty(), "solved in its tile: pass L");
                &tiled.0.locals[tile_at(key)]
            };
            assert!(local.solved.contains_key(&key));
            assert!(
                !whole.0.water.nodes.get(key).expect("the tree's node").full,
                "the tree holds a closed lake"
            );
            // Cells of other tiles drain in across the ring (B1): its tile's cells alone supply
            // less than the super-leaf.
            let f = &local.frag;
            let own: u64 = (0..f.h.leaves)
                .filter(|&l| f.tree[l] == key)
                .map(|l| f.h.supply[l])
                .sum();
            assert!(own < t.h.supply[sl], "supply from other tiles, {case}");
            let foreign_pour = t.bal.pours.iter().any(|&(l, cb, _)| {
                l as usize == sl && tile_at(cb) != tile_at(key) && world.xy(cb).0 < 32
            });
            assert_eq!(
                foreign_pour, dimple,
                "the dimple pours into the tree from the other tile"
            );
        }
    }

    /// With a work directory and file rasters (every pass cached, fragments and `Local`s read
    /// back), a run and its resume equal the in-memory run, pass L2 included.
    #[test]
    fn a_work_directory_run_and_its_resume_match_memory() {
        let (s, ground) = basin(true, true);
        let memory = run_on(&s, &ground);
        let dir = tempfile::tempdir().expect("temp");
        let stamp = serde_json::json!({"test": "basin"});
        for round in ["first", "resumed"] {
            let work = Work::open(&dir.path().join("work"), &stamp).expect("work");
            let r = Rasters::files(s.world, dir.path(), "m").expect("rasters");
            let m = model(&s, &ground, Some(&work), "m", Some(&r)).expect("model");
            let got = (
                m,
                bits(r.level.to_vec().unwrap()),
                r.node.to_vec().unwrap(),
                bits(r.discharge.to_vec().unwrap()),
                r.flow.to_vec().unwrap(),
            );
            assert_same(&got, &memory, round);
            assert!(!got.0.water.remote.is_empty(), "pass L2, {round}");
            assert!(got.0.locals.iter().all(|l| *l == Local::default()));
            let i = tile_of(s.world, s.tile, 40, 24);
            assert_eq!(
                got.0.local(Some(&work), "m", i).expect("local").frag,
                memory.0.locals[i].frag
            );
        }
    }

    #[test]
    fn the_synthetic_world_has_sea_a_lagoon_lakes_and_a_closed_basin() {
        let world = World { w: 64, h: 64 };
        let (m, level, node, q, flow) = run_with(world, &terrain(world), 16, true);
        let level: Vec<f32> = level.into_iter().map(f32::from_bits).collect();
        let q: Vec<f32> = q.into_iter().map(f32::from_bits).collect();
        let at = |x: usize, y: usize| y * 64 + x;
        assert_eq!(node[at(63, 10)], 1, "the east edge is sea");
        assert!(node[at(54, 32)] >= 2, "the lagoon is a lake, not sea");
        assert!(level[at(54, 32)] > -2.0, "it stands above its floor");
        let humid = node[at(16, 16)];
        assert!(humid >= 2, "the humid bowl holds water");
        assert!(
            m.water.nodes.at(humid - 2).full,
            "r = 1 fills the humid bowl to its spill"
        );
        assert_eq!(
            node[at(16, 46)],
            0,
            "r = 0 leaves the dune bowl a dry playa"
        );
        assert!(flow.iter().any(|f| f & 16 != 0), "rivers");
        // Discharge never falls along a receiver on dry land.
        for k in 0..64 * 64 {
            let d = flow[k] & 15;
            if d == 0 || node[k] != 0 {
                continue;
            }
            let (dy, dx) = crate::world::water::geom::OFFSETS[d as usize - 1];
            let j = ((k / 64) as i64 + dy) as usize * 64 + ((k % 64) as i64 + dx) as usize;
            assert!(q[j] >= q[k], "discharge falls from {k} to {j}");
        }
    }

    /// Peak resident memory of this process (VmHWM), in MB.
    fn peak_rss_mb() -> f64 {
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        let kb = status
            .lines()
            .find_map(|l| l.strip_prefix("VmHWM:"))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<f64>().ok());
        kb.unwrap_or(f64::NAN) / 1024.0
    }

    /// Counts and histograms of a run on a real window (review C2.4).
    fn print_counts(m: &Model, node: &[u32], flow: &[u8]) {
        let nodes = &m.water.nodes.0;
        let lakes = nodes.iter().filter(|n| n.lake).count();
        let lake_cells = node
            .iter()
            .filter(|&&n| n >= 2 && m.water.nodes.at(n - 2).lake)
            .count();
        eprintln!(
            "global leaves {} depressions {} nodes {} lakes {lakes} ponds {} trees solved apart {} \
             (across tiles {}) lake cells {lake_cells} river cells {} (cycles fail the run)",
            m.water.h.leaves,
            m.water.h.len(),
            nodes.len(),
            nodes.len() - lakes,
            m.water.solved.len(),
            m.water.remote.len(),
            flow.iter().filter(|&&f| f & 16 != 0).count(),
        );
        let mut area = [0u64; 24];
        let mut depth = [0u64; 32];
        for n in nodes {
            area[((n.cells as f64).max(1.0).log2() as usize).min(23)] += 1;
            depth[(((n.level - n.low) * 10.0) as usize).min(31)] += 1;
        }
        eprintln!("node area (log2 m² bins): {area:?}\nnode depth (0.1 m bins): {depth:?}");
    }

    /// cargo test … pipeline -- --ignored --nocapture
    #[test]
    #[ignore]
    fn probe_model_at_the_dam_spot() {
        use crate::world::water::probe;
        let Some((win, z)) = probe::window(25645, 24400, 1024) else {
            return;
        };
        let world = World {
            w: win.w as u64,
            h: win.h as u64,
        };
        let mut s = setup(world, 256, 0.05);
        s.classes = None;
        s.table.lake_min_m2 = 500.0;
        s.nw_m = [win.x0 as f64, (probe::WORLD.h - win.y0) as f64];
        let r = Rasters::memory(world);
        let m = model(&s, &Layer::memory(world, z.clone()), None, "m", Some(&r)).expect("model");
        let node = r.node.to_vec().unwrap();
        let flow = r.flow.to_vec().unwrap();
        eprintln!("dam spot E25645 N24400, 1,024², tiles 256², uniform rain, r = 1:");
        print_counts(&m, &node, &flow);
        let wet: Vec<bool> = (0..z.len())
            .map(|k| node[k] >= 2 && m.water.nodes.at(node[k] - 2).lake || flow[k] & 16 != 0)
            .collect();
        probe::shade_png("t9-dam-spot-water", &z, win.w, &wet);
    }

    /// Exactness on real terrain: arid windows with water (r between 0 and 1; dunes and
    /// badlands have r = 0 and hold none), 2,048² with the layout's classes, uncontracted as
    /// one tile against tiles of 512², 300², 256² and 128² with contraction. Every raster
    /// and the node table bit-identical, and some tree solved across tiles (pass L2).
    #[test]
    #[ignore]
    fn probe_tiles_match_the_whole_on_arid_windows() {
        use crate::world::water::probe;
        let mut across = 0;
        for (name, east, north) in [
            ("Grand Canyon (19, r 0.05)", 27169, 44546),
            ("tower karst (14, r 0.02)", 33280, 19327),
            ("stratovolcano (11, r 0.5)", 33280, 31487),
        ] {
            let Some((win, z)) = probe::window(east, north, 2048) else {
                return;
            };
            let Some(class) = probe::classes(win) else {
                return;
            };
            let world = World {
                w: win.w as u64,
                h: win.h as u64,
            };
            let ground = Layer::memory(world, z);
            let classes = Layer::memory(world, class);
            let run = |tile: u64, contract: bool| {
                let mut s = setup(world, tile, 0.05);
                s.table.lake_min_m2 = 500.0;
                s.threads = 4;
                s.contract = contract;
                s.nw_m = [win.x0 as f64, (probe::WORLD.h - win.y0) as f64];
                s.classes = Some(Classes::new(classes.clone(), 1, world).expect("classes"));
                let t = std::time::Instant::now();
                let out = run_on(&s, &ground);
                let w = &out.0.water;
                eprintln!(
                    "{name} E{east} N{north} tiles {tile}² contraction {contract}: {:.0} s; \
                     global leaves {}, nodes {} (closed {}), trees solved apart {} (across \
                     tiles {})",
                    t.elapsed().as_secs_f64(),
                    w.h.leaves,
                    w.nodes.0.len(),
                    w.nodes.0.iter().filter(|n| n.need > 0).count(),
                    w.solved.len(),
                    w.remote.len()
                );
                out
            };
            let whole = run(2048, false);
            for tile in [512, 300, 256, 128] {
                let tiled = run(tile, true);
                assert_same(&tiled, &whole, &format!("{name}, tiles {tile}"));
                across += tiled.0.water.remote.len();
            }
            eprintln!("{name}: every tiling with contraction = whole uncontracted, bit for bit");
        }
        eprintln!("trees solved across tiles (pass L2), all windows and tilings: {across}");
        assert!(across > 0, "a tree solved across tiles (pass L2)");
    }

    /// Upstream area (cells) of the raw surface's own D8 network: steepest strictly lower
    /// neighbour, no fill (pits end paths), world edges drain off.
    fn raw_d8(w: usize, z: &[f32]) -> Vec<u64> {
        let world = World {
            w: w as u64,
            h: (z.len() / w) as u64,
        };
        let next = |k: usize| {
            let (x, y) = world.xy(k as u64);
            if world.is_edge(x, y) {
                return Next::End;
            }
            let mut best: Option<(f32, u64)> = None;
            for j in 0..8 {
                let (nx, ny) = world.neighbour(x, y, j).expect("inside");
                let n = world.index(nx, ny);
                let drop = (z[k] - z[n as usize])
                    / crate::world::water::geom::step_len(j as u8 + 1) as f32;
                if drop > 0.0 && best.is_none_or(|(b, _)| drop > b) {
                    best = Some((drop, n));
                }
            }
            best.map_or(Next::End, |b| Next::Cell(b.1 as u32))
        };
        let acc = accumulate(z.len(), next, |_| 1);
        assert_eq!(acc.cycles, 0, "steepest descent has no cycles");
        acc.q
    }

    /// 8-neighbour steps from each cell to the nearest cell of `on`, capped at 255.
    fn distance_to(w: usize, on: &[bool]) -> Vec<u8> {
        let world = World {
            w: w as u64,
            h: (on.len() / w) as u64,
        };
        let mut dist = vec![255u8; on.len()];
        let mut queue = std::collections::VecDeque::new();
        for (k, _) in on.iter().enumerate().filter(|p| *p.1) {
            dist[k] = 0;
            queue.push_back(k);
        }
        while let Some(k) = queue.pop_front() {
            if dist[k] >= 254 {
                continue;
            }
            let (x, y) = world.xy(k as u64);
            for j in 0..8 {
                let Some((nx, ny)) = world.neighbour(x, y, j) else {
                    continue;
                };
                let n = world.index(nx, ny) as usize;
                if dist[n] == 255 && !on[n] {
                    dist[n] = dist[k] + 1;
                    queue.push_back(n);
                }
            }
        }
        dist
    }

    /// Straight runs of a receiver field on the cells `on`: (cells on, of them in a
    /// same-direction run of 60 or more steps ahead, maximal runs over 150 steps).
    fn straight_runs(w: usize, dir: &[u8], on: &[bool]) -> (usize, usize, usize) {
        let h = dir.len() / w;
        let step = |k: usize, d: u8| -> Option<usize> {
            let (dy, dx) = crate::world::water::geom::OFFSETS[d as usize - 1];
            let (x, y) = ((k % w) as i64 + dx, (k / w) as i64 + dy);
            (x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h)
                .then(|| y as usize * w + x as usize)
        };
        let back = |k: usize, d: u8| -> Option<usize> {
            let (dy, dx) = crate::world::water::geom::OFFSETS[d as usize - 1];
            let (x, y) = ((k % w) as i64 - dx, (k / w) as i64 - dy);
            (x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h)
                .then(|| y as usize * w + x as usize)
        };
        let ahead = |k: usize, cap: usize| {
            let (d, mut c, mut run) = (dir[k], k, 0);
            while run < cap {
                match step(c, d) {
                    Some(n) if dir[n] == d && on[n] => (c, run) = (n, run + 1),
                    _ => break,
                }
            }
            run
        };
        let (mut total, mut straight, mut long) = (0, 0, 0);
        for k in 0..dir.len() {
            if dir[k] == 0 || !on[k] {
                continue;
            }
            total += 1;
            if ahead(k, 60) >= 60 {
                straight += 1;
            }
            let starts = back(k, dir[k]).is_none_or(|p| dir[p] != dir[k] || !on[p]);
            if starts && ahead(k, 151) > 150 {
                long += 1;
            }
        }
        (total, straight, long)
    }

    /// The raw surface's own accumulation network: steepest descent on the raw ground, and
    /// in depressions the priority flood's route (Barnes 2014) to the window edge.
    fn raw_network(w: usize, z: &[f32]) -> (Vec<u8>, Vec<u64>) {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        let world = World {
            w: w as u64,
            h: (z.len() / w) as u64,
        };
        use crate::world::water::geom::{OFFSETS, step_len};
        use crate::world::water::watershed::ordered;
        let opposite = |j: usize| {
            let (dy, dx) = OFFSETS[j];
            OFFSETS
                .iter()
                .position(|&o| o == (-dy, -dx))
                .expect("opposite") as u8
                + 1
        };
        let mut dir = vec![0u8; z.len()];
        // The neighbour that first reached each cell, as a direction byte.
        let mut parent = vec![0u8; z.len()];
        let (mut seen, mut done) = (vec![false; z.len()], vec![false; z.len()]);
        let mut level = z.to_vec();
        let mut heap = BinaryHeap::new();
        for k in 0..z.len() {
            let (x, y) = world.xy(k as u64);
            if world.is_edge(x, y) {
                seen[k] = true;
                heap.push(Reverse((ordered(z[k]), k)));
            }
        }
        // Each cell drains, when popped, to its steepest strictly lower neighbour popped
        // before it, else along the flood's route: always to an earlier cell, so no cycles.
        while let Some(Reverse((_, c))) = heap.pop() {
            done[c] = true;
            let (x, y) = world.xy(c as u64);
            let mut best: Option<(f32, u8)> = None;
            for j in 0..8 {
                let Some((nx, ny)) = world.neighbour(x, y, j) else {
                    continue;
                };
                let n = world.index(nx, ny) as usize;
                let drop = (z[c] - z[n]) / step_len(j as u8 + 1) as f32;
                if done[n] && drop > 0.0 && best.is_none_or(|(b, _)| drop > b) {
                    best = Some((drop, j as u8 + 1));
                }
                if !seen[n] {
                    seen[n] = true;
                    level[n] = z[n].max(level[c]);
                    parent[n] = opposite(j);
                    heap.push(Reverse((ordered(level[n]), n)));
                }
            }
            if !world.is_edge(x, y) {
                dir[c] = best.map_or(parent[c], |b| b.1);
            }
        }
        let next = |k: usize| match dir[k] {
            0 => Next::End,
            d => {
                let (x, y) = world.xy(k as u64);
                let (nx, ny) = world.neighbour(x, y, d as usize - 1).expect("inside");
                Next::Cell(world.index(nx, ny) as u32)
            }
        };
        let acc = accumulate(z.len(), next, |_| 1);
        assert_eq!(acc.cycles, 0, "the raw network has no cycles");
        (dir, acc.q)
    }

    /// Carry (expert, E49249): the full model on a 4,096² raw window at E49249 N16381 (the
    /// straight river of the v2 diagnosis), tiles of 1,024², the layout's classes, uniform
    /// rain (Q is area). Thalweg (Q ≥ 0.05 km²) straight runs against the raw surface's own
    /// network, runs inside ponds apart; Q ≥ 1 km² rivers against the raw valley line; lakes
    /// per km² per class; global leaves; peak RSS; 512 m hillshade crops with water tinted.
    /// Run in release under systemd-run (MemoryMax ≤ 10G).
    #[test]
    #[ignore]
    fn probe_model_at_e49249() {
        use crate::world::water::probe;
        use crate::world::water::table::{DEFAULT, Table};
        let started = std::time::Instant::now();
        let Some((win, z)) = probe::window(49249, 16381, 4096) else {
            return;
        };
        let w = win.w;
        let world = World {
            w: w as u64,
            h: win.h as u64,
        };
        let Some(class) = probe::classes(win) else {
            return;
        };
        let mut s = setup(world, 1024, 0.05);
        s.table = Table::parse(DEFAULT).expect("table");
        s.threads = 4;
        s.nw_m = [win.x0 as f64, (probe::WORLD.h - win.y0) as f64];
        s.classes =
            Some(Classes::new(Layer::memory(world, class.clone()), 1, world).expect("classes"));
        let r = Rasters::memory(world);
        let m = model(&s, &Layer::memory(world, z.clone()), None, "m", Some(&r)).expect("model");
        let model_s = started.elapsed().as_secs_f64();
        let node = r.node.to_vec().unwrap();
        let flow = r.flow.to_vec().unwrap();
        let q = r.discharge.to_vec().unwrap();
        eprintln!(
            "E49249 N16381, 4,096² (x0 {} y0 {}), tiles 1,024², classes of fujix-collage-55, \
             uniform rain, lake from {} m²; model {model_s:.0} s",
            win.x0, win.y0, s.table.lake_min_m2
        );
        print_counts(&m, &node, &flow);

        // Straight runs: the model's thalwegs (dry and in full ponds apart) against the raw
        // surface's own network at the same threshold.
        let dir: Vec<u8> = flow.iter().map(|f| f & 15).collect();
        let pond = |k: usize| node[k] >= 2;
        let thalweg: Vec<bool> = flow.iter().map(|f| f & 16 != 0).collect();
        let dry: Vec<bool> = (0..z.len()).map(|k| thalweg[k] && !pond(k)).collect();
        let wet: Vec<bool> = (0..z.len()).map(|k| thalweg[k] && pond(k)).collect();
        let (raw_dir, raw_q) = raw_network(w, &z);
        let raw_on: Vec<bool> = raw_q.iter().map(|&a| a >= 50_000).collect();
        let share = |(n, s, long): (usize, usize, usize)| {
            format!(
                "{s} of {n} ({:.3}), runs over 150 m: {long}",
                s as f64 / n.max(1) as f64
            )
        };
        eprintln!("60 m straight runs on thalwegs (Q >= 0.05 km²):");
        eprintln!(
            "  model, all:       {}",
            share(straight_runs(w, &dir, &thalweg))
        );
        eprintln!(
            "  model, dry land:  {}",
            share(straight_runs(w, &dir, &dry))
        );
        eprintln!(
            "  model, in ponds:  {}",
            share(straight_runs(w, &dir, &wet))
        );
        eprintln!(
            "  raw network:      {}",
            share(straight_runs(w, &raw_dir, &raw_on))
        );

        // Placement: distance (8-neighbour steps) from each Q >= 1 km² model river cell to
        // the nearest Q >= 1 km² cell of the raw surface's own D8 network (steepest descent
        // on raw, no fill: pits end paths).
        let bins: [(u8, u8); 10] = [
            (0, 0),
            (1, 1),
            (2, 2),
            (3, 3),
            (4, 5),
            (6, 10),
            (11, 20),
            (21, 50),
            (51, 254),
            (255, 255),
        ];
        let placement = |label: &str, area: &[u64]| {
            let on: Vec<bool> = area.iter().map(|&a| a >= 1_000_000).collect();
            let near = distance_to(w, &on);
            let mut hist = [0u64; 10];
            let mut rivers = 0u64;
            for k in 0..z.len() {
                if thalweg[k] && q[k] >= 1.0 {
                    rivers += 1;
                    let b = bins.iter().position(|b| (b.0..=b.1).contains(&near[k]));
                    hist[b.expect("bin")] += 1;
                }
            }
            let within = |cap: u8| {
                let n: u64 = bins
                    .iter()
                    .zip(&hist)
                    .filter(|(b, _)| b.1 <= cap)
                    .map(|(_, &n)| n)
                    .sum();
                n as f64 / rivers.max(1) as f64
            };
            eprintln!(
                "placement against {label}: {rivers} model river cells with Q >= 1 km²; {} \
                 network cells with Q >= 1 km² (largest area {} m²)\n  distance to the nearest \
                 (cells) 0, 1, 2, 3, 4-5, 6-10, 11-20, 21-50, 51-254, 255+: {hist:?}\n  within \
                 3 cells {:.3}, within 10 cells {:.3}",
                on.iter().filter(|&&o| o).count(),
                area.iter().max().copied().unwrap_or(0),
                within(3),
                within(10)
            );
        };
        placement("the raw D8 network (no fill)", &raw_d8(w, &z));
        placement(
            "the raw network routed through depressions (fallback: pits end every D8 path)",
            &raw_q,
        );

        // Secondary: the lowest raw cell within ±10 cells across the flow (dry rivers).
        let mut offsets = [0u64; 11];
        let mut rise = 0f64;
        let mut big = 0u64;
        for k in 0..z.len() {
            if !dry[k] || q[k] < 1.0 {
                continue;
            }
            let (dy, dx) = crate::world::water::geom::OFFSETS[dir[k] as usize - 1];
            let (px, py) = (-dy, dx);
            let (x, y) = ((k % w) as i64, (k / w) as i64);
            let at = |t: i64| {
                let (cx, cy) = (x + t * px, y + t * py);
                (cx >= 0 && cy >= 0 && cx < w as i64 && cy < win.h as i64)
                    .then(|| z[cy as usize * w + cx as usize])
            };
            let mut low = (z[k], 0i64);
            for t in -10i64..=10 {
                let Some(zc) = at(t) else { continue };
                if zc < low.0 || (zc == low.0 && t.abs() < low.1.abs()) {
                    low = (zc, t);
                }
            }
            offsets[low.1.unsigned_abs() as usize] += 1;
            rise += f64::from(z[k] - low.0);
            big += 1;
        }
        let n3: u64 = offsets[..4].iter().sum();
        eprintln!(
            "secondary, {big} dry river cells: lowest raw cell within ±10 cells across the \
             flow at offset 0..10: {offsets:?}; within 3 cells {:.3}; mean height above it \
             {:.3} m",
            n3 as f64 / big.max(1) as f64,
            rise / big.max(1) as f64
        );

        // Confetti check: labelled lakes per km² per class (by the class at the lake's key).
        let mut area: BTreeMap<u8, u64> = BTreeMap::new();
        for &c in &class {
            *area.entry(c).or_insert(0) += 1;
        }
        let mut lakes: BTreeMap<u8, (u64, u64)> = BTreeMap::new();
        for n in m.water.nodes.0.iter().filter(|n| n.lake) {
            let (x, y) = world.xy(n.key);
            let e = lakes
                .entry(class[(y * world.w + x) as usize])
                .or_insert((0, 0));
            (e.0, e.1) = (e.0 + 1, e.1 + n.cells);
        }
        for (c, cells) in &area {
            let (n, lc) = lakes.get(c).copied().unwrap_or((0, 0));
            let km2 = *cells as f64 / 1e6;
            eprintln!(
                "class {c:>2} (r {:.2}): {km2:.2} km², {n} lakes ({:.2}/km², {lc} m² of water)",
                f64::from(s.table.r_milli(*c)) / 1000.0,
                n as f64 / km2
            );
        }
        eprintln!(
            "peak RSS {:.0} MB, total {:.0} s",
            peak_rss_mb(),
            started.elapsed().as_secs_f64()
        );

        // 512 m crops: the window centre, and the faint dotted line at E49107-49417
        // N16200-16225 (centred on E49262 N16212). Thalwegs and lakes tinted.
        let tint: Vec<bool> = (0..z.len())
            .map(|k| thalweg[k] || node[k] >= 2 && m.water.nodes.at(node[k] - 2).lake)
            .collect();
        for (name, east, north) in [
            ("t9-e49249-centre", 49249u64, 16381u64),
            ("t9-e49249-dotted", 49262, 16212),
        ] {
            let cx = (east - win.x0) as usize;
            let cy = (probe::WORLD.h - 1 - north - win.y0) as usize;
            let (x0, y0) = (cx - 256, cy - 256);
            let crop_of = |v: &dyn Fn(usize) -> f32| -> Vec<f32> {
                (0..512 * 512)
                    .map(|i| v((y0 + i / 512) * w + x0 + i % 512))
                    .collect()
            };
            let zc = crop_of(&|k| z[k]);
            let tc: Vec<bool> = crop_of(&|k| f32::from(u8::from(tint[k])))
                .iter()
                .map(|&v| v > 0.0)
                .collect();
            probe::shade_png(name, &zc, 512, &tc);
        }
    }
}
