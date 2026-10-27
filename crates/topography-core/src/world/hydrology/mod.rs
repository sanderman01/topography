//! Hydrology v2: water as a surface height on a generated ground grid. Sea, lakes at their
//! spill level, breached depressions, flow accumulation and rivers, with a validation report.
//! See docs/workflows/terrain-cascade.md ("Hydrology v2").

mod erosion;
mod flood;
mod grid;
mod lakes;
mod moisture;
mod report;
mod rivers;
mod water;

use crate::acquisition::{Result, digest, ensure, io_context};
use flood::{Dims, RIVER, SEA, receiver};
use lakes::Kind;
use serde_json::{Value, json};
use std::{collections::HashMap, fs, path::PathBuf, time::Instant};

/// Rivers start at this drainage area by default (32 m trunk network).
pub const DEFAULT_RIVER_KM2: f64 = 10.0;
/// Judgment constant: the deepest cut breaching may make below a depression's spill. Deeper
/// depressions are kept (lakes at the spill lowered by this much, or dry sinks).
pub const BREACH_MAX_M: f32 = 30.0;

pub struct Options {
    pub grid: PathBuf,
    pub out: PathBuf,
    pub coarse: Option<PathBuf>,
    pub wind_deg: Option<f64>,
    /// Water table TOML; the built-in configs/hydrology/water-v1.toml when absent.
    pub water: Option<PathBuf>,
    pub river_km2: f64,
}

pub struct Outcome {
    pub out: PathBuf,
    pub passed: bool,
    pub summary: String,
}

