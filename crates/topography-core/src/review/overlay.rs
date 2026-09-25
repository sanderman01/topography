//! Read-only cover overlays cropped from integrity-checked v2 prepared products.
use crate::acquisition::{
    AcquisitionError, Result, digest, ensure, hash_file, io_context,
    raster::{command, run},
    read_small,
};
use serde_json::{Value, json};
use std::{fs, path::Path};

// Layer, prepared artifact and bit offset in the packed cell byte.
const LAYERS: [(&str, &str, u32); 3] = [
    ("water", "water.tif", 0),
    ("snow_ice", "snow-ice.tif", 2),
    ("vegetation", "vegetation.tif", 4),
];

/// Pack each native review cell as 2-bit 0 unknown / 1 absent / 2 present per layer.
/// Only the manifest identity and the three masks are verified; nothing is written.
pub(crate) fn render(manifest: &Path, review_sha256: &str, size: [u32; 2]) -> Result<Value> {
    let bytes = read_small(manifest)?;
    let id = digest(&bytes);
    let directory = manifest
        .parent()
        .filter(|p| p.file_name().and_then(|s| s.to_str()) == Some(id.as_str()))
        .ok_or_else(|| AcquisitionError::Invalid("prepared manifest integrity mismatch".into()))?;
    let body: Value = serde_json::from_slice(&bytes)?;
    ensure(
        body["schema_version"] == 2
            && body["kind"] == "prepared_native_region"
            && body["policy"] == "native_ground_cover_v1",
        "cover overlays require a v2 prepared product",
    )?;
    let parents: Vec<&Value> = body["parents"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["review_sha256"] == review_sha256)
        .collect();
    ensure(
        parents.len() == 1,
        "prepared product does not contain this review",
    )?;
    let [x, y, w, h]: [u32; 4] = serde_json::from_value(parents[0]["destination_window"].clone())?;
    let grid: [u32; 2] = serde_json::from_value(body["grid"]["size"].clone())?;
    ensure(
        [w, h] == size
            && u64::from(x) + u64::from(w) <= u64::from(grid[0])
            && u64::from(y) + u64::from(h) <= u64::from(grid[1]),
        "prepared window does not match the review crop",
    )?;
    let cells = w as usize * h as usize;
    let scratch = io_context("stage cover overlay", tempfile::tempdir())?;
    let mut packed = vec![0u8; cells];
    for (name, file, shift) in LAYERS {
        let artifact = body["artifacts"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|a| a["path"] == file)
            .ok_or_else(|| AcquisitionError::Invalid(format!("prepared product lacks {file}")))?;
        // Hash a private copy so GDAL reads exactly the verified bytes.
        let copy = scratch.path().join(file);
        io_context("copy cover mask", fs::copy(directory.join(file), &copy))?;
        let (sha256, length) = hash_file(&copy)?;
        ensure(
            artifact["sha256"] == sha256.as_str() && artifact["bytes"] == length,
            format!("prepared artifact integrity mismatch: {file}"),
        )?;
        let crop = scratch.path().join(format!("{name}-crop.tif"));
        let window = [x, y, w, h].map(|n| n.to_string());
        run(command("gdal_translate")
            .args(["-q", "-of", "GTiff", "-srcwin"])
            .args(&window)
            .arg(&copy)
            .arg(&crop))?;
        let samples = super::band(&crop, scratch.path(), name, cells, false)?;
        for (cell, value) in packed.iter_mut().zip(samples) {
            // Prepared water is binary 0 land / 255 water; cover masks are categorical.
            let category = match (name, value) {
                ("water", 0) => 1,
                ("water", 255) => 2,
                ("water", _) => 3,
                (_, value) => value,
            };
            ensure(category <= 2, format!("invalid {name} category"))?;
            *cell |= category << shift;
        }
    }
    Ok(json!({"manifest_sha256": id, "width": w, "height": h, "cells": super::base64(&packed)}))
}
