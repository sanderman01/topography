//! Water bodies: the sea (id 1), lakes (labelled nodes by key, R28; river pools take their
//! river's id, R26), and rivers as main stems: at a confluence the branch with the larger
//! discharge keeps its body (ties: the smaller cell index), so a river body runs from its head
//! to its mouth. A full lake or pool joins the main-stem choice at its outlet's outside cell
//! with its whole discharge: a lake that wins starts its outflow river there (a gully never
//! takes it); a pool that wins carries its largest inflow's river through it. Writes the ids
//! of the sea, lakes, pools and thalweg cells and returns the body list (v2's `bodies.json`
//! fields).

use super::flow::{Next, ends};
use super::geom::{OFFSETS, Rect, World};
use super::hierarchy::NONE;
use super::hydraulic::Geometry;
use super::layers::Layer;
use super::nodes::Nodes;
use super::pipeline::{Model, Rasters, Setup, Water};
use super::rain::Q_PER_KM2;
use super::store::{Work, cached, for_tiles};
use crate::acquisition::{AcquisitionError, Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Dry basins listed from this area (as v2's world lists).
const DRY_MIN_KM2: f64 = 0.01;

fn invalid(msg: impl Into<String>) -> AcquisitionError {
    AcquisitionError::Invalid(msg.into())
}

/// `c` beats `best`: more discharge, ties to the smaller world index.
fn better(c: (f32, u64), best: Option<(f32, u64)>) -> bool {
    best.is_none_or(|b| c.0 > b.0 || (c.0 == b.0 && c.1 < b.1))
}

/// The thalweg neighbour draining into (x, y) with the most discharge, leaving out window
/// cells `skip` names: (discharge, world index).
fn best_up(
    world: World,
    win: Rect,
    flow: &[u8],
    q: &[f32],
    x: u64,
    y: u64,
    skip: impl Fn(usize) -> bool,
) -> Option<(f32, u64)> {
    let mut best = None;
    for k in 0..8 {
        let Some((nx, ny)) = world.neighbour(x, y, k) else {
            continue;
        };
        let Some(j) = win.local(nx, ny) else {
            continue;
        };
        let d = flow[j] & 15;
        if flow[j] & 16 == 0 || d == 0 || skip(j) {
            continue;
        }
        let (dy, dx) = OFFSETS[d as usize - 1];
        if (nx as i64 + dx, ny as i64 + dy) != (x as i64, y as i64) {
            continue;
        }
        let c = (q[j], world.index(nx, ny));
        if better(c, best) {
            best = Some(c);
        }
    }
    best
}

/// The thalweg neighbour draining into (x, y) with the most discharge (ties: the smaller world
/// index).
pub(crate) fn main_up(
    world: World,
    win: Rect,
    flow: &[u8],
    q: &[f32],
    x: u64,
    y: u64,
) -> Option<u64> {
    best_up(world, win, flow, q, x, y, |_| false).map(|b| b.1)
}

/// A window of the model's rasters.
struct View {
    win: Rect,
    flow: Vec<u8>,
    q: Vec<f32>,
    node: Vec<u32>,
}

impl View {
    fn read(r: &Rasters, win: Rect) -> Result<Self> {
        Ok(Self {
            win,
            flow: r.flow.read(win)?,
            q: r.discharge.read(win)?,
            node: r.node.read(win)?,
        })
    }
    fn at(&self, x: u64, y: u64) -> usize {
        self.win.local(x, y).expect("in window")
    }
}

/// Main stems with full lakes and pools as members.
struct Stems<'a> {
    world: World,
    w: &'a Water,
    /// Full lakes and pools on a river (ordinals), by their outlet's outside cell.
    joins: HashMap<u64, Vec<u32>>,
    /// Pools on a river by ordinal: their largest inflowing thalweg cell, if any.
    pools: HashMap<u32, Option<u64>>,
}

impl Stems<'_> {
    fn in_pool(&self, node: u32) -> bool {
        node >= 2 && self.pools.contains_key(&(node - 2))
    }
    /// The next cell up the main stem from (x, y); None at a head. A full node competes with
    /// its outlet's inside cell (that cell's discharge is the node's total).
    fn up(&self, v: &View, x: u64, y: u64) -> Option<u64> {
        let near = best_up(self.world, v.win, &v.flow, &v.q, x, y, |j| {
            self.in_pool(v.node[j])
        });
        let mut best = near.map(|b| (b.0, b.1, None));
        for &o in self
            .joins
            .get(&self.world.index(x, y))
            .into_iter()
            .flatten()
        {
            let inside = self.w.nodes.at(o).outlet[0];
            let (ix, iy) = self.world.xy(inside);
            let c = (v.q[v.at(ix, iy)], inside);
            if better(c, best.map(|b| (b.0, b.1))) {
                best = Some((c.0, c.1, Some(o)));
            }
        }
        match best? {
            (_, c, None) => Some(c),
            // A lake starts a river here; a pool goes on up its largest inflow.
            (_, _, Some(o)) => self.pools.get(&o).copied().flatten(),
        }
    }
    /// Each core cell's end up its main stem: `Node(head cell)`, `Out` of the tile, or `End`
    /// (off the thalweg or in a pool).
    fn chains(&self, core: Rect, v: &View) -> Result<Vec<Next>> {
        let next = |k: usize| {
            let (x, y) = core.world_xy(k);
            let j = v.at(x, y);
            if v.flow[j] & 16 == 0 || self.in_pool(v.node[j]) {
                return Next::End;
            }
            match self.up(v, x, y) {
                Some(u) => {
                    let (ux, uy) = self.world.xy(u);
                    core.local(ux, uy)
                        .map_or(Next::Out(u), |j| Next::Cell(j as u32))
                }
                None => Next::Node(self.world.index(x, y)),
            }
        };
        let (end, cycles) = ends(core.cells(), next);
        ensure(cycles == 0, "a main stem runs in a cycle")?;
        Ok(end)
    }
}

/// Pass P per tile: listed pools whose outlet's outside cell lies here, and whether it is a
/// thalweg cell outside every listed pool (the pool is on a river); thalweg cells draining
/// into a listed pool (ordinal, discharge, cell, own node raster value).
type Probed = (Vec<(u32, bool)>, Vec<(u32, f32, u64, u32)>);
/// Pass G per tile: river heads, entry cells (ring cells and pool inflows) with their chain
/// ends, and each pool's outlet chain end.
type Chased = (Vec<u64>, Vec<(u64, Next)>, Vec<(u32, Next)>);
/// A tile's rivers: by id, axial and diagonal steps, and mouths (cell, discharge, parent id).
type RiverAttrs = BTreeMap<u32, (u64, u64, Vec<(u64, f32, u32)>)>;

fn round(v: f64, places: i32) -> f64 {
    let k = 10f64.powi(places);
    (v * k).round() / k
}

