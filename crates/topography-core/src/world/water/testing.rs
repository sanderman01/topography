//! Test-only: a synthetic grid with sea, a lagoon, a humid basin, an arid basin and rivers.

use super::geom::World;
use super::layers::{Classes, Layer};
use super::params;
use super::pipeline::Setup;
use super::rain::Rain;
use super::table::{DEFAULT, Table};

/// Falls east into the sea; a humid bowl at (16, 16), an arid bowl at (16, 46) (dune class),
/// a lagoon behind a rim at (54, 32), bumps and noise.
pub(crate) fn terrain(world: World) -> Vec<f32> {
    let bowl = |x: f32, y: f32, cx: f32, cy: f32, r: f32, depth: f32| {
        let d2 = ((x - cx).powi(2) + (y - cy).powi(2)) / (r * r);
        if d2 < 1.0 { -depth * (1.0 - d2) } else { 0.0 }
    };
    (0..world.w * world.h)
        .map(|i| {
            let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
            let mut z = 30.0 - 0.5 * x
                + 3.0 * (0.3 * x).sin() * (0.25 * y).cos()
                + ((i * 7919) % 23) as f32 * 0.05;
            z += bowl(x, y, 16.0, 16.0, 7.0, 9.0) + bowl(x, y, 16.0, 46.0, 7.0, 9.0);
            let r = ((x - 54.0).powi(2) + (y - 32.0).powi(2)).sqrt();
            if r < 3.0 {
                z = -2.0 + r * 0.3;
            } else if r < 5.0 {
                z = z.max(2.5);
            }
            z
        })
        .collect()
}

/// Dunes (8, r = 0: playas) in the south-west quarter, plain (18, r = 1) elsewhere.
pub(crate) fn classes(world: World) -> Vec<u8> {
    (0..world.w * world.h)
        .map(|i| {
            if i % world.w < world.w / 2 && i / world.w >= world.h / 2 {
                8
            } else {
                18
            }
        })
        .collect()
}

/// The synthetic world's run: 1 m cells, 3 threads, lakes from 4 cells (the world is small;
/// real runs use the table's area), uniform rain, contraction on.
pub(crate) fn setup(world: World, tile: u64, river_km2: f64) -> Setup {
    let mut table = Table::parse(DEFAULT).expect("table");
    table.lake_min_m2 = 4.0;
    let params = params(&table, 1.0, Some(river_km2));
    let classes = Classes::new(Layer::memory(world, classes(world)), 1, world).expect("classes");
    Setup {
        world,
        nw_m: [0.0, world.h as f64],
        spacing_m: 1.0,
        tile,
        threads: 3,
        classes: Some(classes),
        table,
        rain: Rain::uniform(2.0),
        params,
        contract: true,
    }
}

/// `terrain(world)` (64² or larger) with a V valley in x ≥ 24, y ≤ 20 falling east 2 cm a
/// cell to x = 40 and 8 cm a cell past it to the map edge, its sides rising 0.3 m a cell, and
/// a one-cell dam at x = 40 whose crest is 1 m above the floor and rises 2 cm a row (one
/// spill): the pond behind it is long enough to be worth its gorge, and is breached (as
/// breach.rs's `dammed_at`).
pub(crate) fn dammed(world: World) -> Vec<f32> {
    let mut z = terrain(world);
    for y in 0..=20u64 {
        let side = (y as f32 - 10.0).abs();
        for x in 24..world.w {
            let floor = |x: u64| {
                let x = x as f32;
                if x <= 40.0 {
                    15.0 - 0.02 * (x - 24.0)
                } else {
                    14.68 - 0.08 * (x - 40.0)
                }
            };
            let mut v = floor(x) + 0.3 * side;
            if x == 40 {
                v = v.max(floor(39) + 1.0 + 0.02 * side);
            }
            z[(y * world.w + x) as usize] = v;
        }
    }
    z
}

/// A grid directory holding `dammed(world)` at 1 m with `classes(world)` and a 2 × 2
/// moisture field.
pub(crate) fn grid_dir(dir: &std::path::Path, world: World) -> std::path::PathBuf {
    use std::fs;
    let grid = dir.join("grid");
    fs::create_dir_all(&grid).expect("grid dir");
    let z: Vec<u8> = dammed(world).iter().flat_map(|v| v.to_le_bytes()).collect();
    fs::write(grid.join("elevation.f32"), z).expect("elevation");
    fs::write(grid.join("classes.u8"), classes(world)).expect("classes");
    let moisture: Vec<u8> = [1.0f32, 0.6, 1.4, 0.8]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    fs::write(grid.join("moisture.f32"), moisture).expect("moisture");
    let half = world.w.max(world.h) as f64 / 2.0;
    let manifest = serde_json::json!({
        "size": [world.w, world.h], "spacing_m": 1.0, "origin": [0.0, world.h as f64],
        "moisture": {"file": "moisture.f32", "size": [2, 2], "spacing_m": half,
                     "nw_m": [0.0, world.h as f64], "scale_mm_h": 2.0},
        "discharge": {"mean_land_mm_h": 2.0},
    });
    fs::write(
        grid.join("grid.json"),
        serde_json::to_vec(&manifest).expect("json"),
    )
    .expect("grid.json");
    // The synthetic bowls are small: a table whose lake minimum is 4 m² (pass it as `water`).
    let table = DEFAULT.replace("lake_min_m2 = 2000.0", "lake_min_m2 = 4.0");
    assert_ne!(table, DEFAULT, "the table's lake minimum moved");
    let table = format!(
        "# Test table for the synthetic 64² fixture (lake_min_m2 = 4), written by\n\
         # testing::grid_dir; not the production config (configs/hydrology/water-v2.toml).\n{table}"
    );
    fs::write(grid.join("water.toml"), table).expect("water.toml");
    grid
}
