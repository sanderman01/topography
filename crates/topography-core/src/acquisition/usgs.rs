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
    /// Declarations completed from tile metadata and GeoTIFF headers.
    pub completed: usize,
}

fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let start = xml.find(&format!("<{name}>"))? + name.len() + 2;
    let end = xml[start..].find(&format!("</{name}>"))? + start;
    Some(xml[start..end].trim())
}

fn iso(date: &str) -> Option<String> {
    (date.len() >= 8 && date[..8].bytes().all(|b| b.is_ascii_digit()))
        .then(|| format!("{}-{}-{}", &date[..4], &date[4..6], &date[6..8]))
}

/// Fill survey, dates, CRS, datum and spacing for a standard 3DEP 1 m tile from its
/// FGDC metadata XML (pinned as the original metadata) and its GeoTIFF header.
fn complete(record: &mut SourceRecord, dir: &Path, id: &str) -> Result<()> {
    let url = record.source_urls[0].clone();
    let project = url
        .split("/Projects/")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .ok_or_else(|| AcquisitionError::Invalid("not a 3DEP project URL".into()))?
        .to_owned();
    let xml_url = url
        .replacen("/TIFF/", "/metadata/", 1)
        .replace(".tif", ".xml");
    let xml_bytes = http::query(&xml_url)?;
    let xml = String::from_utf8_lossy(&xml_bytes).into_owned();
    ensure(
        xml.contains("U.S. Geological Survey"),
        "tile metadata does not name the USGS",
    )?;
    let (Some(begin), Some(end)) = (
        tag(&xml, "begdate").and_then(iso),
        tag(&xml, "enddate").and_then(iso),
    ) else {
        return Err(AcquisitionError::Invalid(
            "tile metadata lacks survey dates".into(),
        ));
    };
    ensure(
        xml.contains("NAVD88"),
        "tile metadata does not state NAVD88",
    )?;
    http::pace(&url);
    let info: serde_json::Value = serde_json::from_slice(&super::raster::run(
        super::raster::command("gdalinfo")
            .args(["-json", "-nomd", "-norat", "-noct"])
            .arg(format!("/vsicurl/{url}")),
    )?)?;
    let epsg = info["stac"]["proj:epsg"]
        .as_u64()
        .ok_or_else(|| AcquisitionError::Invalid("GeoTIFF header has no EPSG code".into()))?;
    let t: Vec<f64> = serde_json::from_value(info["geoTransform"].clone())?;
    ensure(
        t.len() == 6 && t[1] > 0.0 && t[5] < 0.0 && t[2] == 0.0 && t[4] == 0.0,
        "GeoTIFF is not a north-up grid",
    )?;
    let name = format!("{id}.xml");
    write_new(&dir.join(&name), &xml_bytes)?;
    record.original_metadata_path = name.into();
    record.survey_id = Some(project.clone());
    record.acquisition_dates = vec![format!("{begin}/{end}")];
    record.spatial.horizontal_crs = Some(format!("EPSG:{epsg}"));
    record.spatial.vertical_datum = Some("NAVD88 (per the tile's USGS metadata)".into());
    record.spatial.horizontal_units = Some("metre".into());
    record.spatial.vertical_units = Some("metre".into());
    record.spatial.native_spacing_m = Some([t[1], -t[5]]);
    record.processing_history = vec![format!(
        "USGS 3DEP standard 1 m bare-earth DEM tile, project {project}; declaration completed from the tile's FGDC metadata and GeoTIFF header at discovery."
    )];
    Ok(())
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
        completed: 0,
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
        let mut record = record;
        if saved_response.is_none() {
            // Complete the declaration from the tile's own metadata and header;
            // a tile that cannot be completed stays minimal and fails qualification.
            match complete(&mut record, stage.path(), &id) {
                Ok(()) => summary.completed += 1,
                Err(error) => eprintln!("{id}: declaration left incomplete: {error}"),
            }
        }
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