/// River pools: full lake nodes a river flows into (inflow ≥ the river start) whose area is
/// below 20 · w(Q)² of that river; they are labelled as the river leaving them. Node key →
/// outlet cell.
pub(crate) fn pools(s: &Setup, m: &Model, geo: &Geometry) -> BTreeMap<u64, u64> {
    let thr = s.params.river_km2;
    m.water
        .nodes
        .0
        .iter()
        .filter(|n| n.lake && n.full)
        .filter_map(|n| {
            let q = m.flow.node_total.get(&n.key).copied().unwrap_or(0) as f64 / Q_PER_KM2;
            let w = geo.width_m(q);
            (q >= thr && (n.cells as f64) * s.spacing_m * s.spacing_m < 20.0 * w * w)
                .then_some((n.key, n.outlet[1]))
        })
        .collect()
}

/// The longest trickle (4,096 cells at 32 m); flow is acyclic, so it is only a safety cap.
const TRICKLE_MAX_M: f64 = 131_072.0;

/// The outflow paths of the full labelled lakes in `from` that are not rivers yet (B16): from
/// the outside spill cell along the receivers to the first sea, lake (`is_lake` of the node
/// raster's ordinal) or model thalweg cell, a closed pond's pit or the map edge, through full
/// ponds. Marked trickle cells carry less than the river start's discharge and are walked
/// through, so the result is the same before and after `mark_trickles`. Returns the sorted
/// cells off cold ground and how many trickles reached `TRICKLE_MAX_M`.
pub(crate) fn trickles(
    s: &Setup,
    from: &Nodes,
    is_lake: impl Fn(u32) -> bool,
    r: &Rasters,
) -> Result<(Vec<u64>, u64)> {
    let world = s.world;
    let (flow, node, q) = (
        r.flow.cells(false)?,
        r.node.cells(false)?,
        r.discharge.cells(false)?,
    );
    let max = (TRICKLE_MAX_M / s.spacing_m).ceil() as usize;
    let (mut out, mut capped) = (Vec::new(), 0u64);
    for n in from.0.iter().filter(|n| n.full && n.lake) {
        let mut c = n.outlet[1];
        let mut ended = false;
        for _ in 0..max {
            let (f, nd) = (flow.get(c)?, node.get(c)?);
            let thalweg = f & 16 != 0 && q.get(c)? >= s.river_km2();
            if thalweg || nd == 1 || (nd >= 2 && is_lake(nd - 2)) {
                ended = true;
                break;
            }
            // A cell draining off the map (or a pond's pit) has no receiver; like any thalweg
            // it stays unmarked.
            let d = f & 15;
            let (x, y) = world.xy(c);
            let Some((nx, ny)) = (d != 0)
                .then(|| world.neighbour(x, y, d as usize - 1))
                .flatten()
            else {
                ended = true;
                break;
            };
            out.push(c);
            c = world.index(nx, ny);
        }
        capped += u64::from(!ended);
    }
    out.sort_unstable();
    out.dedup();
    // No river on snow and ice: the outflow runs under it and shows again past its edge.
    let cold = s.no_river_cells(&out)?;
    let out = out
        .into_iter()
        .zip(cold)
        .filter_map(|(c, cold)| (!cold).then_some(c))
        .collect();
    Ok((out, capped))
}

/// Sets the thalweg bit on `cells` (idempotent).
pub(crate) fn mark_trickles(cells: &[u64], r: &Rasters) -> Result<()> {
    let flow = r.flow.cells(true)?;
    for &c in cells {
        flow.set(c, flow.get(c)? | 16)?;
    }
    Ok(())
}

