//! Hydrology v2: water as a surface height on a generated ground grid. Sea, lakes at their
//! spill level, breached depressions, flow accumulation and rivers, with a validation report.
//! Finer levels hold a coarser level's network fixed (`--coarse`); bands carry the previous
//! band's rows (`--north`). See docs/workflows/terrain-cascade.md ("Hydrology v2").

mod band;
mod coarse;
mod erosion;
mod fine;
mod flood;
pub(crate) mod grid;
mod lakes;
mod moisture;
mod report;
mod rivers;
mod water;

pub use band::seam;

use crate::acquisition::{Result, digest, ensure, io_context};
use band::BodyKind;
use flood::{CENTRE, Dims, FIXED, FLOW_CENTRE, FLOW_DRY, FLOW_SINK, OUTLET, RIVER, SEA, receiver};
use lakes::Kind;
use serde_json::{Value, json};
use std::{collections::HashMap, fs, path::PathBuf, time::Instant};

/// Rivers start at this drainage area by default on the trunk level (32 m).
pub const DEFAULT_RIVER_KM2: f64 = 10.0;
/// Judgment constant: the deepest cut breaching may make below a depression's spill. Deeper
/// depressions are kept (lakes at the spill lowered by this much, or dry sinks).
pub const BREACH_MAX_M: f32 = 30.0;
/// Judgment constant: a sink a river runs into reaching this many rows after the seam does not
/// fill to its recorded spill, only to its own (`lakes::fill_to_spill`).
pub(super) const SEAM_LAKE_GAP_ROWS: usize = 3;
/// Judgment constant: the most a coarse lake's dry exit is notched down to the lake level.
const NOTCH_MAX_M: f32 = 0.5;
/// Judgment constant: exits within this share of a lake's main outflow carry it too.
const EXIT_SHARE: f32 = 0.01;

/// Default river start by grid spacing: 10 km² at 32 m and coarser, 0.5 km² at 4 m (from
/// 2 m), 0.05 km² at 1 m.
pub fn default_river_km2(spacing_m: f64) -> f64 {
    if spacing_m >= 16.0 {
        DEFAULT_RIVER_KM2
    } else if spacing_m >= 2.0 {
        0.5
    } else {
        0.05
    }
}

/// Basin `b` (index + 1) whose cells trench widening may lower: a dry or ice basin of a finer
/// level that is not a coarse lake and not on a coarse dry basin (`kept`).
fn open_basin(basins: &[lakes::Basin], kept: &[bool], fine_level: bool, b: u32) -> bool {
    b != 0
        && !kept.get(b as usize - 1).copied().unwrap_or(!fine_level)
        && basins[b as usize - 1].kind != Kind::Lake
}

