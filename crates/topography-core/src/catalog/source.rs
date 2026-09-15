use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::contracts::{ContractError, nonempty, positive, read_toml, require, version};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceRecord {
    schema_version: u32,
    record_id: String,
    synthetic: bool,
    provider: String,
    collection_id: String,
    asset_id: String,
    survey_id: Option<String>,
    acquisition_dates: Vec<String>,
    source_urls: Vec<String>,
    original_metadata_path: PathBuf,
    asset_sha256: Option<String>,
    spatial: Spatial,
    processing_history: Vec<String>,
    license: License,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Spatial {
    /// Discovery bounds never establish an exact coverage polygon.
    bounds_wgs84: Option<[f64; 4]>,
    horizontal_crs: Option<String>,
    vertical_datum: Option<String>,
    horizontal_units: Option<String>,
    vertical_units: Option<String>,
    native_spacing_m: Option<[f64; 2]>,
    ground_points_per_m2: Option<f64>,
    surface_type: SurfaceType,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SurfaceType {
    BareEarthDem,
    ClassifiedPointCloud,
    SurfaceModel,
    Imagery,
    Bathymetry,
    Unknown,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct License {
    training_permission: TrainingPermission,
    license_name: Option<String>,
    license_url: Option<String>,
    evidence: Option<String>,
    attribution: Option<String>,
    edited_map_reuse: Option<String>,
    redistribution: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TrainingPermission {
    Confirmed,
    Prohibited,
    Restricted,
    Ambiguous,
    Unknown,
}

/// Declared metadata only. No assets or supporting evidence are verified.
#[derive(Debug)]
pub struct SourceSummary {
    pub record_id: String,
    pub synthetic: bool,
    pub training_permission_recorded: bool,
    pub unresolved_fields: Vec<&'static str>,
}

/// A well-formed exclusion record is valid. Decoding never authorizes preparation,
/// training, or human acceptance; the future importer must enforce admission gates.
pub fn inspect_source(path: &Path) -> Result<SourceSummary, ContractError> {
    let source: SourceRecord = read_toml(path)?;
    version(source.schema_version)?;
    for (field, value) in [
        ("record_id", &source.record_id),
        ("provider", &source.provider),
        ("collection_id", &source.collection_id),
        ("asset_id", &source.asset_id),
    ] {
        nonempty(value, field)?;
    }
    require(
        !source.original_metadata_path.as_os_str().is_empty(),
        "original_metadata_path",
        "must not be empty",
    )?;
    require(
        !source.source_urls.is_empty(),
        "source_urls",
        "must contain an original source URL",
    )?;
    for url in &source.source_urls {
        nonempty(url, "source_urls")?;
    }
    for date in &source.acquisition_dates {
        nonempty(date, "acquisition_dates")?;
    }
    for step in &source.processing_history {
        nonempty(step, "processing_history")?;
    }
    if let Some(sha256) = &source.asset_sha256 {
        require(
            sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "asset_sha256",
            "must contain exactly 64 hexadecimal digits",
        )?;
    }
    let mut unresolved_fields = Vec::new();
    for (field, value) in [
        ("survey_id", &source.survey_id),
        ("spatial.horizontal_crs", &source.spatial.horizontal_crs),
        ("spatial.vertical_datum", &source.spatial.vertical_datum),
        ("spatial.horizontal_units", &source.spatial.horizontal_units),
        ("spatial.vertical_units", &source.spatial.vertical_units),
        ("license.license_name", &source.license.license_name),
        ("license.license_url", &source.license.license_url),
        ("license.evidence", &source.license.evidence),
        ("license.attribution", &source.license.attribution),
        ("license.edited_map_reuse", &source.license.edited_map_reuse),
        ("license.redistribution", &source.license.redistribution),
    ] {
        if let Some(value) = value {
            nonempty(value, field)?;
        } else {
            unresolved_fields.push(field);
        }
    }
    if source.asset_sha256.is_none() {
        unresolved_fields.push("asset_sha256");
    }
    if source.acquisition_dates.is_empty() {
        unresolved_fields.push("acquisition_dates");
    }
    if source.processing_history.is_empty() {
        unresolved_fields.push("processing_history");
    }
    if let Some([west, south, east, north]) = source.spatial.bounds_wgs84 {
        require(
            [west, south, east, north].iter().all(|n| n.is_finite())
                && (-180.0..=180.0).contains(&west)
                && (-180.0..=180.0).contains(&east)
                && (-90.0..=90.0).contains(&south)
                && (-90.0..=90.0).contains(&north)
                && west < east
                && south < north,
            "spatial.bounds_wgs84",
            "must be finite [west, south, east, north] degrees without antimeridian crossing",
        )?;
    } else {
        unresolved_fields.push("spatial.bounds_wgs84");
    }
    if let Some(spacing) = source.spatial.native_spacing_m {
        for value in spacing {
            positive(value, "spatial.native_spacing_m")?;
        }
    } else {
        unresolved_fields.push("spatial.native_spacing_m");
    }
    if let Some(density) = source.spatial.ground_points_per_m2 {
        require(
            density.is_finite() && density >= 0.0,
            "spatial.ground_points_per_m2",
            "must be finite and nonnegative",
        )?;
    }
    if matches!(source.spatial.surface_type, SurfaceType::Unknown) {
        unresolved_fields.push("spatial.surface_type");
    }
    let training_permission_recorded = matches!(
        source.license.training_permission,
        TrainingPermission::Confirmed
    );
    if training_permission_recorded {
        require(
            source.license.license_name.is_some()
                && source.license.license_url.is_some()
                && source.license.evidence.is_some(),
            "license.training_permission",
            "confirmed requires dataset-specific license_name, license_url, and evidence",
        )?;
    }
    Ok(SourceSummary {
        record_id: source.record_id,
        synthetic: source.synthetic,
        training_permission_recorded,
        unresolved_fields,
    })
}