/// Writes `body` (0 none, 1 sea, lake ids, river ids on thalweg cells, and on the cells of the
/// pools in `pools` (node key → outlet cell, R26) the id of the river leaving them) and
/// returns the `bodies.json` value. A listed pool whose outlet carries no river stays its own
/// node (a lake keeps its lake id): it would otherwise take the level of the sea or lake below.
#[allow(clippy::too_many_arguments)]
pub(crate) fn label(
    s: &Setup,
    w: &Water,
    r: &Rasters,
    body: &Layer<u32>,
    pools: &BTreeMap<u64, u64>,
    work: Option<&Work>,
    tag: &str,
) -> Result<Value> {
    let world = s.world;
    let tiles = s.tiles();
    // B16: every full lake's outflow is a river, whatever its discharge.
    let (cells, _) = trickles(s, &w.nodes, |o| w.nodes.at(o).lake, r)?;
    mark_trickles(&cells, r)?;
    let mut listed: BTreeMap<u32, u64> = BTreeMap::new();
    for (&key, &b) in pools {
        let o = w
            .nodes
            .ordinal(key)
            .ok_or_else(|| invalid(format!("pool {key} is no water node")))?;
        let n = w.nodes.at(o);
        ensure(
            n.full && n.outlet[1] == b,
            format!("pool {key} is no full node draining at {b}"),
        )?;
        listed.insert(o, b);
    }
    let is_listed = |node: u32| node >= 2 && listed.contains_key(&(node - 2));

    // Pass P: which pools are on a river, and what drains into them.
    let probed: Vec<Probed> = for_tiles(&tiles, s.threads, &format!("{tag} P"), |i, core| {
        cached(work, &format!("{tag}/bp/{i:05}"), || {
            let v = View::read(r, core.grow(1, world))?;
            let mut outs = Vec::new();
            for (&o, &b) in &listed {
                let (bx, by) = world.xy(b);
                if core.contains(bx, by) {
                    let j = v.at(bx, by);
                    outs.push((o, v.flow[j] & 16 != 0 && !is_listed(v.node[j])));
                }
            }
            let mut ins = Vec::new();
            for k in 0..core.cells() {
                let (x, y) = core.world_xy(k);
                let j = v.at(x, y);
                if v.flow[j] & 16 == 0 {
                    continue;
                }
                let (rx, ry) = world
                    .neighbour(x, y, (v.flow[j] & 15) as usize - 1)
                    .ok_or_else(|| invalid("a receiver lies outside the world"))?;
                let below = v.node[v.at(rx, ry)];
                if is_listed(below) && below != v.node[j] {
                    ins.push((below - 2, v.q[j], world.index(x, y), v.node[j]));
                }
            }
            Ok((outs, ins))
        })
    })?;
    let mut on_river: HashMap<u32, Option<u64>> = HashMap::new();
    for &(o, ok) in probed.iter().flat_map(|p| p.0.iter()) {
        if ok {
            on_river.insert(o, None);
        }
    }
    let mut inflow: BTreeMap<u32, (f32, u64)> = BTreeMap::new();
    for &(o, q, c, from) in probed.iter().flat_map(|p| p.1.iter()) {
        let from_pool = from >= 2 && on_river.contains_key(&(from - 2));
        if on_river.contains_key(&o) && !from_pool && better((q, c), inflow.get(&o).copied()) {
            inflow.insert(o, (q, c));
        }
    }
    drop(probed);
    for (&o, &(_, c)) in &inflow {
        on_river.insert(o, Some(c));
    }
    let mut joins: HashMap<u64, Vec<u32>> = HashMap::new();
    for (o, n) in w.nodes.0.iter().enumerate() {
        let o = o as u32;
        if n.full && (n.lake || on_river.contains_key(&o)) {
            let ((ax, ay), (bx, by)) = (world.xy(n.outlet[0]), world.xy(n.outlet[1]));
            ensure(
                ax.abs_diff(bx) <= 1 && ay.abs_diff(by) <= 1,
                format!("node {}'s outlet cells are no neighbours", n.key),
            )?;
            joins.entry(n.outlet[1]).or_default().push(o);
        }
    }
    let stems = Stems {
        world,
        w,
        joins,
        pools: on_river,
    };
    let entries_wanted: HashSet<u64> = stems.pools.values().flatten().copied().collect();

    // Pass G: main-stem chains to heads or out of the tile.
    let chased: Vec<Chased> = for_tiles(&tiles, s.threads, &format!("{tag} G"), |i, core| {
        cached(work, &format!("{tag}/bg/{i:05}"), || {
            let v = View::read(r, core.grow(1, world))?;
            let end = stems.chains(core, &v)?;
            let (mut heads, mut entries) = (Vec::new(), Vec::new());
            for (k, &e) in end.iter().enumerate() {
                let (x, y) = core.world_xy(k);
                let c = world.index(x, y);
                if e == Next::Node(c) {
                    heads.push(c);
                }
                if e != Next::End && (core.on_ring(x, y) || entries_wanted.contains(&c)) {
                    entries.push((c, e));
                }
            }
            let mut pool_ends = Vec::new();
            for &o in stems.pools.keys() {
                let (bx, by) = world.xy(listed[&o]);
                if let Some(k) = core.local(bx, by) {
                    ensure(end[k] != Next::End, "a pool's outlet is no river cell")?;
                    pool_ends.push((o, end[k]));
                }
            }
            Ok((heads, entries, pool_ends))
        })
    })?;
    let entries: HashMap<u64, Next> = chased.iter().flat_map(|c| c.1.iter().copied()).collect();
    for &c in &entries_wanted {
        ensure(
            entries.contains_key(&c),
            format!("pool inflow {c} is no river cell"),
        )?;
    }
    let head_of = |mut t: Next| -> Result<u64> {
        for _ in 0..=entries.len() {
            match t {
                Next::Out(c) => {
                    t = *entries
                        .get(&c)
                        .ok_or_else(|| invalid(format!("main stem enters cell {c}, no river")))?
                }
                Next::Node(h) => return Ok(h),
                other => return Err(invalid(format!("main stem ends in {other:?}"))),
            }
        }
        Err(invalid("main stems cycle across tiles"))
    };
    let mut heads: Vec<u64> = chased.iter().flat_map(|c| c.0.iter().copied()).collect();
    heads.sort_unstable();

    // Ids: sea 1, lakes by node key (pools on a river are not lakes), rivers by head cell.
    let mut lake_id: BTreeMap<u32, u32> = BTreeMap::new();
    let mut next_id = 2u32;
    for (o, n) in w.nodes.0.iter().enumerate() {
        if n.lake && !stems.pools.contains_key(&(o as u32)) {
            ensure(n.level.is_finite(), format!("lake {} has no level", n.key))?;
            lake_id.insert(o as u32, next_id);
            next_id += 1;
        }
    }
    ensure(
        1 + lake_id.len() + heads.len() < usize::from(u16::MAX),
        format!(
            "{} lakes and {} rivers: export ids are 16-bit (fewer than 65,535 bodies)",
            lake_id.len(),
            heads.len()
        ),
    )?;
    let head_id: HashMap<u64, u32> = heads
        .iter()
        .enumerate()
        .map(|(i, &h)| (h, next_id + i as u32))
        .collect();
    next_id += heads.len() as u32;
    let river = |t: Next| -> Result<u32> {
        let h = head_of(t)?;
        head_id
            .get(&h)
            .copied()
            .ok_or_else(|| invalid(format!("river head {h} has no id")))
    };
    let mut pool_id: HashMap<u32, u32> = HashMap::new();
    for &(o, e) in chased.iter().flat_map(|c| c.2.iter()) {
        ensure(pool_id.insert(o, river(e)?).is_none(), "a pool found twice")?;
    }
    ensure(
        pool_id.len() == stems.pools.len(),
        "a pool on a river has no outlet cell",
    )?;
    drop(chased);

    // Pass H: write ids and gather attributes.
    let parts: Vec<(RiverAttrs, BTreeMap<u32, u64>, u64)> =
        for_tiles(&tiles, s.threads, &format!("{tag} H"), |_, core| {
            let v = View::read(r, core.grow(1, world))?;
            let end = stems.chains(core, &v)?;
            // The body of a window cell: in the core from its chain, outside it from the
            // neighbouring tile's ring.
            let body_at = |x: u64, y: u64| -> Result<u32> {
                let j = v.at(x, y);
                let thalweg = || match core.local(x, y) {
                    Some(k) => river(end[k]),
                    None => river(Next::Out(world.index(x, y))),
                };
                Ok(match v.node[j] {
                    1 => 1,
                    n if n >= 2 => match pool_id.get(&(n - 2)) {
                        Some(&id) => id,
                        None => match lake_id.get(&(n - 2)) {
                            Some(&id) => id,
                            None if v.flow[j] & 16 != 0 => thalweg()?,
                            None => 0,
                        },
                    },
                    _ if v.flow[j] & 16 != 0 => thalweg()?,
                    _ => 0,
                })
            };
            let (mut rivers, mut lakes, mut sea) = (RiverAttrs::new(), BTreeMap::new(), 0u64);
            let mut ids = vec![0u32; core.cells()];
            for (k, id) in ids.iter_mut().enumerate() {
                let (x, y) = core.world_xy(k);
                let j = v.at(x, y);
                *id = body_at(x, y)?;
                if v.node[j] == 1 {
                    sea += 1;
                } else if v.node[j] >= 2 && lake_id.contains_key(&(v.node[j] - 2)) {
                    *lakes.entry(*id).or_insert(0u64) += 1;
                }
                if v.flow[j] & 16 == 0 || stems.in_pool(v.node[j]) {
                    continue;
                }
                ensure(*id >= 2, format!("thalweg cell ({x}, {y}) has no body"))?;
                let dir = v.flow[j] & 15;
                let e = rivers.entry(*id).or_default();
                if dir > 4 {
                    e.1 += 1;
                } else {
                    e.0 += 1;
                }
                let (rx, ry) = world
                    .neighbour(x, y, dir as usize - 1)
                    .ok_or_else(|| invalid("a receiver lies outside the world"))?;
                let below = body_at(rx, ry)?;
                if below != *id {
                    e.2.push((world.index(x, y), v.q[j], below));
                }
            }
            body.write(core, &ids)?;
            Ok((rivers, lakes, sea))
        })?;

    // Merge.
    let km2 = s.cell_km2();
    let (mut rivers, mut lakes, mut sea) = (RiverAttrs::new(), BTreeMap::new(), 0u64);
    for (rv, lk, se) in parts {
        sea += se;
        for (id, (a, d, m)) in rv {
            let e = rivers.entry(id).or_default();
            (e.0, e.1) = (e.0 + a, e.1 + d);
            e.2.extend(m);
        }
        for (id, n) in lk {
            *lakes.entry(id).or_insert(0u64) += n;
        }
    }
    // A lake's (or gapped river's) parent: the first body down the receivers (as v2).
    let one = |c: u64| {
        let (x, y) = world.xy(c);
        Rect {
            x0: x,
            y0: y,
            w: 1,
            h: 1,
        }
    };
    let parent_below = |mut c: u64| -> Result<u32> {
        for _ in 0..world.w * world.h {
            let b = body.read(one(c))?[0];
            if b != 0 {
                return Ok(b);
            }
            let d = r.flow.read(one(c))?[0] & 15;
            if d == 0 {
                return Ok(0);
            }
            let (x, y) = world.xy(c);
            let (nx, ny) = world
                .neighbour(x, y, d as usize - 1)
                .ok_or_else(|| invalid("a receiver lies outside the world"))?;
            c = world.index(nx, ny);
        }
        Err(invalid("an outflow runs in a cycle"))
    };
    let cell = |c: u64| {
        let (x, y) = world.xy(c);
        json!([x, y])
    };
    let mut list = vec![json!({
        "id": 1, "kind": "sea", "level_m": 0.0, "parent": 0,
        "area_km2": round(sea as f64 * km2, 6),
    })];
    for (&o, &id) in &lake_id {
        let n = w.nodes.at(o);
        let cells = lakes.get(&id).copied().unwrap_or(0);
        let parent = if n.full {
            parent_below(n.outlet[1])?
        } else {
            0
        };
        list.push(json!({
            "id": id, "kind": "lake", "node": n.key, "level_m": f64::from(n.level),
            "area_km2": round(cells as f64 * km2, 6),
            "max_depth_m": round(f64::from(n.level - n.low), 2),
            "exit_cell": if n.full { cell(n.outlet[1]) } else { Value::Null },
            "endorheic": !n.full, "parent": parent,
        }));
    }
    for h in &heads {
        let id = head_id[h];
        let (axial, diagonal, mouths) = rivers.remove(&id).unwrap_or_default();
        let [(mouth, q, parent)] = mouths[..] else {
            return Err(invalid(format!(
                "river {id} (head {h}) has {} mouths",
                mouths.len()
            )));
        };
        let km = (axial as f64 + diagonal as f64 * std::f64::consts::SQRT_2) * s.spacing_m / 1000.0;
        // A river ending on unlabelled ground (a snow gap, a full pond) drains into the first
        // body down its receivers, as a lake's outflow does.
        let parent = match parent {
            0 => match r.flow.read(one(mouth))?[0] & 15 {
                0 => 0,
                d => {
                    let (x, y) = world.xy(mouth);
                    let (nx, ny) = world
                        .neighbour(x, y, d as usize - 1)
                        .ok_or_else(|| invalid("a receiver lies outside the world"))?;
                    parent_below(world.index(nx, ny))?
                }
            },
            p => p,
        };
        list.push(json!({
            "id": id, "kind": "river", "length_km": round(km, 3),
            "drainage_km2": round(f64::from(q), 6), "mouth_cell": cell(mouth), "parent": parent,
        }));
    }
    let min_cells = (DRY_MIN_KM2 / km2).ceil() as u64;
    for d in dry_basins(w, min_cells) {
        list.push(json!({
            "id": next_id, "kind": "dry", "level_m": f64::from(w.h.spill[d]),
            "area_km2": round(w.bal.n[d] as f64 * km2, 6), "endorheic": true, "parent": 0,
        }));
        next_id += 1;
    }
    Ok(json!({
        "schema_version": 1, "kind": "water_bodies",
        "cells": "[column, row] from the north-west corner; a lake's exit_cell is the first cell \
            its routed flow enters outside it",
        "note": "hydrology v3: lakes by node (smallest pit), rivers as main stems (larger \
            discharge keeps the body; a full lake or pool competes at its outlet), river pools \
            labelled as their river",
        "dry_min_km2": DRY_MIN_KM2, "bodies": list,
    }))
}

