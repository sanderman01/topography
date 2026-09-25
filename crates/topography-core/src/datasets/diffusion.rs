//! Diffusion training release: a snapshot of every accepted Ready gallery tile.

use super::*;
use crate::{
    acquisition::io_context,
    catalog::curation,
    preparation::automatic,
    review::{band, gallery},
};
use std::fs;

const LAYERS: [(&str, bool); 5] = [
    ("elevation", true),
    ("validity", false),
    ("water", false),
    ("snow-ice", false),
    ("vegetation", false),
];

fn footprint(grid: &Grid) -> [f64; 4] {
    let right = grid.origin[0] + f64::from(grid.size[0]) * grid.spacing_m[0];
    let bottom = grid.origin[1] - f64::from(grid.size[1]) * grid.spacing_m[1];
    [grid.origin[0], bottom, right, grid.origin[1]]
}

fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

/// Publish raw layers of the current prepared product of each accepted Ready tile.
pub fn build_diffusion(runtime: &Path, gallery_path: &Path, output: &Path) -> Result<PathBuf> {
    ensure(!output.exists(), "release output already exists")?;
    let (gallery, gallery_sha256, snapshots) = gallery::load(gallery_path)?;
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    for (tile, snapshot) in gallery.tiles.iter().zip(&snapshots) {
        if curation::selection(runtime, &tile.review_sha256, snapshot)?["decision"] != "accept" {
            continue;
        }
        // Same readiness rule as the gallery worker: verified product with current game cover.
        let Some(path) = automatic::existing(runtime, &tile.review_sha256)? else {
            continue;
        };
        if !automatic::current_cover(&path) {
            continue;
        }
        let bytes = read_small(&path)?;
        let id = digest(&bytes);
        if !seen.insert(id.clone()) {
            continue;
        }
        let manifest: Value = serde_json::from_slice(&bytes)?;
        let grid: Grid = serde_json::from_value(manifest["grid"].clone())?;
        candidates.push((tile, id, path, manifest, grid));
    }
    // Larger products win footprint overlaps so no terrain is counted twice.
    candidates.sort_by(|a, b| {
        let area = |g: &Grid| {
            f64::from(g.size[0]) * g.spacing_m[0] * f64::from(g.size[1]) * g.spacing_m[1]
        };
        area(&b.4).total_cmp(&area(&a.4)).then(a.1.cmp(&b.1))
    });
    let mut kept: Vec<(Value, [f64; 4])> = Vec::new();
    let mut skipped = Vec::new();
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    io_context("create release parent", fs::create_dir_all(parent))?;
    let stage = io_context("stage release", tempfile::tempdir_in(parent))?;
    let scratch = io_context("release scratch", tempfile::tempdir())?;
    let mut tiles = Vec::new();
    for (tile, id, path, manifest, grid) in candidates {
        let extent = footprint(&grid);
        if kept
            .iter()
            .any(|(crs, other)| *crs == manifest["coordinate_system"] && overlaps(extent, *other))
        {
            skipped.push(json!({"prepared_sha256": id, "reason": "overlaps a larger product"}));
            continue;
        }
        ensure(
            (1..=9).contains(&tile.primary_class_id),
            "tile class must be a legend class 1..=9",
        )?;
        let cells = grid.size[0] as usize * grid.size[1] as usize;
        let mut layers = serde_json::Map::new();
        for (layer, float) in LAYERS {
            let bytes = band(
                &path.with_file_name(format!("{layer}.tif")),
                scratch.path(),
                layer,
                cells,
                float,
            )?;
            let name = format!("{id}/{layer}.{}", if float { "f32" } else { "u8" });
            write_new(&stage.path().join(&name), &bytes)?;
            layers.insert(
                layer.into(),
                json!({"path": name, "sha256": digest(&bytes), "dtype": if float {"float32_le"} else {"uint8"}}),
            );
        }
        tiles.push(json!({
            "prepared_sha256": id,
            "review_sha256": tile.review_sha256,
            "title": tile.title,
            "primary_class_id": tile.primary_class_id,
            "survey": manifest["survey"],
            "coordinate_system": manifest["coordinate_system"],
            "grid": grid,
            "masked_fill_m": manifest["masked_fill_m"],
            "layers": layers,
        }));
        kept.push((manifest["coordinate_system"].clone(), extent));
    }
    ensure(!tiles.is_empty(), "no accepted Ready tiles in this gallery")?;
    let release = json!({
        "schema_version": 1,
        "kind": "diffusion_training_release",
        "gallery_sha256": gallery_sha256,
        "topography_version": env!("CARGO_PKG_VERSION"),
        "mask_codes": {
            "validity": "255 valid ground, 0 masked",
            "water": "255 water, 0 not water",
            "snow-ice": "2 present, 1 absent, 0 unknown",
            "vegetation": "2 present, 1 absent, 0 unknown",
        },
        "tiles": tiles,
        "skipped": skipped,
    });
    write_new(
        &stage.path().join("manifest.json"),
        &serde_json::to_vec_pretty(&release)?,
    )?;
    io_context("publish release", fs::rename(stage.path(), output))?;
    Ok(output.join("manifest.json"))
}
