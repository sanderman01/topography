use crate::{
    acquisition::{
        Result, digest, ensure, hash_file, io_context,
        raster::{command, run},
        read_small, valid_digest, write_new,
    },
    catalog::curation,
    review::{band, bundle},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

const MASK_NAMES_V1: [&str; 4] = ["water", "quality", "repairs", "features"];
const MASK_NAMES_V2: [&str; 6] = [
    "water",
    "quality",
    "repairs",
    "features",
    "snow-ice",
    "vegetation",
];
const MASK_NAMES_V3: [&str; 7] = [
    "water",
    "quality",
    "repairs",
    "features",
    "snow-ice",
    "vegetation",
    "land-cover",
];

fn mask_names(version: u32) -> &'static [&'static str] {
    match version {
        1 => &MASK_NAMES_V1,
        2 => &MASK_NAMES_V2,
        _ => &MASK_NAMES_V3,
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InputFile {
    path: PathBuf,
    sha256: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema_version: u32,
    raw_receipt: InputFile,
    review: InputFile,
    masks: BTreeMap<String, Option<InputFile>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    surface_cover: Option<BTreeMap<String, SurfaceCoverInput>>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SurfaceCoverInput {
    status: String,
    note: String,
    observed_at: Option<String>,
    evidence: Vec<InputFile>,
}

#[derive(Default, Serialize)]
pub(crate) struct Counts {
    pub terrestrial_cells: u64,
    pub valid_terrestrial_cells: u64,
    water_cells: u64,
    unclassified_water_cells: u64,
    unsupported_terrestrial_cells: u64,
    unknown_terrestrial_quality_cells: u64,
    provider_invalid_terrestrial_cells: u64,
    nonfinite_terrestrial_cells: u64,
    repaired_cells: u64,
    defining_feature_cells: u64,
    unsupported_feature_cells: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    ground_cells: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snow_ice_cells: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unknown_snow_ice_cells: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vegetation_cells: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unknown_vegetation_cells: Option<u64>,
}

#[derive(Serialize)]
struct Assessment {
    schema_version: u32,
    kind: &'static str,
    policy: &'static str,
    raw_receipt_sha256: String,
    review_sha256: String,
    asset_sha256: String,
    window_cells: [u32; 4],
    grid: Value,
    masks: BTreeMap<String, Option<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    surface_cover: Option<BTreeMap<String, Value>>,
    counts: Option<Counts>,
    blockers: Vec<String>,
    gdal_version: String,
    topography_version: &'static str,
    admission: &'static str,
}

/// A point-in-time eligibility result, not a permit or prepared product.
#[derive(Serialize)]
pub struct Check {
    pub eligible: bool,
    schema_version: u32,
    kind: &'static str,
    policy: &'static str,
    pub(super) curation_record_sha256: String,
    pub(super) quality_report_sha256: String,
    pub(super) review_sha256: String,
    blockers: Vec<String>,
    admission: &'static str,
    #[serde(skip)]
    pub(super) primary_class_id: Option<u16>,
}

impl Check {
    /// Serialize the portable eligibility snapshot.
    pub fn json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

fn pinned(path: &Path) -> Result<InputFile> {
    Ok(InputFile {
        path: io_context("resolve quality input", fs::canonicalize(path))?,
        sha256: digest(&read_small(path)?),
    })
}

/// Pin a native review and leave every classification explicitly unresolved.
pub fn draft(raw: &Path, review: &Path, output: &Path) -> Result<PathBuf> {
    ensure(!output.exists(), "quality draft output already exists")?;
    bundle::verify(raw, review)?;
    let input = Input {
        schema_version: 1,
        raw_receipt: pinned(raw)?,
        review: pinned(review)?,
        masks: MASK_NAMES_V1
            .into_iter()
            .map(|n| (n.into(), None))
            .collect(),
        surface_cover: None,
    };
    write_new(output, &serde_json::to_vec_pretty(&input)?)?;
    Ok(output.to_path_buf())
}

fn verify_input(base: &Path, input: &InputFile, maximum: u64) -> Result<PathBuf> {
    let path = io_context(
        "resolve quality input",
        fs::canonicalize(base.join(&input.path)),
    )?;
    let metadata = io_context("inspect quality input", fs::metadata(&path))?;
    ensure(
        metadata.is_file() && metadata.len() <= maximum,
        "quality input exceeds size limit or is not a file",
    )?;
    ensure(
        valid_digest(&input.sha256) && hash_file(&path)?.0 == input.sha256,
        "quality input digest mismatch",
    )?;
    Ok(path)
}

fn inspect(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&run(command("gdalinfo")
        .args(["-json", "-noct", "-norat", "-nogcp", "-if", "GTiff"])
        .arg(path))?)?)
}

fn grid_matches(info: &Value, reference: &Value) -> bool {
    info["driverShortName"] == "GTiff"
        && info["size"] == reference["size"]
        && super::same_transform(&info["geoTransform"], &reference["geoTransform"])
        && info["coordinateSystem"] == reference["coordinateSystem"]
}

fn mask_samples(
    path: &Path,
    reference: &Value,
    scratch: &Path,
    name: &str,
    cells: usize,
    provider: bool,
) -> Result<Vec<u8>> {
    let info = inspect(path)?;
    ensure(
        grid_matches(&info, reference),
        format!("{name} grid mismatch"),
    )?;
    ensure(
        info["bands"]
            .as_array()
            .is_some_and(|b| b.len() == 1 && b[0]["type"] == "Byte"),
        format!("{name} must be one Byte band"),
    )?;
    let b = &info["bands"][0];
    ensure(
        b.get("noDataValue").is_none()
            && (b.get("mask").is_none() || b["mask"]["flags"] == json!(["ALL_VALID"])),
        format!("{name} must not have nodata or a hidden validity mask"),
    )?;
    ensure(
        b.get("scale").is_none_or(|v| v == 1) && b.get("offset").is_none_or(|v| v == 0),
        format!("{name} must not have scale/offset"),
    )?;
    let values = band(path, scratch, name, cells, false)?;
    let max = if matches!(name, "water" | "quality" | "snow-ice" | "vegetation") {
        2
    } else {
        1
    };
    let land_cover = crate::acquisition::cover::LAND_COVER_CLASSES;
    ensure(
        values.iter().all(|v| {
            if provider {
                matches!(v, 0 | 255)
            } else if name == "land-cover" {
                land_cover.contains(v)
            } else {
                *v <= max
            }
        }),
        format!("{name} contains an unsupported category"),
    )?;
    Ok(values)
}

fn evaluate(path: &Path) -> Result<(Assessment, bundle::Bundle)> {
    let input_bytes = read_small(path)?;
    let input: Input = serde_json::from_slice(&input_bytes)?;
    ensure(
        matches!(input.schema_version, 1..=3),
        "only quality input versions 1 to 3 are supported",
    )?;
    let names = mask_names(input.schema_version);
    ensure(
        input.masks.len() == names.len() && names.iter().all(|n| input.masks.contains_key(*n)),
        format!(
            "quality input v{} requires exactly the masks {}",
            input.schema_version,
            names.join(", ")
        ),
    )?;
    let base = path.parent().unwrap_or(Path::new("."));
    let raw = verify_input(base, &input.raw_receipt, 1024 * 1024)?;
    let review = verify_input(base, &input.review, 1024 * 1024)?;
    let bundle = bundle::verify(&raw, &review)?;
    let directory = review.parent().unwrap_or(Path::new("."));
    let [_, _, width, height] = bundle.manifest.window_cells;
    // Bundle verification bounds each side to 2048 cells before allocation/decoding.
    let cells = width as usize * height as usize;
    let scratch = io_context("stage quality samples", tempfile::tempdir())?;
    let mut masks = BTreeMap::new();
    let mut samples = BTreeMap::new();
    let mut blockers = Vec::new();
    let mut surface_cover = None;
    if input.schema_version == 1 {
        ensure(
            input.surface_cover.is_none(),
            "quality input v1 cannot declare surface cover",
        )?;
    } else {
        let cover = input.surface_cover.as_ref().ok_or_else(|| {
            crate::acquisition::AcquisitionError::Invalid(
                "quality input v2 and v3 require surface_cover provenance".into(),
            )
        })?;
        ensure(
            cover.len() == 2 && cover.contains_key("snow-ice") && cover.contains_key("vegetation"),
            "surface_cover requires exactly snow-ice and vegetation",
        )?;
        let mut portable = BTreeMap::new();
        for name in ["snow-ice", "vegetation"] {
            let item = &cover[name];
            ensure(
                !item.note.trim().is_empty(),
                format!("{name} surface-cover note is empty"),
            )?;
            if let Some(value) = &item.observed_at {
                ensure(
                    !value.trim().is_empty(),
                    format!("{name} observed_at is empty"),
                )?;
            }
            ensure(
                matches!(item.status.as_str(), "assessed" | "unavailable"),
                format!("{name} surface-cover status unsupported"),
            )?;
            ensure(
                item.status != "unavailable" || name == "vegetation",
                "snow-ice provenance cannot be unavailable",
            )?;
            ensure(
                (item.status == "assessed"
                    && input.masks[name].is_some()
                    && !item.evidence.is_empty())
                    || (item.status == "unavailable" && input.masks[name].is_none()),
                format!("{name} mask and provenance status disagree"),
            )?;
            let mut evidence = Vec::new();
            for file in &item.evidence {
                let evidence_path = verify_input(base, file, 128 * 1024 * 1024)?;
                evidence.push(json!({
                    "sha256": file.sha256,
                    "bytes": io_context("inspect surface-cover evidence", fs::metadata(evidence_path))?.len()
                }));
            }
            portable.insert(
                name.into(),
                json!({
                    "status": item.status,
                    "note": item.note,
                    "observed_at": item.observed_at,
                    "evidence": evidence
                }),
            );
        }
        surface_cover = Some(portable);
    }
    for &name in names {
        if let Some(file) = &input.masks[name] {
            let mask = verify_input(base, file, 128 * 1024 * 1024)?;
            samples.insert(
                name,
                mask_samples(
                    &mask,
                    &bundle.inspection,
                    scratch.path(),
                    name,
                    cells,
                    false,
                )?,
            );
            // Recheck mutable external inputs after GDAL has decoded them.
            verify_input(base, file, 128 * 1024 * 1024)?;
            let bytes = io_context("inspect quality mask", fs::metadata(mask))?.len();
            masks.insert(
                name.into(),
                Some(json!({"sha256": file.sha256, "bytes": bytes})),
            );
        } else {
            masks.insert(name.into(), None);
            if !(input.schema_version >= 2 && name == "vegetation") {
                blockers.push(format!(
                    "{name} mask missing; whole-crop classification unresolved"
                ));
            }
        }
    }
    if input.schema_version >= 2 && !samples.contains_key("vegetation") {
        samples.insert("vegetation", vec![0; cells]);
    }
    let counts = if names.iter().all(|name| samples.contains_key(*name)) {
        let elevation = directory.join("elevation.tif");
        let info = inspect(&elevation)?;
        ensure(
            grid_matches(&info, &bundle.inspection)
                && info["bands"]
                    .as_array()
                    .is_some_and(|b| b.len() == 1 && b[0]["type"] == "Float32"),
            "elevation grid/type mismatch",
        )?;
        ensure(
            info["bands"][0].get("scale").is_none_or(|v| v == 1)
                && info["bands"][0].get("offset").is_none_or(|v| v == 0),
            "scaled elevation is unsupported",
        )?;
        let heights = band(&elevation, scratch.path(), "elevation", cells, true)?;
        let provider = mask_samples(
            &directory.join("validity.tif"),
            &bundle.inspection,
            scratch.path(),
            "provider",
            cells,
            true,
        )?;
        let mut counts = Counts::default();
        for (i, bytes) in heights.chunks_exact(4).enumerate() {
            let finite = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]).is_finite();
            let water = samples["water"][i];
            let quality = samples["quality"][i];
            let snow_ice = if input.schema_version >= 2 {
                samples["snow-ice"][i]
            } else {
                1
            };
            // Snow/ice is a game layer over measured ground, not an exclusion.
            let ground = water == 1;
            let valid = ground && quality == 1 && provider[i] == 255 && finite;
            counts.water_cells += u64::from(water == 2);
            counts.unclassified_water_cells += u64::from(water == 0);
            counts.terrestrial_cells += u64::from(if input.schema_version == 1 {
                water == 1
            } else {
                ground
            });
            counts.valid_terrestrial_cells += u64::from(valid);
            counts.unsupported_terrestrial_cells += u64::from(ground && quality == 2);
            counts.unknown_terrestrial_quality_cells += u64::from(ground && quality == 0);
            counts.provider_invalid_terrestrial_cells += u64::from(ground && provider[i] == 0);
            counts.nonfinite_terrestrial_cells +=
                u64::from(ground && provider[i] == 255 && !finite);
            counts.repaired_cells += u64::from(samples["repairs"][i] == 1);
            counts.defining_feature_cells += u64::from(samples["features"][i] == 1);
            counts.unsupported_feature_cells += u64::from(samples["features"][i] == 1 && !valid);
            if input.schema_version >= 2 {
                *counts.ground_cells.get_or_insert(0) += u64::from(ground);
                *counts.snow_ice_cells.get_or_insert(0) += u64::from(snow_ice == 2);
                *counts.unknown_snow_ice_cells.get_or_insert(0) += u64::from(snow_ice == 0);
                *counts.vegetation_cells.get_or_insert(0) +=
                    u64::from(samples["vegetation"][i] == 2);
                *counts.unknown_vegetation_cells.get_or_insert(0) +=
                    u64::from(samples["vegetation"][i] == 0);
            }
        }
        if counts.unclassified_water_cells > 0 {
            blockers.push("water classification incomplete".into());
        }
        if input.schema_version >= 2 && counts.unknown_snow_ice_cells.unwrap_or(0) > 0 {
            blockers.push("snow/ice classification incomplete".into());
        }
        let denominator = counts.ground_cells.unwrap_or(counts.terrestrial_cells);
        if denominator == 0
            || counts.valid_terrestrial_cells * 1000
                < denominator * crate::preparation::MIN_VALID_GROUND_PERMILLE
        {
            blockers.push("at least 50% valid terrestrial coverage not established".into());
        }
        if counts.nonfinite_terrestrial_cells > 0 {
            blockers.push("nonfinite provider-valid terrestrial elevation".into());
        }
        if counts.unsupported_feature_cells > 0 {
            blockers.push("unsupported cells intersect defining features".into());
        }
        if counts.repaired_cells > 0 {
            blockers.push("repairs require before/after evidence and a reviewed repair policy; unsupported in v1".into());
        }
        Some(counts)
    } else {
        None
    };
    // A changed parent must not leave a completed report describing earlier bytes.
    bundle::verify(&raw, &review)?;
    verify_input(base, &input.raw_receipt, 1024 * 1024)?;
    verify_input(base, &input.review, 1024 * 1024)?;
    if let Some(cover) = &input.surface_cover {
        for item in cover.values() {
            for file in &item.evidence {
                verify_input(base, file, 128 * 1024 * 1024)?;
            }
        }
    }
    ensure(
        read_small(path)? == input_bytes,
        "quality input changed during assessment",
    )?;
    let report = Assessment {
        schema_version: input.schema_version,
        kind: "regional_quality_assessment",
        policy: match input.schema_version {
            1 => "native_quality_v2",
            2 => "native_quality_cover_v1",
            _ => "native_quality_cover_v2",
        },
        raw_receipt_sha256: input.raw_receipt.sha256,
        review_sha256: input.review.sha256,
        asset_sha256: bundle.manifest.asset_sha256.clone(),
        window_cells: bundle.manifest.window_cells,
        grid: json!({"size": bundle.inspection["size"], "geoTransform": bundle.inspection["geoTransform"], "coordinateSystem": bundle.inspection["coordinateSystem"]}),
        masks,
        surface_cover,
        counts,
        blockers,
        gdal_version: String::from_utf8_lossy(&run(command("gdalinfo").arg("--version"))?)
            .trim()
            .into(),
        topography_version: env!("CARGO_PKG_VERSION"),
        admission: "not admitted",
    };
    Ok((report, bundle))
}

