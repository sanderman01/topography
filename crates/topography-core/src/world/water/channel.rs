//! 1 m channels. Each thalweg cell's section (width and depth from its discharge, hydraulic
//! geometry) is cut below the river's surface, which is the routing surface R: the ground on
//! dry thalweg cells, the pond level inside ponds. R never rises along a path, so neither does
//! the river. The section is the thalweg's centre cut (down to surface − depth) tapered by
//! 1 − (r/R)² across the half-width R, so no cell loses more than its centre (no wall is cut
//! by its height above a line). A cell within several rivers' half-widths belongs to the
//! river with the larger discharge (its body's largest), then the nearest cell, then the
//! smaller index (review I3b): a tributary inside a larger river's section joins that river
//! there instead of running beside it; a thalweg cell the larger river does not take keeps its
//! own. A cell is lowered only when it ends up under its river's surface, and never more than
//! CHANNEL_CAP_M below raw (R9); a section cell next to a lake whose level stands above the
//! river's surface is left as it is (R14). Lakes, pools and the sea keep their level. A river
//! cell below the surface of the cell it flows into is raised to it over BACKWATER cells
//! (N18). Dry cells within CORRIDOR cells of a lowered river cell form the corridor the GPU
//! relaxation may touch. At coarser spacings (no channels) rivers are their thalweg cells at
//! R and nothing is cut.

use super::CHANNEL_CAP_M;
use super::geom::{Rect, World};
use super::hydraulic::Geometry;
use super::layers::Layer;
use super::pipeline::{Rasters, Setup, Water};
use super::store::{Work, cached, for_tiles};
use crate::acquisition::{Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Window halo: the widest section's radius plus the corridor fits well inside it.
pub(crate) const HALO: u64 = 64;
pub(crate) const CORRIDOR: u64 = 8;
/// A river cell's surface is raised to that of the river cell it flows into over at most
/// this many cells upstream (N18). HALO covers the widest radius + BACKWATER + CORRIDOR.
pub(crate) const BACKWATER: u64 = 16;
/// Section radii are clamped here (cells) so every section reaching a tile's area lies in
/// its window; real rivers stay far below (30 m wide at most, R5).
const MAX_RADIUS: f64 = (HALO - BACKWATER) as f64;

pub(crate) struct Layers {
    pub carved: Layer<f32>,
    pub surface: Layer<f32>,
    pub body: Layer<u32>,
    pub corridor: Layer<u8>,
}

impl Layers {
    pub fn memory(world: World) -> Self {
        Self {
            carved: Layer::filled(world, 0.0),
            surface: Layer::filled(world, f32::NAN),
            body: Layer::filled(world, 0),
            corridor: Layer::filled(world, 0),
        }
    }
    pub fn files(world: World, dir: &Path) -> Result<Self> {
        Ok(Self {
            carved: Layer::open_or_create(world, &dir.join("carved.f32"), 0.0)?,
            surface: Layer::open_or_create(world, &dir.join("water-surface.f32"), f32::NAN)?,
            body: Layer::open_or_create(world, &dir.join("water-body.u32"), 0)?,
            corridor: Layer::open_or_create(world, &dir.join("corridor.u8"), 0)?,
        })
    }
}

/// Per tile: lowered cells, deepest lowering below the ground, volume m³, river cells,
/// deepest lowering below raw.
type Figure = (u64, f32, f64, u64, f32);
/// Per tile: (river body, largest discharge) and (pond ordinal, crossing body).
type Sizes = (Vec<(u32, f32)>, Vec<(u32, u32)>);

/// Whether each cell of `area` lies within Euclidean distance `c` of a `set` cell (a round
/// dilation, so the relaxation stamps no squares), by per-row prefix counts.
fn dilate(area: Rect, set: &[bool], c: usize) -> Vec<bool> {
    let (w, h) = (area.w, area.h);
    // Each row's prefix counts: pre[y * (w + 1) + x] = set cells of row y left of x.
    let mut pre = vec![0u32; h * (w + 1)];
    for y in 0..h {
        let p = &mut pre[y * (w + 1)..(y + 1) * (w + 1)];
        for x in 0..w {
            p[x + 1] = p[x] + u32::from(set[y * w + x]);
        }
    }
    // The disc's half-width at each row offset.
    let half: Vec<usize> = (0..=c).map(|d| (c * c - d * d).isqrt()).collect();
    let mut out = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            out[y * w + x] = (y.saturating_sub(c)..(y + c + 1).min(h)).any(|yy| {
                let hw = half[yy.abs_diff(y)];
                let p = &pre[yy * (w + 1)..];
                p[(x + hw + 1).min(w)] > p[x.saturating_sub(hw)]
            });
        }
    }
    out
}