pub struct Options {
    pub grid: PathBuf,
    pub out: PathBuf,
    pub coarse: Option<PathBuf>,
    /// The previous band's output, whose core holds this grid's top rows.
    pub north: Option<PathBuf>,
    pub wind_deg: Option<f64>,
    /// Water table TOML; the built-in configs/hydrology/water-v1.toml when absent.
    pub water: Option<PathBuf>,
    /// River start (km² at mean rain); by spacing when absent.
    pub river_km2: Option<f64>,
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
        options.wind_deg.is_none_or(f64::is_finite),
        "--wind-deg must be a number",
    )?;
    ensure(
        options.river_km2.is_none_or(|a| a.is_finite() && a > 0.0),
        "--river-km2 must be positive",
    )?;
    ensure(
        options.north.is_none() || options.coarse.is_some(),
        "--north needs --coarse",
    )?;
    ensure(
        options.coarse.is_none() || options.wind_deg.is_none(),
        "--wind-deg applies to the trunk level only; finer levels reuse the coarse moisture",
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
    let (w, h, n) = (grid.w, grid.h, grid.cells());
    let cell_km2 = grid.cell_km2() as f32;
    let river_km2_f64 = options
        .river_km2
        .unwrap_or_else(|| default_river_km2(grid.spacing_m));
    let river_km2 = river_km2_f64 as f32;
    let coarse = options
        .coarse
        .as_ref()
        .map(|d| coarse::read(d, &grid))
        .transpose()?;
    let north = options
        .north
        .as_ref()
        .map(|d| band::read_north(d, &grid))
        .transpose()?;
    eprintln!(
        "hydrology: {} x {} cells at {} m{}",
        grid.w,
        grid.h,
        grid.spacing_m,
        coarse
            .as_ref()
            .map_or(String::new(), |c| format!(", coarse {} m", c.spacing_m))
    );
    lap("read", &mut t);

    let mut flags = vec![0u8; n];
    let mut dir = vec![0u8; n];
    let mut body = vec![0u32; n];
    // Rows from the previous band, as they are.
    let fixed_rows = north.as_ref().map_or(0, |b| b.rows);
    let fixed_id = |i: usize| north.as_ref().map_or(0, |b| b.body[i]);
    let fixed_kind = |i: usize| {
        north
            .as_ref()
            .and_then(|b| b.kinds.get(&b.body[i]).copied())
    };
    let mut inflow = Vec::new();
    // Fixed cells whose flow in the previous band comes back into this run's cells.
    let reentry = north
        .as_ref()
        .map_or_else(Vec::new, |b| band::reentry(dims, &b.flow, fixed_rows));
    let reenters: Vec<bool> = reentry.iter().map(|&r| r != band::NO_ENTRY).collect();
    // The seam step rule's inputs: blends, carve floors, pockets breaching fills, lakes filled
    // and rivers raised near the seam.
    let mut seam_step = lakes::SeamStep::none();
    if let Some(b) = &north {
        for i in 0..fixed_rows * w {
            ground[i] = b.ground[i];
            flags[i] |= FIXED;
            match fixed_kind(i) {
                Some(BodyKind::Sea) => flags[i] |= SEA,
                Some(BodyKind::River) => {
                    flags[i] |= RIVER;
                    if b.flow[i] & FLOW_CENTRE != 0 {
                        flags[i] |= CENTRE;
                    }
                }
                _ => {}
            }
        }
        if coarse.is_some() && fixed_rows > 0 {
            let last = (fixed_rows - 1) * w;
            let mut raw_last = Vec::new();
            grid::for_each_input_row(&options.grid, grid.w, |y, row| {
                if y + 1 == fixed_rows {
                    raw_last.extend_from_slice(row);
                } else if y >= fixed_rows && y < fixed_rows + report::SEAM_CHECK_ROWS {
                    seam_step.raw.extend_from_slice(row);
                }
            })?;
            // As the report's step rule: a lake's or the sea's depth is no change of the land.
            seam_step.change = (0..w)
                .map(|x| {
                    let (i, s) = (last + x, b.surface[last + x]);
                    let still = flags[i] & SEA != 0 || fixed_kind(i) == Some(BodyKind::Lake);
                    (if s.is_nan() || still { ground[i] } else { s }) - raw_last[x]
                })
                .collect();
            seam_step.fixed_rows = fixed_rows;
            seam_step.surface = b.surface[last..last + w].to_vec();
            seam_step.rise = erosion::SIDE_SLOPE_DEG.to_radians().tan() * grid.spacing_m as f32;
        }
        let still = |i: usize| match fixed_kind(i) {
            Some(BodyKind::Sea) => Some(0.0),
            Some(BodyKind::Lake) => Some(b.surface[i]),
            _ => None,
        };
        let limit = band::seam_limit(
            dims,
            fixed_rows,
            &ground,
            |i| flags[i] & (RIVER | SEA) != 0 || fixed_kind(i) == Some(BodyKind::Lake),
            still,
            BREACH_MAX_M,
            erosion::SIDE_SLOPE_DEG.to_radians().tan(),
            grid.spacing_m,
        );
        // Nor below what the seam step rule allows in the rows it checks.
        let mut limit = limit;
        for (k, l) in limit
            .iter_mut()
            .enumerate()
            .take(report::SEAM_CHECK_ROWS * w)
        {
            *l = l.max(seam_step.bottom(dims, fixed_rows * w + k));
        }
        band::blend_seam(dims, fixed_rows, &mut ground, &b.halo_ground, &limit);
        // Fixed cells draining into this run's cells feed it; the others beside them are
        // outlets, unless their flow in the previous band comes back here (a loop).
        for (i, f) in flags.iter_mut().enumerate().take(fixed_rows * w) {
            let d = b.flow[i] & 15;
            let to = (d != 0)
                .then(|| dims.neighbour(i, d as usize - 1))
                .flatten();
            if let Some(r) = to.filter(|&r| r >= fixed_rows * w) {
                inflow.push((i, r));
            } else if !reenters[i] && dims.neighbours(i).any(|(_, j)| j >= fixed_rows * w) {
                *f |= OUTLET;
            }
        }
        // A river entering here must not turn straight back onto the previous band's ground.
        for &(i, r) in &inflow {
            if b.flow[i] & FLOW_CENTRE != 0 {
                for (_, j) in dims.neighbours(r) {
                    if j < fixed_rows * w && fixed_kind(j) != Some(BodyKind::River) {
                        flags[j] &= !OUTLET;
                    }
                }
            }
        }
    }
    match &coarse {
        Some(c) => {
            for (i, f) in flags.iter_mut().enumerate() {
                if dims.is_edge(i) && *f & FIXED == 0 && c.edge_outlet(w, h, i) {
                    *f |= OUTLET;
                }
            }
        }
        None => flood::edge_outlets(dims, &mut flags),
    }

    // Sea (from the input ground only): below 0 m connected to the world edge, coarse sea or
    // the previous band's sea.
    match &coarse {
        Some(c) => flood::open_sea(
            dims,
            &ground,
            |i| c.world_edge(w, h, i) || c.body[c.cell(w, i)] == 1,
            |i| c.lake(c.cell(w, i)).is_some(),
            &mut flags,
        ),
        None => flood::open_sea(dims, &ground, |i| dims.is_edge(i), |_| false, &mut flags),
    }

    // On finer levels no cut goes deeper than BREACH_MAX_M below the ground here, nor leaves a
    // step at the seam (`band::ground_floor`); the trunk level carves as it did.
    let (mut carve_floor, mut seam_floor): (Vec<f32>, Vec<f32>) = if coarse.is_some() {
        // Beside the previous band's lake or sea in its last row: down to that level.
        let still = |i: usize| match fixed_kind(i) {
            Some(BodyKind::Sea) => Some(0.0),
            Some(BodyKind::Lake) => north.as_ref().map(|b| b.surface[i]),
            _ => None,
        };
        band::ground_floor(
            dims,
            fixed_rows,
            &ground,
            |i| flags[i] & (RIVER | SEA) != 0 || fixed_kind(i) == Some(BodyKind::Lake),
            still,
            BREACH_MAX_M,
            erosion::SIDE_SLOPE_DEG.to_radians().tan(),
            grid.spacing_m,
        )
    } else {
        (vec![f32::NEG_INFINITY; n], Vec::new())
    };
    // Nor, in the rows the seam step rule checks, below what that rule allows.
    for (k, f) in seam_floor.iter_mut().enumerate() {
        let i = fixed_rows * w + k;
        let bottom = seam_step.bottom(dims, i).min(ground[i]);
        *f = f.max(bottom);
        carve_floor[i] = carve_floor[i].max(bottom);
    }
    // Nor, in the core's last rows (which the next band takes), steeper than the side slope
    // below its first row's ground, as that band's own first rows: river beds and coarse paths
    // too, so rivers cross the seam no deeper (`held_floor`, those rows' floor).
    let held = if coarse.is_some() && grid.core_rows[1] < dims.h {
        [
            grid.core_rows[1].saturating_sub(report::SEAM_CHECK_ROWS),
            grid.core_rows[1],
        ]
    } else {
        [0, 0]
    };
    if held[1] > held[0] {
        let limit = band::held_limit(
            dims,
            held[1],
            held[1] - held[0],
            &ground,
            BREACH_MAX_M,
            erosion::SIDE_SLOPE_DEG.to_radians().tan(),
            grid.spacing_m,
        );
        for (k, &l) in limit.iter().enumerate() {
            let i = held[0] * w + k;
            carve_floor[i] = carve_floor[i].max(l.min(ground[i]));
        }
    }
    let held_floor = carve_floor[held[0] * w..held[1] * w].to_vec();
    // Coarse river paths keep only the seam limits (`band::ground_floor`, `held_floor`).
    let path_floor = |i: usize| {
        let seam = i
            .checked_sub(fixed_rows * w)
            .and_then(|k| seam_floor.get(k).copied())
            .unwrap_or(f32::NEG_INFINITY);
        let held = i
            .checked_sub(held[0] * w)
            .and_then(|k| held_floor.get(k).copied())
            .unwrap_or(f32::NEG_INFINITY);
        seam.max(held)
    };

    // The coarse network on the fine ground: lakes at their level, river paths burned in.
    let mut basins = Vec::new();
    let (lake_figures, imprint) = match &coarse {
        Some(c) => {
            let figures = fine::lakes(dims, &mut ground, &flags, &mut body, &mut basins, c, |i| {
                (fixed_kind(i) == Some(BodyKind::Lake)).then(|| {
                    let level = north.as_ref().map_or(f32::NAN, |b| b.surface[i]);
                    (fixed_id(i), level)
                })
            });
            let inflows: Vec<fine::Inflow> = inflow
                .iter()
                .filter(|&&(i, _)| flags[i] & CENTRE != 0 && c.rivers.contains_key(&fixed_id(i)))
                .map(|&(i, r)| fine::Inflow {
                    cell: r as u32,
                    id: fixed_id(i),
                    surface: north.as_ref().map_or(f32::NAN, |b| b.surface[i]),
                    q: north.as_ref().map_or(0.0, |b| b.discharge[i]),
                })
                .collect();
            let imprint = fine::rivers(
                dims,
                &mut ground,
                &flags,
                &body,
                &basins,
                c,
                &inflows,
                fixed_id,
                |j| north.as_ref().map_or(f32::NAN, |b| b.surface[j]),
                path_floor,
                |j| {
                    reentry
                        .get(j)
                        .filter(|&&r| r != band::NO_ENTRY)
                        .map(|&r| r as usize)
                },
                grid.core_rows,
                grid.spacing_m,
            );
            (Some(figures), Some(imprint))
        }
        None => (None, None),
    };
    let coarse_lakes = basins.len() as u32;
    // Burned path slots widen into valleys as breach trenches do.
    if let Some(im) = &imprint {
        for &c in &im.burned {
            flags[c as usize] |= flood::BREACHED;
        }
        for &c in &im.outlets {
            flags[c as usize] |= OUTLET;
        }
    }
    let filled = flood::fill(dims, &ground, &mut flags, &mut dir);
    lap("fill", &mut t);

    // Precipitation: from the input ground on the trunk level, else the coarse moisture.
    let wind_from_deg = match &coarse {
        Some(c) => c.moisture.wind_from_deg,
        None => options
            .wind_deg
            .or_else(|| grid.manifest.get("wind_from_deg").and_then(Value::as_f64)),
    };
    let params = &moisture::PARAMS;
    let rain = coarse.is_none().then(|| {
        let rain = moisture::precipitation(
            &ground,
            |i| flags[i] & SEA == 0,
            grid.w,
            grid.h,
            grid.spacing_m,
            wind_from_deg,
            params,
        );
        let stats = moisture::Stats::new(&rain, grid.w, &flags, wind_from_deg.is_none());
        (rain, stats)
    });
    // Discharge entering across window edges and from the previous band.
    let mut entering: HashMap<usize, f32> = HashMap::new();
    if let Some(c) = &coarse {
        for i in (0..n).filter(|&i| dims.is_edge(i) && flags[i] & FIXED == 0) {
            let q = c.edge_inflow(w, h, i);
            if q > 0.0 {
                entering.insert(i, q);
            }
        }
    }
    let edge_inflow_cells = entering.len() as u64;
    if let Some(b) = &north {
        for &(i, _) in &inflow {
            entering.insert(i, b.discharge[i]);
        }
    }
    // Each cell's rain relative to the land mean, so the threshold holds at mean rain; fixed
    // cells add only what they pass into this grid.
    let weight = |i: usize| -> f32 {
        let fixed = i < fixed_rows * w;
        let add = if fixed || dims.is_edge(i) {
            entering.get(&i).copied().unwrap_or(0.0)
        } else {
            0.0
        };
        if fixed {
            return add;
        }
        let base = match (&rain, &coarse) {
            (Some((rain, stats)), _) => cell_km2 * rain.at(grid.w, i) / stats.mean_land_mm_h,
            (None, Some(c)) if !c.moisture.windless => {
                let x = grid.nw_m[0] + ((i % w) as f64 + 0.5) * grid.spacing_m;
                let y = grid.nw_m[1] - ((i / w) as f64 + 0.5) * grid.spacing_m;
                cell_km2 * c.moisture.at(x, y) * c.moisture.weight
            }
            _ => cell_km2,
        };
        base + add
    };
    lap("moisture", &mut t);

    let mut label = vec![0u32; n];
    let mut deps = lakes::find(dims, &ground, &filled, &dir, classes.as_deref(), &mut label);
    drop(filled);
    let mut on_path = vec![false; if coarse.is_some() { n } else { 0 }];
    for p in imprint.iter().flat_map(|im| &im.paths) {
        for &x in &p.cells {
            on_path[x as usize] = true;
        }
    }
    if let Some(c) = &coarse {
        // No basin in or beside a coarse lake, and no new lake beside a coarse river path
        // (the path is cut down through it later); coarse dry and ice basins stay.
        let mut near_path = vec![false; deps.len()];
        for i in 0..n {
            if label[i] == 0 {
                continue;
            }
            let k = label[i] as usize - 1;
            let d = &mut deps[k];
            d.excluded |= body[i] != 0 || dims.neighbours(i).any(|(_, j)| body[j] != 0);
            near_path[k] |= on_path[i] || dims.neighbours(i).any(|(_, j)| on_path[j]);
            match c.dry_basin(c.cell(w, i)) {
                Some(true) => d.forced = Some(Kind::Sink),
                Some(false) if d.forced.is_none() => d.forced = Some(Kind::Dry),
                _ => {}
            }
        }
        for (d, near) in deps.iter_mut().zip(near_path) {
            d.excluded |= near && d.forced.is_none() && d.class != lakes::GLACIER;
        }
    }
    let mut acc = vec![0f32; n];
    flood::accumulate(dims, &dir, weight, &mut acc);
    for i in 0..n {
        if label[i] != 0 {
            let d = &mut deps[label[i] as usize - 1];
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
    // Finer levels: no new lakes but tarns at 4 m, no new dry basins.
    let fine_table = coarse.as_ref().map(|_| table.fine(grid.spacing_m >= 2.0));
    let (selected, map) = lakes::select(
        &deps,
        &land_by_class,
        fine_table.as_ref().unwrap_or(&table),
        grid.spacing_m * grid.spacing_m,
        river_km2_f64,
    );
    basins.extend(selected);
    // Basins trench widening keeps: all on the trunk level; on finer levels coarse lakes, fine
    // lakes and basins on a coarse dry basin (such as an ice caldera).
    let mut kept_basin = vec![true; basins.len()];
    if coarse.is_some() {
        for (k, d) in deps.iter().enumerate() {
            if let Some(b) = map[k + 1].checked_sub(1) {
                let b = b as usize + coarse_lakes as usize;
                kept_basin[b] = d.forced.is_some() || basins[b].kind == Kind::Lake;
            }
        }
    }
    // A sink kept for a coarse dry basin holds no path cell outside that basin's coarse cells:
    // the path ends at the first sink cell, and a sink reaching up its trench would erase it.
    let path_kept = |basins: &[lakes::Basin], i: usize, id: u32| {
        on_path.get(i).copied().unwrap_or(false)
            && basins[id as usize - 1].kind == Kind::Sink
            && coarse
                .as_ref()
                .is_some_and(|c| c.dry_basin(c.cell(w, i)).is_none())
    };
    for i in 0..n {
        let b = map[label[i] as usize];
        if b != 0 && body[i] == 0 && !path_kept(&basins, i, b + coarse_lakes) {
            body[i] = b + coarse_lakes;
        }
    }
    drop(label);
    let kept_by_budget = basins.len();
    lap("depressions", &mut t);

    // Breach every other depression up to the cap, then route over the result.
    let fine_level = coarse.is_some();
    let carved = flood::breach(
        dims,
        &mut ground,
        &mut body,
        &mut basins,
        &mut flags,
        &mut dir,
        BREACH_MAX_M,
        &carve_floor,
        coarse.is_some(),
        |i, level| seam_step.steps(dims, i, level),
        |pit, deep| {
            let class = class_at(pit);
            let arid =
                class == lakes::GLACIER || table.get(class).lake_share <= lakes::ARID_LAKE_SHARE;
            if fine_level || (deep && arid) {
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
        // Nor a sink left where breaching gave up (such as dry ground below sea level).
        if body[i] as usize > kept_by_budget && path_kept(&basins, i, body[i]) {
            body[i] = 0;
        }
    }
    drop(on_path);
    eprintln!("hydrology: breach lowered {carved} cells");
    lap("breach", &mut t);

    // Coarse river paths burned again after breaching (which may have lowered some of their
    // cells); these cuts widen as trenches do.
    let mut burn_again = (0f32, 0usize);
    let mut fill_again = fine::Fill::default();
    if let Some(im) = &imprint {
        let (cells, max, at, fill) = fine::burn(
            dims,
            &mut ground,
            &body,
            &basins,
            &im.paths,
            &im.cap,
            grid.spacing_m,
        );
        for c in cells {
            flags[c as usize] |= flood::BREACHED;
        }
        burn_again = (max, at);
        fill_again = fill;
    }
    // Trenches widen into small valleys, through glacier too (ice is a surface layer). Kept
    // basins (such as an ice caldera), the sea and cells from the previous band stay as they
    // are; other dry and ice basin cells of a finer level may be lowered and leave the basin.
    let open = |b: u32| open_basin(&basins, &kept_basin, fine_level, b);
    // Floor: a kept basin's level beside a cell, 0 m (no water surfaces yet).
    let kept_level =
        |i: usize| (body[i] != 0 && !open(body[i])).then(|| basins[body[i] as usize - 1].level);
    let floor = |i: usize| {
        dims.neighbours(i)
            .filter_map(|(_, j)| kept_level(j))
            .fold(0f32, f32::max)
            .max(carve_floor[i])
    };
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
    let (mut trenches, lowered) = erosion::widen_trenches(
        dims,
        &mut ground,
        &mut dir,
        |i| flags[i] & (flood::BREACHED | SEA | FIXED) == flood::BREACHED && body[i] == 0,
        |i| flags[i] & (SEA | FIXED) == 0 && (body[i] == 0 || open(body[i])),
        |_| f32::NAN,
        floor,
        |i| breach_cut.get(&(i as u32)).copied().unwrap_or(0.0),
        grid.spacing_m,
        true,
        |i| flags[i] & OUTLET != 0,
    );
    drop(breach_cut);
    let mut left_basins = 0u64;
    for &c in &lowered {
        if body[c as usize] != 0 {
            body[c as usize] = 0;
            left_basins += 1;
        }
    }
    drop(lowered);
    if let Value::Object(m) = &mut trenches {
        m.insert("max_depth_m".into(), json!(trench_depth));
        m.insert("basin_cells_lowered".into(), json!(left_basins));
    }
    // Water follows the steepest descent on the final ground (the flood's order only on flats).
    // World column and row of the first cell: overlapping bands route their rows alike.
    let origin = [
        (grid.nw_m[0] / grid.spacing_m).round() as i64,
        (-grid.nw_m[1] / grid.spacing_m).round() as i64,
    ];
    let (resteered, mut receivers_spilled) =
        flood::steepest(dims, origin, &ground, &flags, &body, &basins, &mut dir);
    // Stream power leaves ice, ice basins and the ground draining onto them as they are.
    let mut frozen = erosion::ice_catchment(dims, &dir, &body, &basins, |i| {
        class_at(i) == lakes::GLACIER
    });
    for f in frozen.iter_mut().take(fixed_rows * w) {
        *f = true;
    }
    // Coarse river paths keep their own course, descending (widening may have lowered cells).
    if let Some(im) = &imprint {
        receivers_spilled += fine::route(dims, &ground, &flags, &mut dir, &im.paths);
        let (_, max, at, fill) = fine::burn(
            dims,
            &mut ground,
            &body,
            &basins,
            &im.paths,
            &im.cap,
            grid.spacing_m,
        );
        fill_again.max_m = fill_again.max_m.max(fill.max_m);
        fill_again.cells += fill.cells;
        if max > burn_again.0 {
            burn_again = (max, at);
        }
    }
    // Fixed cells end the routing here, except where they drain into this run's cells.
    let mut check_dir = Vec::new();
    if let Some(b) = &north {
        dir[..fixed_rows * w].fill(0);
        for &(i, _) in &inflow {
            dir[i] = b.flow[i] & 15;
        }
        check_dir = dir.clone();
        for (d, f) in check_dir.iter_mut().zip(&b.flow) {
            *d = f & 15;
        }
    }
    lap("trenches", &mut t);
    let mut acc = vec![0f32; n];
    let mut cycles = flood::accumulate(dims, &dir, weight, &mut acc);
    // Streams turning back across the seam beside the previous band's river are led into it.
    let seam_joins = band::join_seam_rivers(
        dims,
        fixed_rows,
        &mut ground,
        &flags,
        &body,
        &reenters,
        &mut dir,
        &acc,
        river_km2,
        &carve_floor,
    );
    // Rivers entering here that would leave again onto the previous band's dry ground.
    let inflows_rerouted = band::reroute_inflows(
        dims,
        fixed_rows,
        &mut ground,
        &flags,
        &body,
        &reentry,
        &mut dir,
        &inflow,
        &carve_floor,
    );
    // A river with no way on that does not climb ends where it enters, in a sink.
    for &r in &inflows_rerouted.stuck {
        if body[r] == 0 {
            basins.push(lakes::Basin::kept(Kind::Sink, ground[r], lakes::NO_EXIT));
            kept_basin.push(true);
            body[r] = basins.len() as u32;
            dir[r] = 0;
        }
    }
    if seam_joins > 0 || inflows_rerouted.rivers > 0 || !inflows_rerouted.stuck.is_empty() {
        cycles = flood::accumulate(dims, &dir, weight, &mut acc);
    }
    // A river whose water would have to climb past what its carve floor lets carving lower
    // (near the seam, any rise of the previous band's ground beside it) takes its steepest
    // other way down, goes on where a channel can be cut to its water, or ends there in a
    // sink: no trench below the floor.
    let (mut climbs_turned, mut climbs_routed, mut climbs_ended) = (0u64, 0u64, 0u64);
    if coarse.is_some() {
        let mut path = vec![false; n];
        for p in imprint.iter().flat_map(|im| &im.paths) {
            for &x in &p.cells {
                path[x as usize] = true;
            }
        }
        for _ in 0..4 {
            let at = rivers::climbs(
                dims,
                &ground,
                &flags,
                &body,
                &basins,
                &dir,
                &acc,
                river_km2,
                &carve_floor,
                |i| path[i],
            );
            if at.is_empty() {
                break;
            }
            for (c, level) in at {
                let r = receiver(dims, &dir, c);
                let to = dims
                    .neighbours(c)
                    .filter(|&(_, j)| {
                        Some(j) != r
                            && ground[j] < ground[c]
                            && (flags[j] & FIXED == 0 || flags[j] & (OUTLET | RIVER | SEA) != 0)
                            && !flood::reaches(dims, &dir, j, c)
                    })
                    .max_by(|a, b| {
                        let slope = |&(k, j): &(usize, usize)| {
                            (ground[c] - ground[j]) / Dims::step(k as u8 + 1) as f32
                        };
                        slope(a).total_cmp(&slope(b))
                    });
                match to {
                    Some((_, j)) => {
                        dir[c] = dims.towards(c, j);
                        climbs_turned += 1;
                    }
                    None if body[c] == 0 => {
                        // Across a flat filled where the floor kept the breach from cutting:
                        // on through cells a channel can be cut in to the water level.
                        let route = rivers::route_on(
                            dims,
                            &ground,
                            &flags,
                            &body,
                            &dir,
                            &carve_floor,
                            |i| path[i],
                            c,
                            level,
                        );
                        if let Some(route) = route {
                            for p in route.windows(2) {
                                dir[p[0]] = dims.towards(p[0], p[1]);
                            }
                            climbs_routed += 1;
                            continue;
                        }
                        basins.push(lakes::Basin::kept(Kind::Sink, ground[c], lakes::NO_EXIT));
                        kept_basin.push(true);
                        body[c] = basins.len() as u32;
                        dir[c] = 0;
                        climbs_ended += 1;
                    }
                    None => {}
                }
            }
            cycles = flood::accumulate(dims, &dir, weight, &mut acc);
        }
    }
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
    // Coarse rivers carry the coarse discharge; fixed cells keep the previous band's.
    if let Some(im) = &imprint {
        for (&cell, &q) in &im.q {
            acc[cell as usize] = acc[cell as usize].max(q);
        }
    }
    if let Some(b) = &north {
        acc[..fixed_rows * w].copy_from_slice(&b.discharge);
    }

    // A dry pass-through basin that a river flows into fills as a lake (not glacier basins;
    // on finer levels coarse dry basins stay dry). On finer levels a sink breaching left (not
    // the coarse network's) that a river runs into fills to its spill (or its own lowest one)
    // and overflows too: a river does not end in a dry hollow, such as one below sea level
    // behind a lagoon's rim.
    // Coarse river paths keep their course: no lake grows over a burned path cell.
    let mut on_path = vec![false; n];
    if let Some(im) = &imprint {
        for p in &im.paths {
            for &c in &p.cells {
                on_path[c as usize] = true;
            }
        }
    }
    // How high a coarse path's water may stand: its cap and the previous band's dry ground
    // beside it (`rivers::extract`), there and upstream (water downstream stands no higher).
    let taken_bank = |i: usize| {
        dims.neighbours(i)
            .filter(|&(_, k)| {
                flags[k] & (FIXED | SEA | RIVER) == FIXED
                    && fixed_kind(k) != Some(BodyKind::Lake)
                    && !dims.ahead(dir[i], i, k)
            })
            .map(|(_, k)| ground[k])
            .fold(f32::INFINITY, f32::min)
    };
    let mut path_held: HashMap<u32, f32> = HashMap::new();
    if let Some(im) = &imprint {
        for p in &im.paths {
            let mut held = f32::INFINITY;
            for &c in &p.cells {
                let cap = im.cap.get(&c).copied().unwrap_or(f32::INFINITY);
                held = held.min(cap).min(taken_bank(c as usize));
                let e = path_held.entry(c).or_insert(f32::INFINITY);
                *e = e.min(held);
            }
        }
    }
    // Again while a lake filled this way sends a river on into a further sink.
    let mut river_sinks = lakes::RiverSinks::default();
    for _ in 0..4 {
        let pass = lakes::fill_river_sinks(
            dims,
            &ground,
            &flags,
            &mut body,
            &mut basins,
            &mut kept_basin,
            &mut dir,
            &acc,
            lakes::RiverSinkRule {
                fine: fine_level,
                first: kept_by_budget,
                fixed_rows,
                river_km2,
                keep: &|i| on_path[i],
                cap: &|i| path_held.get(&(i as u32)).copied().unwrap_or(f32::INFINITY),
                seam: &seam_step,
            },
        );
        let again = pass.filled > 0;
        river_sinks.add(pass);
        if !again {
            break;
        }
        cycles = cycles.max(flood::accumulate(dims, &dir, weight, &mut acc));
        if let Some(im) = &imprint {
            for (&cell, &q) in &im.q {
                acc[cell as usize] = acc[cell as usize].max(q);
            }
        }
        if let Some(b) = &north {
            acc[..fixed_rows * w].copy_from_slice(&b.discharge);
        }
    }
    let extended = lakes::extend(dims, &ground, &dir, &flags, &mut body, &basins, |i| {
        on_path[i]
    });
    drop(on_path);
    // Where routed flow leaves each basin: the exit with the most flow. For a lake, among exits
    // carrying that outflow (within `EXIT_SHARE` of it), one that takes its water at the level
    // (water, or ground not above it) first; a main outflow above the level stays the exit.
    let exit_of = |i: usize| receiver(dims, &dir, i).filter(|&r| body[r] != body[i]);
    let mut most = vec![-1f32; basins.len() + 1];
    for i in 0..n {
        let b = body[i] as usize;
        if b == 0 || flags[i] & FIXED != 0 {
            continue;
        }
        if let Some(r) = exit_of(i) {
            most[b] = most[b].max(acc[r]);
        }
    }
    let mut exit_acc = vec![(false, -1f32); basins.len() + 1];
    for i in 0..n {
        let b = body[i] as usize;
        if b == 0 || flags[i] & FIXED != 0 {
            continue;
        }
        if let Some(r) = exit_of(i) {
            let basin = &basins[b - 1];
            let takes = basin.kind == Kind::Lake
                && acc[r] >= most[b] * (1.0 - EXIT_SHARE)
                && (flags[r] & (SEA | RIVER) != 0
                    || (body[r] != 0 && basins[body[r] as usize - 1].kind == Kind::Lake)
                    || ground[r] <= basin.level + report::BANK_TOLERANCE_M);
            if (takes, acc[r]) > exit_acc[b] {
                exit_acc[b] = (takes, acc[r]);
                basins[b - 1].exit = r as u32;
            }
        }
    }
    // A coarse lake spills at its level: a dry exit a little above it (the fine rim beside a
    // lake that misses its coarse outflow) is notched down to it.
    for b in basins.iter().take(coarse_lakes as usize) {
        let e = b.exit as usize;
        let dry = b.exit != lakes::NO_EXIT
            && flags[e] & (SEA | FIXED) == 0
            && (body[e] == 0 || basins[body[e] as usize - 1].kind != Kind::Lake);
        if b.kind == Kind::Lake && dry && ground[e] > b.level && ground[e] <= b.level + NOTCH_MAX_M
        {
            ground[e] = b.level;
        }
    }

    // Rivers with given ids: coarse rivers along their paths, then the previous band's.
    let mut given = rivers::Given {
        held,
        ..Default::default()
    };
    // The core's last rows, which the next band takes: banks there hold even ahead along a
    // river's flow, so that band gets water and banks that agree.
    let mut path_figures = Value::Null;
    let mut unrouted: Vec<u32> = Vec::new();
    let mut path_cap: HashMap<u32, f32> = HashMap::new();
    if let Some(im) = imprint {
        for &id in &im.ids {
            given.rivers.push(rivers::River {
                id,
                coarse: true,
                mouth: 0,
                end: rivers::End::Edge,
                cells: 0,
                wet_cells: 0,
                length_m: 0.0,
                drainage_km2: 0.0,
            });
        }
        let mut cells = 0u64;
        for p in &im.paths {
            for &c in &p.cells {
                // A path stops at a lake, a sink or the previous band, not on a dry basin floor.
                let b = body[c as usize];
                if (b != 0 && basins[b as usize - 1].kind != Kind::Dry)
                    || flags[c as usize] & FIXED != 0
                {
                    break;
                }
                given.cells.push((c, p.river as u32 + 1));
                cells += 1;
            }
        }
        path_figures = json!({
            "coarse_rivers": im.ids.len(),
            "paths": im.paths.len(),
            "path_cells": cells,
            "burned_cells": im.burned.len(),
            "burn_max_m": im.burn_max_m,
            "burn_max_at_cell": im.burn_max_at,
            "burn_after_breach_max_m": burn_again.0,
            "burn_after_breach_max_at_cell": [burn_again.1 % w, burn_again.1 / w],
            // Paths ending in the previous band's water filled up to its bed.
            "path_fill_max_m": im.fill.max_m,
            "path_fill_cells": im.fill.cells,
            "path_fill_after_breach_max_m": fill_again.max_m,
            "path_fill_after_breach_cells": fill_again.cells,
            "receiver_loops": im.receiver_loops,
            "paths_relaid": im.relaid,
            "unrouted_rivers": im.unrouted,
        });
        path_cap = im.cap.clone();
        given.cap = im.cap;
        unrouted = im.unrouted;
    }
    let mut seam_inflow = 0u64;
    if north.is_some() {
        let mut index: HashMap<u32, u32> = given
            .rivers
            .iter()
            .enumerate()
            .map(|(k, r)| (r.id, k as u32 + 1))
            .collect();
        for i in 0..fixed_rows * w {
            if flags[i] & RIVER == 0 {
                continue;
            }
            let id = fixed_id(i);
            let k = *index.entry(id).or_insert_with(|| {
                given.rivers.push(rivers::River {
                    id,
                    coarse: false,
                    mouth: 0,
                    end: rivers::End::Edge,
                    cells: 0,
                    wet_cells: 0,
                    length_m: 0.0,
                    drainage_km2: 0.0,
                });
                given.rivers.len() as u32
            });
            body[i] = k;
        }
        for &(i, r) in &inflow {
            if flags[i] & CENTRE != 0 {
                seam_inflow += 1;
                if !given.rivers[body[i] as usize - 1].coarse {
                    given.follow.push((r as u32, body[i]));
                }
            }
        }
    }
    // Water surfaces: NaN where dry; the previous band's as they are.
    let mut surface = vec![f32::NAN; n];
    if let Some(b) = &north {
        surface[..fixed_rows * w].copy_from_slice(&b.surface);
    }
    let (rivers, band, mut channels) = rivers::extract(
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
        given,
        &carve_floor,
        &seam_step,
    );
    // Channel cuts deeper than one cell's rise widen as trenches do, from the water surface.
    let cuts = std::mem::take(&mut channels.cuts);
    let one_rise = erosion::SIDE_SLOPE_DEG.to_radians().tan() * grid.spacing_m as f32;
    let deep_cut = |i: usize| cuts.get(&(i as u32)).copied().filter(|&c| c > one_rise);
    let open = |b: u32| open_basin(&basins, &kept_basin, fine_level, b);
    // Floor: kept basin levels, the surface of each river a cell is a bank of (not ground
    // ahead along its flow, as the bank check) and no hole below every river beside it.
    let flow = dir.clone();
    let bank_of = |i: usize, j: usize| {
        rivers::owner(&flags, &band, j)
            .is_none_or(|o| !dims.ahead(flow[o], j, i) || (held[0]..held[1]).contains(&(i / w)))
    };
    let floor = |i: usize| {
        let (mut f, mut lowest) = (0f32, f32::INFINITY);
        for (_, j) in dims.neighbours(i) {
            if flags[j] & RIVER != 0 {
                lowest = lowest.min(surface[j]);
                if bank_of(i, j) {
                    f = f.max(surface[j]);
                }
            } else if flags[j] & SEA == 0 && body[j] != 0 && !open(body[j]) {
                f = f.max(basins[body[j] as usize - 1].level);
            }
        }
        if lowest.is_finite() {
            f = f.max(lowest - rivers::HOLE_M);
        }
        f.max(carve_floor[i])
    };
    let (mut channel_widening, lowered) = erosion::widen_trenches(
        dims,
        &mut ground,
        &mut dir,
        |i| flags[i] & (RIVER | FIXED) == RIVER && deep_cut(i).is_some(),
        |i| flags[i] & (SEA | RIVER | FIXED) == 0 && (body[i] == 0 || open(body[i])),
        |i| surface[i],
        floor,
        |i| deep_cut(i).unwrap_or(0.0),
        grid.spacing_m,
        false,
        |i| flags[i] & OUTLET != 0,
    );
    drop((cuts, flow));
    let mut left_basins = 0u64;
    for &c in &lowered {
        let c = c as usize;
        if !check_dir.is_empty() {
            check_dir[c] = dir[c];
        }
        if body[c] != 0 {
            body[c] = 0;
            left_basins += 1;
        }
    }
    drop(lowered);
    // Channel widening may leave a cycle its own repair could not break: spill it.
    let spilled = flood::spill_cycles(dims, &ground, &flags, &mut dir, |_| true);
    if !check_dir.is_empty() {
        check_dir[fixed_rows * w..].copy_from_slice(&dir[fixed_rows * w..]);
    }
    if let Value::Object(m) = &mut channel_widening {
        m.insert("basin_cells_lowered".into(), json!(left_basins));
        m.insert("cycle_cells_spilled".into(), json!(spilled));
    }
    // The previous band's ground over this band's first row (its halo).
    let halo_row: &[f32] = north
        .as_ref()
        .map_or(&[][..], |b| &b.halo_ground[..b.halo_ground.len().min(w)]);
    let lips = rivers::raise_lips(
        dims,
        &mut ground,
        &dir,
        &flags,
        &body,
        &basins,
        &band,
        &surface,
        &channels.raised,
        held,
        north.as_ref().map_or(&[][..], |b| &b.flow[..]),
        halo_row,
        &seam_step,
        grid.spacing_m,
    );
    for &(j, b) in &lips.joins {
        body[j as usize] = b;
    }
    for i in 0..n {
        if flags[i] & FIXED != 0 {
            continue;
        }
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
    // A dry cell left inside a lake (6 or more of its cells around, such as a sill or divide
    // row the lake later surrounded) is not kept raised: it joins the lake at its input ground
    // when that is below the level.
    let mut inside: HashMap<u32, u32> = HashMap::new();
    for _ in 0..2 {
        for j in 0..n {
            if flags[j] & (SEA | RIVER | FIXED) != 0
                || body[j] != 0
                || inside.contains_key(&(j as u32))
            {
                continue;
            }
            let lake_at = |k: usize| {
                (flags[k] & (SEA | RIVER) == 0
                    && body[k] != 0
                    && basins[body[k] as usize - 1].kind == Kind::Lake)
                    .then_some(body[k])
            };
            let wet = dims
                .neighbours(j)
                .filter(|&(_, k)| lake_at(k).is_some() || inside.contains_key(&(k as u32)))
                .count();
            if wet < 6 {
                continue;
            }
            let mut count: HashMap<u32, u32> = HashMap::new();
            for (_, k) in dims.neighbours(j) {
                if let Some(b) = lake_at(k).or_else(|| inside.get(&(k as u32)).copied()) {
                    *count.entry(b).or_insert(0) += 1;
                }
            }
            if let Some((&b, _)) = count.iter().find(|&(_, &c)| c >= 6) {
                inside.insert(j as u32, b);
            }
        }
    }
    let mut notches = 0u64;
    grid::for_each_input_row(&options.grid, grid.w, |y, row| {
        if y < fixed_rows {
            return;
        }
        for (x, &z) in row.iter().enumerate() {
            let j = y * grid.w + x;
            if let Some(&b) = inside.get(&(j as u32)) {
                let level = basins[b as usize - 1].level;
                let g = ground[j].min(z);
                if g < level {
                    (ground[j], body[j], surface[j]) = (g, b, level);
                    notches += 1;
                }
            }
        }
    })?;
    lap("rivers", &mut t);

    // Body ids: sea 1, coarse lakes and rivers keep theirs; new ones from above every id the
    // coarse level and the previous band use: lakes, rivers, then dry basins (not in the
    // raster).
    let mut rivers = rivers;
    let mut next = match &coarse {
        Some(c) => c.max_id.max(north.as_ref().map_or(0, |b| b.max_id)) + 1,
        None => 2,
    };
    for basin in basins
        .iter_mut()
        .filter(|b| b.kind == Kind::Lake && b.id == 0)
    {
        basin.id = next;
        next += 1;
    }
    for river in rivers.iter_mut().filter(|r| r.id == 0 && r.cells > 0) {
        river.id = next;
        next += 1;
    }
    for basin in basins
        .iter_mut()
        .filter(|b| b.kind != Kind::Lake && b.id == 0)
    {
        basin.id = next;
        next += 1;
    }
    let body_id = |i: usize| -> u32 {
        if flags[i] & FIXED != 0 {
            fixed_id(i)
        } else if flags[i] & SEA != 0 {
            1
        } else if flags[i] & RIVER != 0 {
            rivers[body[i] as usize - 1].id
        } else if body[i] != 0 && basins[body[i] as usize - 1].kind == Kind::Lake {
            basins[body[i] as usize - 1].id
        } else {
            0
        }
    };

    // Land still draining uphill (a receiver restored on a cycle after its cell was lowered)
    // takes a lower neighbour, on the routing the checks see.
    let unclimbed = if check_dir.is_empty() {
        flood::unclimb(dims, &ground, &flags, &body, &mut dir)
    } else {
        let changed = flood::unclimb(dims, &ground, &flags, &body, &mut check_dir);
        for &(i, d) in &changed {
            dir[i] = d;
        }
        changed
    };
    if !unclimbed.is_empty() {
        eprintln!("hydrology: {} land cells turned downhill", unclimbed.len());
    }
    drop(unclimbed);

    let mut carve = report::Carve::default();
    let mut slots = report::Slots::new(w, fixed_rows);
    grid::for_each_input_row(&options.grid, grid.w, |y, row| {
        if y >= fixed_rows {
            carve.add(
                y,
                row,
                &ground[y * grid.w..(y + 1) * grid.w],
                grid.spacing_m,
            );
        }
        if coarse.is_some() && y + 1 >= fixed_rows {
            slots.add(y, row, &ground, &surface);
        }
    })?;
    let open_rows = coarse.as_ref().map_or([false; 2], |c| {
        let ch = c.size[1] * c.k;
        [c.off[1] > 0, c.off[1] + h < ch]
    });
    let ctx = report::Context {
        dims,
        ground: &ground,
        dir: if check_dir.is_empty() {
            &dir
        } else {
            &check_dir
        },
        acc: &acc,
        flags: &flags,
        body: &body,
        surface: &surface,
        basins: &basins,
        rivers: &rivers,
        band: &band,
        cell_km2: cell_km2 as f64,
        spacing_m: grid.spacing_m,
        open_rows,
        fixed_rows,
        carve_floor: &carve_floor,
        held,
        halo: halo_row,
        seam: &seam_step,
    };
    // Cycles on this run's final routing (taken cells end it), before accumulation, or in
    // the routing stitched with the previous band's (a loop across the seam: the previous band
    // has none of its own, so each holds a cell of this run).
    let cycles = cycles
        .max(flood::cycle_cells(dims, &dir))
        .max(flood::cycle_cells(dims, &check_dir));
    let (mut checks, mut examples) = report::checks(&ctx, cycles);
    let join_steps = examples.remove("seam_join_steps");
    let mut bodies = report::bodies(&ctx, &body_id);
    // The next free id: the next band and finer level start above it.
    bodies["next_id"] = json!(next);
    let mut figures = report::figures(&ctx, &carve, extended);
    if coarse.is_some() {
        let (carving, carving_figures) = report::carving(
            &ctx,
            &carve_floor,
            &held_floor,
            fixed_rows,
            &body_id,
            &carve,
            &slots,
        );
        checks.extend(carving);
        figures.insert("carving".into(), carving_figures);
    }
    if let Some(c) = &coarse {
        // Rivers reaching the sea here, through the rivers they join.
        let to_sea: std::collections::HashSet<u32> = rivers
            .iter()
            .filter(|r| {
                let mut r = *r;
                for _ in 0..rivers.len() {
                    match r.end {
                        rivers::End::Sea => return true,
                        rivers::End::Joins(b) if b != 0 => r = &rivers[b as usize - 1],
                        _ => return false,
                    }
                }
                false
            })
            .map(|r| r.id)
            .collect();
        let (consistency, consistency_figures) = fine::consistency(
            dims,
            &flags,
            &surface,
            &basins,
            c,
            &unrouted,
            grid.core_rows,
            &path_cap,
            &body_id,
            &to_sea,
        );
        checks.extend(consistency);
        // Coarse lakes still leaking after the sill rounds.
        let left = lake_figures
            .as_ref()
            .and_then(|f| f["leaks_left"].as_u64())
            .unwrap_or(0);
        checks.insert("coarse_lake_leaks_left".into(), json!(left));
        let exits_moved = basins
            .iter()
            .filter(|b| b.kind == Kind::Lake && b.exit != lakes::NO_EXIT)
            .filter(|b| {
                c.lakes
                    .get(&b.id)
                    .and_then(|l| l.exit)
                    .is_some_and(|[ex, ey]| {
                        let [qx, qy] = c.abs(c.cell(w, b.exit as usize));
                        qx.abs_diff(ex) > 1 || qy.abs_diff(ey) > 1
                    })
            })
            .map(|b| json!({ "lake": b.id, "exit_cell": [b.exit as usize % w, b.exit as usize / w] }))
            .collect::<Vec<_>>();
        figures.insert(
            "coarse".into(),
            json!({
                "spacing_m": c.spacing_m,
                "lakes": lake_figures,
                "rivers": path_figures,
                "consistency": consistency_figures,
                "lake_exits_off_the_coarse_exit": exits_moved.len(),
                "lake_exits_off_the_coarse_exit_examples": &exits_moved[..exits_moved.len().min(5)],
                "edge_inflow_cells": edge_inflow_cells,
            }),
        );
    }
    if north.is_some() {
        // A centreline from the previous band must go on as water here (or end in a basin).
        let cut: Vec<[usize; 2]> = band::cut_at_seam(&inflow, &flags, &body)
            .into_iter()
            .map(|i| [i % w, i / w])
            .collect();
        checks.insert("rivers_cut_at_seam".into(), json!(cut.len()));
        figures.insert("rivers_cut_at_seam_cells".into(), json!(cut));
        figures.insert(
            "band".into(),
            json!({
                "fixed_rows": fixed_rows,
                "inflow_cells": inflow.len(),
                "river_inflow_cells": seam_inflow,
                "streams_pruned_at_seam": channels.streams_pruned,
                "streams_joined_across_seam": seam_joins,
                "inflows_rerouted": inflows_rerouted.rivers,
                "inflow_reroute_cells_lowered": inflows_rerouted.cells,
                "inflow_reroute_max_cut_m": inflows_rerouted.max_m,
                "inflows_ended_in_a_sink": inflows_rerouted.stuck.len(),
            }),
        );
    }
    figures.insert("check_examples".into(), Value::Object(examples));
    if let Some(v) = join_steps {
        figures.insert("seam_join_steps".into(), v);
    }
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
            "centreline_cells_raised_to_a_lake": channels.raised.len(),
        }),
    );
    figures.insert("stream_power".into(), stream_power);
    figures.insert("trenches".into(), trenches);
    figures.insert("channel_widening".into(), channel_widening);
    figures.insert("receivers_set_by_steepest_descent".into(), json!(resteered));
    figures.insert("receivers_spilled".into(), json!(receivers_spilled));
    figures.insert("river_climbs_turned".into(), json!(climbs_turned));
    figures.insert("river_climbs_routed_on".into(), json!(climbs_routed));
    figures.insert("river_climbs_ended_in_a_sink".into(), json!(climbs_ended));
    figures.insert(
        "river_sinks_filled_to_own_spill".into(),
        json!({
            "sinks": river_sinks.spills,
            "lakes": river_sinks.spill_lakes,
            "joined_a_lake": river_sinks.joined,
            "left_by_reason": river_sinks.left,
        }),
    );
    figures.insert("lip_cells_raised".into(), json!(lips.cells));
    figures.insert("lip_max_raise_m".into(), json!(lips.max_m));
    figures.insert("lip_raise_volume_m3".into(), json!(lips.volume_m3.round()));
    figures.insert("late_bank_cells_raised".into(), json!(lips.late_banks));
    figures.insert(
        "notch_cells_joined_to_a_lake".into(),
        json!(lips.joins.len() as u64 + notches),
    );
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
    let flow_byte = |i: usize| -> u8 {
        if let Some(b) = north.as_ref().filter(|_| flags[i] & FIXED != 0) {
            return b.flow[i];
        }
        let mut f = dir[i];
        if flags[i] & CENTRE != 0 {
            f |= FLOW_CENTRE;
        }
        if flags[i] & (SEA | RIVER) == 0 && body[i] != 0 {
            match basins[body[i] as usize - 1].kind {
                Kind::Lake => {}
                Kind::Dry => f |= FLOW_DRY,
                Kind::Sink => f |= FLOW_DRY | FLOW_SINK,
            }
        }
        f
    };
    stage.raster("flow.u8", n, |i| [flow_byte(i)])?;
    let (moisture_entry, precipitation) = match (&coarse, &rain) {
        (Some(c), _) => {
            let m = &c.moisture;
            stage.raster("moisture.f32", m.values.len(), |k| {
                m.values[k].to_le_bytes()
            })?;
            let mut entry = m.entry.clone();
            entry.insert("nw_m".into(), json!(m.nw_m));
            entry.insert("from".into(), json!("the coarse level, unchanged"));
            (
                Value::Object(entry),
                json!({
                    "source": "coarse moisture.f32, bilinear; discharge weight moisture × \
                        scale_mm_h / the coarse mean_land_mm_h",
                    "weight_per_moisture": m.weight,
                    "windless": m.windless,
                    "wind_from_deg": wind_from_deg,
                }),
            )
        }
        (None, Some((rain, stats))) => {
            stage.raster("moisture.f32", rain.mm_h.len(), |k| {
                stats.moisture(rain.mm_h[k]).to_le_bytes()
            })?;
            let mut precipitation = stats.json();
            if let Value::Object(m) = &mut precipitation {
                m.insert("wind_from_deg".into(), json!(wind_from_deg));
                m.insert("params".into(), params.json());
            }
            (
                json!({
                    "file": "moisture.f32",
                    "size": [rain.w, rain.h],
                    "spacing_m": grid.spacing_m * rain.k as f64,
                    "nw_m": grid.nw_m,
                    "dtype": "float32",
                    "value": if wind_from_deg.is_some() {
                        json!("precipitation / scale_mm_h, clipped to 0..1")
                    } else {
                        json!(format!("{} everywhere (no wind)", moisture::NEUTRAL_MOISTURE))
                    },
                    "scale_mm_h": stats.p95_land_mm_h,
                    "scale": "95th percentile of precipitation on land cells",
                }),
                precipitation,
            )
        }
        (None, None) => unreachable!("rain is computed without a coarse level"),
    };
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
        "precipitation": precipitation,
        "constants": report::constants(river_km2_f64, BREACH_MAX_M),
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
    let mean_land_mm_h = match (&coarse, &rain) {
        (Some(c), _) => {
            c.moisture.entry["scale_mm_h"].as_f64().unwrap_or(1.0) / c.moisture.weight as f64
        }
        (None, Some((_, stats))) => stats.mean_land_mm_h as f64,
        (None, None) => 1.0,
    };
    manifest.insert(
        "discharge".into(),
        json!({
            "file": "discharge.f32",
            "unit": "km² at the mean land precipitation (drainage area weighted by rain / mean)",
            "mean_land_mm_h": mean_land_mm_h,
        }),
    );
    manifest.insert("moisture".into(), moisture_entry);
    manifest.insert(
        "flow".into(),
        json!({
            "file": "flow.u8",
            "bits": "low 4: D8 receiver (0 none; 1 N, 2 S, 3 W, 4 E, 5 NW, 6 NE, 7 SW, 8 SE); \
                16 river centreline; 32 dry basin; 64 endorheic",
        }),
    );
    let mut provenance = json!({
        "tool": "topography world hydrology",
        "version": env!("CARGO_PKG_VERSION"),
        "inputs": inputs,
        "water_table": { "source": water_source, "sha256": digest(water_text.as_bytes()) },
        "river_km2": river_km2_f64,
        "outputs": stage.digests.clone(),
    });
    if let (Some(c), Value::Object(m)) = (&coarse, &mut provenance) {
        m.insert(
            "coarse".into(),
            json!({
                "dir": options.coarse.as_ref().map(|d| d.display().to_string()),
                "files": c.digests,
            }),
        );
    }
    if let (Some(b), Value::Object(m)) = (&north, &mut provenance) {
        m.insert(
            "north".into(),
            json!({
                "dir": options.north.as_ref().map(|d| d.display().to_string()),
                "files": b.digests,
            }),
        );
    }
    manifest.insert("provenance".into(), provenance);
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
