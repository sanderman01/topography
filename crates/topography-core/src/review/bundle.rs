//! Integrity-checked review inputs shared by curation consumers.

use crate::acquisition::{
    self, Result, digest, ensure, hash_file, read_json, read_small, valid_digest,
};
use crate::catalog::source::{SourceRecord, validate_source};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    path::{Component, Path},
};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Artifact {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub schema_version: u32,
    pub kind: String,
    pub review_state: String,
    pub raw_receipt_sha256: String,
    pub asset_sha256: String,
    pub window_cells: [u32; 4],
    pub resampling: String,
    pub gdal_version: String,
    pub topography_version: String,
    pub validity: String,
    pub admission: String,
    pub artifacts: Vec<Artifact>,
}

pub(crate) struct Bundle {
    pub manifest: Manifest,
    pub source: SourceRecord,
    pub inspection: Value,
    pub metrics: Option<Value>,
}

pub(crate) fn verify(raw_path: &Path, review_path: &Path) -> Result<Bundle> {
    let raw = acquisition::verify(raw_path)?;
    verify_with_raw(raw_path, review_path, &raw)
}

/// Reuse a raw receipt verified in this operation; still verify the entire crop.
pub(crate) fn verify_with_raw(
    raw_path: &Path,
    review_path: &Path,
    raw: &acquisition::RawReceipt,
) -> Result<Bundle> {
    let manifest: Manifest = read_json(review_path)?;
    ensure(
        manifest.schema_version == 1
            && manifest.kind == "native_grid_review"
            && manifest.review_state == "unreviewed"
            && manifest.resampling == "none",
        "unsupported native review",
    )?;
    ensure(
        manifest.raw_receipt_sha256 == digest(&read_small(raw_path)?)
            && manifest.asset_sha256 == raw.asset_sha256,
        "review does not match raw receipt",
    )?;
    let objects = raw_path
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| {
            acquisition::AcquisitionError::Invalid("raw receipt needs a storage root".into())
        })?
        .join("objects");
    let source: SourceRecord = toml::from_str(&String::from_utf8_lossy(&read_small(
        &objects.join(&raw.source_sha256),
    )?))
    .map_err(|e| acquisition::AcquisitionError::Invalid(format!("invalid retained source: {e}")))?;
    validate_source(&source)?;
    let parent = review_path.parent().unwrap_or(Path::new("."));
    ensure(manifest.artifacts.len() <= 16, "too many review artifacts")?;
    let mut names = BTreeSet::new();
    for a in &manifest.artifacts {
        let mut components = Path::new(&a.path).components();
        ensure(
            matches!(components.next(), Some(Component::Normal(_)))
                && components.next().is_none()
                && names.insert(a.path.as_str())
                && valid_digest(&a.sha256),
            "invalid or duplicate review artifact",
        )?;
        ensure(
            a.bytes <= 128 * 1024 * 1024,
            "review artifact exceeds 128 MiB",
        )?;
        ensure(
            hash_file(&parent.join(&a.path))? == (a.sha256.clone(), a.bytes),
            format!("review artifact integrity mismatch: {}", a.path),
        )?;
    }
    for required in ["elevation.tif", "validity.tif", "inspection.json"] {
        ensure(
            names.contains(required),
            format!("review missing {required}"),
        )?;
    }
    let inspection: Value = read_json(&parent.join("inspection.json"))?;
    let [x, y, w, h] = manifest.window_cells;
    ensure(
        w > 0 && h > 0 && w <= 2048 && h <= 2048 && inspection["size"] == serde_json::json!([w, h]),
        "review window/dimensions mismatch",
    )?;
    let original: Value = read_json(&objects.join(&raw.inspection_sha256))?;
    ensure(
        u64::from(x) + u64::from(w) <= original["size"][0].as_u64().unwrap_or(0)
            && u64::from(y) + u64::from(h) <= original["size"][1].as_u64().unwrap_or(0),
        "review window outside raw grid",
    )?;
    let a: [f64; 6] = serde_json::from_value(original["geoTransform"].clone())?;
    let b: [f64; 6] = serde_json::from_value(inspection["geoTransform"].clone())?;
    let expected = [
        a[0] + f64::from(x) * a[1] + f64::from(y) * a[2],
        a[1],
        a[2],
        a[3] + f64::from(x) * a[4] + f64::from(y) * a[5],
        a[4],
        a[5],
    ];
    ensure(
        b.iter()
            .zip(expected)
            .all(|(value, expected)| value.is_finite() && (value - expected).abs() <= 1e-9)
            && original["coordinateSystem"] == inspection["coordinateSystem"],
        "review coordinate grid mismatch",
    )?;
    let metrics = if names.contains("metrics.json") {
        Some(read_json(&parent.join("metrics.json"))?)
    } else {
        None
    };
    Ok(Bundle {
        manifest,
        source,
        inspection,
        metrics,
    })
}
