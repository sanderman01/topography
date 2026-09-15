use super::{AcquisitionError, Result, digest, ensure, http, io_context, read_small, write_new};
use crate::catalog::source::{
    License, SourceRecord, Spatial, SurfaceType, TrainingPermission, validate_source,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

const COLLECTION: &str = "Digital Elevation Model (DEM) 1 meter";

// Provider responses may add fields; their original bytes remain authoritative metadata.
#[derive(Deserialize)]
struct Products {
    total: u64,
    items: Vec<Product>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Product {
    source_id: String,
    title: String,
    #[serde(rename = "downloadURL")]
    download_url: String,
    format: String,
    size_in_bytes: Option<u64>,
    bounding_box: Bounds,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Bounds {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

#[derive(Debug, Serialize)]
pub struct DiscoverySummary {
    pub schema_version: u32,
    pub provider: &'static str,
    pub collection: &'static str,
    pub query_bounds_wgs84: [f64; 4],
    pub total_matches: u64,
    pub returned: usize,
    pub truncated: bool,
    pub known_download_bytes: u64,
    pub unknown_size_assets: usize,
    pub source_files: Vec<PathBuf>,
    pub discovery_mode: &'static str,
}

/// Query at most 100 candidates. A truncated result explicitly requires narrowing
/// the area; it never represents complete polygon coverage or training permission.
pub fn discover_usgs(
    bounds: [f64; 4],
    limit: u32,
    output: &Path,
    saved_response: Option<&Path>,
) -> Result<DiscoverySummary> {
    ensure(
        (1..=100).contains(&limit),
        "discovery max-items must be in 1..=100",
    )?;
    ensure(
        bounds.iter().all(|v| v.is_finite())
            && bounds[0] >= -180.0
            && bounds[2] <= 180.0
            && bounds[1] >= -90.0
            && bounds[3] <= 90.0
            && bounds[0] < bounds[2]
            && bounds[1] < bounds[3],
        "bbox must be ordered west,south,east,north degrees without antimeridian crossing",
    )?;
    ensure(
        !output.exists(),
        "discovery output already exists; choose a new directory",
    )?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    io_context("create discovery parent", fs::create_dir_all(parent))?;
    let stage = io_context("stage discovery", tempfile::tempdir_in(parent))?;
    let response = stage.path().join("response.json");
    if let Some(saved) = saved_response {
        write_new(&response, &read_small(saved)?)?;
    } else {
        http::discover(bounds, limit, &response)?;
    }
    let bytes = read_small(&response)?;
    let products: Products = serde_json::from_slice(&bytes)?;
    ensure(
        products.items.len() <= limit as usize && products.total >= products.items.len() as u64,
        "USGS response exceeds the requested limit or has inconsistent counts",
    )?;
    let mut summary = DiscoverySummary {
        schema_version: 1,
        provider: "usgs",
        collection: COLLECTION,
        query_bounds_wgs84: bounds,
        total_matches: products.total,
        returned: products.items.len(),
        truncated: products.total > products.items.len() as u64,
        known_download_bytes: 0,
        unknown_size_assets: 0,
        source_files: Vec::new(),
        discovery_mode: if saved_response.is_some() {
            "saved_response"
        } else {
            "tnm_api"
        },
    };
    let mut ids = BTreeSet::new();
    for item in products.items {
        ensure(
            !item.source_id.trim().is_empty() && ids.insert(item.source_id.clone()),
            "USGS response has empty or duplicate source IDs",
        )?;
        ensure(
            item.format == "GeoTIFF" && item.download_url.to_ascii_lowercase().ends_with(".tif"),
            "USGS returned a non-GeoTIFF product",
        )?;
        http::validate_url(&item.download_url)?;
        let id = format!("usgs-{}", &digest(item.source_id.as_bytes())[..16]);
        let record = SourceRecord {
            schema_version: 1,
            record_id: id.clone(),
            synthetic: false,
            provider: "usgs".into(),
            collection_id: COLLECTION.into(),
            asset_id: item.source_id,
            survey_id: None,
            acquisition_dates: Vec::new(),
            source_urls: vec![item.download_url],
            original_metadata_path: "response.json".into(),
            asset_sha256: None,
            spatial: Spatial {
                bounds_wgs84: Some([
                    item.bounding_box.min_x,
                    item.bounding_box.min_y,
                    item.bounding_box.max_x,
                    item.bounding_box.max_y,
                ]),
                horizontal_crs: None,
                vertical_datum: None,
                horizontal_units: None,
                vertical_units: None,
                native_spacing_m: None,
                ground_points_per_m2: None,
                surface_type: SurfaceType::BareEarthDem,
            },
            processing_history: Vec::new(),
            license: License {
                training_permission: TrainingPermission::Unknown,
                license_name: None,
                license_url: None,
                evidence: None,
                attribution: None,
                edited_map_reuse: None,
                redistribution: None,
            },
        };
        validate_source(&record)?;
        let toml = toml::to_string_pretty(&record)
            .map_err(|e| AcquisitionError::Invalid(format!("encode source record: {e}")))?;
        let name = format!("{id}.toml");
        write_new(&stage.path().join(&name), toml.as_bytes())?;
        summary.source_files.push(PathBuf::from(name));
        if let Some(size) = item.size_in_bytes {
            summary.known_download_bytes = summary
                .known_download_bytes
                .checked_add(size)
                .ok_or_else(|| AcquisitionError::Invalid("download-size sum overflow".into()))?;
        } else {
            summary.unknown_size_assets += 1;
        }
        // Titles are retained in response.json; publication dates are not survey dates.
        ensure(!item.title.trim().is_empty(), "USGS product title is empty")?;
    }
    write_new(
        &stage.path().join("discovery.json"),
        &serde_json::to_vec_pretty(&summary)?,
    )?;
    io_context(
        "publish discovery directory",
        fs::rename(stage.path(), output),
    )?;
    Ok(summary)
}
