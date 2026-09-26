//! Immutable scoped pilot datasets with current prepared-source checks.

mod diffusion;
mod fitting;
mod numeric;
pub use diffusion::build_diffusion;
pub use fitting::{build_fit, verify_fit};
pub use numeric::evaluate;

use crate::{
    acquisition::{AcquisitionError, Result, digest, ensure, read_small, valid_digest, write_new},
    config, preparation,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Role {
    Calibration,
    Validation,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Normalization {
    pub offset_m: f32,
    pub scale_m: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    id: String,
    role: Role,
    prepared: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Selection {
    schema_version: u32,
    purpose: String,
    maximum_context_m: f64,
    separation_buffer_m: f64,
    pub normalization: Normalization,
    groups: Vec<Group>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Approval {
    schema_version: u32,
    kind: String,
    selection_sha256: String,
    actor_kind: String,
    actor_name: String,
    recorder: String,
    decision: String,
    exact_response: String,
    scope: String,
    reviewed_proposal_sha256: String,
    reviewed_page_sha256: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Release {
    schema_version: u32,
    kind: String,
    purpose: String,
    selection_document: String,
    approval_document: String,
    topography_version: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Grid {
    pub origin: [f64; 2],
    pub size: [u32; 2],
    pub spacing_m: [f64; 2],
}

pub(crate) struct Member {
    pub id: String,
    pub group: String,
    pub role: Role,
    pub path: PathBuf,
    pub manifest: Value,
    pub grid: Grid,
}

pub(crate) fn storage(runtime: &Path, name: &str) -> Result<PathBuf> {
    config::inspect_runtime(runtime)?
        .paths
        .into_iter()
        .find(|(key, _)| *key == name)
        .map(|(_, path)| path)
        .ok_or_else(|| AcquisitionError::Invalid(format!("missing {name} storage")))
}

fn short_text(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 4096
}

fn documents(release: &Release) -> Result<Selection> {
    ensure(
        matches!(release.schema_version, 1 | 2)
            && release.kind == "numeric_evaluation_release"
            && release.purpose == "numeric_evaluation_only",
        "unsupported dataset release",
    )?;
    let selection: Selection = serde_json::from_str(&release.selection_document)?;
    let approval: Approval = serde_json::from_str(&release.approval_document)?;
    ensure(
        selection.schema_version == release.schema_version
            && selection.purpose == "numeric_evaluation_only",
        "unsupported dataset selection",
    )?;
    ensure(
        approval.schema_version == 1
            && approval.kind == "numeric_evaluation_approval"
            && approval.decision == "approve"
            && approval.scope == "numeric_evaluation_only"
            && approval.selection_sha256 == digest(release.selection_document.as_bytes()),
        "approval must pin this evaluation selection",
    )?;
    ensure(
        approval.actor_kind == "human"
            && short_text(&approval.actor_name)
            && short_text(&approval.recorder)
            && short_text(&approval.exact_response),
        "approval requires a human actor, recorder and exact response",
    )?;
    ensure(
        valid_digest(&approval.reviewed_proposal_sha256)
            && valid_digest(&approval.reviewed_page_sha256),
        "approval requires reviewed proposal and page identities",
    )?;
    ensure(
        selection.maximum_context_m.is_finite()
            && selection.maximum_context_m > 0.0
            && selection.maximum_context_m <= 8192.0
            && selection.separation_buffer_m.is_finite()
            && selection.separation_buffer_m >= selection.maximum_context_m,
        "separation buffer must cover the positive maximum context (at most 8192 m)",
    )?;
    ensure(
        selection.normalization.offset_m == 0.0 && selection.normalization.scale_m == 2048.0,
        "v1 numeric normalization requires zero offset and 2048 m scale",
    )?;
    ensure(
        (2..=32).contains(&selection.groups.len()),
        "dataset requires 2..=32 groups",
    )?;
    Ok(selection)
}

fn distance(a: &Grid, b: &Grid) -> f64 {
    let right = |g: &Grid| g.origin[0] + f64::from(g.size[0]) * g.spacing_m[0];
    let bottom = |g: &Grid| g.origin[1] - f64::from(g.size[1]) * g.spacing_m[1];
    let x = (a.origin[0] - right(b))
        .max(b.origin[0] - right(a))
        .max(0.0);
    let y = (bottom(a) - b.origin[1])
        .max(bottom(b) - a.origin[1])
        .max(0.0);
    x.hypot(y)
}

fn members(runtime: &Path, selection: &Selection) -> Result<Vec<Member>> {
    let root = storage(runtime, "prepared")?;
    let mut groups = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let mut roles = BTreeSet::new();
    let mut asset_roles = BTreeMap::new();
    let mut result: Vec<Member> = Vec::new();
    // Validate all identities before touching files, including duplicate membership.
    for group in &selection.groups {
        ensure(
            !group.id.is_empty()
                && group.id.len() <= 80
                && group
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                && groups.insert(group.id.clone()),
            "invalid or duplicate group ID",
        )?;
        roles.insert(group.role.clone());
        ensure(!group.prepared.is_empty(), "empty geographic group")?;
        for id in &group.prepared {
            ensure(
                valid_digest(id) && ids.insert(id.clone()),
                "invalid or duplicate prepared identity",
            )?;
        }
    }
    ensure(
        roles.len() == 2,
        "dataset requires both calibration and validation",
    )?;
    ensure(ids.len() <= 32, "dataset is limited to 32 regions")?;
    for group in &selection.groups {
        for id in &group.prepared {
            let path = root.join(id).join("manifest.json");
            ensure(
                preparation::verify(runtime, &path)?.0,
                "prepared parent is not registered and current",
            )?;
            let bytes = read_small(&path)?;
            ensure(digest(&bytes) == *id, "prepared parent identity changed")?;
            let manifest: Value = serde_json::from_slice(&bytes)?;
            // v3 products add only the land-cover layer, which v2 selections ignore.
            let version = manifest["schema_version"].as_u64();
            ensure(
                version == Some(u64::from(selection.schema_version))
                    || (selection.schema_version == 2 && version == Some(3)),
                "dataset selection and prepared surface-cover contract versions differ",
            )?;
            let grid: Grid = serde_json::from_value(manifest["grid"].clone())?;
            ensure(
                grid.size.iter().all(|n| (1..=2048).contains(n))
                    && (0..2).all(|i| {
                        f64::from(grid.size[i]) * grid.spacing_m[i] <= selection.maximum_context_m
                    }),
                "evaluation grid exceeds 2048 cells or declared maximum context",
            )?;
            if let Some(first) = result.first() {
                ensure(
                    manifest["coordinate_system"] == first.manifest["coordinate_system"]
                        && manifest["vertical_datum"] == first.manifest["vertical_datum"],
                    "evaluation regions require a common metric CRS and vertical datum",
                )?;
            }
            let parents = manifest["parents"]
                .as_array()
                .ok_or_else(|| AcquisitionError::Invalid("missing prepared parents".into()))?;
            for parent in parents {
                let asset: String = serde_json::from_value(parent["asset_sha256"].clone())?;
                if let Some(role) = asset_roles.insert(asset, group.role.clone()) {
                    ensure(role == group.role, "source asset crosses geographic roles")?;
                }
            }
            result.push(Member {
                id: id.clone(),
                group: group.id.clone(),
                role: group.role.clone(),
                path,
                manifest,
                grid,
            });
        }
    }
    for (i, a) in result.iter().enumerate() {
        for b in &result[i + 1..] {
            let separation = distance(&a.grid, &b.grid);
            // Touching disjoint source regions are allowed within the same role.
            let overlap = a.grid.origin[0]
                < b.grid.origin[0] + f64::from(b.grid.size[0]) * b.grid.spacing_m[0]
                && b.grid.origin[0]
                    < a.grid.origin[0] + f64::from(a.grid.size[0]) * a.grid.spacing_m[0]
                && a.grid.origin[1]
                    > b.grid.origin[1] - f64::from(b.grid.size[1]) * b.grid.spacing_m[1]
                && b.grid.origin[1]
                    > a.grid.origin[1] - f64::from(a.grid.size[1]) * a.grid.spacing_m[1];
            ensure(!overlap, "dataset footprints overlap")?;
            ensure(
                a.role == b.role || separation >= selection.separation_buffer_m,
                "geographic separation is smaller than the declared buffer",
            )?;
        }
    }
    Ok(result)
}

pub(crate) fn checked(runtime: &Path, path: &Path) -> Result<(String, Selection, Vec<Member>)> {
    let bytes = read_small(path)?;
    let id = digest(&bytes);
    ensure(
        path.parent()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
            == Some(id.as_str()),
        "dataset manifest integrity mismatch",
    )?;
    let release: Release = serde_json::from_slice(&bytes)?;
    let selection = documents(&release)?;
    let members = members(runtime, &selection)?;
    ensure(
        read_small(path)? == bytes,
        "dataset manifest changed during verification",
    )?;
    Ok((id, selection, members))
}

/// Publish an evaluation-only release after validating scoped approval and current parents.
pub fn build(runtime: &Path, selection: &Path, approval: &Path) -> Result<PathBuf> {
    let selection_bytes = read_small(selection)?;
    let approval_bytes = read_small(approval)?;
    let utf8 = |bytes| {
        String::from_utf8(bytes)
            .map_err(|_| AcquisitionError::Invalid("dataset documents must be UTF-8".into()))
    };
    let selected: Selection = serde_json::from_slice(&selection_bytes)?;
    let release = Release {
        schema_version: selected.schema_version,
        kind: "numeric_evaluation_release".into(),
        purpose: "numeric_evaluation_only".into(),
        selection_document: utf8(selection_bytes.clone())?,
        approval_document: utf8(approval_bytes.clone())?,
        topography_version: env!("CARGO_PKG_VERSION").into(),
    };
    let spec = documents(&release)?;
    members(runtime, &spec)?;
    ensure(
        read_small(selection)? == selection_bytes && read_small(approval)? == approval_bytes,
        "dataset input documents changed",
    )?;
    let bytes = serde_json::to_vec_pretty(&release)?;
    ensure(bytes.len() <= 1024 * 1024, "dataset manifest exceeds 1 MiB")?;
    let path = storage(runtime, "datasets")?
        .join(digest(&bytes))
        .join("manifest.json");
    write_new(&path, &bytes)?;
    // Concurrent source edits leave a preserved historical release, never a usable current one.
    checked(runtime, &path)?;
    Ok(path)
}

/// Verify release identity and current source status; a historical release may become unusable.
pub fn verify(runtime: &Path, path: &Path) -> Result<String> {
    let (id, selection, members) = checked(runtime, path)?;
    Ok(serde_json::to_string_pretty(
        &json!({"schema_version":selection.schema_version,"kind":"numeric_dataset_status",
        "manifest_sha256":id,"current":true,"purpose":"numeric_evaluation_only",
        "surface_cover":if selection.schema_version == 2 {"assessed"} else {"legacy_assessment_pending"},
        "regions":members.len(),"training_admission":"not admitted"}),
    )?)
}