/// Dry basins: top-level depressions with no water anywhere in their tree, off the sea, of at
/// least `min_cells` below their spill; by node key. A tree settled in a tile is one global
/// super-leaf, wet exactly when it is a node or its inside was solved apart (`solved`), so the
/// list does not depend on the tiling.
fn dry_basins(w: &Water, min_cells: u64) -> Vec<usize> {
    let n = w.h.len();
    let mut wet = vec![false; n];
    // Children come before parents.
    for d in 0..n {
        wet[d] |= w.bal.node[d] != NONE
            || w.h.is_leaf(d as u32) && w.solved.binary_search(&w.minpit[d]).is_ok();
        let p = w.h.parent[d];
        if p != NONE && wet[d] {
            wet[p as usize] = true;
        }
    }
    let mut dry: Vec<(u64, usize)> = (0..n)
        .filter(|&d| w.h.parent[d] == NONE && !wet[d] && !w.bal.sea[d] && w.bal.n[d] >= min_cells)
        .map(|d| (w.minpit[d], d))
        .collect();
    dry.sort_unstable();
    dry.into_iter().map(|(_, d)| d).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::water::layers::Classes;
    use crate::world::water::nodes::{Node, Nodes};
    use crate::world::water::pipeline::model;
    use crate::world::water::table::SNOW_PRESENT;
    use crate::world::water::testing::{setup, terrain};

    fn run_pools(
        tile: u64,
        contract: bool,
        pools: impl Fn(&Model) -> BTreeMap<u64, u64>,
    ) -> (Model, Vec<u32>, Value, Vec<u8>, Vec<u32>) {
        let world = World { w: 64, h: 64 };
        let mut s = setup(world, tile, 1e-5);
        s.contract = contract;
        let r = Rasters::memory(world);
        let m = model(
            &s,
            &Layer::memory(world, terrain(world)),
            None,
            "m",
            Some(&r),
        )
        .expect("model");
        let body = Layer::filled(world, 0u32);
        let json = label(&s, &m.water, &r, &body, &pools(&m), None, "m").expect("bodies");
        let (flow, node) = (r.flow.to_vec().unwrap(), r.node.to_vec().unwrap());
        (m, body.to_vec().unwrap(), json, flow, node)
    }

    fn run(tile: u64) -> (Vec<u32>, Value, Vec<u8>) {
        let (_, body, json, flow, _) = run_pools(tile, true, |_| BTreeMap::new());
        (body, json, flow)
    }

    /// The larger discharge keeps the body; ties go to the smaller cell; off-thalweg and
    /// cells draining elsewhere never count.
    #[test]
    fn the_larger_branch_is_the_main_stem() {
        let world = World { w: 3, h: 3 };
        let win = Rect::whole(world);
        // Centre (1, 1) = 4. West (3) drains E (4), north (1) drains S (2), NE (2) drains
        // SW (7), east (5) drains W (3) but is off the thalweg, south (7) drains S.
        let mut flow = [0u8; 9];
        (flow[3], flow[1], flow[2], flow[5], flow[7]) = (4 | 16, 2 | 16, 7 | 16, 3, 2 | 16);
        let mut q = [0f32; 9];
        (q[3], q[1], q[2], q[5], q[7]) = (2.0, 3.0, 1.0, 9.0, 9.0);
        assert_eq!(main_up(world, win, &flow, &q, 1, 1), Some(1));
        q[3] = 3.0;
        assert_eq!(
            main_up(world, win, &flow, &q, 1, 1),
            Some(1),
            "tie: smaller index"
        );
        q[3] = 3.5;
        assert_eq!(main_up(world, win, &flow, &q, 1, 1), Some(3));
        assert_eq!(main_up(world, win, &flow, &q, 0, 0), None, "a head");
    }

    /// A river cut by a snow band (no river cells on it) keeps a link: its parent is the
    /// first body down its receivers, the river below the band.
    #[test]
    fn a_river_ending_at_a_snow_gap_drains_into_the_body_below() {
        let world = World { w: 64, h: 64 };
        let mut s = setup(world, 16, 1e-5);
        let mut c = s.params.climate.expect("climate");
        c.cold_box_m = 0.0;
        s.params.climate = Some(c);
        let snow: Vec<u8> = (0..64 * 64)
            .map(|k| {
                if (30..34).contains(&(k % 64)) {
                    SNOW_PRESENT
                } else {
                    1
                }
            })
            .collect();
        s.snow = Some(Classes::new(Layer::memory(world, snow), 1, world).unwrap());
        let r = Rasters::memory(world);
        let m = model(
            &s,
            &Layer::memory(world, terrain(world)),
            None,
            "m",
            Some(&r),
        )
        .expect("model");
        let body = Layer::filled(world, 0u32);
        let json = label(&s, &m.water, &r, &body, &BTreeMap::new(), None, "m").expect("bodies");
        let (body, flow) = (body.to_vec().unwrap(), r.flow.to_vec().unwrap());
        let down = |k: usize| -> Option<usize> {
            let d = flow[k] & 15;
            (d != 0).then(|| {
                let (dy, dx) = OFFSETS[d as usize - 1];
                ((k / 64) as i64 + dy) as usize * 64 + ((k % 64) as i64 + dx) as usize
            })
        };
        let mut gapped = 0;
        for river in json["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|b| b["kind"] == "river")
        {
            let [x, y] = [0, 1].map(|i| river["mouth_cell"][i].as_u64().unwrap() as usize);
            let mut c = down(y * 64 + x);
            let snowy = c.is_some_and(|c| (30..34).contains(&(c % 64)));
            while let Some(k) = c.filter(|&k| body[k] == 0) {
                c = down(k);
            }
            let want = c.map_or(0, |k| u64::from(body[k]));
            assert_eq!(river["parent"].as_u64().unwrap(), want, "{river}");
            gapped += usize::from(snowy && want > 1);
        }
        assert!(gapped > 0, "a river ends at the snow and drains on");
    }

    #[test]
    fn tiles_label_alike() {
        let (body, json, _) = run(64);
        let (b16, j16, _) = run(16);
        assert_eq!(b16, body);
        assert_eq!(j16, json);
    }

    #[test]
    fn every_river_has_one_mouth_and_known_parents() {
        let (body, json, flow) = run(16);
        let list = json["bodies"].as_array().expect("bodies");
        let ids: HashSet<u64> = list.iter().map(|b| b["id"].as_u64().unwrap()).collect();
        let rivers: Vec<&Value> = list.iter().filter(|b| b["kind"] == "river").collect();
        assert!(!rivers.is_empty());
        for r in &rivers {
            let p = r["parent"].as_u64().unwrap();
            assert!(p == 0 || ids.contains(&p), "parent {p}");
        }
        // Mouths: thalweg cells whose receiver is not a thalweg cell of the same body.
        let mut mouths = HashMap::<u32, u32>::new();
        for k in 0..64 * 64 {
            if flow[k] & 16 == 0 {
                continue;
            }
            let (dy, dx) = OFFSETS[(flow[k] & 15) as usize - 1];
            let j = ((k / 64) as i64 + dy) as usize * 64 + ((k % 64) as i64 + dx) as usize;
            if flow[j] & 16 == 0 || body[j] != body[k] {
                *mouths.entry(body[k]).or_insert(0) += 1;
            }
        }
        assert!(mouths.values().all(|&m| m == 1), "{mouths:?}");
        assert_eq!(mouths.len(), rivers.len());
    }

    /// R26: a full lake marked a river pool takes the id of the river leaving it and is no
    /// lake; at any tiling.
    #[test]
    fn a_pool_is_labelled_as_the_river_leaving_it() {
        let pools = |m: &Model| -> BTreeMap<u64, u64> {
            let n = m
                .water
                .nodes
                .0
                .iter()
                .find(|n| n.lake && n.full && n.cells > 20)
                .expect("a full lake");
            BTreeMap::from([(n.key, n.outlet[1])])
        };
        let (m, body, json, flow, node) = run_pools(16, true, pools);
        let (_, b64, j64, ..) = run_pools(64, true, pools);
        assert_eq!((&body, &json), (&b64, &j64), "tilings");
        let (&key, &b) = pools(&m).iter().next().unwrap();
        let o = m.water.nodes.ordinal(key).unwrap();
        assert!(flow[b as usize] & 16 != 0, "the pool's outlet is a river");
        let id = body[b as usize];
        let cells: Vec<usize> = (0..node.len()).filter(|&k| node[k] == o + 2).collect();
        assert!(!cells.is_empty());
        assert!(
            cells.iter().all(|&k| body[k] == id),
            "pool cells take the river"
        );
        let list = json["bodies"].as_array().unwrap();
        assert!(
            list.iter().all(|b| b["node"].as_u64() != Some(key)),
            "no lake"
        );
        let river = list.iter().find(|b| b["id"] == id).expect("listed");
        assert_eq!(river["kind"], "river");
    }

    /// A hand-drawn 64 x 32 scene of rasters (flow, discharge, node) and its nodes. Rivers run
    /// east along row 10 to the sea (column 63):
    /// - lake A (x 4-8, y 8-12, full, total 50) drains at (9, 10); a gully down column 9
    ///   (y 3-9) drains into (9, 10) too;
    /// - pool P (lake, x 22-25, y 9-11) takes the row-10 river at (22, 10) and a small
    ///   tributary down column 23 (y 4-8); it drains at (26, 10);
    /// - pool Q (pond with flat receivers along row 10, x 41-43, y 9-11) drains at (44, 10);
    /// - lake B (x 44-47, y 20-23) is fed down column 46 (y 14-19) and drains at (48, 22),
    ///   in another tile at tiles of 8 and 16, east along row 22 to the sea;
    /// - lake C (x 30-31, y 2-3, total 0.5) drains at (32, 2) over dry cells along row 2 to
    ///   (35, 2) and down column 35 into the row-10 river at (35, 10);
    /// - lake D (x 2-3, y 25-26) is closed.
    struct Scene {
        flow: Vec<u8>,
        q: Vec<f32>,
        node: Vec<u32>,
        nodes: Nodes,
        /// Pool keys → outlet cells.
        pools: BTreeMap<u64, u64>,
    }

    const SW: u64 = 64;

    fn at(x: u64, y: u64) -> u64 {
        y * SW + x
    }

    fn dir_to(a: (u64, u64), b: (u64, u64)) -> u8 {
        let d = (b.1 as i64 - a.1 as i64, b.0 as i64 - a.0 as i64);
        OFFSETS.iter().position(|&o| o == d).expect("neighbours") as u8 + 1
    }

    fn scene() -> Scene {
        scene_with(false)
    }

    /// The scene, with `pond` a full pond (x 35, y 5-6) on lake C's dry outflow path.
    fn scene_with(pond: bool) -> Scene {
        let n = (SW * 32) as usize;
        let (mut flow, mut q, mut node) = (vec![0u8; n], vec![0f32; n], vec![0u32; n]);
        // Thalweg path: each cell drains to the next; the last one to `then`; discharge
        // from q0 rising by 1 a cell.
        let mut path = |cells: &[(u64, u64)], then: (u64, u64), q0: f32, thalweg: bool| {
            for (i, &c) in cells.iter().enumerate() {
                let to = cells.get(i + 1).copied().unwrap_or(then);
                let k = at(c.0, c.1) as usize;
                flow[k] = dir_to(c, to) | if thalweg { 16 } else { 0 };
                q[k] = q0 + i as f32;
            }
        };
        let row = |y: u64, xs: std::ops::RangeInclusive<u64>| -> Vec<(u64, u64)> {
            xs.map(|x| (x, y)).collect()
        };
        let col = |x: u64, ys: std::ops::RangeInclusive<u64>| -> Vec<(u64, u64)> {
            ys.map(|y| (x, y)).collect()
        };
        path(&col(9, 3..=9), (9, 10), 1.0, true);
        path(&row(10, 9..=21), (22, 10), 58.0, true);
        path(&col(23, 4..=8), (23, 9), 1.0, true);
        path(&row(10, 26..=40), (41, 10), 80.0, true);
        path(&row(10, 44..=62), (63, 10), 100.0, true);
        path(&col(46, 14..=19), (46, 20), 1.0, true);
        path(&row(22, 48..=62), (63, 22), 10.0, true);
        path(&row(2, 32..=34), (35, 2), 0.6, false);
        path(&col(35, 2..=9), (35, 10), 0.7, false);
        // Pool Q's flat receivers: its row 10 runs east to the outlet, rows 9 and 11 into it.
        for x in 41..=43u64 {
            let k = at(x, 10) as usize;
            flow[k] = dir_to((x, 10), (x + 1, 10)) | 16;
            q[k] = 96.0 + (x - 41) as f32;
            for y in [9, 11] {
                let k = at(x, y) as usize;
                flow[k] = dir_to((x, y), (x, 10));
                q[k] = 1.0;
            }
        }
        for y in 0..32 {
            node[at(63, y) as usize] = 1;
        }
        // Nodes: (cells, lake, full, outlet [inside, outside], total).
        let rect = |x: std::ops::RangeInclusive<u64>, y: std::ops::RangeInclusive<u64>| {
            x.flat_map(|x| y.clone().map(move |y| (x, y)))
                .collect::<Vec<_>>()
        };
        let mut list = vec![
            (rect(4..=8, 8..=12), true, true, [(8, 10), (9, 10)], 50.0),
            (
                rect(22..=25, 9..=11),
                true,
                true,
                [(25, 10), (26, 10)],
                79.0,
            ),
            (
                rect(41..=43, 9..=11),
                false,
                true,
                [(43, 10), (44, 10)],
                98.0,
            ),
            (
                rect(44..=47, 20..=23),
                true,
                true,
                [(47, 22), (48, 22)],
                9.0,
            ),
            (rect(30..=31, 2..=3), true, true, [(31, 2), (32, 2)], 0.5),
            (rect(2..=3, 25..=26), true, false, [(0, 0), (0, 0)], 1.0),
        ];
        if pond {
            list.push((rect(35..=35, 5..=6), false, true, [(35, 6), (35, 7)], 0.0));
        }
        list.sort_by_key(|l| l.0.iter().map(|&(x, y)| at(x, y)).min());
        let mut nodes = Vec::new();
        for (o, (cells, lake, full, outlet, total)) in list.into_iter().enumerate() {
            let key = cells.iter().map(|&(x, y)| at(x, y)).min().unwrap();
            for &(x, y) in &cells {
                let k = at(x, y) as usize;
                node[k] = o as u32 + 2;
                if lake {
                    q[k] = total;
                }
            }
            nodes.push(Node {
                key,
                level: 5.0,
                low: 4.0,
                full,
                lake,
                outlet: outlet.map(|(x, y)| at(x, y)),
                cells: cells.len() as u64,
                ..Default::default()
            });
        }
        let pools = nodes
            .iter()
            .filter(|n| n.key == at(22, 9) || n.key == at(41, 9))
            .map(|n| (n.key, n.outlet[1]))
            .collect();
        Scene {
            flow,
            q,
            node,
            nodes: Nodes(nodes),
            pools,
        }
    }

    fn label_scene(sc: &Scene, tile: u64, pools: &BTreeMap<u64, u64>) -> (Vec<u32>, Value) {
        let world = World { w: SW, h: 32 };
        let s = setup(world, tile, 1e-5);
        let r = Rasters::memory(world);
        r.flow.write(Rect::whole(world), &sc.flow).unwrap();
        r.discharge.write(Rect::whole(world), &sc.q).unwrap();
        r.node.write(Rect::whole(world), &sc.node).unwrap();
        let w = Water {
            h: Default::default(),
            resolver: Default::default(),
            top: Vec::new(),
            bal: Default::default(),
            minpit: Vec::new(),
            nodes: sc.nodes.clone(),
            solved: Vec::new(),
            remote: Vec::new(),
            remote_of: BTreeMap::new(),
            exits: Default::default(),
        };
        let body = Layer::filled(world, 0u32);
        let json = label(&s, &w, &r, &body, pools, None, "t").expect("bodies");
        (body.to_vec().unwrap(), json)
    }

    /// The scene at tiles of 64 (whole), 16 and 8: bit-identical.
    fn scene_labels(pools: bool) -> (Vec<u32>, Value) {
        let sc = scene();
        let p = if pools {
            sc.pools.clone()
        } else {
            BTreeMap::new()
        };
        let whole = label_scene(&sc, 64, &p);
        for tile in [16, 8] {
            assert_eq!(label_scene(&sc, tile, &p), whole, "tiles {tile}");
        }
        whole
    }

    fn entry(json: &Value, id: u32) -> &Value {
        json["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["id"] == id)
            .unwrap_or_else(|| panic!("body {id} listed"))
    }

    #[test]
    fn a_gully_at_a_lake_outlet_joins_the_lake_outflow() {
        let (body, json) = scene_labels(true);
        let id = |x, y| body[at(x, y) as usize];
        let (outflow, gully, lake) = (id(9, 10), id(9, 9), id(6, 10));
        assert_ne!(outflow, gully, "the lake's outflow keeps its own river");
        assert_eq!(id(9, 3), gully);
        assert_eq!(
            entry(&json, gully)["parent"],
            outflow,
            "the gully is a tributary"
        );
        assert_eq!(entry(&json, gully)["mouth_cell"], json!([9, 9]));
        assert_eq!(entry(&json, lake)["kind"], "lake");
        assert_eq!(entry(&json, lake)["parent"], outflow);
        assert_eq!(entry(&json, outflow)["kind"], "river");
    }

    #[test]
    fn a_river_keeps_one_id_and_one_mouth_through_its_pools() {
        let (body, json) = scene_labels(true);
        let id = |x, y| body[at(x, y) as usize];
        let main = id(9, 10);
        for (x, y) in [
            (21, 10),
            (23, 9),
            (24, 11),
            (26, 10),
            (40, 10),
            (42, 9),
            (44, 10),
            (62, 10),
        ] {
            assert_eq!(id(x, y), main, "({x}, {y}) on the main river");
        }
        let river = entry(&json, main);
        assert_eq!(river["mouth_cell"], json!([62, 10]));
        assert_eq!(river["parent"], 1, "the sea");
        // Cells 9..=21, 26..=40, 44..=62 of row 10 (pool cells are not counted).
        assert_eq!(river["length_km"], 0.047);
        let trib = id(23, 4);
        assert_ne!(trib, main);
        assert_eq!(entry(&json, trib)["parent"], main);
        assert_eq!(entry(&json, trib)["mouth_cell"], json!([23, 8]));
        // No lake entry for pool P.
        let rows = json["bodies"].as_array().unwrap();
        assert!(rows.iter().all(|b| b["node"] != at(22, 9)));
        // Lake C's outflow is a river (B16) down the dry cells to the main river.
        let c_out = entry(&json, id(30, 2))["parent"].as_u64().unwrap();
        assert_eq!(entry(&json, c_out as u32)["kind"], "river");
        assert_eq!(entry(&json, c_out as u32)["parent"], main);
    }

    #[test]
    fn without_pools_a_pool_lake_is_a_lake_with_its_own_outflow() {
        let (body, json) = scene_labels(false);
        let id = |x, y| body[at(x, y) as usize];
        let (inflow, lake, outflow) = (id(21, 10), id(23, 10), id(26, 10));
        assert_eq!(entry(&json, lake)["kind"], "lake");
        assert_ne!(inflow, outflow);
        assert_eq!(entry(&json, inflow)["parent"], lake);
        assert_eq!(entry(&json, lake)["parent"], outflow);
        // Pond Q is no body: its thalweg row is the river that leaves it.
        assert_eq!(id(42, 10), outflow);
        assert_eq!(id(42, 9), 0);
        assert_eq!(id(44, 10), outflow);
    }

    #[test]
    fn a_lake_draining_into_another_tile_starts_its_river_there() {
        let (body, json) = scene_labels(true);
        let id = |x, y| body[at(x, y) as usize];
        let (lake, outflow, feeder) = (id(45, 21), id(48, 22), id(46, 14));
        assert_eq!(entry(&json, lake)["kind"], "lake");
        assert_eq!(entry(&json, lake)["parent"], outflow);
        assert_eq!(entry(&json, lake)["exit_cell"], json!([48, 22]));
        assert_eq!(entry(&json, feeder)["parent"], lake);
        assert_eq!(entry(&json, outflow)["mouth_cell"], json!([62, 22]));
        assert_eq!(entry(&json, id(2, 25))["parent"], 0, "closed");
        assert_eq!(entry(&json, id(2, 25))["endorheic"], true);
    }

    /// Real terrain: the canyon window (E27169 N44546, 2,048², river start 0.05 km², the
    /// layout's classes, uniform rain), whole uncontracted and in 512² tiles with contraction:
    /// identical bodies. River bodies, their length, and river pairs running within 20 m of
    /// each other for over 100 m (a double river). Writes t10-canyon-bodies.png, each river
    /// body in its own colour, lakes blue.
    /// cargo test … bodies -- --ignored --nocapture
    #[test]
    #[ignore]
    fn probe_bodies_in_the_canyon() {
        use crate::world::water::layers::Classes;
        use crate::world::water::probe;
        use crate::world::water::table::{DEFAULT, Table};
        let (east, north) = (27169, 44546);
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
        let classes = Layer::memory(world, class);
        let ground = Layer::memory(world, z.clone());
        let run = |tile: u64, contract: bool| {
            let mut s = setup(world, tile, 0.05);
            s.table.lake_min_m2 = Table::parse(DEFAULT).expect("table").lake_min_m2;
            s.threads = 4;
            s.contract = contract;
            s.nw_m = [win.x0 as f64, (probe::WORLD.h - win.y0) as f64];
            s.classes = Some(Classes::new(classes.clone(), 1, world).expect("classes"));
            let r = Rasters::memory(world);
            let m = model(&s, &ground, None, "m", Some(&r)).expect("model");
            let body = Layer::filled(world, 0u32);
            let json = label(&s, &m.water, &r, &body, &BTreeMap::new(), None, "m").expect("bodies");
            (body.to_vec().unwrap(), json)
        };
        let (body, json) = run(512, true);
        let whole = run(2048, false);
        assert!(
            whole == (body.clone(), json.clone()),
            "whole and tiled bodies differ"
        );
        eprintln!("canyon: bodies whole uncontracted = 512² tiles with contraction");
        let list = json["bodies"].as_array().unwrap();
        let kind: HashMap<u32, &str> = list
            .iter()
            .map(|b| {
                (
                    b["id"].as_u64().unwrap() as u32,
                    b["kind"].as_str().unwrap(),
                )
            })
            .collect();
        let rivers: Vec<&Value> = list.iter().filter(|b| b["kind"] == "river").collect();
        let km: f64 = rivers
            .iter()
            .map(|b| b["length_km"].as_f64().unwrap())
            .sum();
        eprintln!(
            "canyon E{east} N{north} 2,048², tiles 512²: {} river bodies, {km:.1} km; {} lakes; \
             {} dry basins",
            rivers.len(),
            list.iter().filter(|b| b["kind"] == "lake").count(),
            list.iter().filter(|b| b["kind"] == "dry").count(),
        );
        let is_river = |id: u32| kind.get(&id) == Some(&"river");
        // Cells of river A within 20 m of river B, by pair.
        let (w, h) = (win.w as i64, win.h as i64);
        let mut near: BTreeMap<(u32, u32), (u64, usize)> = BTreeMap::new();
        let mut seen = Vec::new();
        for (k, &a) in body.iter().enumerate() {
            if !is_river(a) {
                continue;
            }
            let (x, y) = ((k as i64) % w, (k as i64) / w);
            seen.clear();
            for dy in -20i64..=20 {
                for dx in -20i64..=20 {
                    let (nx, ny) = (x + dx, y + dy);
                    if dx * dx + dy * dy > 400 || nx < 0 || ny < 0 || nx >= w || ny >= h {
                        continue;
                    }
                    let b = body[(ny * w + nx) as usize];
                    if b != a && is_river(b) && !seen.contains(&b) {
                        seen.push(b);
                    }
                }
            }
            for &b in &seen {
                near.entry((a, b)).or_insert((0, k)).0 += 1;
            }
        }
        let parent = |id: u32| {
            list.iter()
                .find(|b| b["id"] == id)
                .and_then(|b| b["parent"].as_u64())
                .unwrap_or(0) as u32
        };
        let mut pairs = 0;
        for (&(a, b), &(cells, k)) in &near {
            if cells <= 100 {
                continue;
            }
            pairs += 1;
            let (x, y) = (
                win.x0 + k as u64 % win.w as u64,
                win.y0 + k as u64 / win.w as u64,
            );
            let how = if parent(a) == b {
                "joins it"
            } else if parent(b) == a {
                "is joined by it"
            } else {
                "unrelated"
            };
            eprintln!(
                "river {a} runs {cells} m within 20 m of river {b} ({how}); from E{x} N{}",
                probe::WORLD.h - 1 - y
            );
        }
        eprintln!("river pairs within 20 m for over 100 m: {pairs}");
        let lake = |id: u32| kind.get(&id) == Some(&"lake");
        // Rivers drawn 5 cells wide, so they show at a glance.
        let mut shown = body.clone();
        for (k, &id) in body.iter().enumerate() {
            if !is_river(id) {
                continue;
            }
            let (x, y) = ((k as i64) % w, (k as i64) / w);
            for (dx, dy) in (-2i64..=2).flat_map(|dx| (-2i64..=2).map(move |dy| (dx, dy))) {
                let (nx, ny) = (x + dx, y + dy);
                if nx >= 0 && ny >= 0 && nx < w && ny < h {
                    shown[(ny * w + nx) as usize] = id;
                }
            }
        }
        probe::shade_tinted("t10-canyon-bodies", &z, win.w, |k| {
            let id = shown[k];
            if is_river(id) {
                let c = (id.wrapping_mul(2654435761) >> 8) as usize;
                let hue = [
                    [6, 2, 2],
                    [2, 6, 2],
                    [6, 6, 1],
                    [6, 2, 6],
                    [1, 6, 6],
                    [6, 4, 1],
                ];
                Some(hue[c % hue.len()])
            } else {
                lake(id).then_some([2, 3, 6])
            }
        });
    }

    #[test]
    fn a_small_pool_on_a_river_is_labelled_as_that_river() {
        let world = World { w: 64, h: 64 };
        let s = setup(world, 16, 1e-5);
        let r = Rasters::memory(world);
        let m = model(
            &s,
            &Layer::memory(world, terrain(world)),
            None,
            "m",
            Some(&r),
        )
        .expect("model");
        // A huge factor makes every lake with river inflow a pool.
        let p = pools(&s, &m, &Geometry::new(1e9));
        assert!(!p.is_empty(), "the humid bowl takes a river");
        let body = Layer::filled(world, 0u32);
        let json = label(&s, &m.water, &r, &body, &p, None, "m").expect("bodies");
        let lakes = json["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|b| b["kind"] == "lake")
            .count();
        let all_lakes = m.water.nodes.0.iter().filter(|n| n.lake).count();
        assert_eq!(lakes, all_lakes - p.len(), "pools are not listed as lakes");
        let ids = body.to_vec().unwrap();
        let node = r.node.to_vec().unwrap();
        for k in 0..ids.len() {
            if node[k] >= 2 && p.contains_key(&m.water.nodes.at(node[k] - 2).key) {
                assert!(ids[k] > 1, "a pool cell carries its river's id");
            }
        }
    }

    /// Dry basins do not depend on the tiling or contraction (here from one cell: the
    /// synthetic world is far below 0.01 km²).
    #[test]
    fn dry_basins_list_alike() {
        let dry = |tile, contract| {
            let (m, ..) = run_pools(tile, contract, |_| BTreeMap::new());
            let w = &m.water;
            dry_basins(w, 1)
                .into_iter()
                .map(|d| (w.minpit[d], w.h.spill[d].to_bits(), w.bal.n[d]))
                .collect::<Vec<_>>()
        };
        let whole = dry(64, false);
        assert!(!whole.is_empty(), "the dune bowl is dry");
        for (tile, contract) in [(64, true), (16, true), (16, false), (8, true)] {
            assert_eq!(
                dry(tile, contract),
                whole,
                "tiles {tile} contraction {contract}"
            );
        }
    }

    #[test]
    fn a_full_lake_always_starts_a_river_at_its_outflow() {
        let world = World { w: 64, h: 64 };
        let s = setup(world, 64, 1e9); // no river reaches the start
        let r = Rasters::memory(world);
        let m = model(
            &s,
            &Layer::memory(world, terrain(world)),
            None,
            "m",
            Some(&r),
        )
        .expect("model");
        let lake = m
            .water
            .nodes
            .0
            .iter()
            .find(|n| n.full && n.lake)
            .expect("a full lake");
        let walk = || trickles(&s, &m.water.nodes, |o| m.water.nodes.at(o).lake, &r);
        let (cells, capped) = walk().expect("trickles");
        assert!(
            cells.contains(&lake.outlet[1]),
            "the trickle starts at the outflow cell"
        );
        assert_eq!(capped, 0);
        let body = Layer::filled(world, 0u32);
        let v = label(&s, &m.water, &r, &body, &BTreeMap::new(), None, "l").expect("label");
        let rivers = v["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|b| b["kind"] == "river")
            .count();
        assert!(rivers >= 1, "the outflow is a labelled river");
        assert!(body.read(Rect::whole(world)).unwrap()[lake.outlet[1] as usize] > 1);
        // Marked cells are walked through again: the same trickles after labelling.
        assert_eq!(walk().expect("trickles").0, cells);
    }

    /// Lake C's outflow crosses a full pond (x 35, y 5-6) and runs on to the main river.
    #[test]
    fn a_lake_outflow_runs_on_through_a_full_pond() {
        let sc = scene_with(true);
        let whole = label_scene(&sc, 64, &sc.pools);
        for tile in [16, 8] {
            assert_eq!(label_scene(&sc, tile, &sc.pools), whole, "tiles {tile}");
        }
        let (body, json) = whole;
        let id = |x, y| body[at(x, y) as usize];
        let c_out = id(32, 2);
        assert_eq!(entry(&json, c_out)["kind"], "river");
        for y in [4, 5, 6, 7, 9] {
            assert_eq!(id(35, y), c_out, "(35, {y}) on lake C's outflow");
        }
        assert_eq!(
            entry(&json, c_out)["parent"],
            id(9, 10),
            "it joins the main river"
        );
    }
}