/// Write repeatable measured evidence, including blockers. Makes no decisions.
pub fn assess(input: &Path, output: &Path) -> Result<PathBuf> {
    ensure(!output.exists(), "quality report output already exists")?;
    let (report, _) = evaluate(input)?;
    write_new(output, &serde_json::to_vec_pretty(&report)?)?;
    Ok(output.to_path_buf())
}

/// Recompute quality and verify the pinned current decision and its evidence.
/// Later preparation must rerun this check; no data is published or admitted.
pub fn check(runtime: &Path, input: &Path, record_sha256: &str) -> Result<Check> {
    ensure(
        valid_digest(record_sha256),
        "invalid curation record digest",
    )?;
    let (report, bundle) = evaluate(input)?;
    let report_sha256 = digest(&serde_json::to_vec_pretty(&report)?);
    let mut blockers = report.blockers.clone();
    let (claims, primary_class_id) = curation::preparation_check(
        runtime,
        &report.review_sha256,
        record_sha256,
        &report_sha256,
        report.counts.as_ref(),
        &bundle,
    )?;
    blockers.extend(claims);
    Ok(Check {
        eligible: blockers.is_empty(),
        schema_version: report.schema_version,
        kind: "preparation_eligibility",
        policy: report.policy,
        curation_record_sha256: record_sha256.into(),
        quality_report_sha256: report_sha256,
        review_sha256: report.review_sha256,
        blockers,
        admission: "not admitted",
        primary_class_id,
    })
}

