//! Profile-driven, content-addressed preparation of one reviewed crop.

use crate::{
    acquisition::{
        AcquisitionError, Result, digest, ensure, hash_file, io_context,
        raster::{command, run},
        read_json, read_small, valid_digest, write_new,
    },
    catalog::curation,
    preparation::{self, region},
    review::bundle,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const FINDINGS: [&str; 6] = [
    "coordinates",
    "ground_support",
    "artifacts_and_repairs",
    "feature_fidelity",
    "water_classification",
    "unsupported_feature_gaps",
];

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Evidence {
    pub(crate) id: String,
    pub(crate) path: PathBuf,
    pub(crate) sha256: String,
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) source_url: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Finding {
    pub(crate) note: String,
    pub(crate) evidence_ids: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Water {
    AllTerrestrial { windows: Vec<[u32; 4]> },
    Mask { path: PathBuf, sha256: String },
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProfileV1 {
    schema_version: u32,
    policy: String,
    source_sha256: String,
    asset_sha256: String,
    evidence: Vec<Evidence>,
    permission_evidence_ids: Vec<String>,
    findings: BTreeMap<String, Finding>,
    water: Water,
    #[serde(default)]
    blockers: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Cover {
    AllAbsent {
        windows: Vec<[u32; 4]>,
        note: String,
        observed_at: Option<String>,
        evidence_ids: Vec<String>,
    },
    Mask {
        path: PathBuf,
        sha256: String,
        note: String,
        observed_at: Option<String>,
        evidence_ids: Vec<String>,
    },
    Unavailable {
        note: String,
        observed_at: Option<String>,
        evidence_ids: Vec<String>,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProfileV2 {
    schema_version: u32,
    policy: String,
    source_sha256: String,
    asset_sha256: String,
    evidence: Vec<Evidence>,
    permission_evidence_ids: Vec<String>,
    findings: BTreeMap<String, Finding>,
    water: Water,
    snow_ice: Cover,
    vegetation: Cover,
    #[serde(default)]
    blockers: Vec<String>,
}

struct ProfileData {
    schema_version: u32,
    policy: String,
    source_sha256: String,
    asset_sha256: String,
    evidence: Vec<Evidence>,
    permission_evidence_ids: Vec<String>,
    findings: BTreeMap<String, Finding>,
    water: Water,
    covers: Option<BTreeMap<String, Cover>>,
    blockers: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PackageCover {
    status: String,
    note: String,
    observed_at: Option<String>,
    evidence_ids: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FileRef {
    path: PathBuf,
    sha256: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Package {
    schema_version: u32,
    kind: String,
    policy: String,
    raw_receipt: FileRef,
    review: FileRef,
    pub(crate) profile_sha256: String,
    source_sha256: String,
    asset_sha256: String,
    pub(crate) evidence: Vec<Evidence>,
    pub(crate) permission_evidence_ids: Vec<String>,
    pub(crate) findings: BTreeMap<String, Finding>,
    pub(crate) source_license: Value,
    masks: BTreeMap<String, FileRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    surface_cover: Option<BTreeMap<String, PackageCover>>,
    grid: Value,
    window_cells: [u32; 4],
    admission: String,
}

impl Package {
    pub(crate) fn parents(&self) -> (&Path, &Path) {
        (&self.raw_receipt.path, &self.review.path)
    }

    pub(crate) fn schema_version(&self) -> u32 {
        self.schema_version
    }
}

fn verified_profile(path: &Path) -> Result<(ProfileData, Vec<u8>)> {
    let bytes = read_small(path)?;
    let profile = decode_profile(&bytes)?;
    validate_profile_contract(&profile)?;
    ensure(
        profile.blockers.is_empty(),
        format!(
            "profile has technical blockers: {}",
            profile.blockers.join("; ")
        ),
    )?;
    let base = path.parent().unwrap_or(Path::new("."));
    for item in &profile.evidence {
        ensure(
            hash_file(&base.join(&item.path))?.0 == item.sha256,
            "profile evidence digest mismatch",
        )?;
    }
    Ok((profile, bytes))
}

/// Check a profile's contract and pinned files without requiring it to be blocker-free.
pub(crate) fn check_profile(path: &Path) -> Result<u32> {
    let profile = decode_profile(&read_small(path)?)?;
    validate_profile_contract(&profile)?;
    let base = path.parent().unwrap_or(Path::new("."));
    for item in &profile.evidence {
        ensure(
            hash_file(&base.join(&item.path))?.0 == item.sha256,
            "profile evidence digest mismatch",
        )?;
    }
    let mut masks = Vec::new();
    if let Water::Mask { path, sha256 } = &profile.water {
        masks.push((path, sha256));
    }
    for cover in profile.covers.iter().flat_map(BTreeMap::values) {
        if let Cover::Mask { path, sha256, .. } = cover {
            masks.push((path, sha256));
        }
    }
    for (path, sha256) in masks {
        ensure(
            hash_file(&base.join(path))?.0 == *sha256,
            "profile mask digest mismatch",
        )?;
    }
    Ok(profile.schema_version)
}

fn decode_profile(bytes: &[u8]) -> Result<ProfileData> {
    let value: Value = serde_json::from_slice(bytes)?;
    let profile = match value["schema_version"].as_u64() {
        Some(1) => {
            let value: ProfileV1 = serde_json::from_value(value)?;
            ProfileData {
                schema_version: value.schema_version,
                policy: value.policy,
                source_sha256: value.source_sha256,
                asset_sha256: value.asset_sha256,
                evidence: value.evidence,
                permission_evidence_ids: value.permission_evidence_ids,
                findings: value.findings,
                water: value.water,
                covers: None,
                blockers: value.blockers,
            }
        }
        Some(2) => {
            let value: ProfileV2 = serde_json::from_value(value)?;
            ProfileData {
                schema_version: value.schema_version,
                policy: value.policy,
                source_sha256: value.source_sha256,
                asset_sha256: value.asset_sha256,
                evidence: value.evidence,
                permission_evidence_ids: value.permission_evidence_ids,
                findings: value.findings,
                water: value.water,
                covers: Some(BTreeMap::from([
                    ("snow-ice".into(), value.snow_ice),
                    ("vegetation".into(), value.vegetation),
                ])),
                blockers: value.blockers,
            }
        }
        _ => {
            return Err(AcquisitionError::Invalid(
                "unsupported automatic preparation profile".into(),
            ));
        }
    };
    Ok(profile)
}

fn validate_profile_contract(profile: &ProfileData) -> Result<()> {
    ensure(
        (profile.schema_version == 1 && profile.policy == "supplier_qa_pilot_v1")
            || (profile.schema_version == 2 && profile.policy == "supplier_qa_cover_v1"),
        "unsupported automatic preparation profile",
    )?;
    ensure(
        valid_digest(&profile.source_sha256) && valid_digest(&profile.asset_sha256),
        "invalid profile source identity",
    )?;
    ensure(
        profile.evidence.len() <= 15 && !profile.evidence.is_empty(),
        "profile requires 1..=15 evidence items",
    )?;
    let mut ids = BTreeSet::new();
    for item in &profile.evidence {
        ensure(
            !item.id.is_empty()
                && item.id.len() <= 64
                && item
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                && ids.insert(item.id.as_str()),
            "invalid or duplicate profile evidence ID",
        )?;
        ensure(
            safe_relative(&item.path),
            "profile evidence path must be relative and contained",
        )?;
        ensure(
            valid_digest(&item.sha256)
                && !item.description.trim().is_empty()
                && item
                    .source_url
                    .as_deref()
                    .is_none_or(|url| url.starts_with("https://")),
            "invalid profile evidence metadata",
        )?;
    }
    let refs = |values: &[String]| values.iter().all(|id| ids.contains(id.as_str()));
    ensure(
        !profile.permission_evidence_ids.is_empty() && refs(&profile.permission_evidence_ids),
        "permission requires captured profile evidence",
    )?;
    ensure(
        profile.findings.len() == FINDINGS.len()
            && FINDINGS.iter().all(|n| profile.findings.contains_key(*n)),
        "profile requires exactly six technical findings",
    )?;
    for finding in profile.findings.values() {
        ensure(
            !finding.note.trim().is_empty()
                && !finding.evidence_ids.is_empty()
                && refs(&finding.evidence_ids),
            "profile finding requires a note and captured evidence",
        )?;
    }
    if let Water::Mask { path, sha256 } = &profile.water {
        ensure(
            safe_relative(path) && valid_digest(sha256),
            "profile water path must be relative and contained",
        )?;
    }
    match (&profile.covers, profile.schema_version) {
        (None, 1) => {}
        (Some(covers), 2)
            if covers.len() == 2
                && covers.contains_key("snow-ice")
                && covers.contains_key("vegetation") =>
        {
            for (name, cover) in covers {
                validate_cover(name, cover, &ids)?;
            }
        }
        _ => {
            return Err(AcquisitionError::Invalid(
                "profile cover contract/version mismatch".into(),
            ));
        }
    }
    Ok(())
}

fn validate_cover(name: &str, cover: &Cover, evidence: &BTreeSet<&str>) -> Result<()> {
    let (note, observed_at, evidence_ids) = match cover {
        Cover::AllAbsent {
            windows,
            note,
            observed_at,
            evidence_ids,
        } => {
            ensure(
                !windows.is_empty(),
                format!("{name} all-absent assessment needs windows"),
            )?;
            (note, observed_at, evidence_ids)
        }
        Cover::Mask {
            path,
            sha256,
            note,
            observed_at,
            evidence_ids,
        } => {
            ensure(
                safe_relative(path) && valid_digest(sha256),
                format!("invalid {name} mask reference"),
            )?;
            (note, observed_at, evidence_ids)
        }
        Cover::Unavailable {
            note,
            observed_at,
            evidence_ids,
        } => {
            ensure(
                name == "vegetation",
                "snow/ice cannot be unavailable in a v2 profile",
            )?;
            (note, observed_at, evidence_ids)
        }
    };
    ensure(
        !note.trim().is_empty(),
        format!("{name} assessment note must not be empty"),
    )?;
    if let Some(value) = observed_at {
        ensure(
            !value.trim().is_empty(),
            format!("{name} observed_at must be nonempty or null"),
        )?;
    }
    ensure(
        evidence_ids.iter().all(|id| evidence.contains(id.as_str()))
            && (matches!(cover, Cover::Unavailable { .. }) || !evidence_ids.is_empty()),
        format!("{name} assessment requires known evidence IDs"),
    )
}

fn safe_relative(path: &Path) -> bool {
    path.components().next().is_some()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

fn validate_profile_scope(
    profile: &ProfileData,
    raw: &Path,
    receipt: &crate::acquisition::RawReceipt,
    bundle: &bundle::Bundle,
) -> Result<()> {
    let objects = raw
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| AcquisitionError::Invalid("raw receipt needs storage root".into()))?
        .join("objects");
    let original: Value = read_json(&objects.join(&receipt.inspection_sha256))?;
    let source_width = original["size"][0].as_u64().unwrap_or(0);
    let source_height = original["size"][1].as_u64().unwrap_or(0);
    let [x, y, width, height] = bundle.manifest.window_cells;
    let scoped = |windows: &[[u32; 4]], name: &str| -> Result<()> {
        ensure(
            !windows.is_empty()
                && windows.iter().all(|[wx, wy, ww, wh]| {
                    *ww > 0
                        && *wh > 0
                        && u64::from(*wx) + u64::from(*ww) <= source_width
                        && u64::from(*wy) + u64::from(*wh) <= source_height
                }),
            format!("{name} windows must be nonempty and inside the source grid"),
        )?;
        ensure(
            windows.iter().any(|[wx, wy, ww, wh]| {
                x >= *wx
                    && y >= *wy
                    && u64::from(x) + u64::from(width) <= u64::from(*wx) + u64::from(*ww)
                    && u64::from(y) + u64::from(height) <= u64::from(*wy) + u64::from(*wh)
            }),
            format!("review lies outside assessed {name} windows"),
        )
    };
    if let Water::AllTerrestrial { windows } = &profile.water {
        scoped(windows, "all-terrestrial")?;
    }
    if let Some(covers) = &profile.covers {
        for (name, cover) in covers {
            if let Cover::AllAbsent { windows, .. } = cover {
                scoped(windows, name)?;
            }
        }
    }
    Ok(())
}

fn vrt_mask(path: &Path, width: u32, height: u32, value: u8) -> Result<()> {
    let bin = path.with_extension("bin");
    write_new(&bin, &vec![value; width as usize * height as usize])?;
    let name = bin
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| AcquisitionError::Invalid("mask path must be UTF-8".into()))?;
    let body = format!(
        "<VRTDataset rasterXSize=\"{width}\" rasterYSize=\"{height}\"><VRTRasterBand dataType=\"Byte\" band=\"1\" subClass=\"VRTRawRasterBand\"><SourceFilename relativeToVRT=\"1\">{name}</SourceFilename><ImageOffset>0</ImageOffset><PixelOffset>1</PixelOffset><LineOffset>{width}</LineOffset><ByteOrder>LSB</ByteOrder></VRTRasterBand></VRTDataset>"
    );
    let vrt = path.with_extension("vrt");
    write_new(&vrt, body.as_bytes())?;
    run(command("gdal_translate")
        .args([
            "-q",
            "-of",
            "GTiff",
            "-mask",
            "none",
            "-a_nodata",
            "none",
            "-co",
            "COMPRESS=DEFLATE",
        ])
        .arg(&vrt)
        .arg(path))?;
    Ok(())
}

fn add_georeference(path: &Path, reference: &Value) -> Result<()> {
    let transform: [f64; 6] = serde_json::from_value(reference["geoTransform"].clone())?;
    let wkt = reference["coordinateSystem"]["wkt"]
        .as_str()
        .ok_or_else(|| AcquisitionError::Invalid("review CRS WKT missing".into()))?;
    let temp = path.with_extension("geo.tif");
    run(command("gdal_translate")
        .args([
            "-q",
            "-of",
            "GTiff",
            "-a_srs",
            wkt,
            "-a_ullr",
            &transform[0].to_string(),
            &transform[3].to_string(),
            &(transform[0] + transform[1] * reference["size"][0].as_f64().unwrap_or(0.0))
                .to_string(),
            &(transform[3] + transform[5] * reference["size"][1].as_f64().unwrap_or(0.0))
                .to_string(),
            "-co",
            "COMPRESS=DEFLATE",
        ])
        .arg(path)
        .arg(&temp))?;
    io_context("replace georeferenced mask", fs::rename(&temp, path))
}

fn cover_metadata(cover: &Cover) -> PackageCover {
    let (status, note, observed_at, evidence_ids) = match cover {
        Cover::AllAbsent {
            note,
            observed_at,
            evidence_ids,
            ..
        }
        | Cover::Mask {
            note,
            observed_at,
            evidence_ids,
            ..
        } => ("assessed", note, observed_at, evidence_ids),
        Cover::Unavailable {
            note,
            observed_at,
            evidence_ids,
        } => ("unavailable", note, observed_at, evidence_ids),
    };
    PackageCover {
        status: status.into(),
        note: note.clone(),
        observed_at: observed_at.clone(),
        evidence_ids: evidence_ids.clone(),
    }
}

fn surface_cover_input(
    covers: &BTreeMap<String, PackageCover>,
    evidence: &[Evidence],
    base: &Path,
) -> Result<Value> {
    let by_id: BTreeMap<_, _> = evidence
        .iter()
        .map(|item| (item.id.as_str(), item))
        .collect();
    let mut out = serde_json::Map::new();
    for (name, cover) in covers {
        let mut pinned = Vec::new();
        for id in &cover.evidence_ids {
            let item = by_id.get(id.as_str()).ok_or_else(|| {
                AcquisitionError::Invalid(format!("{name} references missing packaged evidence"))
            })?;
            pinned.push(json!({"path":base.join(&item.path),"sha256":item.sha256}));
        }
        out.insert(name.clone(), json!({"status":cover.status,"note":cover.note,"observed_at":cover.observed_at,"evidence":pinned}));
    }
    Ok(Value::Object(out))
}

fn quality_masks(
    masks: &BTreeMap<String, FileRef>,
    covers: Option<&BTreeMap<String, PackageCover>>,
    base: &Path,
) -> BTreeMap<String, Value> {
    masks
        .iter()
        .map(|(name, file)| {
            let unavailable = name == "vegetation"
                && covers
                    .and_then(|values| values.get(name))
                    .is_some_and(|cover| cover.status == "unavailable");
            (
                name.clone(),
                if unavailable {
                    Value::Null
                } else {
                    json!({"path":base.join(&file.path),"sha256":file.sha256})
                },
            )
        })
        .collect()
}

fn crop_cover_mask(
    name: &str,
    cover: &Cover,
    profile_base: &Path,
    output: &Path,
    bundle: &bundle::Bundle,
    raw: &Path,
    receipt: &crate::acquisition::RawReceipt,
) -> Result<()> {
    let [x, y, width, height] = bundle.manifest.window_cells;
    match cover {
        Cover::AllAbsent { .. } => {
            vrt_mask(output, width, height, 1)?;
            add_georeference(output, &bundle.inspection)
        }
        Cover::Unavailable { .. } => {
            vrt_mask(output, width, height, 0)?;
            add_georeference(output, &bundle.inspection)
        }
        Cover::Mask { path, sha256, .. } => {
            let source = profile_base.join(path);
            ensure(
                hash_file(&source)?.0 == *sha256,
                format!("{name} mask digest mismatch"),
            )?;
            verify_source_cover(&source, raw, receipt, name)?;
            run(command("gdal_translate")
                .args([
                    "-q",
                    "-of",
                    "GTiff",
                    "-srcwin",
                    &x.to_string(),
                    &y.to_string(),
                    &width.to_string(),
                    &height.to_string(),
                    "-mask",
                    "none",
                    "-a_nodata",
                    "none",
                    "-co",
                    "COMPRESS=DEFLATE",
                ])
                .arg(&source)
                .arg(output))?;
            ensure(
                hash_file(&source)?.0 == *sha256,
                format!("{name} mask changed during crop"),
            )?;
            Ok(())
        }
    }
}

/// Validate a source assessment and publish immutable preflight masks. No curation is written.
pub fn stage(raw: &Path, review: &Path, profile_path: &Path, output: &Path) -> Result<PathBuf> {
    let bundle = bundle::verify(raw, review)?;
    let receipt = crate::acquisition::verify(raw)?;
    let (profile, profile_bytes) = verified_profile(profile_path)?;
    ensure(
        receipt.source_sha256 == profile.source_sha256
            && receipt.asset_sha256 == profile.asset_sha256
            && bundle.manifest.asset_sha256 == profile.asset_sha256,
        "profile source or asset identity mismatch",
    )?;
    validate_profile_scope(&profile, raw, &receipt, &bundle)?;
    let [x, y, width, height] = bundle.manifest.window_cells;
    ensure(
        package_permission_confirmed(&bundle.source.license)?,
        "source does not declare confirmed training permission",
    )?;
    io_context("create automatic package root", fs::create_dir_all(output))?;
    let temp = io_context("stage automatic package", tempfile::tempdir_in(output))?;
    let mut evidence = profile.evidence.clone();
    for item in &mut evidence {
        let source = profile_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(&item.path);
        item.path = PathBuf::from(format!("evidence-{}", item.id));
        write_new(&temp.path().join(&item.path), &read_small(&source)?)?;
    }
    let mut masks = BTreeMap::new();
    for (name, value) in [("quality", 1), ("repairs", 0), ("features", 0)] {
        let path = temp.path().join(format!("{name}.tif"));
        vrt_mask(&path, width, height, value)?;
        add_georeference(&path, &bundle.inspection)?;
        masks.insert(
            name.into(),
            FileRef {
                path: PathBuf::from(format!("{name}.tif")),
                sha256: hash_file(&path)?.0,
            },
        );
    }
    let water_path = temp.path().join("water.tif");
    match &profile.water {
        Water::AllTerrestrial { .. } => {
            vrt_mask(&water_path, width, height, 1)?;
            add_georeference(&water_path, &bundle.inspection)?;
        }
        Water::Mask { path, sha256 } => {
            let source = profile_path.parent().unwrap_or(Path::new(".")).join(path);
            ensure(
                valid_digest(sha256) && hash_file(&source)?.0 == *sha256,
                "water mask digest mismatch",
            )?;
            verify_source_cover(&source, raw, &receipt, "water")?;
            run(command("gdal_translate")
                .args([
                    "-q",
                    "-of",
                    "GTiff",
                    "-srcwin",
                    &x.to_string(),
                    &y.to_string(),
                    &width.to_string(),
                    &height.to_string(),
                    "-mask",
                    "none",
                    "-a_nodata",
                    "none",
                    "-co",
                    "COMPRESS=DEFLATE",
                ])
                .arg(&source)
                .arg(&water_path))?;
            ensure(
                hash_file(&source)?.0 == *sha256,
                "water mask changed during crop",
            )?;
        }
    }
    masks.insert(
        "water".into(),
        FileRef {
            path: PathBuf::from("water.tif"),
            sha256: hash_file(&water_path)?.0,
        },
    );
    let surface_cover = if let Some(covers) = &profile.covers {
        let mut metadata = BTreeMap::new();
        for (name, cover) in covers {
            let path = temp.path().join(format!("{name}.tif"));
            crop_cover_mask(
                name,
                cover,
                profile_path.parent().unwrap_or(Path::new(".")),
                &path,
                &bundle,
                raw,
                &receipt,
            )?;
            masks.insert(
                name.clone(),
                FileRef {
                    path: PathBuf::from(format!("{name}.tif")),
                    sha256: hash_file(&path)?.0,
                },
            );
            metadata.insert(name.clone(), cover_metadata(cover));
        }
        Some(metadata)
    } else {
        None
    };
    preflight_source(&bundle)?;
    let quality_input = temp.path().join("preflight-input.json");
    let mut quality_value = json!({
        "schema_version":profile.schema_version,
        "raw_receipt":{"path":fs::canonicalize(raw).map_err(|e| AcquisitionError::Io { context:"resolve raw receipt".into(), source:e })?,"sha256":digest(&read_small(raw)?)},
        "review":{"path":fs::canonicalize(review).map_err(|e| AcquisitionError::Io { context:"resolve review".into(), source:e })?,"sha256":digest(&read_small(review)?)},
        "masks":quality_masks(&masks, surface_cover.as_ref(), temp.path())
    });
    if let Some(covers) = &surface_cover {
        quality_value["surface_cover"] = surface_cover_input(covers, &evidence, temp.path())?;
    }
    write_new(&quality_input, &serde_json::to_vec_pretty(&quality_value)?)?;
    let preflight = temp.path().join("preflight-report.json");
    preparation::assess(&quality_input, &preflight)?;
    let assessment: Value = read_json(&preflight)?;
    ensure(
        assessment["blockers"].as_array().is_some_and(Vec::is_empty),
        format!("automatic preflight blocked: {}", assessment["blockers"]),
    )?;
    for path in [quality_input, preflight] {
        io_context("remove transient preflight metadata", fs::remove_file(path))?;
    }
    for name in [
        "quality",
        "repairs",
        "features",
        "water",
        "snow-ice",
        "vegetation",
    ] {
        for extension in ["bin", "vrt"] {
            let path = temp.path().join(format!("{name}.{extension}"));
            if path.exists() {
                io_context("remove transient mask source", fs::remove_file(path))?;
            }
        }
    }
    write_new(&temp.path().join("profile.json"), &profile_bytes)?;
    let transform: [f64; 6] = serde_json::from_value(bundle.inspection["geoTransform"].clone())?;
    let package = Package {
        schema_version: profile.schema_version,
        kind: "automatic_preparation_package".into(),
        policy: profile.policy,
        raw_receipt: FileRef {
            path: fs::canonicalize(raw).map_err(|e| AcquisitionError::Io {
                context: "resolve raw receipt".into(),
                source: e,
            })?,
            sha256: digest(&read_small(raw)?),
        },
        review: FileRef {
            path: fs::canonicalize(review).map_err(|e| AcquisitionError::Io {
                context: "resolve review".into(),
                source: e,
            })?,
            sha256: digest(&read_small(review)?),
        },
        profile_sha256: digest(&profile_bytes),
        source_sha256: profile.source_sha256,
        asset_sha256: profile.asset_sha256,
        evidence,
        permission_evidence_ids: profile.permission_evidence_ids,
        findings: profile.findings,
        source_license: serde_json::to_value(&bundle.source.license)?,
        masks,
        surface_cover,
        grid: json!({"origin":[transform[0],transform[3]],"size":[width,height],"spacing_m":[transform[1],-transform[5]]}),
        window_cells: bundle.manifest.window_cells,
        admission: "not admitted".into(),
    };
    let manifest_bytes = serde_json::to_vec_pretty(&package)?;
    write_new(&temp.path().join("manifest.json"), &manifest_bytes)?;
    let dir = output.join(digest(&manifest_bytes));
    if dir.exists() {
        verify_package(&dir.join("manifest.json"))?;
    } else {
        io_context("publish automatic package", fs::rename(temp.path(), &dir))?;
    }
    let manifest = dir.join("manifest.json");
    verify_package(&manifest)?;
    Ok(manifest)
}

fn verify_package(path: &Path) -> Result<Package> {
    let package: Package = read_json(path)?;
    ensure(
        matches!(package.schema_version, 1 | 2)
            && package.kind == "automatic_preparation_package"
            && ((package.schema_version == 1 && package.policy == "supplier_qa_pilot_v1")
                || (package.schema_version == 2 && package.policy == "supplier_qa_cover_v1"))
            && package.admission == "not admitted",
        "unsupported automatic preparation package",
    )?;
    let base = path.parent().unwrap_or(Path::new("."));
    let profile_bytes = read_small(&base.join("profile.json"))?;
    let profile = decode_profile(&profile_bytes)?;
    validate_profile_contract(&profile)?;
    ensure(
        profile.blockers.is_empty(),
        format!(
            "profile has technical blockers: {}",
            profile.blockers.join("; ")
        ),
    )?;
    ensure(
        digest(&read_small(&package.raw_receipt.path)?) == package.raw_receipt.sha256
            && digest(&read_small(&package.review.path)?) == package.review.sha256,
        "automatic package parent changed",
    )?;
    ensure(
        path.parent()
            .and_then(Path::file_name)
            .and_then(|v| v.to_str())
            == Some(digest(&read_small(path)?).as_str()),
        "automatic package manifest identity mismatch",
    )?;
    ensure(
        digest(&profile_bytes) == package.profile_sha256,
        "automatic package profile integrity mismatch",
    )?;
    let verified = bundle::verify(&package.raw_receipt.path, &package.review.path)?;
    let receipt = crate::acquisition::verify(&package.raw_receipt.path)?;
    validate_profile_scope(&profile, &package.raw_receipt.path, &receipt, &verified)?;
    preflight_source(&verified)?;
    let transform: [f64; 6] = serde_json::from_value(verified.inspection["geoTransform"].clone())?;
    let expected_grid = json!({
        "origin":[transform[0],transform[3]],
        "size":[verified.manifest.window_cells[2],verified.manifest.window_cells[3]],
        "spacing_m":[transform[1],-transform[5]],
    });
    ensure(
        profile.schema_version == package.schema_version
            && profile.blockers.is_empty()
            && package.policy == profile.policy
            && package.source_sha256 == profile.source_sha256
            && package.asset_sha256 == profile.asset_sha256
            && receipt.source_sha256 == profile.source_sha256
            && receipt.asset_sha256 == profile.asset_sha256
            && verified.manifest.asset_sha256 == profile.asset_sha256
            && package.window_cells == verified.manifest.window_cells
            && package.grid == expected_grid
            && package.permission_evidence_ids == profile.permission_evidence_ids
            && serde_json::to_value(&package.findings)? == serde_json::to_value(&profile.findings)?
            && package.source_license == serde_json::to_value(&verified.source.license)?,
        "automatic package claims differ from its captured parents",
    )?;
    ensure(
        package_permission_confirmed(&verified.source.license)?,
        "source does not declare confirmed training permission",
    )?;
    ensure(
        package.evidence.len() == profile.evidence.len()
            && package
                .evidence
                .iter()
                .zip(&profile.evidence)
                .all(|(packaged, source)| {
                    packaged.id == source.id
                        && packaged.sha256 == source.sha256
                        && packaged.description == source.description
                        && packaged.source_url == source.source_url
                        && packaged.path == Path::new(&format!("evidence-{}", packaged.id))
                }),
        "automatic package evidence differs from its captured profile",
    )?;
    let expected_cover = profile.covers.as_ref().map(|covers| {
        covers
            .iter()
            .map(|(name, cover)| (name.clone(), cover_metadata(cover)))
            .collect::<BTreeMap<_, _>>()
    });
    ensure(
        serde_json::to_value(&package.surface_cover)? == serde_json::to_value(&expected_cover)?,
        "automatic package surface-cover metadata differs from its captured profile",
    )?;
    ensure(
        package.masks.len() == if package.schema_version == 1 { 4 } else { 6 },
        "automatic package masks incomplete",
    )?;
    let required = if package.schema_version == 1 {
        vec!["water", "quality", "repairs", "features"]
    } else {
        vec![
            "water",
            "quality",
            "repairs",
            "features",
            "snow-ice",
            "vegetation",
        ]
    };
    for name in required {
        let f = package.masks.get(name).ok_or_else(|| {
            AcquisitionError::Invalid(format!("automatic package missing {name} mask"))
        })?;
        ensure(
            f.path == Path::new(&format!("{name}.tif"))
                && hash_file(&base.join(&f.path))?.0 == f.sha256,
            "automatic package mask integrity mismatch",
        )?;
    }
    for item in &package.evidence {
        ensure(
            hash_file(&base.join(&item.path))?.0 == item.sha256,
            "automatic package evidence integrity mismatch",
        )?;
    }
    Ok(package)
}

/// Qualify the current Yes and build/register its one-crop prepared product.
pub fn prepare(
    runtime: &Path,
    package_path: &Path,
    legend: &Path,
    expected_head: &str,
) -> Result<PathBuf> {
    ensure(
        valid_digest(expected_head),
        "invalid expected curation head",
    )?;
    let package = verify_package(package_path)?;
    let base = package_path.parent().unwrap_or(Path::new("."));
    let work = base.join("work");
    io_context("create automatic work directory", fs::create_dir_all(&work))?;
    let quality_input = work.join("quality-input.json");
    let mut input = json!({"schema_version":package.schema_version,"raw_receipt":{"path":package.raw_receipt.path,"sha256":package.raw_receipt.sha256},"review":{"path":package.review.path,"sha256":package.review.sha256},"masks":quality_masks(&package.masks, package.surface_cover.as_ref(), base)});
    if let Some(covers) = &package.surface_cover {
        input["surface_cover"] = surface_cover_input(covers, &package.evidence, base)?;
    }
    write_new(&quality_input, &serde_json::to_vec_pretty(&input)?)?;
    let report = work.join("quality-report.json");
    let report_stage = io_context(
        "stage repeated quality assessment",
        tempfile::tempdir_in(&work),
    )?;
    let recomputed = report_stage.path().join("report.json");
    preparation::assess(&quality_input, &recomputed)?;
    let recomputed_bytes = read_small(&recomputed)?;
    write_new(&report, &recomputed_bytes)?;
    let report_value: Value = read_json(&report)?;
    let report_bytes = read_small(&report)?;
    let qualified = curation::qualify_automatic(
        runtime,
        expected_head,
        &package,
        base,
        &report_bytes,
        &report_value,
        &digest(&read_small(legend)?),
    )?;
    let request = work.join(format!("request-{qualified}.json"));
    write_new(
        &request,
        &serde_json::to_vec_pretty(
            &json!({"schema_version":package.schema_version,"inputs":[{"quality_path":quality_input,"curation_record_sha256":qualified}],"grid":package.grid,"masked_fill_m":0.0}),
        )?,
    )?;
    region::build(runtime, &request)
}

fn package_permission_confirmed(license: &crate::catalog::source::License) -> Result<bool> {
    Ok(serde_json::to_value(license)?["training_permission"] == "confirmed")
}

pub(crate) fn verify_source_cover(
    path: &Path,
    raw: &Path,
    receipt: &crate::acquisition::RawReceipt,
    name: &str,
) -> Result<()> {
    ensure(
        fs::metadata(path)
            .map_err(|e| AcquisitionError::Io {
                context: "inspect source water mask".into(),
                source: e,
            })?
            .len()
            <= 128 * 1024 * 1024,
        format!("source {name} mask exceeds 128 MiB"),
    )?;
    let info: Value = serde_json::from_slice(&run(command("gdalinfo")
        .args(["-json", "-noct", "-norat", "-nogcp", "-if", "GTiff"])
        .arg(path))?)?;
    let objects = raw
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| AcquisitionError::Invalid("raw receipt needs storage root".into()))?
        .join("objects");
    let source: Value = read_json(&objects.join(&receipt.inspection_sha256))?;
    ensure(
        info["driverShortName"] == "GTiff"
            && info["size"] == source["size"]
            && super::same_transform(&info["geoTransform"], &source["geoTransform"])
            && info["coordinateSystem"] == source["coordinateSystem"],
        format!("source {name} mask must match the exact source grid and CRS"),
    )?;
    ensure(
        info["bands"].as_array().is_some_and(|b| {
            b.len() == 1
                && b[0]["type"] == "Byte"
                && b[0].get("noDataValue").is_none()
                && (b[0].get("mask").is_none() || b[0]["mask"]["flags"] == json!(["ALL_VALID"]))
                && b[0].get("scale").is_none_or(|v| v == 1)
                && b[0].get("offset").is_none_or(|v| v == 0)
        }),
        format!("source {name} mask must be one complete Byte band"),
    )?;
    let [w, h]: [u32; 2] = serde_json::from_value(info["size"].clone())?;
    let scratch = io_context("inspect source water values", tempfile::tempdir())?;
    let values = crate::review::band(
        path,
        scratch.path(),
        &format!("source-{name}"),
        w as usize * h as usize,
        false,
    )?;
    ensure(
        values.iter().all(|v| *v <= 2),
        format!("source {name} mask contains an unsupported category"),
    )
}

fn preflight_source(bundle: &bundle::Bundle) -> Result<()> {
    let source = &bundle.source;
    ensure(
        !source.synthetic,
        "synthetic source excluded from automatic preparation",
    )?;
    ensure(
        matches!(
            source.spatial.surface_type,
            crate::catalog::source::SurfaceType::BareEarthDem
        ),
        "bare-earth DEM surface type required",
    )?;
    ensure(
        source.survey_id.is_some()
            && !source.acquisition_dates.is_empty()
            && source.spatial.vertical_datum.is_some()
            && source.spatial.horizontal_crs.is_some()
            && !source.processing_history.is_empty(),
        "survey, dates, CRS, vertical datum or processing history unresolved",
    )?;
    let transform: [f64; 6] = serde_json::from_value(bundle.inspection["geoTransform"].clone())?;
    ensure(
        crate::review::metric_grid(&bundle.inspection, source, &transform)
            && (0.5..=1.0).contains(&transform[1])
            && (0.5..=1.0).contains(&-transform[5])
            && source.spatial.native_spacing_m == Some([transform[1], -transform[5]]),
        "native 0.5-1 m declared metric grid required",
    )?;
    Ok(())
}

/// Return a fully verified registered product whose current parents include this review.
pub fn existing(runtime: &Path, review: &str) -> Result<Option<PathBuf>> {
    ensure(valid_digest(review), "invalid review digest")?;
    for path in curation::prepared_for_review(runtime, review)? {
        if preparation::verify(runtime, &path)?.0 {
            return Ok(Some(path));
        }
    }
    Ok(None)
}
