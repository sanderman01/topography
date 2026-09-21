use crate::{
    acquisition::{
        Result, digest, ensure, hash_file, io_context,
        raster::{command, run},
        read_json, read_small, valid_digest, write_new,
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

const MASK_NAMES: [&str; 4] = ["water", "quality", "repairs", "features"];

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
    curation_record_sha256: String,
    quality_report_sha256: String,
    review_sha256: String,
    blockers: Vec<String>,
    admission: &'static str,
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
        masks: MASK_NAMES.into_iter().map(|n| (n.into(), None)).collect(),
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
        && info["geoTransform"] == reference["geoTransform"]
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
    let max = if matches!(name, "water" | "quality") {
        2
    } else {
        1
    };
    ensure(
        values.iter().all(|v| {
            if provider {
                matches!(v, 0 | 255)
            } else {
                *v <= max
            }
        }),
        format!("{name} contains an unsupported category"),
    )?;
    Ok(values)
}

fn evaluate(path: &Path) -> Result<(Assessment, bundle::Bundle)> {
    let input: Input = read_json(path)?;
    ensure(
        input.schema_version == 1,
        "only quality input version 1 is supported",
    )?;
    ensure(
        input.masks.len() == 4 && MASK_NAMES.iter().all(|n| input.masks.contains_key(*n)),
        "quality input requires exactly water, quality, repairs and features masks",
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
    for name in MASK_NAMES {
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
            blockers.push(format!(
                "{name} mask missing; whole-crop classification unresolved"
            ));
        }
    }
    let counts = if samples.len() == 4 {
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
            let valid = water == 1 && quality == 1 && provider[i] == 255 && finite;
            counts.water_cells += u64::from(water == 2);
            counts.unclassified_water_cells += u64::from(water == 0);
            counts.terrestrial_cells += u64::from(water == 1);
            counts.valid_terrestrial_cells += u64::from(valid);
            counts.unsupported_terrestrial_cells += u64::from(water == 1 && quality == 2);
            counts.unknown_terrestrial_quality_cells += u64::from(water == 1 && quality == 0);
            counts.provider_invalid_terrestrial_cells += u64::from(water == 1 && provider[i] == 0);
            counts.nonfinite_terrestrial_cells +=
                u64::from(water == 1 && provider[i] == 255 && !finite);
            counts.repaired_cells += u64::from(samples["repairs"][i] == 1);
            counts.defining_feature_cells += u64::from(samples["features"][i] == 1);
            counts.unsupported_feature_cells += u64::from(samples["features"][i] == 1 && !valid);
        }
        if counts.unclassified_water_cells > 0 {
            blockers.push("water classification incomplete".into());
        }
        if counts.terrestrial_cells == 0
            || counts.valid_terrestrial_cells * 1000 < counts.terrestrial_cells * 995
        {
            blockers.push("at least 99.5% valid terrestrial coverage not established".into());
        }
        if counts.nonfinite_terrestrial_cells > 0 {
            blockers.push("nonfinite provider-valid terrestrial elevation".into());
        }
        if counts.defining_feature_cells == 0 {
            blockers.push("defining features have not been marked".into());
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
    let report = Assessment {
        schema_version: 1,
        kind: "regional_quality_assessment",
        policy: "native_quality_v1",
        raw_receipt_sha256: input.raw_receipt.sha256,
        review_sha256: input.review.sha256,
        asset_sha256: bundle.manifest.asset_sha256.clone(),
        window_cells: bundle.manifest.window_cells,
        grid: json!({"size": bundle.inspection["size"], "geoTransform": bundle.inspection["geoTransform"], "coordinateSystem": bundle.inspection["coordinateSystem"]}),
        masks,
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
    blockers.extend(curation::preparation_check(
        runtime,
        &report.review_sha256,
        record_sha256,
        &report_sha256,
        report.counts.as_ref(),
        &bundle,
    )?);
    Ok(Check {
        eligible: blockers.is_empty(),
        schema_version: 1,
        kind: "preparation_eligibility",
        policy: report.policy,
        curation_record_sha256: record_sha256.into(),
        quality_report_sha256: report_sha256,
        review_sha256: report.review_sha256,
        blockers,
        admission: "not admitted",
    })
}