pub(super) struct Ready {
    pub schema_version: u32,
    pub check: Check,
    pub bundle: bundle::Bundle,
    pub report: Vec<u8>,
    pub heights: Vec<u8>,
    pub provider: Vec<u8>,
    pub masks: BTreeMap<String, Vec<u8>>,
}

/// Load bounded samples only after current eligibility is established. Compare
/// both measurements again after loading to bind the working bytes to evidence.
pub(super) fn ready(runtime: &Path, path: &Path, record: &str) -> Result<Ready> {
    let input_bytes = read_small(path)?;
    let check = check(runtime, path, record)?;
    ensure(
        check.eligible,
        format!("preparation blocked: {}", check.blockers.join("; ")),
    )?;
    let (report, bundle) = evaluate(path)?;
    let report = serde_json::to_vec_pretty(&report)?;
    ensure(
        digest(&report) == check.quality_report_sha256,
        "quality assessment changed during preparation",
    )?;
    let input: Input = serde_json::from_slice(&input_bytes)?;
    let base = path.parent().unwrap_or(Path::new("."));
    let review = verify_input(base, &input.review, 1024 * 1024)?;
    let directory = review.parent().unwrap_or(Path::new("."));
    let [_, _, w, h] = bundle.manifest.window_cells;
    let cells = w as usize * h as usize;
    let scratch = io_context("stage preparation samples", tempfile::tempdir())?;
    let heights = band(
        &directory.join("elevation.tif"),
        scratch.path(),
        "elevation",
        cells,
        true,
    )?;
    let provider = band(
        &directory.join("validity.tif"),
        scratch.path(),
        "provider",
        cells,
        false,
    )?;
    let mut masks = BTreeMap::new();
    for &name in mask_names(input.schema_version) {
        if let Some(file) = input.masks[name].as_ref() {
            let mask = verify_input(base, file, 128 * 1024 * 1024)?;
            masks.insert(
                name.into(),
                band(&mask, scratch.path(), name, cells, false)?,
            );
            verify_input(base, file, 128 * 1024 * 1024)?;
        } else if input.schema_version >= 2 && name == "vegetation" {
            masks.insert(name.into(), vec![0; cells]);
        } else {
            return Err(crate::acquisition::AcquisitionError::Invalid(
                "missing preparation mask".into(),
            ));
        }
    }
    ensure(
        read_small(path)? == input_bytes,
        "quality input changed during preparation",
    )?;
    let (after, _) = evaluate(path)?;
    ensure(
        serde_json::to_vec_pretty(&after)? == report,
        "quality assessment changed during preparation",
    )?;
    Ok(Ready {
        schema_version: input.schema_version,
        check,
        bundle,
        report,
        heights,
        provider,
        masks,
    })
}
