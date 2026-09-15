use std::path::Path;

use serde::Deserialize;

use super::PresetStatus;
use crate::contracts::{ContractError, nonempty, positive, read_toml, require, version};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorldPreset {
    schema_version: u32,
    status: PresetStatus,
    name: String,
    grid: Grid,
    training: Training,
    generation: Generation,
    layout: Layout,
    review: Review,
    export: Export,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Grid {
    cells: [u32; 2],
    spacing_m: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Training {
    tile_cells: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Generation {
    scales_m: Vec<f64>,
    window_cells: u32,
    stride_cells: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Layout {
    sketch_cells: [u32; 2],
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Review {
    clearances_m: Vec<f64>,
    vehicle: Option<Vehicle>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Vehicle {
    width_m: f64,
    length_m: f64,
    clearance_margin_m: f64,
    speed_m_s: f64,
    minimum_turning_diameter_m: f64,
    climb_rate_m_s: f64,
    ceiling_elevation_m: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Export {
    tile_cells: u32,
    sampling: Sampling,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Sampling {
    SharedEdgeVertices,
}

/// Derived geometry only. This does not allocate terrain or estimate working memory.
#[derive(Debug)]
pub struct WorldSummary {
    pub name: String,
    /// Axis order is x, y. Internal grids sample cell centers.
    pub cells: [u32; 2],
    pub extent_m: [f64; 2],
    pub spacing_m: f64,
    pub float32_layer_bytes: u64,
    pub export_tiles: [u32; 2],
    /// Final tile width and height, including both edge vertices.
    pub final_tile_vertices: [u32; 2],
    pub training_tile_cells: u32,
    pub generation_window_cells: u32,
    pub generation_stride_cells: u32,
    pub vehicle_configured: bool,
}

/// Read and validate a v1 world preset, rejecting unknown fields and unsafe sizes.
pub fn inspect_world(path: &Path) -> Result<WorldSummary, ContractError> {
    let world: WorldPreset = read_toml(path)?;
    version(world.schema_version)?;
    let PresetStatus::Draft = world.status;
    nonempty(&world.name, "name")?;
    positive(world.grid.spacing_m, "grid.spacing_m")?;
    require(
        world.grid.cells.iter().all(|&v| v > 0),
        "grid.cells",
        "both axes must be greater than zero",
    )?;
    let extent_m = world
        .grid
        .cells
        .map(|cells| f64::from(cells) * world.grid.spacing_m);
    require(
        extent_m.iter().all(|v| v.is_finite() && *v > 0.0),
        "grid",
        "derived extent must be finite and positive",
    )?;
    let count = u64::from(world.grid.cells[0]) * u64::from(world.grid.cells[1]);
    let float32_layer_bytes = count.checked_mul(4).ok_or_else(|| ContractError::Invalid {
        field: "grid.cells".into(),
        reason: "Float32 layer size exceeds a 64-bit byte count".into(),
    })?;
    require(
        world.training.tile_cells > 0,
        "training.tile_cells",
        "must be greater than zero",
    )?;
    require(
        world.generation.window_cells > 0,
        "generation.window_cells",
        "must be greater than zero",
    )?;
    require(
        world.generation.stride_cells > 0
            && world.generation.stride_cells <= world.generation.window_cells,
        "generation.stride_cells",
        "must be in 1..=window_cells",
    )?;
    require(
        !world.generation.scales_m.is_empty(),
        "generation.scales_m",
        "must contain at least the final spacing",
    )?;
    for &scale in &world.generation.scales_m {
        positive(scale, "generation.scales_m")?;
        for extent in extent_m {
            let cells = extent / scale;
            // Floating-point arithmetic tolerance, not a terrain acceptance tolerance.
            let tolerance = 8.0 * f64::EPSILON * cells.abs().max(1.0);
            require(
                cells.is_finite() && cells >= 1.0 && (cells - cells.round()).abs() <= tolerance,
                "generation.scales_m",
                "each scale must divide both physical extents into whole cells",
            )?;
        }
    }
    require(
        world.generation.scales_m.windows(2).all(|s| s[0] > s[1]),
        "generation.scales_m",
        "must be strictly descending from coarse to fine",
    )?;
    require(
        world.generation.scales_m.last() == Some(&world.grid.spacing_m),
        "generation.scales_m",
        "last scale must equal grid.spacing_m",
    )?;
    require(
        world.layout.sketch_cells.iter().all(|&v| v > 0),
        "layout.sketch_cells",
        "both axes must be greater than zero",
    )?;
    require(
        !world.review.clearances_m.is_empty(),
        "review.clearances_m",
        "must contain a review clearance",
    )?;
    for &clearance in &world.review.clearances_m {
        positive(clearance, "review.clearances_m")?;
    }
    if let Some(vehicle) = &world.review.vehicle {
        for (field, value) in [
            ("width_m", vehicle.width_m),
            ("length_m", vehicle.length_m),
            ("speed_m_s", vehicle.speed_m_s),
            (
                "minimum_turning_diameter_m",
                vehicle.minimum_turning_diameter_m,
            ),
            ("climb_rate_m_s", vehicle.climb_rate_m_s),
        ] {
            positive(value, &format!("review.vehicle.{field}"))?;
        }
        require(
            vehicle.clearance_margin_m.is_finite() && vehicle.clearance_margin_m >= 0.0,
            "review.vehicle.clearance_margin_m",
            "must be finite and nonnegative",
        )?;
        if let Some(ceiling) = vehicle.ceiling_elevation_m {
            require(
                ceiling.is_finite(),
                "review.vehicle.ceiling_elevation_m",
                "must be finite",
            )?;
        }
    }
    require(
        world.export.tile_cells == 1024,
        "export.tile_cells",
        "v1 full export tiles span exactly 1024 intervals",
    )?;
    let Sampling::SharedEdgeVertices = world.export.sampling;
    Ok(WorldSummary {
        name: world.name,
        cells: world.grid.cells,
        extent_m,
        spacing_m: world.grid.spacing_m,
        float32_layer_bytes,
        export_tiles: world
            .grid
            .cells
            .map(|cells| cells.div_ceil(world.export.tile_cells)),
        final_tile_vertices: world
            .grid
            .cells
            .map(|cells| (cells - 1) % world.export.tile_cells + 2),
        training_tile_cells: world.training.tile_cells,
        generation_window_cells: world.generation.window_cells,
        generation_stride_cells: world.generation.stride_cells,
        vehicle_configured: world.review.vehicle.is_some(),
    })
}
