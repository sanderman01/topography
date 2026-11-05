//! Hydrology v3: water as one field (ground, water surface, discharge) from one model at every
//! level. Exact graph passes on tiles: the priority-flood spill graph (Barnes 2016), the
//! depression hierarchy and Fill-Spill-Merge (Barnes, Callaghan and Wickert 2020), fall-line
//! receivers, flat ponds (Barnes 2014) and accumulation (Barnes 2017); at 1 m also channels.
//! The GPU bank relaxation runs in Python between `run` and `finish`.
//! See docs/superpowers/specs/2026-11-05-hydrology-v3-design.md.
// Items are wired to the CLI in Task 14, which removes this allow.
#![allow(dead_code)]

mod balance;
mod bodies;
mod breach;
mod channel;
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

use crate::acquisition::{Result, digest, ensure, io_context, read_small};
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};

/// Judgment value (spec): standing water at least this deep is a lake; shallower is a pond.
pub(crate) const LAKE_DEPTH_M: f32 = 0.3;
/// Channel depth cap at 1 m; a 1 m breach plus its channel cuts at most this much too.
pub(crate) const CHANNEL_CAP_M: f32 = 2.0;
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

/// Runs model → breach → model → bodies → channels into `work` (resumable). Returns a
/// one-line summary; the figures go to `work/stages.json`.
pub fn run(o: &Options) -> Result<String> {
    let (s, raw, stamp) = setup(&o.grid, o.tile, o.threads, o.water.as_deref(), o.river_km2)?;
    let work = store::Work::open(&o.work, &stamp)?;
    let wk = Some(&work);
    let geo = hydraulic::Geometry::new(s.table.m3s_per_km2);
    let p = &s.params;
    let m1_tag = stage_tag(
        "m1",
        &json!({"river_km2": p.river_km2, "contract": s.contract}),
    );
    let b_tag = stage_tag(
        "b",
        &json!({"after": m1_tag, "cap": p.breach_cap_m, "radius": p.breach_radius,
                "channels": p.channels, "m3s_per_km2": geo.m3s_per_km2}),
    );
    let m1 = pipeline::model(&s, &raw, wk, &m1_tag, None)?;
    let breached = layers::Layer::open_or_create(s.world, &work.path("breached.f32"), 0.0)?;
    let breaches = breach::breach(&s, &m1, &raw, &breached, &geo, wk, &b_tag)?;
    drop(m1);
    let m2_tag = stage_tag("m2", &json!({"after": b_tag}));
    let rasters = pipeline::Rasters::files(s.world, &work.path(""), "m2")?;
    let m2 = pipeline::model(&s, &breached, wk, &m2_tag, Some(&rasters))?;
    let body = layers::Layer::open_or_create(s.world, &work.path("body.u32"), 0)?;
    let pools = bodies::pools(&s, &m2, &geo);
    let l_tag = stage_tag("l", &json!({"after": m2_tag, "pools": pools}));
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
    let c_tag = stage_tag(
        "c",
        &json!({"after": l_tag, "m3s_per_km2": geo.m3s_per_km2, "cap": CHANNEL_CAP_M,
                "halo": channel::HALO, "corridor": channel::CORRIDOR, "backwater": channel::BACKWATER}),
    );
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
    let stages = json!({
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
        "tags": [m1_tag, b_tag, m2_tag, l_tag, c_tag],
    });
    io_context(
        "write stages.json",
        fs::write(
            work.path("stages.json"),
            serde_json::to_vec_pretty(&stages)?,
        ),
    )?;
    eprintln!("water: done{}", peak());
    Ok(format!(
        "{}: {} lakes, {} rivers, {} breaches, {} river cells; next: relax, then water-finish",
        o.work.display(),
        n("lake"),
        n("river"),
        stages["breach"]["breaches"],
        stages["channels"]["river_cells"],
    ))
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
        fs::write(grid.join("moisture.f32"), 0.5f32.to_le_bytes()).unwrap();
        assert!(run(&o).is_err(), "other moisture needs a fresh work dir");
        fs::write(grid.join("moisture.f32"), 1.0f32.to_le_bytes()).unwrap();
        assert_eq!(run(&o).expect("the original moisture resumes"), first);
        assert!(
            run(&Options { tile: 32, ..o }).is_err(),
            "another tile size needs a fresh work dir"
        );
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
    /// (hillshade of the carved ground: lakes and sea blue, rivers cyan, the dry corridor
    /// yellow) and prints stages.json with R9 and no-rise figures.
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
        let corridor = layers::Layer::<u8>::open(world, &work.join("corridor.u8"))
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
        let count = |c: u8| corridor.iter().filter(|&&v| v == c).count();
        eprintln!(
            "{name}: deepest below raw {below_raw:.3} m (cap {CHANNEL_CAP_M}), raised cells {raised}, \
             surface rises downstream {} (largest {:.3} m), corridor: {} lowered, {} dry",
            rises.0,
            rises.1,
            count(2),
            count(1)
        );
        assert!(below_raw <= CHANNEL_CAP_M + 1e-4 && raised == 0, "R9");
        assert_eq!(dry_in_crossed, 0, "river-crossed ponds are wet (R7/R26 ruling)");
        probe::shade_tinted(&format!("t12-{name}-carved"), &carved, size, |k| {
            if !surface[k].is_finite() {
                (corridor[k] == 1).then_some([6, 6, 3])
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
}
