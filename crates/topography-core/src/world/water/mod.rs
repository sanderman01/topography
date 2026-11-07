//! Hydrology v3: water as one field (ground, water surface, discharge) from one model at every
//! level. Exact graph passes on tiles: the priority-flood spill graph (Barnes 2016), the
//! depression hierarchy and Fill-Spill-Merge (Barnes, Callaghan and Wickert 2020), fall-line
//! receivers, flat ponds (Barnes 2014) and accumulation (Barnes 2017); at 1 m also channels.
//! See docs/superpowers/specs/2026-11-05-hydrology-v3-design.md.

mod balance;
mod bodies;
mod breach;
mod channel;
mod checks;
mod contract;
mod flats;
mod flow;
mod geom;
mod hierarchy;
mod hydraulic;
mod layers;
mod nodes;
mod pipeline;
#[cfg(test)]
mod probe;
mod rain;
mod receivers;
mod store;
mod table;
#[cfg(test)]
mod testing;
mod watershed;

use crate::acquisition::{AcquisitionError, Result, digest, ensure, io_context, read_small};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Judgment value (spec): standing water at least this deep is a lake; shallower is a pond.
pub(crate) const LAKE_DEPTH_M: f32 = 0.3;
/// Channel depth cap at 1 m; a 1 m breach plus its channel cuts at most this much too.
pub(crate) const CHANNEL_CAP_M: f32 = 2.0;
/// Flowing water over a bed that is not cut runs this deep (export ruling: every wet cell
/// stands above its ground). At most half the checks' 1 cm tolerance (`SILL_M`), so f32
/// rounding at world heights (ulp 4.9e-4 m at 4975 m) cannot push a min-depth surface over it.
pub(crate) const MIN_DEPTH_M: f32 = 0.005;
const _: () = assert!(MIN_DEPTH_M <= checks::SILL_M / 2.0);
/// A gorge's bed falls over at least this many times its deepest cut (spec).
pub(crate) const GORGE_RATIO: f32 = 20.0;
/// Gorge cells lie within this many cells of their spill cell, so every tiling decides alike.
pub(crate) const BREACH_RADIUS: u64 = 256;
/// Judgment value: rivers from this size (km² at mean rain) are main rivers for the
/// one-thalweg check.
pub(crate) const MAIN_RIVER_KM2: f64 = 1.0;
/// Export ids are 16-bit (fewer than 65,535 water bodies); a run must stay well under that
/// (judgment value, review C2).
pub(crate) const BODY_BUDGET: usize = 50_000;
/// Default tile side in cells.
pub const DEFAULT_TILE: u64 = 4096;

/// A run's thresholds: the table's level for the grid spacing (review N19).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Params {
    /// River start, km² at mean rain.
    pub river_km2: f64,
    /// Deepest breach cut (at 1 m the channel cap: breach plus channel stay within it through
    /// the channels' raw − 2 m clamp).
    pub breach_cap_m: f32,
    /// Gorge cells lie within this many cells of their spill cell (tests lower it so a gorge
    /// crosses a tile seam).
    pub breach_radius: u64,
    /// Carve channels (1 m only).
    pub channels: bool,
}

impl Params {
    /// The `change_max` check's bound: this level's breach cap + 0.5 m.
    pub fn change_max_m(&self) -> f32 {
        self.breach_cap_m + 0.5
    }
}

pub(crate) fn params(table: &table::Table, spacing_m: f64, river_km2: Option<f64>) -> Params {
    let level = table.level(spacing_m);
    Params {
        river_km2: river_km2.unwrap_or(level.river_km2),
        breach_cap_m: level.breach_cap_m,
        breach_radius: BREACH_RADIUS,
        channels: spacing_m < 2.0,
    }
}

pub struct Options {
    pub grid: PathBuf,
    pub work: PathBuf,
    pub tile: u64,
    pub threads: usize,
    /// Water table TOML; the built-in configs/hydrology/water-v2.toml when absent.
    pub water: Option<PathBuf>,
    /// River start (km² at mean rain); by spacing when absent.
    pub river_km2: Option<f64>,
}

/// The run's setup from a grid directory (grid.json with a `moisture` entry, elevation.f32,
/// optional classes.u8 with `classes_cell_m`), the raw ground, and the stamp a work directory
/// must match to be resumed.
pub(crate) fn setup(
    grid: &Path,
    tile: u64,
    threads: usize,
    water: Option<&Path>,
    river_km2: Option<f64>,
) -> Result<(pipeline::Setup, layers::Layer<f32>, Value)> {
    ensure(tile >= 16, "--tile must be at least 16 cells")?;
    ensure(
        river_km2.is_none_or(|a| a.is_finite() && a > 0.0),
        "--river-km2 must be positive",
    )?;
    let bytes = read_small(&grid.join("grid.json"))?;
    let manifest: Map<String, Value> = serde_json::from_slice(&bytes)?;
    let g = crate::world::hydrology::grid::shape(&manifest)?;
    let world = geom::World {
        w: g.w as u64,
        h: g.h as u64,
    };
    ensure(
        world.w >= 3 && world.h >= 3 && world.w * world.h < u32::MAX as u64 * 2,
        "grid size out of range",
    )?;
    let raw = layers::Layer::open(world, &grid.join("elevation.f32"))?;
    let classes = if grid.join("classes.u8").exists() {
        let cell_m = manifest
            .get("classes_cell_m")
            .and_then(Value::as_f64)
            .unwrap_or(g.spacing_m);
        let k = cell_m / g.spacing_m;
        ensure(
            k >= 1.0 && k.fract() == 0.0,
            "classes_cell_m must be a whole multiple of spacing_m",
        )?;
        let k = k as u64;
        let cw = geom::World {
            w: world.w.div_ceil(k),
            h: world.h.div_ceil(k),
        };
        let layer = layers::Layer::open(cw, &grid.join("classes.u8"))?;
        Some(layers::Classes::new(layer, k, world)?)
    } else {
        None
    };
    let (text, source) = match water {
        Some(p) => (
            io_context(format!("read {}", p.display()), fs::read_to_string(p))?,
            p.display().to_string(),
        ),
        None => (table::DEFAULT.to_string(), "built-in water-v2.toml".into()),
    };
    let rain = rain::Rain::read(grid, &manifest)?;
    // Large inputs by size and modification time; the small moisture field by digest.
    let identity = |name: &str| -> Result<Value> {
        let path = grid.join(name);
        if !path.exists() {
            return Ok(Value::Null);
        }
        let m = io_context(format!("inspect {name}"), fs::metadata(&path))?;
        Ok(json!({"bytes": m.len(), "modified": format!("{:?}", m.modified().ok())}))
    };
    let moisture = rain.entry["file"].as_str().unwrap_or("moisture.f32");
    let stamp = json!({
        "grid.json": digest(&bytes),
        "elevation.f32": identity("elevation.f32")?,
        "classes.u8": identity("classes.u8")?,
        "moisture": {"file": moisture, "sha256": crate::acquisition::hash_file(&grid.join(moisture))?.0},
        "tile": tile,
        "water_table": {"source": source, "sha256": digest(text.as_bytes())},
        "river_km2": river_km2,
        "version": env!("CARGO_PKG_VERSION"),
        "code": code_identity()?,
    });
    let table = table::Table::parse(&text)?;
    let params = params(&table, g.spacing_m, river_km2);
    let s = pipeline::Setup {
        world,
        nw_m: g.nw_m,
        spacing_m: g.spacing_m,
        tile,
        threads: threads.max(1),
        classes,
        table,
        rain,
        params,
        contract: true,
    };
    Ok((s, raw, stamp))
}

