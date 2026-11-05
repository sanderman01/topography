//! Water bodies: the sea (id 1), lakes (labelled nodes by key, R28; river pools take their
//! river's id, R26), and rivers as main stems: at a confluence the branch with the larger
//! discharge keeps its body (ties: the smaller cell index), so a river body runs from its head
//! to its mouth. Writes the ids of the sea, lakes, pools and thalweg cells and returns the body
//! list (v2's `bodies.json` fields).

use super::flow::{Next, ends};
use super::geom::{OFFSETS, Rect, World};
use super::hierarchy::NONE;
use super::layers::Layer;
use super::pipeline::{Rasters, Setup, Water};
use super::store::{Work, cached, for_tiles};
use crate::acquisition::{AcquisitionError, Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Dry basins listed from this area (as v2's world lists).
const DRY_MIN_KM2: f64 = 0.01;

fn invalid(msg: impl Into<String>) -> AcquisitionError {
    AcquisitionError::Invalid(msg.into())
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
    let mut best: Option<(f32, u64)> = None;
    for k in 0..8 {
        let Some((nx, ny)) = world.neighbour(x, y, k) else {
            continue;
        };
        let Some(j) = win.local(nx, ny) else {
            continue;
        };
        let d = flow[j] & 15;
        if flow[j] & 16 == 0 || d == 0 {
            continue;
        }
        let (dy, dx) = OFFSETS[d as usize - 1];
        if (nx as i64 + dx, ny as i64 + dy) != (x as i64, y as i64) {
            continue;
        }
        let c = (q[j], world.index(nx, ny));
        if best.is_none_or(|b| c.0 > b.0 || (c.0 == b.0 && c.1 < b.1)) {
            best = Some(c);
        }
    }
    best.map(|b| b.1)
}

/// A tile's main-stem chains: each core cell's end up its main stem (`Node(head cell)`, `Out`
/// of the tile, or `End` off the thalweg).
fn chains(world: World, core: Rect, win: Rect, flow: &[u8], q: &[f32]) -> Result<Vec<Next>> {
    let at = |k: usize| {
        let (x, y) = core.world_xy(k);
        win.local(x, y).expect("core in window")
    };
    let next = |k: usize| {
        if flow[at(k)] & 16 == 0 {
            return Next::End;
        }
        let (x, y) = core.world_xy(k);
        match main_up(world, win, flow, q, x, y) {
            Some(u) => {
                let (ux, uy) = world.xy(u);
                core.local(ux, uy)
                    .map_or(Next::Out(u), |j| Next::Cell(j as u32))
            }
            None => Next::Node(world.index(x, y)),
        }
    };
    let (end, cycles) = ends(core.cells(), next);
    ensure(cycles == 0, "a main stem runs in a cycle")?;
    Ok(end)
}

/// Pass G per tile: river heads off pools, heads in pools, thalweg ring cells with their
/// chain ends, and each pool's outlet chain end (pools whose outlet cell is in this tile).
type Chased = (Vec<u64>, Vec<u64>, Vec<(u64, Next)>, Vec<(u64, Next)>);
/// A tile's rivers: by id, axial and diagonal steps, and mouths (cell, discharge, parent id).
type RiverAttrs = BTreeMap<u32, (u64, u64, Vec<(u64, f32, u32)>)>;
/// A tile's lakes: by id, cells and the body at its outlet cell.
type LakeAttrs = BTreeMap<u32, (u64, Vec<u32>)>;

/// Writes `body` (0 none, 1 sea, lake ids, river ids on thalweg cells, and on the cells of the
/// pools in `pools` (node key → outlet cell, R26) the id of the river leaving them) and
/// returns the `bodies.json` value.
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
    for &key in pools.keys() {
        ensure(
            w.nodes.get(key).is_some(),
            format!("pool {key} is no water node"),
        )?;
    }
    let in_pool = |n: u32| n >= 2 && pools.contains_key(&w.nodes.at(n - 2).key);

    // Pass G: main-stem chains to heads or out of the tile.
    let chased: Vec<Chased> = for_tiles(&tiles, s.threads, &format!("{tag} G"), |i, core| {
        cached(work, &format!("{tag}/bg/{i:05}"), || {
            let win = core.grow(1, world);
            let (flow, q, node) = (r.flow.read(win)?, r.discharge.read(win)?, r.node.read(win)?);
            let end = chains(world, core, win, &flow, &q)?;
            let (mut heads, mut pool_heads, mut ring) = (Vec::new(), Vec::new(), Vec::new());
            for (k, &e) in end.iter().enumerate() {
                let (x, y) = core.world_xy(k);
                let c = world.index(x, y);
                if e == Next::Node(c) {
                    let j = win.local(x, y).expect("core in window");
                    if in_pool(node[j]) {
                        pool_heads.push(c);
                    } else {
                        heads.push(c);
                    }
                }
                if e != Next::End && core.on_ring(x, y) {
                    ring.push((c, e));
                }
            }
            let mut pool_ends = Vec::new();
            for (&key, &b) in pools {
                let (bx, by) = world.xy(b);
                let Some(k) = core.local(bx, by) else {
                    continue;
                };
                ensure(
                    end[k] != Next::End,
                    format!("pool {key}'s outlet cell {b} is no river cell"),
                )?;
                pool_ends.push((key, end[k]));
            }
            Ok((heads, pool_heads, ring, pool_ends))
        })
    })?;
    let ring: HashMap<u64, Next> = chased.iter().flat_map(|c| c.2.iter().copied()).collect();
    let head_of = |mut t: Next| -> Result<u64> {
        for _ in 0..=ring.len() {
            match t {
                Next::Out(c) => {
                    t = *ring
                        .get(&c)
                        .ok_or_else(|| invalid(format!("main stem enters cell {c}, no river")))?
                }
                Next::Node(h) => return Ok(h),
                other => return Err(invalid(format!("main stem ends in {other:?}"))),
            }
        }
        Err(invalid("main stems cycle across tiles"))
    };
    // Each pool's river: the head of its outlet cell's main stem.
    let mut pool_head: BTreeMap<u64, u64> = BTreeMap::new();
    for &(key, e) in chased.iter().flat_map(|c| c.3.iter()) {
        ensure(
            pool_head.insert(key, head_of(e)?).is_none(),
            format!("pool {key} found twice"),
        )?;
    }
    ensure(
        pool_head.len() == pools.len(),
        "a pool's outlet cell lies outside the world",
    )?;
    // Heads inside pools start a river only when it is a pool's river; other chains there
    // stay inside the pool, labelled as it.
    let pool_rivers: HashSet<u64> = pool_head.values().copied().collect();
    let mut heads: Vec<u64> = chased
        .iter()
        .flat_map(|c| {
            c.0.iter()
                .copied()
                .chain(c.1.iter().copied().filter(|h| pool_rivers.contains(h)))
        })
        .collect();
    drop(chased);
    heads.sort_unstable();

    // Ids: sea 1, lakes by node key (pools are not lakes), rivers by head cell.
    let mut lake_id: BTreeMap<u32, u32> = BTreeMap::new();
    let mut next_id = 2u32;
    for (o, n) in w.nodes.0.iter().enumerate() {
        if n.lake && !pools.contains_key(&n.key) {
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
    let pool_id: HashMap<u64, u32> = pool_head
        .iter()
        .map(|(&key, &h)| Ok((key, river(Next::Node(h))?)))
        .collect::<Result<_>>()?;
    // Full lakes by their outlet cell (outside), for their parent.
    let mut outlet_of: HashMap<u64, Vec<u32>> = HashMap::new();
    for &o in lake_id.keys() {
        let n = w.nodes.at(o);
        if n.full {
            outlet_of.entry(n.outlet[1]).or_default().push(o);
        }
    }

    // Pass H: write ids and gather attributes.
    let parts: Vec<(RiverAttrs, LakeAttrs, u64)> =
        for_tiles(&tiles, s.threads, &format!("{tag} H"), |_, core| {
            let win = core.grow(2, world);
            let (flow, q, node) = (r.flow.read(win)?, r.discharge.read(win)?, r.node.read(win)?);
            let end = chains(world, core, win, &flow, &q)?;
            // The body of a cell of the window: in the core from its chain, outside it from
            // the neighbouring tile's ring.
            let body_at = |x: u64, y: u64| -> Result<u32> {
                let j = win.local(x, y).expect("in window");
                let thalweg = || match core.local(x, y) {
                    Some(k) => river(end[k]),
                    None => river(Next::Out(world.index(x, y))),
                };
                Ok(match node[j] {
                    1 => 1,
                    n if n >= 2 => match pool_id.get(&w.nodes.at(n - 2).key) {
                        Some(&id) => id,
                        None => match lake_id.get(&(n - 2)) {
                            Some(&id) => id,
                            None if flow[j] & 16 != 0 => thalweg()?,
                            None => 0,
                        },
                    },
                    _ if flow[j] & 16 != 0 => thalweg()?,
                    _ => 0,
                })
            };
            let (mut rivers, mut lakes, mut sea) = (RiverAttrs::new(), LakeAttrs::new(), 0u64);
            let mut ids = vec![0u32; core.cells()];
            for (k, id) in ids.iter_mut().enumerate() {
                let (x, y) = core.world_xy(k);
                let j = win.local(x, y).expect("core in window");
                *id = body_at(x, y)?;
                if node[j] == 1 {
                    sea += 1;
                } else if node[j] >= 2 && lake_id.contains_key(&(node[j] - 2)) {
                    lakes.entry(*id).or_default().0 += 1;
                }
                if let Some(list) = outlet_of.get(&world.index(x, y)) {
                    for o in list {
                        lakes.entry(lake_id[o]).or_default().1.push(*id);
                    }
                }
                if flow[j] & 16 == 0 {
                    continue;
                }
                ensure(*id >= 2, format!("thalweg cell ({x}, {y}) has no body"))?;
                let dir = flow[j] & 15;
                let e = rivers.entry(*id).or_default();
                if dir > 4 {
                    e.1 += 1;
                } else {
                    e.0 += 1;
                }
                let (rx, ry) = world
                    .neighbour(x, y, dir as usize - 1)
                    .ok_or_else(|| invalid("a receiver lies outside the world"))?;
                let rj = win.local(rx, ry).expect("receiver in window");
                let below = body_at(rx, ry)?;
                if flow[rj] & 16 == 0 || below != *id {
                    e.2.push((world.index(x, y), q[j], below));
                }
            }
            body.write(core, &ids)?;
            Ok((rivers, lakes, sea))
        })?;

    // Merge.
    let km2 = s.cell_km2();
    let (mut rivers, mut lakes, mut sea) = (RiverAttrs::new(), LakeAttrs::new(), 0u64);
    for (rv, lk, se) in parts {
        sea += se;
        for (id, (a, d, m)) in rv {
            let e = rivers.entry(id).or_default();
            (e.0, e.1) = (e.0 + a, e.1 + d);
            e.2.extend(m);
        }
        for (id, (n, p)) in lk {
            let e = lakes.entry(id).or_default();
            e.0 += n;
            e.1.extend(p);
        }
    }
    let cell = |c: u64| {
        let (x, y) = world.xy(c);
        json!([x, y])
    };
    let mut list = vec![json!({
        "id": 1, "kind": "sea", "level_m": 0.0, "parent": 0, "area_km2": sea as f64 * km2,
    })];
    for (&o, &id) in &lake_id {
        let n = w.nodes.at(o);
        let (cells, parents) = lakes.remove(&id).unwrap_or_default();
        ensure(
            parents.len() == usize::from(n.full),
            format!("lake {} has {} outlet cells", n.key, parents.len()),
        )?;
        list.push(json!({
            "id": id, "kind": "lake", "node": n.key, "level_m": n.level,
            "area_km2": cells as f64 * km2, "max_depth_m": n.level - n.low,
            "exit_cell": if n.full { cell(n.outlet[1]) } else { Value::Null },
            "endorheic": !n.full, "parent": parents.first().copied().unwrap_or(0),
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
        list.push(json!({
            "id": id, "kind": "river",
            "length_km": (axial as f64 + diagonal as f64 * std::f64::consts::SQRT_2)
                * s.spacing_m / 1000.0,
            "drainage_km2": q, "mouth_cell": cell(mouth), "parent": parent,
        }));
    }
    let min_cells = (DRY_MIN_KM2 / km2).ceil() as u64;
    for d in dry_basins(w, min_cells) {
        list.push(json!({
            "id": next_id, "kind": "dry", "level_m": w.h.spill[d],
            "area_km2": w.bal.n[d] as f64 * km2, "endorheic": true, "parent": 0,
        }));
        next_id += 1;
    }
    Ok(json!({
        "schema_version": 1, "kind": "water_bodies",
        "cells": "[column, row] from the north-west corner; a lake's exit_cell is the first cell \
            its routed flow enters outside it",
        "note": "hydrology v3: lakes by node (smallest pit), rivers as main stems (larger \
            discharge keeps the body), river pools labelled as their river",
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
    use crate::world::water::pipeline::{Model, model};
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

    /// Real terrain: the canyon window (E27169 N44546, 2,048², river start 0.05 km², the
    /// layout's classes, uniform rain). River bodies, their length, and river pairs running
    /// within 20 m of each other for over 100 m (a double river). Writes
    /// t10-canyon-bodies.png, each river body in its own colour, lakes blue.
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
        let mut s = setup(world, 512, 0.05);
        s.table.lake_min_m2 = Table::parse(DEFAULT).expect("table").lake_min_m2;
        s.threads = 4;
        s.nw_m = [win.x0 as f64, (probe::WORLD.h - win.y0) as f64];
        s.classes = Some(Classes::new(Layer::memory(world, class), 1, world).expect("classes"));
        let r = Rasters::memory(world);
        let m = model(&s, &Layer::memory(world, z.clone()), None, "m", Some(&r)).expect("model");
        let body = Layer::filled(world, 0u32);
        let json = label(&s, &m.water, &r, &body, &BTreeMap::new(), None, "m").expect("bodies");
        let body = body.to_vec().unwrap();
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
        for k in 0..body.len() {
            let a = body[k];
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
}