/// Run hydrology on `--grid` into a new `--out` directory. Outputs are published even when a
/// validation check fails (`passed` false), so the report can be read. Timings go to stderr.
pub fn run(options: &Options) -> Result<Outcome> {
    ensure(
        options.coarse.is_none(),
        "--coarse is not implemented yet (hydrology v2, fine levels)",
    )?;
    ensure(
        options.wind_deg.is_none_or(f64::is_finite),
        "--wind-deg must be a number",
    )?;
    ensure(
        options.river_km2.is_finite() && options.river_km2 > 0.0,
        "--river-km2 must be positive",
    )?;
    ensure(
        !options.out.exists(),
        format!("{} already exists", options.out.display()),
    )?;
    let started = Instant::now();
    let lap = |name: &str, t: &mut Instant| {
        eprintln!("hydrology: {name} {:.1} s", t.elapsed().as_secs_f64());
        *t = Instant::now();
    };
    let mut t = Instant::now();

    let (water_text, water_source) = match &options.water {
        Some(path) => (
            io_context(format!("read {}", path.display()), fs::read_to_string(path))?,
            path.display().to_string(),
        ),
        None => (water::DEFAULT.to_string(), "built-in water-v1.toml".into()),
    };
    let table = water::Table::parse(&water_text)?;
    let grid::Input {
        grid,
        elevation: mut ground,
        classes,
        digests: inputs,
    } = grid::read(&options.grid)?;
    let dims = Dims {
        w: grid.w,
        h: grid.h,
    };
    let n = grid.cells();
    let cell_km2 = grid.cell_km2() as f32;
    let river_km2 = options.river_km2 as f32;
    eprintln!(
        "hydrology: {} x {} cells at {} m",
        grid.w, grid.h, grid.spacing_m
    );
    lap("read", &mut t);

    // Sea (from the input ground only) and the plain filled surface give the depressions.
    let mut flags = vec![0u8; n];
    let mut dir = vec![0u8; n];
    flood::open_sea(dims, &ground, |_| false, &mut flags);
    let filled = flood::fill(dims, &ground, &mut flags, &mut dir);
    lap("fill", &mut t);

    // Precipitation from the input ground; discharge weights each cell by its rain relative
    // to the land mean, so the river threshold holds at mean rain.
    let wind_from_deg = options
        .wind_deg
        .or_else(|| grid.manifest.get("wind_from_deg").and_then(Value::as_f64));
    let params = &moisture::PARAMS;
    let rain = moisture::precipitation(
        &ground,
        |i| flags[i] & SEA == 0,
        grid.w,
        grid.h,
        grid.spacing_m,
        wind_from_deg,
        params,
    );
    let rain_stats = moisture::Stats::new(&rain, grid.w, &flags, wind_from_deg.is_none());
    let rain_weight = |i: usize| cell_km2 * rain.at(grid.w, i) / rain_stats.mean_land_mm_h;
    lap("moisture", &mut t);
    let mut body = vec![0u32; n];
    let mut deps = lakes::find(dims, &ground, &filled, &dir, classes.as_deref(), &mut body);
    drop(filled);
    let mut acc = vec![0f32; n];
    flood::accumulate(dims, &dir, rain_weight, &mut acc);
    for i in 0..n {
        if body[i] != 0 {
            let d = &mut deps[body[i] as usize - 1];
            d.inflow_km2 = d.inflow_km2.max(acc[i]);
        }
    }
    drop(acc);
    let class_at = |i: usize| classes.as_ref().map_or(0, |c| c[i]);
    let mut land_by_class = [0u64; 256];
    for i in 0..n {
        if flags[i] & SEA == 0 {
            land_by_class[class_at(i) as usize] += 1;
        }
    }
    let (mut basins, map) = lakes::select(
        &deps,
        &land_by_class,
        &table,
        grid.spacing_m * grid.spacing_m,
        options.river_km2,
    );
    lakes::relabel(&mut body, &map);
    let kept_by_budget = basins.len();
    lap("depressions", &mut t);

    // Breach every other depression up to the cap, then route over the result.
    let carved = flood::breach(
        dims,
        &mut ground,
        &mut body,
        &mut basins,
        &mut flags,
        &mut dir,
        BREACH_MAX_M,
        |pit| {
            let class = class_at(pit);
            if class == lakes::GLACIER || table.get(class).lake_share <= lakes::ARID_LAKE_SHARE {
                Kind::Sink
            } else {
                Kind::Lake
            }
        },
    );
    for i in 0..n {
        if flags[i] & SEA != 0 {
            dir[i] = 0;
        }
    }
    eprintln!("hydrology: breach lowered {carved} cells");
    lap("breach", &mut t);

    // Trenches widen into small valleys; ice and the ground draining onto it stay as they are.
    let frozen = erosion::ice_catchment(dims, &dir, &body, &basins, |i| {
        class_at(i) == lakes::GLACIER
    });
    drop(classes);
    let mut breach_cut: HashMap<u32, f32> = HashMap::new();
    grid::for_each_input_row(&options.grid, grid.w, |y, row| {
        for (x, &z) in row.iter().enumerate() {
            let i = y * grid.w + x;
            if flags[i] & flood::BREACHED != 0 {
                breach_cut.insert(i as u32, z - ground[i]);
            }
        }
    })?;
    let trench_depth = breach_cut.values().copied().fold(0f32, f32::max);
    let mut trenches = erosion::widen_trenches(
        dims,
        &mut ground,
        &mut dir,
        &flags,
        &body,
        &basins,
        &frozen,
        |i| breach_cut.get(&(i as u32)).copied().unwrap_or(0.0),
        grid.spacing_m,
    );
    drop(breach_cut);
    if let Value::Object(m) = &mut trenches {
        m.insert("max_depth_m".into(), json!(trench_depth));
    }
    lap("trenches", &mut t);
    let mut acc = vec![0f32; n];
    let cycles = flood::accumulate(dims, &dir, rain_weight, &mut acc);
    lap("accumulate", &mut t);

    // Stream power at the trunk-network level only, before lakes take river inflow.
    let stream_power = if grid.spacing_m >= erosion::MIN_SPACING_M {
        let before = ground.clone();
        erosion::stream_power(
            dims,
            &mut ground,
            &dir,
            &acc,
            &flags,
            &body,
            &basins,
            &frozen,
            grid.spacing_m,
        );
        let mut breached = vec![false; n];
        grid::for_each_input_row(&options.grid, grid.w, |y, row| {
            for (x, &z) in row.iter().enumerate() {
                breached[y * grid.w + x] = before[y * grid.w + x] < z;
            }
        })?;
        let figures = erosion::figures(
            dims,
            &before,
            &ground,
            &breached,
            |i| flags[i] & SEA == 0 && body[i] == 0 && acc[i] >= river_km2,
            grid.spacing_m,
        );
        lap("stream power", &mut t);
        figures
    } else {
        json!({ "skipped": "finer than the trunk-network level" })
    };
    drop(frozen);

    // A dry pass-through basin that a river flows into fills as a lake (not glacier basins).
    let mut inflow = vec![0f32; basins.len() + 1];
    for i in 0..n {
        inflow[body[i] as usize] = inflow[body[i] as usize].max(acc[i]);
    }
    for (b, basin) in basins.iter_mut().enumerate() {
        if basin.kind == Kind::Dry && !basin.ice && inflow[b + 1] >= river_km2 {
            basin.kind = Kind::Lake;
            basin.filled_by_river = true;
        }
    }
    let extended = lakes::extend(dims, &ground, &dir, &flags, &mut body, &basins);
    // Where routed flow leaves each basin: the exit with the most flow.
    let mut exit_acc = vec![-1f32; basins.len() + 1];
    for i in 0..n {
        let b = body[i] as usize;
        if b == 0 {
            continue;
        }
        if let Some(r) = receiver(dims, &dir, i).filter(|&r| body[r] as usize != b) {
            if acc[r] > exit_acc[b] {
                exit_acc[b] = acc[r];
                basins[b - 1].exit = r as u32;
            }
        }
    }

    let mut surface = vec![f32::NAN; n];
    let (rivers, band, channels) = rivers::extract(
        dims,
        &mut ground,
        &dir,
        &acc,
        &mut flags,
        &mut body,
        &basins,
        river_km2,
        grid.spacing_m,
        &mut surface,
    );
    let lips = rivers::raise_lips(
        dims,
        &mut ground,
        &dir,
        &flags,
        &body,
        &basins,
        &band,
        grid.spacing_m,
    );
    for i in 0..n {
        if flags[i] & SEA != 0 {
            surface[i] = 0.0;
        } else if flags[i] & RIVER == 0 && body[i] != 0 {
            let basin = &basins[body[i] as usize - 1];
            if basin.kind == Kind::Lake {
                surface[i] = basin.level;
            }
        }
    }
    let holes = rivers::fill_holes(
        dims,
        &mut ground,
        &dir,
        &flags,
        &body,
        &basins,
        &band,
        &surface,
        grid.spacing_m,
    );
    lap("rivers", &mut t);

    // Body ids: sea 1, lakes in basin order, rivers, then dry basins (not in the raster).
    let mut basin_id = vec![0u32; basins.len() + 1];
    let mut next = 2;
    for (b, basin) in basins.iter().enumerate() {
        if basin.kind == Kind::Lake {
            basin_id[b + 1] = next;
            next += 1;
        }
    }
    let river_base = next - 1;
    next += rivers.len() as u32;
    for (b, basin) in basins.iter().enumerate() {
        if basin.kind != Kind::Lake {
            basin_id[b + 1] = next;
            next += 1;
        }
    }
    let body_id = |i: usize| -> u32 {
        if flags[i] & SEA != 0 {
            1
        } else if flags[i] & RIVER != 0 {
            river_base + body[i]
        } else if body[i] != 0 && basins[body[i] as usize - 1].kind == Kind::Lake {
            basin_id[body[i] as usize]
        } else {
            0
        }
    };

    let mut carve = report::Carve::default();
    grid::for_each_input_row(&options.grid, grid.w, |y, row| {
        carve.add(row, &ground[y * grid.w..(y + 1) * grid.w], grid.spacing_m);
    })?;
    let ctx = report::Context {
        dims,
        ground: &ground,
        dir: &dir,
        acc: &acc,
        flags: &flags,
        body: &body,
        surface: &surface,
        basins: &basins,
        rivers: &rivers,
        band: &band,
        cell_km2: cell_km2 as f64,
        spacing_m: grid.spacing_m,
    };
    let checks = report::checks(&ctx, cycles);
    let bodies = report::bodies(&ctx, &body_id, &basin_id, river_base);
    let mut figures = report::figures(&ctx, &carve, extended);
    figures.insert("depressions".into(), json!(deps.len()));
    figures.insert("kept_by_budget".into(), json!(kept_by_budget));
    figures.insert(
        "channels".into(),
        json!({
            "channel_cells_beside_centreline": channels.band_cells,
            "carved_cells": channels.cut_cells,
            "carved_cells_over_10m": channels.deep_cells,
            "max_cut_m": channels.max_cut_m,
            "max_cut_at_cell": [channels.max_cut_at % grid.w, channels.max_cut_at / grid.w],
            "carve_volume_m3": channels.volume_m3.round(),
            "max_bank_drop_m": channels.max_bank_drop_m,
            "seabed_cells_lowered_past_mouths": channels.sea_cells_lowered,
            "widest_m": channels.widest_m,
            "deepest_m": channels.deepest_m,
        }),
    );
    figures.insert("stream_power".into(), stream_power);
    figures.insert("trenches".into(), trenches);
    figures.insert("lip_cells_raised".into(), json!(lips.cells));
    figures.insert("lip_max_raise_m".into(), json!(lips.max_m));
    figures.insert("lip_raise_volume_m3".into(), json!(lips.volume_m3.round()));
    figures.insert(
        "holes_filled".into(),
        json!({
            "cells_raised": holes.cells,
            "max_raise_m": holes.max_m,
            "volume_m3": holes.volume_m3.round(),
        }),
    );
    lap("report", &mut t);

    let mut stage = grid::Stage::new(&options.out)?;
    stage.raster("elevation.f32", n, |i| ground[i].to_le_bytes())?;
    stage.raster("water-surface.f32", n, |i| surface[i].to_le_bytes())?;
    stage.raster("water-body.u32", n, |i| body_id(i).to_le_bytes())?;
    stage.raster("discharge.f32", n, |i| acc[i].to_le_bytes())?;
    let scale = rain_stats.p95_land_mm_h;
    stage.raster("moisture.f32", rain.mm_h.len(), |c| {
        rain_stats.moisture(rain.mm_h[c]).to_le_bytes()
    })?;
    stage.json("bodies.json", &bodies)?;
    let mut precipitation = rain_stats.json();
    if let Value::Object(m) = &mut precipitation {
        m.insert("wind_from_deg".into(), json!(wind_from_deg));
        m.insert("params".into(), params.json());
    }
    let failures: Vec<&str> = checks
        .iter()
        .filter(|(_, v)| v.as_u64() != Some(0))
        .map(|(k, _)| k.as_str())
        .collect();
    let passed = failures.is_empty();
    let report = json!({
        "schema_version": 1,
        "kind": "hydrology_report",
        "passed": passed,
        "failures": failures,
        "checks": checks,
        "figures": figures,
        "precipitation": precipitation,
        "constants": report::constants(options.river_km2, BREACH_MAX_M),
    });
    stage.json("report.json", &report)?;
    let mut manifest = grid.manifest.clone();
    // Cascade level names by spacing.
    let level = [(128.0, "l1"), (32.0, "l2"), (4.0, "l3"), (1.0, "l4")]
        .iter()
        .find(|(m, _)| *m == grid.spacing_m)
        .map_or(Value::Null, |(_, l)| json!(l));
    manifest.entry("level").or_insert(level);
    manifest.insert("wind_from_deg".into(), json!(wind_from_deg));
    manifest.insert(
        "discharge".into(),
        json!({
            "file": "discharge.f32",
            "unit": "km² at the mean land precipitation (drainage area weighted by rain / mean)",
            "mean_land_mm_h": rain_stats.mean_land_mm_h,
        }),
    );
    manifest.insert(
        "moisture".into(),
        json!({
            "file": "moisture.f32",
            "size": [rain.w, rain.h],
            "spacing_m": grid.spacing_m * rain.k as f64,
            "dtype": "float32",
            "value": if wind_from_deg.is_some() {
                json!("precipitation / scale_mm_h, clipped to 0..1")
            } else {
                json!(format!("{} everywhere (no wind)", moisture::NEUTRAL_MOISTURE))
            },
            "scale_mm_h": scale,
            "scale": "95th percentile of precipitation on land cells",
        }),
    );
    manifest.insert(
        "provenance".into(),
        json!({
            "tool": "topography world hydrology",
            "version": env!("CARGO_PKG_VERSION"),
            "inputs": inputs,
            "water_table": { "source": water_source, "sha256": digest(water_text.as_bytes()) },
            "river_km2": options.river_km2,
            "outputs": stage.digests.clone(),
        }),
    );
    stage.json("grid.json", &Value::Object(manifest))?;
    let out = stage.publish()?;
    lap("write", &mut t);
    // Peak resident memory where the OS reports it (Linux).
    let peak = fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .map(String::from)
        })
        .map_or(String::new(), |l| format!(", peak {}", l[6..].trim()));
    eprintln!(
        "hydrology: total {:.1} s{peak}",
        started.elapsed().as_secs_f64()
    );

    let f = &figures;
    let summary = format!(
        concat!(
            "{}\nhydrology {}: sea {:.1}%, {} lakes {:.1} km² ({} over budget), ",
            "{} dry basins ({} sinks), {} rivers {:.0} km{}"
        ),
        out.display(),
        if passed { "passed" } else { "FAILED" },
        f["sea_share"].as_f64().unwrap_or(0.0) * 100.0,
        f["lakes"],
        f["lake_km2"].as_f64().unwrap_or(0.0),
        f["over_budget_lakes"],
        f["dry_basins"],
        f["over_budget_sinks"],
        f["river_systems"],
        f["river_length_km"].as_f64().unwrap_or(0.0),
        if passed {
            String::new()
        } else {
            format!("\nfailed checks: {}", failures.join(", "))
        },
    );
    Ok(Outcome {
        out,
        passed,
        summary,
    })
}