/// The running binary's digest: any code change invalidates a work directory's caches,
/// even within one crate version (hashed once per process).
fn code_identity() -> Result<String> {
    static CODE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    if let Some(c) = CODE.get() {
        return Ok(c.clone());
    }
    let exe = io_context("locate the running binary", std::env::current_exe())?;
    let (sha, _) = crate::acquisition::hash_file(&exe)?;
    Ok(CODE.get_or_init(|| sha).clone())
}

/// Peak resident memory where the OS reports it (Linux), for the logs.
fn peak() -> String {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .map(|l| format!(", peak {}", l[6..].trim()))
        })
        .unwrap_or_default()
}

/// A stage's cache tag: its name and a digest of everything it depends on (the previous
/// stage's tag and its own settings), so changed settings never resume stale results.
fn stage_tag(name: &str, inputs: &Value) -> String {
    format!("{name}-{}", &digest(inputs.to_string().as_bytes())[..12])
}

/// The stages' cache tags. Each digests the tags of the stages whose outputs it reads and its
/// own settings; the stamp covers the grid, water table, tile size and binary. The flags
/// (`walls`, `bed_grade`, ...) mark code changes; any other code change needs a rerun from
/// that stage (docs/workflows/terrain-cascade.md, "Rerunning from a stage").
#[derive(Clone, Debug, PartialEq)]
struct Tags {
    m1: String,
    b: String,
    m2: String,
    l: String,
    c: String,
    m3: String,
    s: String,
}

impl Tags {
    /// m1 reads the raw ground.
    fn m1(p: &Params, contract: bool) -> String {
        stage_tag(
            "m1",
            &json!({"river_km2": p.river_km2, "contract": contract}),
        )
    }
    /// b reads the raw ground and m1.
    fn b(m1: &str, p: &Params, m3s_per_km2: f64) -> String {
        stage_tag(
            "b",
            &json!({"after": m1, "cap": p.breach_cap_m, "radius": p.breach_radius,
                    "channels": p.channels, "m3s_per_km2": m3s_per_km2,
                    "no_cut_deg": channel::NO_CUT_DEG, "taper_deg": breach::TAPER_DEG, "walls": true}),
        )
    }
    /// m2 reads the breached ground (b).
    fn m2(b: &str) -> String {
        stage_tag("m2", &json!({"after": b}))
    }
    /// l reads m2 and the river pools found on it.
    fn l(m2: &str, pools: &BTreeMap<u64, u64>) -> String {
        stage_tag("l", &json!({"after": m2, "pools": pools}))
    }
    /// c reads the raw and breached grounds, m2 and the labels (l, after m2 and b).
    fn c(l: &str, m3s_per_km2: f64) -> String {
        stage_tag(
            "c",
            &json!({"after": l, "m3s_per_km2": m3s_per_km2, "cap": CHANNEL_CAP_M,
                    "halo": channel::HALO, "backwater": channel::BACKWATER,
                    "walls": channel::WALL_ROUNDS, "restore": channel::RESTORE_ROUNDS,
                    "no_cut_deg": channel::NO_CUT_DEG, "bed_grade": true,
                    "pond_rims": true, "lake_mouths": true, "rim_sill_m": checks::SILL_M,
                    "min_depth_m": MIN_DEPTH_M, "shore_dry": true}),
        )
    }
    /// The final routing reads the ground and the prior (breached) ground its receivers keep,
    /// by content (sha256), with m1's settings. So `finish` on the carved ground reuses the
    /// run's m3, a channels rerun that leaves the carved ground alone keeps it, and another
    /// ground never reads stale passes.
    fn m3(m1: &str, ground: &str, breached: &str) -> String {
        stage_tag(
            "m3",
            &json!({"after": m1, "ground": ground, "breached": breached, "kept": true, "kept_ties": true}),
        )
    }
    /// Settle reads m3 on the carved ground and the channels' surfaces, bodies and labelled
    /// lakes (c): m3's content tag alone survives a channels rerun.
    fn s(m3: &str, c: &str) -> String {
        stage_tag(
            "s",
            &json!({"after": m3, "channels": c, "lakes_at_label": true}),
        )
    }
    /// Every tag, from the settings, the pools on m2 and the carved and breached grounds'
    /// sha256 (what `finish` recomputes; `run` builds them stage by stage alike).
    fn all(
        p: &Params,
        contract: bool,
        m3s_per_km2: f64,
        pools: &BTreeMap<u64, u64>,
        carved: &str,
        breached: &str,
    ) -> Self {
        let m1 = Self::m1(p, contract);
        let b = Self::b(&m1, p, m3s_per_km2);
        let m2 = Self::m2(&b);
        let l = Self::l(&m2, pools);
        let c = Self::c(&l, m3s_per_km2);
        let m3 = Self::m3(&m1, carved, breached);
        let s = Self::s(&m3, &c);
        Self {
            m1,
            b,
            m2,
            l,
            c,
            m3,
            s,
        }
    }
    fn list(&self) -> Value {
        json!([self.m1, self.b, self.m2, self.l, self.c, self.m3, self.s])
    }
}

fn sha256(path: &Path) -> Result<String> {
    Ok(crate::acquisition::hash_file(path)?.0)
}

/// The final routing's tag on `ground` (see `Tags::m3`).
fn final_tag(m1_tag: &str, ground: &Path, breached: &Path) -> Result<String> {
    Ok(Tags::m3(m1_tag, &sha256(ground)?, &sha256(breached)?))
}

