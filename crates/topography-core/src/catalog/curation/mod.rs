//! Append-only crop decisions and captured evidence; no preparation or admission.

mod journal;

use crate::{
    acquisition::{
        AcquisitionError, Result, digest, ensure, io_context, read_json, read_small, valid_digest,
        write_new,
    },
    catalog::{
        inspect_legend,
        source::{License, TrainingPermission},
    },
    review::bundle,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ActorKind {
    Unassigned,
    Human,
    Assistant,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Actor {
    kind: ActorKind,
    name: String,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Decision {
    Annotate,
    Accept,
    Reject,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum FindingState {
    Unknown,
    Pass,
    Fail,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Finding {
    state: FindingState,
    note: String,
    evidence_ids: Vec<String>,
}

const FINDINGS: [&str; 6] = [
    "coordinates",
    "ground_support",
    "artifacts_and_repairs",
    "feature_fidelity",
    "water_classification",
    "unsupported_feature_gaps",
];

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Coverage {
    terrestrial_cells: u64,
    valid_terrestrial_cells: u64,
    note: String,
    evidence_ids: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Claims {
    actor: Actor,
    decision: Decision,
    reason: String,
    primary_class_id: Option<u16>,
    findings: BTreeMap<String, Finding>,
    coverage: Option<Coverage>,
    license: License,
    permission_evidence_ids: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    id: String,
    description: String,
    source_url: Option<String>,
    sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EvidenceInput {
    path: PathBuf,
    evidence: Evidence,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Target {
    raw_receipt_sha256: String,
    review_sha256: String,
    legend_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Draft {
    schema_version: u32,
    raw_receipt_path: PathBuf,
    review_path: PathBuf,
    legend_path: PathBuf,
    target: Target,
    previous_record_sha256: Option<String>,
    claims: Claims,
    evidence: Vec<EvidenceInput>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema_version: u32,
    kind: String,
    target: Target,
    previous_record_sha256: Option<String>,
    asset_sha256: String,
    window_cells: [u32; 4],
    legend_version: u32,
    claims: Claims,
    evidence: Vec<Evidence>,
    preparation_blockers: Vec<String>,
    admission: String,
    topography_version: String,
}

impl Record {
    fn objects(&self) -> Vec<&String> {
        [
            &self.target.raw_receipt_sha256,
            &self.target.review_sha256,
            &self.target.legend_sha256,
        ]
        .into_iter()
        .chain(self.evidence.iter().map(|e| &e.sha256))
        .collect()
    }
}

fn absolute(path: &Path) -> Result<PathBuf> {
    io_context(
        format!("resolve {}", path.display()),
        fs::canonicalize(path),
    )
}

fn json<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec_pretty(value)?;
    ensure(
        bytes.len() <= 1024 * 1024,
        "curation metadata exceeds 1 MiB",
    )?;
    Ok(bytes)
}

/// Create an unassigned JSON draft with pinned inputs. Does not create a catalog.
pub fn draft(
    runtime: &Path,
    raw: &Path,
    review: &Path,
    legend: &Path,
    output: &Path,
) -> Result<PathBuf> {
    ensure(!output.exists(), "draft output already exists")?;
    let bundle = bundle::verify(raw, review)?;
    inspect_legend(legend)?;
    let target = Target {
        raw_receipt_sha256: digest(&read_small(raw)?),
        review_sha256: digest(&read_small(review)?),
        legend_sha256: digest(&read_small(legend)?),
    };
    let store = journal::Store::locate(runtime)?;
    let previous_record_sha256 = if store.db.exists() {
        store.head(&target.review_sha256)?
    } else {
        None
    };
    let value = Draft {
        schema_version: 1,
        raw_receipt_path: absolute(raw)?,
        review_path: absolute(review)?,
        legend_path: absolute(legend)?,
        target,
        previous_record_sha256,
        claims: Claims {
            actor: Actor {
                kind: ActorKind::Unassigned,
                name: String::new(),
            },
            decision: Decision::Annotate,
            reason: String::new(),
            primary_class_id: None,
            findings: FINDINGS
                .iter()
                .map(|s| {
                    (
                        (*s).into(),
                        Finding {
                            state: FindingState::Unknown,
                            note: String::new(),
                            evidence_ids: vec![],
                        },
                    )
                })
                .collect(),
            coverage: None,
            license: bundle.source.license,
            permission_evidence_ids: vec![],
        },
        evidence: vec![],
    };
    let path = if output.is_absolute() {
        output.to_path_buf()
    } else {
        io_context("read working directory", std::env::current_dir())?.join(output)
    };
    write_new(&path, &json(&value)?)?;
    Ok(path)
}

fn validate_claims(
    claims: &Claims,
    evidence: &[Evidence],
    active: &[(u16, String)],
    cells: u64,
) -> Result<()> {
    ensure(
        claims.actor.kind != ActorKind::Unassigned && !claims.actor.name.trim().is_empty(),
        "assign an actor kind and name before recording",
    )?;
    ensure(
        !claims.reason.trim().is_empty(),
        "decision reason must not be empty",
    )?;
    ensure(
        claims.decision == Decision::Annotate || claims.actor.kind == ActorKind::Human,
        "accept/reject requires an explicit human actor",
    )?;
    if let Some(id) = claims.primary_class_id {
        ensure(
            active.iter().any(|(active, _)| *active == id),
            "primary class ID is not active in the pinned legend",
        )?;
    }
    ensure(
        claims.decision != Decision::Accept || claims.primary_class_id.is_some(),
        "accept requires an explicit primary class ID",
    )?;
    ensure(
        evidence.len() <= 16,
        "at most 16 evidence documents are supported",
    )?;
    let mut ids = BTreeSet::new();
    for e in evidence {
        ensure(
            !e.id.is_empty()
                && e.id.len() <= 64
                && e.id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                && ids.insert(e.id.as_str())
                && valid_digest(&e.sha256)
                && !e.description.trim().is_empty(),
            "invalid or duplicate evidence identity",
        )?;
        if let Some(url) = &e.source_url {
            ensure(
                url.starts_with("https://"),
                "evidence source_url must use HTTPS",
            )?;
        }
    }
    let references = |list: &[String]| -> Result<()> {
        ensure(
            !list.is_empty() && list.iter().all(|id| ids.contains(id.as_str())),
            "finding requires known evidence IDs",
        )
    };
    ensure(
        claims.findings.len() == FINDINGS.len()
            && FINDINGS.iter().all(|k| claims.findings.contains_key(*k)),
        "quality findings must contain exactly the six supported categories",
    )?;
    for finding in claims.findings.values() {
        if finding.state != FindingState::Unknown {
            ensure(
                !finding.note.trim().is_empty(),
                "resolved finding requires a note",
            )?;
            references(&finding.evidence_ids)?;
        } else if !finding.evidence_ids.is_empty() {
            references(&finding.evidence_ids)?;
        }
    }
    if let Some(c) = &claims.coverage {
        ensure(
            c.terrestrial_cells > 0
                && c.terrestrial_cells <= cells
                && c.valid_terrestrial_cells <= c.terrestrial_cells
                && !c.note.trim().is_empty(),
            "invalid terrestrial coverage counts or note",
        )?;
        references(&c.evidence_ids)?;
    }
    if !claims.permission_evidence_ids.is_empty() {
        references(&claims.permission_evidence_ids)?;
    }
    if matches!(
        claims.license.training_permission,
        TrainingPermission::Confirmed
    ) {
        for value in [
            &claims.license.license_name,
            &claims.license.license_url,
            &claims.license.evidence,
            &claims.license.attribution,
            &claims.license.edited_map_reuse,
            &claims.license.redistribution,
        ] {
            ensure(
                value.as_ref().is_some_and(|v| !v.trim().is_empty()),
                "confirmed permission requires dataset-specific evidence and all reuse/attribution obligations",
            )?;
        }
        references(&claims.permission_evidence_ids)?;
    }
    Ok(())
}

fn claim_blockers(claims: &Claims, bundle: &bundle::Bundle) -> Vec<String> {
    let mut out = Vec::new();
    if claims.decision != Decision::Accept {
        out.push("human acceptance absent or superseded".into());
    }
    if claims.primary_class_id.is_none() {
        out.push("primary class unset".into());
    }
    if bundle.source.synthetic {
        out.push("synthetic source excluded from real-terrain pool".into());
    }
    if !matches!(
        claims.license.training_permission,
        TrainingPermission::Confirmed
    ) {
        out.push("ML-training permission unconfirmed or excluded".into());
    }
    for (name, finding) in &claims.findings {
        if finding.state != FindingState::Pass {
            out.push(format!("{name}: {:?}", finding.state).to_lowercase());
        }
    }
    if !claims
        .coverage
        .as_ref()
        .is_some_and(|c| c.valid_terrestrial_cells * 1000 >= c.terrestrial_cells * 995)
    {
        out.push("at least 99.5% valid terrestrial coverage not established".into());
    }
    let source = &bundle.source;
    if source.survey_id.is_none()
        || source.acquisition_dates.is_empty()
        || source.spatial.vertical_datum.is_none()
    {
        out.push("survey, dates or vertical datum unresolved".into());
    }
    let transform: Vec<f64> =
        serde_json::from_value(bundle.inspection["geoTransform"].clone()).unwrap_or_default();
    if transform.len() != 6
        || !crate::review::metric_grid(&bundle.inspection, source, &transform)
        || !(0.5..=1.0).contains(&transform[1])
        || !(0.5..=1.0).contains(&-transform[5])
    {
        out.push(
            "native 0.5–1 m metric grid not established; no lower-fidelity exception implemented"
                .into(),
        );
    }
    out
}

fn blockers(claims: &Claims, bundle: &bundle::Bundle) -> Vec<String> {
    let mut out = claim_blockers(claims, bundle);
    // The historical precheck has no water classification. The new assessment
    // decodes samples independently and evaluates terrestrial cells only.
    if !bundle.metrics.as_ref().is_some_and(|m| {
        m["nonfinite_valid_cells"] == 0 && m["finite_valid_cells"].as_u64().is_some_and(|n| n > 0)
    }) {
        out.push("finite provider-valid samples not established".into());
    }
    // Preserve the historical v1 policy; eligibility is checked separately.
    out.push("regional preparation and independent admission checks not implemented".into());
    out
}

pub(crate) fn preparation_check(
    runtime: &Path,
    review: &str,
    expected_record: &str,
    assessment: &str,
    counts: Option<&crate::preparation::Counts>,
    bundle: &bundle::Bundle,
) -> Result<Vec<String>> {
    let store = journal::Store::locate(runtime)?;
    let history = store.history(review)?;
    let (id, bytes) = history
        .last()
        .ok_or_else(|| AcquisitionError::Invalid("crop has no curation records".into()))?;
    ensure(
        id == expected_record,
        "stale curation record; use the current decision head",
    )?;
    let record: Record = serde_json::from_slice(bytes)?;
    ensure(
        record.target.raw_receipt_sha256 == bundle.manifest.raw_receipt_sha256
            && record.asset_sha256 == bundle.manifest.asset_sha256,
        "assessment and curation parents differ",
    )?;
    let mut out = claim_blockers(&record.claims, bundle);
    let source = &bundle.source;
    if !matches!(
        source.spatial.surface_type,
        crate::catalog::source::SurfaceType::BareEarthDem
    ) {
        out.push("bare-earth DEM surface type required".into());
    }
    if source.spatial.horizontal_crs.is_none() || source.processing_history.is_empty() {
        out.push("horizontal CRS declaration or processing history unresolved".into());
    }
    let proj = &bundle.inspection["stac"]["proj:projjson"];
    let horizontal = if proj["type"] == "CompoundCRS" {
        &proj["components"][0]
    } else {
        proj
    };
    let epsg = horizontal["id"]["code"].as_u64().filter(|code| *code > 0);
    if horizontal["type"] != "ProjectedCRS"
        || horizontal["id"]["authority"] != "EPSG"
        || !epsg.is_some_and(|code| {
            source.spatial.horizontal_crs.as_deref() == Some(format!("EPSG:{code}").as_str())
        })
    {
        out.push("declared horizontal CRS must match the raster's horizontal EPSG code; other declarations unsupported in v1".into());
    }
    let transform: [f64; 6] = serde_json::from_value(bundle.inspection["geoTransform"].clone())?;
    if source.spatial.native_spacing_m != Some([transform[1], -transform[5]]) {
        out.push("declared native spacing differs from the reviewed grid or is unresolved".into());
    }
    let evidence: BTreeSet<&str> = record
        .evidence
        .iter()
        .filter(|e| e.sha256 == assessment)
        .map(|e| e.id.as_str())
        .collect();
    let captured = |ids: &[String]| ids.iter().any(|id| evidence.contains(id.as_str()));
    for name in [
        "ground_support",
        "artifacts_and_repairs",
        "water_classification",
        "unsupported_feature_gaps",
    ] {
        if !captured(&record.claims.findings[name].evidence_ids) {
            out.push(format!("{name}: exact assessment evidence not referenced"));
        }
    }
    match (&record.claims.coverage, counts) {
        (Some(claim), Some(measured)) => {
            if claim.terrestrial_cells != measured.terrestrial_cells
                || claim.valid_terrestrial_cells != measured.valid_terrestrial_cells
            {
                out.push("declared coverage differs from measured assessment".into());
            }
            if !captured(&claim.evidence_ids) {
                out.push("coverage: exact assessment evidence not referenced".into());
            }
        }
        _ => out.push("measured and reviewed terrestrial coverage required".into()),
    }
    ensure(
        store.head(review)?.as_deref() == Some(expected_record),
        "stale curation record; head changed during assessment",
    )?;
    Ok(out)
}

/// Capture evidence and append a decision. Replaying identical input is idempotent.
/// Actor identity is a local declaration, not an authenticated signature.
pub fn record(path: &Path, runtime: &Path) -> Result<String> {
    let draft: Draft = read_json(path)?;
    ensure(
        draft.schema_version == 1,
        "only curation draft version 1 is supported",
    )?;
    let base = path.parent().unwrap_or(Path::new("."));
    let raw_path = absolute(&base.join(&draft.raw_receipt_path))?;
    let review_path = absolute(&base.join(&draft.review_path))?;
    let legend_path = absolute(&base.join(&draft.legend_path))?;
    let bundle = bundle::verify(&raw_path, &review_path)?;
    let legend = inspect_legend(&legend_path)?;
    let mut objects = BTreeMap::new();
    for (path, expected) in [
        (&raw_path, &draft.target.raw_receipt_sha256),
        (&review_path, &draft.target.review_sha256),
        (&legend_path, &draft.target.legend_sha256),
    ] {
        let bytes = read_small(path)?;
        ensure(
            valid_digest(expected) && digest(&bytes) == *expected,
            "pinned target digest mismatch",
        )?;
        objects.insert(expected.clone(), bytes);
    }
    ensure(
        draft
            .previous_record_sha256
            .as_ref()
            .is_none_or(|id| valid_digest(id)),
        "invalid previous record digest",
    )?;
    ensure(
        draft.evidence.len() <= 16,
        "at most 16 evidence documents are supported",
    )?;
    let mut evidence = Vec::new();
    for input in draft.evidence {
        let bytes = read_small(&base.join(input.path))?;
        ensure(
            digest(&bytes) == input.evidence.sha256,
            "evidence digest mismatch; pin the reviewed bytes",
        )?;
        objects.insert(input.evidence.sha256.clone(), bytes);
        evidence.push(input.evidence);
    }
    let [_, _, w, h] = bundle.manifest.window_cells;
    validate_claims(
        &draft.claims,
        &evidence,
        &legend.classes,
        u64::from(w) * u64::from(h),
    )?;
    let preparation_blockers = blockers(&draft.claims, &bundle);
    let record = Record {
        schema_version: 1,
        kind: "crop_curation".into(),
        target: draft.target,
        previous_record_sha256: draft.previous_record_sha256,
        asset_sha256: bundle.manifest.asset_sha256,
        window_cells: bundle.manifest.window_cells,
        legend_version: legend.legend_version,
        claims: draft.claims,
        evidence,
        preparation_blockers,
        admission: "not admitted".into(),
        topography_version: env!("CARGO_PKG_VERSION").into(),
    };
    journal::Store::locate(runtime)?.append(&record, &objects, &raw_path, &review_path)
}

/// Verify the selected history and return its current record as portable JSON.
pub fn status(runtime: &Path, review_sha256: &str) -> Result<String> {
    let store = journal::Store::locate(runtime)?;
    let records = store.history(review_sha256)?;
    let (_, bytes) = records
        .last()
        .ok_or_else(|| AcquisitionError::Invalid("crop has no curation records".into()))?;
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

/// Check indexed history and evidence, and report published records awaiting registration.
pub fn check(runtime: &Path) -> Result<String> {
    journal::Store::locate(runtime)?.check()
}

/// Export verified small records/evidence. Raw and crop rasters need separate backup.
pub fn export(runtime: &Path, review_sha256: &str, output: &Path) -> Result<PathBuf> {
    journal::Store::locate(runtime)?.export(review_sha256, output)
}
