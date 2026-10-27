//! Hydrology v2: water as a surface height on a generated ground grid. Sea, lakes at their
//! spill level, breached depressions, flow accumulation and rivers, with a validation report.
//! See docs/workflows/terrain-cascade.md ("Hydrology v2").

mod flood;
mod grid;
mod lakes;
mod report;
mod rivers;
mod water;

use crate::acquisition::{Result, digest, ensure, io_context};
use flood::{Dims, RIVER, SEA, receiver};
use lakes::Kind;
use serde_json::{Value, json};
use std::{fs, path::PathBuf, time::Instant};

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
        options.wind_deg.is_none(),
        "--wind-deg is not implemented yet (hydrology v2, moisture)",
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
    let mut body = vec![0u32; n];
    let mut deps = lakes::find(dims, &ground, &filled, &dir, classes.as_deref(), &mut body);
    drop(filled);
    let mut acc = vec![0f32; n];
    flood::accumulate(dims, &dir, cell_km2, &mut acc);
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
    drop(classes);
    for i in 0..n {
        if flags[i] & SEA != 0 {
            dir[i] = 0;
        }
    }
    eprintln!("hydrology: breach lowered {carved} cells");
    lap("breach", &mut t);
    let mut acc = vec![0f32; n];
    let cycles = flood::accumulate(dims, &dir, cell_km2, &mut acc);
    lap("accumulate", &mut t);

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
    let (rivers, bed_cut) = rivers::extract(
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
        cell_km2: cell_km2 as f64,
        spacing_m: grid.spacing_m,
    };
    let checks = report::checks(&ctx, cycles);
    let bodies = report::bodies(&ctx, &body_id, &basin_id, river_base);
    let mut figures = report::figures(&ctx, &carve, extended);
    figures.insert("depressions".into(), json!(deps.len()));
    figures.insert("kept_by_budget".into(), json!(kept_by_budget));
    figures.insert("river_bed_cut_cells".into(), json!(bed_cut.cells));
    figures.insert("river_bed_max_cut_m".into(), json!(bed_cut.max_m));
    figures.insert("lip_cells_raised".into(), json!(lips.cells));
    figures.insert("lip_max_raise_m".into(), json!(lips.max_m));
    figures.insert("lip_raise_volume_m3".into(), json!(lips.volume_m3.round()));
    lap("report", &mut t);

    let mut stage = grid::Stage::new(&options.out)?;
    stage.raster("elevation.f32", n, |i| ground[i].to_le_bytes())?;
    stage.raster("water-surface.f32", n, |i| surface[i].to_le_bytes())?;
    stage.raster("water-body.u32", n, |i| body_id(i).to_le_bytes())?;
    stage.raster("discharge.f32", n, |i| acc[i].to_le_bytes())?;
    stage.json("bodies.json", &bodies)?;
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
    eprintln!("hydrology: total {:.1} s", started.elapsed().as_secs_f64());

    let f = &figures;
    let summary = format!(
        "{}\nhydrology {}: sea {:.1}%, {} lakes {:.1} km² ({} over budget), {} dry basins ({} sinks), {} rivers {:.0} km{}",
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