/// Settle tag directories other than `keep`. Settle rewrites the channels' surfaces and
/// bodies in place, so it must never run on surfaces another settle wrote.
fn other_settles(work: &store::Work, keep: &str) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for e in io_context("list the work directory", fs::read_dir(work.path("")))? {
        let e = io_context("list the work directory", e)?;
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("s-") && name != keep && e.path().is_dir() {
            out.push(e.path());
        }
    }
    Ok(out)
}

/// Removes the final-routing sets (13 B per cell: level, node, discharge, flow; plus their
/// pass caches) of other ground revisions once a finish on `keep` is published. A later
/// finish on such a ground recomputes them.
fn drop_other_finals(work: &store::Work, keep: &str) -> Result<()> {
    let dir = work.path("");
    for e in io_context("list the work directory", fs::read_dir(&dir))? {
        let e = io_context("list the work directory", e)?;
        let name = e.file_name().to_string_lossy().to_string();
        let tag = name.split(['-', '.']).take(2).collect::<Vec<_>>().join("-");
        if !name.starts_with("m3-") || tag == keep {
            continue;
        }
        let path = e.path();
        let gone = if path.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        io_context(format!("remove {}", path.display()), gone)?;
    }
    Ok(())
}

/// Runs model → breach → model → bodies → channels into `work` (resumable). Returns a
/// one-line summary; the figures go to `work/stages.json`.
pub fn run(o: &Options) -> Result<String> {
    let (s, raw, stamp) = setup(&o.grid, o.tile, o.threads, o.water.as_deref(), o.river_km2)?;
    let work = store::Work::open(&o.work, &stamp)?;
    let wk = Some(&work);
    let geo = hydraulic::Geometry::new(s.table.m3s_per_km2);
    let p = &s.params;
    let m1_tag = Tags::m1(p, s.contract);
    let b_tag = Tags::b(&m1_tag, p, geo.m3s_per_km2);
    let m1 = pipeline::model(&s, &raw, wk, &m1_tag, None)?;
    let breached = layers::Layer::open_or_create(s.world, &work.path("breached.f32"), 0.0)?;
    let breaches = breach::breach(&s, &m1, &raw, &breached, &geo, wk, &b_tag)?;
    drop(m1);
    let m2_tag = Tags::m2(&b_tag);
    let rasters = pipeline::Rasters::files(s.world, &work.path(""), "m2")?;
    let m2 = pipeline::model(&s, &breached, wk, &m2_tag, Some(&rasters))?;
    let body = layers::Layer::open_or_create(s.world, &work.path("body.u32"), 0)?;
    let pools = bodies::pools(&s, &m2, &geo);
    let l_tag = Tags::l(&m2_tag, &pools);
    let done = format!("{l_tag}/bodies");
    let bodies = if work.is_done(&done) {
        serde_json::from_slice(&io_context(
            "read bodies.json",
            fs::read(work.path("bodies.json")),
        )?)?
    } else {
        let v = bodies::label(&s, &m2.water, &rasters, &body, &pools, wk, &l_tag)?;
        io_context(
            "write bodies.json",
            fs::write(work.path("bodies.json"), serde_json::to_vec_pretty(&v)?),
        )?;
        work.mark_done(&done)?;
        v
    };
    let c_tag = Tags::c(&l_tag, geo.m3s_per_km2);
    // Channels about to (re)write the surfaces void every earlier settle of them.
    let c_done = (0..s.tiles().len()).all(|i| work.is_done(&format!("{c_tag}/ch/{i:05}")));
    if !c_done {
        for d in other_settles(&work, "")? {
            io_context(format!("remove {}", d.display()), fs::remove_dir_all(&d))?;
        }
    }
    let out = channel::Layers::files(s.world, &work.path(""))?;
    let channels = channel::channels(
        &s, &m2.water, &raw, &breached, &rasters, &body, &out, &geo, wk, &c_tag,
    )?;
    let n = |kind: &str| {
        bodies["bodies"]
            .as_array()
            .map_or(0, |b| b.iter().filter(|e| e["kind"] == kind).count())
    };
    // Node area (log2 m² bins) and depth (0.1 m bins) histograms, for the lake-count probe
    // before gate 2 (review C2.4).
    let nodes = &m2.water.nodes.0;
    let cell_m2 = s.spacing_m * s.spacing_m;
    let (mut area, mut depth) = (vec![0u64; 32], vec![0u64; 40]);
    for nd in nodes {
        area[((nd.cells as f64 * cell_m2).max(1.0).log2() as usize).min(31)] += 1;
        depth[(((nd.level - nd.low) * 10.0).max(0.0) as usize).min(39)] += 1;
    }
    let mut stages = json!({
        "breach": breaches,
        "channels": channels,
        "leaves": m2.water.h.leaves, "depressions": m2.water.h.len(),
        "settled_trees_solved_in_tiles": m2.water.solved.len(),
        "nodes": nodes.len(),
        "lakes": n("lake"), "rivers": n("river"), "dry_basins": n("dry"), "river_pools": pools.len(),
        "ponds": nodes.iter().filter(|nd| !nd.lake).count(),
        "node_area_log2_m2_hist": area, "node_depth_dm_hist": depth,
        // Lake-balance supply (cells × r, r up to 3) that overflowed to the sea or off the map.
        "to_sea_r_cells": m2.water.bal.to_sea as f64 / table::MILLI as f64,
    });
    drop(m2);
    // The routing on the carved ground (m3: `finish` checks this ground on the same one) and
    // the written water settled on it (review: drainage_rises). m3 keeps m2's receivers where
    // they are not higher (ties keep them), so the routing stays on the carved rivers.
    let carved = layers::Layer::open(s.world, &work.path("carved.f32"))?;
    let m3_tag = final_tag(
        &m1_tag,
        &work.path("carved.f32"),
        &work.path("breached.f32"),
    )?;
    let s_tag = Tags::s(&m3_tag, &c_tag);
    if !work.path(&format!("{s_tag}/settled.bin")).exists() {
        let old = other_settles(&work, &s_tag)?;
        ensure(
            old.is_empty(),
            format!(
                "the channels' surfaces were settled already ({}); delete {} to rerun from \
                 the channels",
                old.iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                work.path(&c_tag).display()
            ),
        )?;
    }
    let lakes: HashSet<u32> = bodies["bodies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["kind"] == "lake")
        .filter_map(|b| b["id"].as_u64().and_then(|id| u32::try_from(id).ok()))
        .collect();
    let settled = work.cached(&format!("{s_tag}/settled"), || {
        let finals = pipeline::Rasters::files(s.world, &work.path(""), &m3_tag)?;
        pipeline::model_on(&s, &carved, Some(&breached), wk, &m3_tag, Some(&finals))?;
        channel::settle(&s, &finals, &carved, &out, &lakes, wk, &s_tag)
    })?;
    let lift_cell = (settled.3 != u64::MAX).then(|| s.world.xy(settled.3));
    stages["settled"] = json!({"raised_cells": settled.0, "wetted_cells": settled.1,
                               "max_raise_m": settled.2,
                               "max_raise_cell": lift_cell.map(|(x, y)| [x, y]),
                               "max_raise_body": settled.4});
    if let Some((x, y)) = lift_cell {
        eprintln!(
            "water: settle raised {} cells ({} wetted), most {:.2} m at ({x}, {y}) body {}",
            settled.0, settled.1, settled.2, settled.4
        );
    }
    stages["tags"] = json!([m1_tag, b_tag, m2_tag, l_tag, c_tag, m3_tag, s_tag]);
    io_context(
        "write stages.json",
        fs::write(
            work.path("stages.json"),
            serde_json::to_vec_pretty(&stages)?,
        ),
    )?;
    eprintln!("water: done{}", peak());
    Ok(format!(
        "{}: {} lakes, {} rivers, {} breaches, {} river cells; next: water-finish",
        o.work.display(),
        n("lake"),
        n("river"),
        stages["breach"]["breaches"],
        stages["channels"]["river_cells"],
    ))
}

pub struct FinishOptions {
    pub grid: PathBuf,
    pub work: PathBuf,
    /// The final ground (`work/carved.f32`).
    pub ground: PathBuf,
    pub out: PathBuf,
    pub tile: u64,
    pub threads: usize,
    pub water: Option<PathBuf>,
    pub river_km2: Option<f64>,
    /// Review spots (east, north in metres) for the `dam_drops_at_spots` figure; checks only,
    /// never part of the work stamp.
    pub spots: Vec<[f64; 2]>,
}

pub struct Outcome {
    pub out: PathBuf,
    pub passed: bool,
    pub summary: String,
}

/// Checks the finished run on the final ground and publishes `out` (also when a check fails,
/// so the report can be read).
pub fn finish(o: &FinishOptions) -> Result<Outcome> {
    ensure(
        !o.out.exists(),
        format!("{} already exists", o.out.display()),
    )?;
    let (s, raw, stamp) = setup(&o.grid, o.tile, o.threads, o.water.as_deref(), o.river_km2)?;
    let work = store::Work::open(&o.work, &stamp)?;
    let stages: Value = serde_json::from_slice(&io_context(
        "read stages.json (run `world water` to the end first)",
        fs::read(work.path("stages.json")),
    )?)?;
    // The tags recomputed, not read: a run that stopped after rewriting a stage leaves an
    // older stages.json.
    let stale = "stages.json is not from this work directory's last stages; run `world water` \
                 to the end first";
    let geo = hydraulic::Geometry::new(s.table.m3s_per_km2);
    let m1_tag = Tags::m1(&s.params, s.contract);
    let b_tag = Tags::b(&m1_tag, &s.params, geo.m3s_per_km2);
    let m2_tag = Tags::m2(&b_tag);
    // Checked before m2 is read, so a stale m2 is refused rather than recomputed.
    let first = [&m1_tag, &b_tag, &m2_tag];
    ensure(
        (0..3).all(|i| stages["tags"][i] == first[i].as_str()),
        stale,
    )?;
    let rasters = pipeline::Rasters::open(s.world, &work.path(""), "m2")?;
    let breached = layers::Layer::open(s.world, &work.path("breached.f32"))?;
    // The second run's nodes (cached): the lakes as written.
    let m2 = pipeline::model(&s, &breached, Some(&work), &m2_tag, None)?;
    let pools = bodies::pools(&s, &m2, &geo);
    let carved = work.path("carved.f32");
    let (carved_sha, breached_sha) = (sha256(&carved)?, sha256(&work.path("breached.f32"))?);
    let tags = Tags::all(
        &s.params,
        s.contract,
        geo.m3s_per_km2,
        &pools,
        &carved_sha,
        &breached_sha,
    );
    ensure(stages["tags"] == tags.list(), stale)?;
    ensure(
        work.is_done(&format!("{}/bodies", tags.l))
            && work.path(&format!("{}/settled.bin", tags.s)).exists(),
        "run `world water` to the end first",
    )?;
    let mut bodies: Value = serde_json::from_slice(&io_context(
        "read bodies.json",
        fs::read(work.path("bodies.json")),
    )?)?;
    // Lake levels as the exact f32 the rasters hold (earlier label runs rounded them to cm).
    for b in bodies["bodies"].as_array_mut().into_iter().flatten() {
        if b["kind"] == "lake" {
            let key = b["node"].as_u64().unwrap_or(u64::MAX);
            let n = m2.water.nodes.get(key).ok_or_else(|| {
                AcquisitionError::Invalid(format!("lake {}: node {key} not in the model", b["id"]))
            })?;
            b["level_m"] = json!(f64::from(n.level));
        }
    }
    let list = bodies["bodies"].as_array().cloned().unwrap_or_default();
    // Lake and sea levels as the export reads them (f64 → f32), for `lake_level_json`.
    let json_levels: checks::Levels = list
        .iter()
        .filter(|b| b["kind"] == "lake" || b["kind"] == "sea")
        .filter_map(|b| {
            let id = u32::try_from(b["id"].as_u64()?).ok()?;
            Some((id, b["level_m"].as_f64().map_or(f32::NAN, |v| v as f32)))
        })
        .collect();
    let mut lakes = checks::LakeInfo::new();
    let mut lake_short = 0u64;
    for b in list.iter().filter(|b| b["kind"] == "lake") {
        let id = u32::try_from(b["id"].as_u64().unwrap_or(0))
            .map_err(|_| AcquisitionError::Invalid("a lake id exceeds u32".into()))?;
        let key = b["node"].as_u64().unwrap_or(u64::MAX);
        let n = m2.water.nodes.get(key).ok_or_else(|| {
            AcquisitionError::Invalid(format!("lake {id}: node {key} not in the model"))
        })?;
        lakes.insert(id, (n.level, n.full, n.outlet, n.need));
        // Its labelled cells against the balance's need.
        let area = b["area_km2"].as_f64().unwrap_or(0.0);
        if n.need > 0 && ((area / s.cell_km2()).round() as u64) < n.need {
            lake_short += 1;
        }
    }
    // Every full node's exit (lakes, R26 pools, ponds) for the dam figures, by tile.
    let lake_of: HashMap<u64, u32> = lakes
        .iter()
        .filter_map(|(&id, _)| {
            list.iter()
                .find(|b| b["kind"] == "lake" && b["id"].as_u64() == Some(u64::from(id)))
                .and_then(|b| b["node"].as_u64())
                .map(|key| (key, id))
        })
        .collect();
    let mut exits: Vec<Vec<checks::Exit>> = vec![Vec::new(); s.tiles().len()];
    for n in m2.water.nodes.0.iter().filter(|n| n.full) {
        let (x, y) = s.world.xy(n.outlet[1]);
        exits[geom::tile_of(s.world, s.tile, x, y)].push(checks::Exit {
            key: n.key,
            level: n.level,
            outlet: n.outlet,
            lake: lake_of.get(&n.key).copied(),
        });
    }
    drop(m2);
    // The third model run, on the final ground (R27); `run` made it already for the carved
    // ground (same tag).
    let ground = layers::Layer::open(s.world, &o.ground)?;
    let same = |a: &Path, b: &Path| matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y);
    let m3_tag = if same(&o.ground, &carved) {
        tags.m3.clone()
    } else {
        Tags::m3(&m1_tag, &sha256(&o.ground)?, &breached_sha)
    };
    // Receivers kept from the breached ground where still valid (scale diagnosis 3).
    let m3 = pipeline::model_on(
        &s,
        &ground,
        Some(&breached),
        Some(&work),
        &m3_tag,
        Some(&pipeline::Rasters::files(s.world, &work.path(""), &m3_tag)?),
    )?;
    let ponds = m3.water.nodes.0.iter().filter(|n| !n.lake).count();
    drop(m3);
    let finals = pipeline::Rasters::open(s.world, &work.path(""), &m3_tag)?;
    let layers = checks::CheckLayers {
        raw,
        ground,
        surface: layers::Layer::open(s.world, &work.path("water-surface.f32"))?,
        body: layers::Layer::open(s.world, &work.path("water-body.u32"))?,
        level: finals.level.clone(),
        node: finals.node.clone(),
        flow: finals.flow.clone(),
        discharge: finals.discharge.clone(),
        flow_written: rasters.flow.clone(),
        node_written: rasters.node.clone(),
        level_written: rasters.level.clone(),
        breached: breached.clone(),
    };
    let spots: Vec<checks::Spot> = o
        .spots
        .iter()
        .map(|&[e, n]| {
            let (x, y) = (
                ((e - s.nw_m[0]) / s.spacing_m).floor(),
                ((s.nw_m[1] - n) / s.spacing_m).floor(),
            );
            let inside = x >= 0.0 && y >= 0.0 && x < s.world.w as f64 && y < s.world.h as f64;
            checks::Spot {
                en: [e, n],
                cell: inside.then_some([x as u64, y as u64]),
            }
        })
        .collect();
    let tiles = store::for_tiles(&s.tiles(), s.threads, "checks", |i, core| {
        checks::check_tile(&s, &layers, core, &lakes, &json_levels, &exits[i], &spots)
    })?;
    let of = |kind: &'static str| list.iter().filter(move |e| e["kind"] == kind);
    let sum = |kind: &'static str, field: &str| {
        // From +0.0: an empty float sum is −0.0.
        of(kind)
            .filter_map(|e| e[field].as_f64())
            .fold(0.0, |a, v| a + v)
    };
    let (n_lakes, n_rivers) = (of("lake").count(), of("river").count());
    let mut figures = Map::new();
    figures.insert("stages".into(), stages.clone());
    figures.insert("lakes".into(), json!(n_lakes));
    figures.insert("lake_km2".into(), json!(sum("lake", "area_km2")));
    figures.insert("ponds".into(), json!(ponds));
    figures.insert("rivers".into(), json!(n_rivers));
    figures.insert("river_km".into(), json!(sum("river", "length_km")));
    figures.insert("sea_km2".into(), json!(sum("sea", "area_km2")));
    let extra = checks::CheckExtra {
        // The model refuses flow cycles (an error ends `finish`), so none reach the checks.
        cycles: 0,
        bodies: n_lakes + n_rivers,
        lake_short,
        lakes,
        cell_km2: s.cell_km2(),
        spots,
    };
    let (report, passed) = checks::report(tiles, extra, figures);

    let mut stage = crate::world::hydrology::grid::Stage::new(&o.out)?;
    stage.copy("elevation.f32", &o.ground)?;
    stage.copy("water-surface.f32", &work.path("water-surface.f32"))?;
    stage.copy("water-body.u32", &work.path("water-body.u32"))?;
    stage.copy(
        "discharge.f32",
        &work.path(&format!("{m3_tag}-discharge.f32")),
    )?;
    stage.copy("flow.u8", &work.path(&format!("{m3_tag}-flow.u8")))?;
    stage.json("bodies.json", &bodies)?;
    stage.json("report.json", &report)?;
    let mut manifest: Map<String, Value> =
        serde_json::from_slice(&read_small(&o.grid.join("grid.json"))?)?;
    manifest.insert("water_model".into(), json!("hydrology v3"));
    manifest.insert(
        "discharge".into(),
        json!({
            "file": "discharge.f32",
            "unit": "km² at the mean land precipitation (drainage area weighted by rain / mean)",
            "mean_land_mm_h": s.rain.mean_land_mm_h,
        }),
    );
    manifest.insert(
        "flow".into(),
        json!({
            "file": "flow.u8",
            "bits": "low 4: D8 receiver (0 none; 1 N, 2 S, 3 W, 4 E, 5 NW, 6 NE, 7 SW, 8 SE); \
                     16 river thalweg; 32 dry basin; 64 endorheic",
        }),
    );
    manifest.insert(
        "provenance".into(),
        json!({
            "tool": "topography world water", "version": env!("CARGO_PKG_VERSION"),
            "stamp": stamp, "stages": stages["tags"], "final_model": m3_tag,
            "outputs": stage.digests.clone(),
        }),
    );
    stage.json("grid.json", &Value::Object(manifest))?;
    let out = stage.publish()?;
    drop_other_finals(&work, &m3_tag)?;
    let summary = format!(
        "{}\nwater {}: {n_lakes} lakes, {n_rivers} rivers{}",
        out.display(),
        if passed { "passed" } else { "FAILED" },
        if passed {
            String::new()
        } else {
            format!("\nfailed checks: {}", report["failures"])
        },
    );
    Ok(Outcome {
        out,
        passed,
        summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_resumes_from_its_work_directory() {
        let dir = tempfile::tempdir().expect("temp");
        let grid = testing::grid_dir(dir.path(), geom::World { w: 64, h: 64 });
        let o = Options {
            grid: grid.clone(),
            work: dir.path().join("work"),
            tile: 16,
            threads: 2,
            water: Some(grid.join("water.toml")),
            river_km2: Some(1e-5),
        };
        let first = run(&o).expect("run");
        let carved = fs::read(dir.path().join("work/carved.f32")).unwrap();
        let stages = fs::read(dir.path().join("work/stages.json")).unwrap();
        let again = run(&o).expect("resume");
        assert_eq!(first, again);
        assert_eq!(
            fs::read(dir.path().join("work/carved.f32")).unwrap(),
            carved
        );
        assert_eq!(
            fs::read(dir.path().join("work/stages.json")).unwrap(),
            stages
        );
        let moisture = fs::read(grid.join("moisture.f32")).unwrap();
        let mut other = moisture.clone();
        other[..4].copy_from_slice(&0.5f32.to_le_bytes());
        fs::write(grid.join("moisture.f32"), other).unwrap();
        assert!(run(&o).is_err(), "other moisture needs a fresh work dir");
        fs::write(grid.join("moisture.f32"), moisture).unwrap();
        assert_eq!(run(&o).expect("the original moisture resumes"), first);
        assert!(
            run(&Options { tile: 32, ..o }).is_err(),
            "another tile size needs a fresh work dir"
        );
    }

    /// A 128² window of the real surface (tests/fixtures/water/node-order) whose tiles of 64
    /// list their water nodes out of key order; the work store refused those tables until
    /// each tile's table was sorted by key (b101ca1).
    #[test]
    fn tiles_whose_nodes_come_out_of_key_order_run() {
        let dir = tempfile::tempdir().expect("temp");
        for tile in [64, 128] {
            let o = Options {
                grid: "../../tests/fixtures/water/node-order".into(),
                work: dir.path().join(format!("work-{tile}")),
                tile,
                threads: 2,
                water: None,
                river_km2: None,
            };
            run(&o).unwrap_or_else(|e| panic!("tile {tile}: {e}"));
        }
    }

    /// Writes tests/fixtures/water/grid for scripts/check.sh: cargo test … write_check_fixture -- --ignored
    #[test]
    #[ignore]
    fn write_check_fixture() {
        let dir = std::path::Path::new("../../tests/fixtures/water");
        let _ = std::fs::remove_dir_all(dir.join("grid"));
        testing::grid_dir(dir, geom::World { w: 64, h: 64 });
    }

    /// m3 is keyed on the grounds' contents: a rewritten carved ground with the same bytes
    /// (a channels rerun that changes surfaces only) keeps its tag; other bytes do not.
    #[test]
    fn the_final_tag_follows_the_grounds_contents() {
        let dir = tempfile::tempdir().expect("temp");
        let (g, b, g2) = (
            dir.path().join("carved.f32"),
            dir.path().join("breached.f32"),
            dir.path().join("copy.f32"),
        );
        fs::write(&g, [1u8; 64]).unwrap();
        fs::write(&b, [2u8; 64]).unwrap();
        let tag = final_tag("m1-x", &g, &b).unwrap();
        fs::write(&g2, [1u8; 64]).unwrap();
        assert_eq!(final_tag("m1-x", &g2, &b).unwrap(), tag);
        assert_ne!(final_tag("m1-y", &g, &b).unwrap(), tag);
        fs::write(&g2, [3u8; 64]).unwrap();
        assert_ne!(final_tag("m1-x", &g2, &b).unwrap(), tag);
        assert_ne!(final_tag("m1-x", &g, &g).unwrap(), tag);
    }

    /// Each stage's tag changes with every input it reads (directly or through the stages
    /// before it) and with nothing else. Before 7d4079f settle's tag missed the channels.
    #[test]
    fn each_stage_tag_follows_what_the_stage_reads() {
        let p = params(&table::Table::parse(table::DEFAULT).unwrap(), 1.0, None);
        let pools = BTreeMap::from([(7u64, 9u64)]);
        let tags = |p: &Params, contract, m3s, pools: &BTreeMap<u64, u64>, g: &str, b: &str| {
            Tags::all(p, contract, m3s, pools, g, b)
        };
        let base = tags(&p, true, 1.0, &pools, "g", "b");
        let changed = |t: Tags| {
            let names = ["m1", "b", "m2", "l", "c", "m3", "s"];
            let (a, z) = (base.list(), t.list());
            (0..7)
                .filter(|&i| a[i] != z[i])
                .map(|i| names[i])
                .collect::<Vec<_>>()
        };
        let all = vec!["m1", "b", "m2", "l", "c", "m3", "s"];
        // m3 reads the breached ground by content, so a new breach reaches it that way.
        let after_b = vec!["b", "m2", "l", "c", "s"];
        let with = |f: fn(&mut Params)| {
            let mut q = p;
            f(&mut q);
            q
        };
        let other = BTreeMap::from([(7u64, 10u64)]);
        assert_eq!(
            changed(tags(
                &with(|q| q.river_km2 = 0.1),
                true,
                1.0,
                &pools,
                "g",
                "b"
            )),
            all
        );
        assert_eq!(changed(tags(&p, false, 1.0, &pools, "g", "b")), all);
        for f in [
            (|q: &mut Params| q.breach_cap_m = 3.0) as fn(&mut Params),
            |q| q.breach_radius = 9,
            |q| q.channels = false,
        ] {
            assert_eq!(
                changed(tags(&with(f), true, 1.0, &pools, "g", "b")),
                after_b
            );
        }
        assert_eq!(changed(tags(&p, true, 2.0, &pools, "g", "b")), after_b);
        assert_eq!(
            changed(tags(&p, true, 1.0, &other, "g", "b")),
            ["l", "c", "s"]
        );
        assert_eq!(changed(tags(&p, true, 1.0, &pools, "G", "b")), ["m3", "s"]);
        assert_eq!(changed(tags(&p, true, 1.0, &pools, "g", "B")), ["m3", "s"]);
    }

    /// `finish` refuses a stages.json older than the stages; rerunning the channels clears
    /// earlier settles of their surfaces, and settle never runs on surfaces another settle
    /// rewrote.
    #[test]
    fn reruns_never_mix_stages() {
        let dir = tempfile::tempdir().expect("temp");
        let grid = testing::grid_dir(dir.path(), geom::World { w: 64, h: 64 });
        let o = Options {
            grid: grid.clone(),
            work: dir.path().join("work"),
            tile: 16,
            threads: 2,
            water: Some(grid.join("water.toml")),
            river_km2: Some(1e-5),
        };
        let first = run(&o).expect("run");
        let w = |f: &str| o.work.join(f);
        let names = [
            "stages.json",
            "carved.f32",
            "water-surface.f32",
            "water-body.u32",
        ];
        let outputs = || names.map(|f| fs::read(w(f)).unwrap());
        let before = outputs();
        let stages: Value = serde_json::from_slice(&before[0]).unwrap();
        let tag = |i: usize| stages["tags"][i].as_str().unwrap().to_string();
        let (c_tag, s_tag) = (tag(4), tag(6));
        let finish_to = |out: &str| {
            finish(&FinishOptions {
                grid: grid.clone(),
                work: o.work.clone(),
                ground: w("carved.f32"),
                out: dir.path().join(out),
                tile: 16,
                threads: 2,
                water: Some(grid.join("water.toml")),
                river_km2: Some(1e-5),
                spots: Vec::new(),
            })
        };
        let mut stale = stages.clone();
        stale["tags"][4] = json!("c-000000000000");
        fs::write(w("stages.json"), serde_json::to_vec(&stale).unwrap()).unwrap();
        assert!(finish_to("out1").is_err(), "a stale stages.json");
        fs::write(w("stages.json"), &before[0]).unwrap();
        assert!(finish_to("out2").expect("finish").passed);

        // A settle under another tag (as after a settle code change) is refused...
        fs::rename(w(&s_tag), w("s-000000000000")).unwrap();
        assert!(run(&o).is_err(), "settled surfaces are not settled again");
        // ...until the channels rerun, which clears it.
        fs::remove_dir_all(w(&c_tag)).unwrap();
        assert_eq!(run(&o).expect("rerun from the channels"), first);
        assert!(!w("s-000000000000").exists());
        assert!(outputs() == before, "the rerun writes the same bytes");
    }

    #[test]
    fn params_follow_the_spacing() {
        let t = table::Table::parse(table::DEFAULT).expect("table");
        let p = |s, r| params(&t, s, r);
        assert_eq!(
            p(32.0, None),
            Params {
                river_km2: 10.0,
                breach_cap_m: 20.0,
                breach_radius: BREACH_RADIUS,
                channels: false
            }
        );
        assert_eq!(
            p(4.0, None),
            Params {
                river_km2: 0.5,
                breach_cap_m: 5.0,
                breach_radius: BREACH_RADIUS,
                channels: false
            }
        );
        assert_eq!(
            p(1.0, None),
            Params {
                river_km2: 0.05,
                breach_cap_m: 2.0,
                breach_radius: BREACH_RADIUS,
                channels: true
            }
        );
        assert_eq!(p(1.0, Some(0.2)).river_km2, 0.2);
        assert_eq!(
            p(4.0, None).change_max_m(),
            5.5,
            "each level's own breach cap + 0.5 m"
        );
    }

    /// A `size`² grid directory of the raw surface at (east, north) with the layout's classes
    /// and the world's moisture (mean rain from the chain's 4 m grid), run through every stage
    /// into scratch/hydrology-v3/probes/t12-<name>-{grid,work}; writes t12-<name>-carved.png
    /// (hillshade of the carved ground: lakes and sea blue, rivers cyan) and prints stages.json
    /// with R9 and no-rise figures.
    fn probe_stages(name: &str, east: u64, north: u64, size: usize) {
        const L4V2: &str = "../../data/worlds/fujix-55-l4v2";
        const CHAIN: &str = "../../data/worlds/fujix-55-chain/hydrology/4m/grid.json";
        let Some((win, z)) = probe::window(east, north, size) else {
            return;
        };
        let classes = probe::classes(win).expect("classes");
        let read = |p: &str| -> Value {
            serde_json::from_slice(&fs::read(p).unwrap_or_else(|e| panic!("{p}: {e}"))).unwrap()
        };
        let moisture = read(&format!("{L4V2}/manifest.json"))["moisture"].clone();
        let mean = read(CHAIN)["discharge"]["mean_land_mm_h"].clone();
        assert!(
            moisture.is_object() && mean.is_f64(),
            "real moisture and mean rain"
        );
        let out = Path::new(probe::OUT);
        let grid = out.join(format!("t12-{name}-grid"));
        let work = out.join(format!("t12-{name}-work"));
        let _ = fs::remove_dir_all(&work);
        fs::create_dir_all(&grid).unwrap();
        let bytes: Vec<u8> = z.iter().flat_map(|v| v.to_le_bytes()).collect();
        fs::write(grid.join("elevation.f32"), bytes).unwrap();
        fs::write(grid.join("classes.u8"), &classes).unwrap();
        fs::copy(format!("{L4V2}/moisture.f32"), grid.join("moisture.f32")).unwrap();
        let manifest = json!({
            "size": [size, size], "spacing_m": 1.0,
            "origin": [win.x0 as f64, (probe::WORLD.h - win.y0) as f64],
            "classes_cell_m": 1.0, "moisture": moisture, "discharge": {"mean_land_mm_h": mean},
            "probe": format!("E{east} N{north}, {size}² of {}", probe::RAW),
        });
        fs::write(
            grid.join("grid.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let o = Options {
            grid,
            work: work.clone(),
            tile: 512,
            threads: 4,
            water: None,
            river_km2: None,
        };
        let started = std::time::Instant::now();
        let line = run(&o).expect("run");
        eprintln!(
            "{line} ({:.0} s{})",
            started.elapsed().as_secs_f64(),
            peak()
        );
        let stages = fs::read_to_string(work.join("stages.json")).unwrap();
        eprintln!("stages.json:\n{stages}");

        let world = geom::World {
            w: size as u64,
            h: size as u64,
        };
        let layer = |f: &str| layers::Layer::<f32>::open(world, &work.join(f)).unwrap();
        let carved = layer("carved.f32").to_vec().unwrap();
        let surface = layer("water-surface.f32").to_vec().unwrap();
        let body = layers::Layer::<u32>::open(world, &work.join("water-body.u32"))
            .unwrap()
            .to_vec()
            .unwrap();
        let flow = layers::Layer::<u8>::open(world, &work.join("m2-flow.u8"))
            .unwrap()
            .to_vec()
            .unwrap();
        let bodies = read(work.join("bodies.json").to_str().unwrap());
        let river_ids: std::collections::HashSet<u64> = bodies["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|b| b["kind"] == "river")
            .map(|b| b["id"].as_u64().unwrap())
            .collect();
        let below_raw = (0..z.len()).map(|k| z[k] - carved[k]).fold(0f32, f32::max);
        let raised = (0..z.len()).filter(|&k| carved[k] > z[k]).count();
        let mut rises = (0usize, 0f32);
        for (k, &f) in flow.iter().enumerate() {
            if f & 15 == 0 || !surface[k].is_finite() {
                continue;
            }
            let (x, y) = (k as u64 % world.w, k as u64 / world.w);
            let (nx, ny) = world.neighbour(x, y, (f & 15) as usize - 1).unwrap();
            let j = world.index(nx, ny) as usize;
            if surface[j].is_finite() && surface[j] > surface[k] + 1e-4 {
                rises = (rises.0 + 1, rises.1.max(surface[j] - surface[k]));
            }
        }
        // Ponds a river crosses (R7/R26 ruling): pond cells are node cells without a lake id
        // in the labelled body raster; a dry one under such a pond's water is a defect.
        let node = layers::Layer::<u32>::open(world, &work.join("m2-node.u32"))
            .unwrap()
            .to_vec()
            .unwrap();
        let labelled = layers::Layer::<u32>::open(world, &work.join("body.u32"))
            .unwrap()
            .to_vec()
            .unwrap();
        let lake_ids: std::collections::HashSet<u64> = bodies["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|b| b["kind"] == "lake")
            .map(|b| b["id"].as_u64().unwrap())
            .collect();
        let pond = |k: usize| node[k] >= 2 && !lake_ids.contains(&u64::from(labelled[k]));
        let crossed: std::collections::HashSet<u32> = (0..z.len())
            .filter(|&k| pond(k) && flow[k] & 16 != 0)
            .map(|k| node[k])
            .collect();
        let dry_in_crossed = (0..z.len())
            .filter(|&k| pond(k) && crossed.contains(&node[k]) && !surface[k].is_finite())
            .count();
        // Dry cells more than 1 m below a neighbouring water surface.
        let mut under = (0usize, 0f32);
        for k in 0..z.len() {
            if surface[k].is_finite() {
                continue;
            }
            let (x, y) = (k as u64 % world.w, k as u64 / world.w);
            let top = (0..8)
                .filter_map(|d| world.neighbour(x, y, d))
                .map(|(nx, ny)| surface[world.index(nx, ny) as usize])
                .filter(|s| s.is_finite())
                .fold(f32::NEG_INFINITY, f32::max);
            if top - carved[k] > 1.0 {
                under = (under.0 + 1, under.1.max(top - carved[k]));
            }
        }
        eprintln!(
            "{name}: river-crossed ponds {}, dry cells below their level {dry_in_crossed}; \
             dry cells > 1 m below a neighbouring water surface {} (deepest {:.2} m)",
            crossed.len(),
            under.0,
            under.1
        );
        eprintln!(
            "{name}: deepest below raw {below_raw:.3} m (cap {CHANNEL_CAP_M}), raised cells {raised}, \
             surface rises downstream {} (largest {:.3} m)",
            rises.0, rises.1
        );
        assert!(below_raw <= CHANNEL_CAP_M + 1e-4 && raised == 0, "R9");
        assert_eq!(
            dry_in_crossed, 0,
            "river-crossed ponds are wet (R7/R26 ruling)"
        );
        probe::shade_tinted(&format!("t12-{name}-carved"), &carved, size, |k| {
            if !surface[k].is_finite() {
                None
            } else if river_ids.contains(&u64::from(body[k])) {
                Some([1, 5, 6])
            } else {
                Some([2, 3, 6])
            }
        });
    }

    /// cargo test … world::water::tests::probe -- --ignored --nocapture (under MemoryMax=6G)
    #[test]
    #[ignore]
    fn probe_stages_in_the_canyon() {
        probe_stages("canyon", 27169, 44546, 2048);
    }

    #[test]
    #[ignore]
    fn probe_stages_at_e49249() {
        probe_stages("e49249", 49249, 16381, 2048);
    }

    #[test]
    #[ignore]
    fn probe_stages_at_the_dam_spot() {
        probe_stages("dam-spot", 25645, 24400, 2048);
    }

    /// The stages probe, then `finish` on its carved ground into
    /// scratch/hydrology-v3/probes/t13-<name>-out; prints the checks and the failures' cells.
    fn probe_finish(name: &str, east: u64, north: u64) {
        if probe::window(east, north, 16).is_none() {
            return;
        }
        probe_stages(name, east, north, 2048);
        let out = Path::new(probe::OUT);
        let work = out.join(format!("t12-{name}-work"));
        let dst = out.join(format!("t13-{name}-out"));
        let _ = fs::remove_dir_all(&dst);
        let f = FinishOptions {
            grid: out.join(format!("t12-{name}-grid")),
            work: work.clone(),
            ground: work.join("carved.f32"),
            out: dst.clone(),
            tile: 512,
            threads: 4,
            water: None,
            river_km2: None,
            spots: Vec::new(),
        };
        let started = std::time::Instant::now();
        let outcome = finish(&f).expect("finish");
        let report: Value =
            serde_json::from_slice(&fs::read(dst.join("report.json")).unwrap()).expect("report");
        eprintln!(
            "{name}: {} ({:.0} s{})\nchecks: {}\nexamples: {}",
            outcome.summary,
            started.elapsed().as_secs_f64(),
            peak(),
            report["checks"],
            report["examples"]
        );
    }

    #[test]
    #[ignore]
    fn probe_finish_in_the_canyon() {
        probe_finish("canyon", 27169, 44546);
    }

    #[test]
    #[ignore]
    fn probe_finish_at_e49249() {
        probe_finish("e49249", 49249, 16381);
    }

    #[test]
    #[ignore]
    fn probe_finish_at_the_dam_spot() {
        probe_finish("dam-spot", 25645, 24400);
    }

    /// The other dam spots (R31 baseline for Plan B).
    #[test]
    #[ignore]
    fn probe_finish_at_dam_e25307() {
        probe_finish("dam-e25307", 25307, 24585);
    }

    #[test]
    #[ignore]
    fn probe_finish_at_dam_e20071() {
        probe_finish("dam-e20071", 20071, 30501);
    }

    #[test]
    #[ignore]
    fn probe_finish_at_dam_e43966() {
        probe_finish("dam-e43966", 43966, 21633);
    }
}