/// Keeps the two best candidates of distinct river bodies, least first (a body's candidate
/// is replaced only by a better one of the same body). Exact under any insertion order.
fn offer(slots: &mut [(f32, u32, u32); 2], cand: (f32, u32, u32), body: impl Fn(u32) -> u32) {
    let less = |a: (f32, u32, u32), b: (f32, u32, u32)| {
        b.2 == u32::MAX || a.0 < b.0 || (a.0 == b.0 && (a.1, a.2) < (b.1, b.2))
    };
    let b = body(cand.2);
    let same = |s: (f32, u32, u32)| s.2 != u32::MAX && body(s.2) == b;
    if same(slots[0]) {
        if less(cand, slots[0]) {
            slots[0] = cand;
        }
    } else if same(slots[1]) {
        if less(cand, slots[1]) {
            slots[1] = cand;
            if less(slots[1], slots[0]) {
                slots.swap(0, 1);
            }
        }
    } else if less(cand, slots[0]) {
        slots[1] = slots[0];
        slots[0] = cand;
    } else if less(cand, slots[1]) {
        slots[1] = cand;
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn channels(
    s: &Setup,
    w: &Water,
    raw: &Layer<f32>,
    ground: &Layer<f32>,
    r: &Rasters,
    body: &Layer<u32>,
    out: &Layers,
    geo: &Geometry,
    work: Option<&Work>,
    tag: &str,
) -> Result<Value> {
    let world = s.world;
    let tiles = s.tiles();
    let cells_per_m = 1.0 / s.spacing_m;
    let is_lake_node = |n: u32| n >= 2 && w.nodes.at(n - 2).lake;

    // Each river body's largest discharge (the larger river takes a shared section, I3), and
    // the bodies whose thalwegs cross each full pond (by node ordinal).
    let sizes: Vec<Sizes> = for_tiles(
        &tiles,
        s.threads,
        &format!("{tag} river sizes"),
        |i, core| {
            cached(work, &format!("{tag}/chq/{i:05}"), || {
                let (node, flow) = (r.node.read(core)?, r.flow.read(core)?);
                let (q, b) = (r.discharge.read(core)?, body.read(core)?);
                let mut m: BTreeMap<u32, f32> = BTreeMap::new();
                let mut crossing: Vec<(u32, u32)> = Vec::new();
                for k in 0..core.cells() {
                    if flow[k] & 16 == 0 || node[k] == 1 || is_lake_node(node[k]) {
                        continue;
                    }
                    let (x, y) = core.world_xy(k);
                    ensure(b[k] >= 2, format!("thalweg cell ({x}, {y}) has no body"))?;
                    let e = m.entry(b[k]).or_insert(0.0);
                    *e = e.max(q[k]);
                    if node[k] >= 2 && w.nodes.at(node[k] - 2).full {
                        crossing.push((node[k] - 2, b[k]));
                    }
                }
                crossing.sort_unstable();
                crossing.dedup();
                Ok((m.into_iter().collect(), crossing))
            })
        },
    )?;
    let mut size: HashMap<u32, f32> = HashMap::new();
    for (id, q) in sizes.iter().flat_map(|p| p.0.iter()) {
        let e = size.entry(*id).or_insert(0.0);
        *e = e.max(*q);
    }
    // A pond a river crosses is wetted whole at its level as part of the largest river
    // crossing it (smaller id on ties): ruling extending R7/R26; labels only.
    let mut pond_body: HashMap<u32, u32> = HashMap::new();
    for &(o, id) in sizes.iter().flat_map(|p| p.1.iter()) {
        let better = pond_body
            .get(&o)
            .is_none_or(|&b| size[&id] > size[&b] || (size[&id] == size[&b] && id < b));
        if better {
            pond_body.insert(o, id);
        }
    }
    drop(sizes);

    let parts: Vec<Figure> =
        for_tiles(&tiles, s.threads, &format!("{tag} channels"), |i, core| {
            let name = format!("{tag}/ch/{i:05}");
            if let Some(wk) = work {
                if wk.is_done(&name) {
                    return wk.cached(&format!("{name}-figures"), || Ok((0, 0.0, 0.0, 0, 0.0)));
                }
            }
            let win = core.grow(HALO + CORRIDOR, world);
            let area = core.grow(CORRIDOR + BACKWATER, world);
            let (g, rw) = (ground.read(win)?, raw.read(win)?);
            let (level, node, flow) = (r.level.read(win)?, r.node.read(win)?, r.flow.read(win)?);
            let (q, bw) = (r.discharge.read(win)?, body.read(win)?);
            let is_lake = |j: usize| is_lake_node(node[j]);
            let thalweg = |j: usize| flow[j] & 16 != 0 && node[j] != 1 && !is_lake(j);
            let radius = |t: usize| {
                if s.params.channels {
                    (geo.width_m(f64::from(q[t])) / 2.0 * cells_per_m).min(MAX_RADIUS)
                } else {
                    0.0
                }
            };
            let river = |t: usize| if level[t].is_finite() { level[t] } else { g[t] };
            // The centre cut: down to the surface − depth, within the raw − cap clamp.
            let centre = |t: usize| {
                if !s.params.channels {
                    return 0.0;
                }
                let floor = (river(t) - geo.depth_m(f64::from(q[t]))).max(rw[t] - CHANNEL_CAP_M);
                (g[t] - floor).max(0.0)
            };

            // The two best thalwegs of distinct rivers for every area cell: (−body size,
            // distance², window index), least first.
            let none = (0.0, 0, u32::MAX);
            let mut best: Vec<[(f32, u32, u32); 2]> = vec![[none; 2]; area.cells()];
            for (t, id) in bw.iter().enumerate() {
                if !thalweg(t) {
                    continue;
                }
                let qb = size.get(id).copied().ok_or_else(|| {
                    crate::acquisition::AcquisitionError::Invalid(format!(
                        "river body {id} has no size"
                    ))
                })?;
                let (tx, ty) = win.world_xy(t);
                let rad = radius(t);
                let reach = rad.floor() as i64;
                for dy in -reach..=reach {
                    for dx in -reach..=reach {
                        let d2 = (dx * dx + dy * dy) as u32;
                        if f64::from(d2) > rad * rad {
                            continue;
                        }
                        let (Some(x), Some(y)) =
                            (tx.checked_add_signed(dx), ty.checked_add_signed(dy))
                        else {
                            continue;
                        };
                        let Some(a) = area.local(x, y) else { continue };
                        offer(&mut best[a], (-qb, d2, t as u32), |c| bw[c as usize]);
                    }
                }
            }

            let n = area.cells();
            let (mut carved, mut surface, mut ids) =
                (vec![0f32; n], vec![f32::NAN; n], vec![0u32; n]);
            let (mut lowered, mut is_river) = (vec![false; n], vec![false; n]);
            for a in 0..n {
                let (x, y) = area.world_xy(a);
                let j = win.local(x, y).expect("area in window");
                carved[a] = g[j];
                ids[a] = bw[j];
                if node[j] == 1 || is_lake(j) {
                    surface[a] = level[j];
                    continue;
                }
                let ranked: Vec<(usize, u32)> = best[a]
                    .iter()
                    .filter(|c| c.2 != u32::MAX)
                    .map(|c| (c.2 as usize, c.1))
                    .collect();
                let own = (thalweg(j) && ranked.iter().all(|c| c.0 != j)).then_some((j, 0));
                for (t, d2) in ranked.into_iter().chain(own) {
                    let rs = river(t);
                    let below_a_lake = (0..8).any(|k| {
                        world
                            .neighbour(x, y, k)
                            .and_then(|(nx, ny)| win.local(nx, ny))
                            .is_some_and(|j2| is_lake(j2) && level[j2] > rs)
                    });
                    if below_a_lake && t != j {
                        continue;
                    }
                    let rad = radius(t);
                    let taper = if d2 == 0 {
                        1.0
                    } else {
                        1.0 - (f64::from(d2) / (rad * rad)) as f32
                    };
                    let cut = (g[j] - centre(t) * taper)
                        .max(rw[j] - CHANNEL_CAP_M)
                        .min(g[j]);
                    if cut < rs || t == j {
                        lowered[a] = cut < g[j];
                        carved[a] = cut;
                        surface[a] = rs;
                        ids[a] = bw[t];
                        is_river[a] = true;
                        break;
                    }
                }
                // A full pond a river crosses: wet at its level (or a section's higher
                // surface) as one river, like an R26 pool.
                if let Some(&id) = (node[j] >= 2)
                    .then(|| pond_body.get(&(node[j] - 2)))
                    .flatten()
                {
                    surface[a] = if surface[a].is_finite() {
                        surface[a].max(level[j])
                    } else {
                        level[j]
                    };
                    ids[a] = id;
                    is_river[a] = true;
                }
            }
            // Backwater (N18): a river cell whose surface lies below that of the cell it flows
            // into is raised to it, and a dry cell draining into a river surface above its
            // ground is wetted to it as that river, BACKWATER Jacobi steps upstream (water
            // deepens; ground is not touched). Every core cell's chain lies in `area`, so
            // every tiling agrees.
            // (cell, receiver) in `area`, as u32 pairs to keep 4096² tiles small.
            let links: Vec<(u32, u32)> = (0..n)
                .filter(|&a| is_river[a] || !surface[a].is_finite())
                .filter_map(|a| {
                    let (x, y) = area.world_xy(a);
                    let d = flow[win.local(x, y).expect("area in window")] & 15;
                    (d != 0)
                        .then(|| world.neighbour(x, y, d as usize - 1))
                        .flatten()
                        .and_then(|(nx, ny)| area.local(nx, ny))
                        .map(|b| (a as u32, b as u32))
                })
                .collect();
            for _ in 0..BACKWATER {
                // Read every value before writing any (Jacobi).
                let change: Vec<(usize, f32, u32)> = links
                    .iter()
                    .map(|&(a, b)| (a as usize, b as usize))
                    .filter(|&(a, b)| {
                        is_river[b]
                            && if is_river[a] {
                                surface[b] > surface[a]
                            } else {
                                surface[b] > carved[a]
                            }
                    })
                    .map(|(a, b)| (a, surface[b], ids[b]))
                    .collect();
                if change.is_empty() {
                    break;
                }
                for (a, v, id) in change {
                    if !is_river[a] {
                        ids[a] = id;
                        is_river[a] = true;
                    }
                    surface[a] = v;
                }
            }
            // The corridor: dry cells within CORRIDOR cells of a lowered river cell.
            let near = dilate(area, &lowered, CORRIDOR as usize);
            let at = |k: usize| {
                let (x, y) = core.world_xy(k);
                area.local(x, y).expect("core in area")
            };
            let corridor: Vec<u8> = (0..core.cells())
                .map(|k| {
                    let a = at(k);
                    if lowered[a] {
                        2
                    } else if surface[a].is_finite() || !near[a] {
                        0
                    } else {
                        1
                    }
                })
                .collect();
            let cc: Vec<f32> = (0..core.cells()).map(|k| carved[at(k)]).collect();
            let cs: Vec<f32> = (0..core.cells()).map(|k| surface[at(k)]).collect();
            let ci: Vec<u32> = (0..core.cells()).map(|k| ids[at(k)]).collect();
            out.carved.write(core, &cc)?;
            out.surface.write(core, &cs)?;
            out.body.write(core, &ci)?;
            out.corridor.write(core, &corridor)?;
            let mut fig: Figure = (0, 0.0, 0.0, 0, 0.0);
            for k in 0..core.cells() {
                let (x, y) = core.world_xy(k);
                let j = win.local(x, y).expect("core in window");
                let drop = g[j] - cc[k];
                if drop > 0.0 {
                    fig.0 += 1;
                    fig.1 = fig.1.max(drop);
                    fig.2 += f64::from(drop) * s.spacing_m * s.spacing_m;
                    fig.4 = fig.4.max(rw[j] - cc[k]);
                }
                if cs[k].is_finite() && node[j] != 1 && !is_lake(j) {
                    fig.3 += 1;
                }
            }
            if let Some(wk) = work {
                wk.cached(&format!("{name}-figures"), || Ok(fig))?;
                wk.mark_done(&name)?;
            }
            Ok(fig)
        })?;
    Ok(json!({
        "lowered_cells": parts.iter().map(|p| p.0).sum::<u64>(),
        "max_lowering_m": parts.iter().map(|p| p.1).fold(0.0f32, f32::max),
        "max_below_raw_m": parts.iter().map(|p| p.4).fold(0.0f32, f32::max),
        "volume_m3": parts.iter().map(|p| p.2).sum::<f64>().round(),
        "river_cells": parts.iter().map(|p| p.3).sum::<u64>(),
        "width": "3 * Q^0.5 m, Q in m³/s", "depth": "min(0.3 * Q^0.4, 2) m",
        "m3s_per_km2": geo.m3s_per_km2,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::water::bodies::label;
    use crate::world::water::geom::OFFSETS;
    use crate::world::water::pipeline::model;
    use crate::world::water::testing::{setup, terrain};

    type Run = (Vec<f32>, Vec<f32>, Vec<f32>, Vec<u32>, Vec<u8>, Vec<u8>);

    /// The synthetic world at 256²: with 32-cell tiles a window (tile + 2 · 72) is smaller than
    /// the world, so window edges fall among the rivers (N9).
    fn run(tile: u64) -> Run {
        let world = World { w: 256, h: 256 };
        run_on(world, terrain(world), tile, 3.6e4, 1e-5)
    }

    fn run_on(world: World, z: Vec<f32>, tile: u64, m3s_per_km2: f64, river_km2: f64) -> Run {
        run_setup(setup(world, tile, river_km2), z, m3s_per_km2, None)
    }

    /// `rasters` receives the model's node and level layers when given.
    fn run_setup(
        s: Setup,
        z: Vec<f32>,
        m3s_per_km2: f64,
        rasters: Option<&mut (Vec<u32>, Vec<f32>)>,
    ) -> Run {
        let world = s.world;
        let raw = Layer::memory(world, z);
        let r = Rasters::memory(world);
        let m = model(&s, &raw, None, "m", Some(&r)).expect("model");
        let body = Layer::filled(world, 0u32);
        label(&s, &m.water, &r, &body, &BTreeMap::new(), None, "m").expect("bodies");
        let out = Layers::memory(world);
        // Wide channels on a small grid at 3.6e4 m³/s per km² (radii up to about 25 cells,
        // inside HALO with BACKWATER and CORRIDOR).
        let geo = Geometry::new(m3s_per_km2);
        channels(&s, &m.water, &raw, &raw, &r, &body, &out, &geo, None, "c").expect("channels");
        if let Some(v) = rasters {
            *v = (r.node.to_vec().unwrap(), r.level.to_vec().unwrap());
            // Pond cells are no lake nodes.
            assert!(v.0.iter().all(|&n| n < 2 || !m.water.nodes.at(n - 2).lake));
        }
        (
            raw.to_vec().unwrap(),
            out.carved.to_vec().unwrap(),
            out.surface.to_vec().unwrap(),
            out.body.to_vec().unwrap(),
            out.corridor.to_vec().unwrap(),
            r.flow.to_vec().unwrap(),
        )
    }

    #[test]
    fn only_river_cells_are_lowered_and_never_past_the_cap() {
        let (raw, carved, surface, body, corridor, _) = run(32);
        let mut lowered = 0;
        for k in 0..raw.len() {
            assert!(carved[k] <= raw[k], "raised at {k}");
            assert!(
                carved[k] >= raw[k] - CHANNEL_CAP_M - 1e-5,
                "cut past the cap at {k}"
            );
            if carved[k] < raw[k] {
                lowered += 1;
                assert!(
                    surface[k].is_finite() && body[k] > 1,
                    "a dry or unlabelled cell was lowered at {k}"
                );
                assert!(
                    carved[k] < surface[k],
                    "a lowered cell above its river at {k}"
                );
                assert_eq!(corridor[k], 2);
            }
        }
        assert!(lowered > 20, "channels were cut");
        assert!(corridor.contains(&1));
    }

    /// Every wet cell with a receiver: the receiver's surface is not higher.
    fn assert_no_rise(w: usize, surface: &[f32], flow: &[u8]) {
        for (k, &f) in flow.iter().enumerate() {
            if f & 15 == 0 || !surface[k].is_finite() {
                continue;
            }
            let (dy, dx) = OFFSETS[(f & 15) as usize - 1];
            let j = ((k / w) as i64 + dy) as usize * w + ((k % w) as i64 + dx) as usize;
            if surface[j].is_finite() {
                assert!(surface[j] <= surface[k] + 1e-5, "rises from {k} to {j}");
            }
        }
    }

    #[test]
    fn river_surfaces_never_rise_downstream() {
        let (_, _, surface, _, _, flow) = run(32);
        assert_no_rise(256, &surface, &flow);
    }

    #[test]
    fn a_tributary_meets_a_larger_river_at_its_surface() {
        // A trunk valley along row 40 falling east, a tributary valley down column 80 from the
        // north, joining it; the trunk's section is wide (3.6e4 m³/s per km²), so the tributary's
        // last cells lie inside it (N18).
        let world = World { w: 256, h: 80 };
        let z: Vec<f32> = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
                let trunk =
                    50.0 - 0.1 * x + 0.3 * (y - 40.0).abs() + ((i * 7919) % 7) as f32 * 0.002;
                let trib = if y < 40.0 {
                    50.0 - 0.1 * 80.0 + 0.05 * (40.0 - y) + 0.3 * (x - 80.0).abs()
                } else {
                    f32::INFINITY
                };
                trunk.min(trib)
            })
            .collect();
        let (raw, carved, surface, body, _, flow) = run_on(world, z, 32, 3.6e4, 1e-5);
        assert_no_rise(256, &surface, &flow);
        // The tributary's cell next to the trunk thalweg carries the trunk's body (I3b).
        let at = |x: usize, y: usize| y * 256 + x;
        assert!(flow[at(80, 30)] & 16 != 0, "the tributary is a river");
        assert_eq!(
            body[at(80, 39)],
            body[at(100, 40)],
            "it joins the trunk inside its section"
        );
        // Its banks inside the trunk's radius stay wet and keep their channel (the next
        // river takes a cell the larger one rejects).
        let mut cut = 0;
        for y in 6..20 {
            for x in [79, 81] {
                assert!(surface[at(x, y)].is_finite(), "dry bank at ({x}, {y})");
                cut += usize::from(carved[at(x, y)] < raw[at(x, y)]);
            }
        }
        assert!(cut >= 20, "tributary banks cut: {cut} of 28");
    }

    /// A V valley with 1:1 walls: each wall cell loses at most the centre cut (the taper of
    /// the thalweg's cut), never its height above the river (Task 11 lesson).
    #[test]
    fn steep_walls_lose_no_more_than_the_centre_cut() {
        let world = World { w: 128, h: 41 };
        let z: Vec<f32> = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
                40.0 - 0.05 * x + (y - 20.0).abs() + ((i * 7919) % 7) as f32 * 0.001
            })
            .collect();
        // About 5 m wide and 0.4 m deep where the valley leaves the map.
        let m3s = 1e3;
        let (raw, carved, surface, _, _, flow) = run_on(world, z, 32, m3s, 1e-3);
        let geo = Geometry::new(m3s);
        let (s, r) = (setup(world, 32, 1e-3), Rasters::memory(world));
        model(&s, &Layer::memory(world, raw.clone()), None, "m", Some(&r)).expect("model");
        let qmax = r
            .discharge
            .to_vec()
            .unwrap()
            .into_iter()
            .fold(0f32, f32::max);
        let dmax = geo.depth_m(f64::from(qmax));
        assert!(dmax < 1.0 && geo.width_m(f64::from(qmax)) > 4.0);
        let mut bed = 0;
        for k in 0..raw.len() {
            let drop = raw[k] - carved[k];
            assert!(
                drop <= dmax + 1e-4,
                "cut {drop} m at {k}, deeper than the centre"
            );
            if drop > 0.0 {
                assert!(carved[k] < surface[k]);
                bed += usize::from(flow[k] & 16 != 0);
            }
            // A wall cell a metre or more above the river would end above its surface.
            if (k / 128).abs_diff(20) >= 2 {
                assert_eq!(drop, 0.0, "a wall cell was cut at {k}");
            }
        }
        assert!(bed > 50, "the valley floor carries a channel");
    }

    /// A river through a 1 m deep pond (a dam across a valley; no lake: the lake minimum is
    /// out of reach): every pond cell is wet at the pond's level, as one river body.
    #[test]
    fn a_pond_a_river_crosses_is_wet_at_its_level() {
        let world = World { w: 96, h: 41 };
        let z: Vec<f32> = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
                let dam = if (46.0..=50.0).contains(&x) { 1.2 } else { 0.0 };
                40.0 - 0.05 * x + 0.3 * (y - 20.0).abs() + dam + ((i * 7919) % 7) as f32 * 0.001
            })
            .collect();
        let mut s = setup(world, 32, 1e-5);
        s.table.lake_min_m2 = 1e9;
        let mut nl = (Vec::new(), Vec::new());
        let (_, carved, surface, body, _, _) = run_setup(s, z, 1e3, Some(&mut nl));
        let (node, level) = nl;
        let pond = node[20 * 96 + 40];
        assert!(pond >= 2, "the dam holds a pond");
        let cells: Vec<usize> = (0..node.len()).filter(|&k| node[k] == pond).collect();
        let depth = level[cells[0]] - cells.iter().map(|&k| carved[k]).fold(f32::MAX, f32::min);
        assert!(
            cells.len() > 50 && depth > 0.9,
            "{} cells, {depth} m deep",
            cells.len()
        );
        for &k in &cells {
            assert_eq!(surface[k], level[k], "pond cell {k} at the level");
            assert_eq!(body[k], body[cells[0]], "pond cell {k} in one body");
        }
        assert!(body[cells[0]] > 1);
    }

    #[test]
    fn channels_are_alike_at_every_tiling() {
        let a = run(256);
        let b = run(32);
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&a.1), bits(&b.1));
        assert_eq!(bits(&a.2), bits(&b.2));
        assert_eq!(a.3, b.3);
        assert_eq!(a.4, b.4);
    }

    #[test]
    fn a_dilation_reaches_c_cells() {
        let area = Rect {
            x0: 0,
            y0: 0,
            w: 9,
            h: 7,
        };
        let mut set = vec![false; 63];
        set[3 * 9 + 4] = true;
        let d = dilate(area, &set, 2);
        for (k, &v) in d.iter().enumerate() {
            let (x, y) = (k % 9, k / 9);
            let d2 = x.abs_diff(4).pow(2) + y.abs_diff(3).pow(2);
            assert_eq!(v, d2 <= 4, "at {x},{y}: round, not square");
        }
    }
}
