//! A narrowly scoped child of an unchanged, geographically checked numeric release.

use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FitApproval {
    schema_version: u32,
    kind: String,
    scope: String,
    dataset_sha256: String,
    actor_kind: String,
    actor_name: String,
    recorder: String,
    decision: String,
    exact_response: String,
    plan_sha256: String,
    proposal_sha256: String,
    trial: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FitRelease {
    schema_version: u32,
    kind: String,
    purpose: String,
    dataset_sha256: String,
    approval_document: String,
    topography_version: String,
}

fn validate(runtime: &Path, release: &FitRelease) -> Result<Vec<Member>> {
    ensure(
        release.schema_version == 1
            && release.kind == "representation_fit_release"
            && release.purpose == "representation_fit_only"
            && valid_digest(&release.dataset_sha256),
        "unsupported fitting release",
    )?;
    let approval: FitApproval = serde_json::from_str(&release.approval_document)?;
    ensure(
        approval.schema_version == 1
            && approval.kind == "representation_fit_approval"
            && approval.scope == release.purpose
            && approval.dataset_sha256 == release.dataset_sha256
            && approval.decision == "approve"
            && approval.trial == "scalar_codec_fit_v1",
        "fitting approval must pin the dataset and scalar_codec_fit_v1 scope",
    )?;
    ensure(
        approval.actor_kind == "human"
            && short_text(&approval.actor_name)
            && short_text(&approval.recorder)
            && short_text(&approval.exact_response)
            && valid_digest(&approval.plan_sha256)
            && valid_digest(&approval.proposal_sha256),
        "fitting approval requires human response and plan/proposal identities",
    )?;
    let parent = storage(runtime, "datasets")?
        .join(&release.dataset_sha256)
        .join("manifest.json");
    let (_, _, members) = checked(runtime, &parent)?;
    Ok(members)
}

/// Publish separate permission for the single bounded scalar trial; no production admission.
pub fn build_fit(runtime: &Path, dataset: &Path, approval: &Path) -> Result<PathBuf> {
    let (id, _, _) = checked(runtime, dataset)?;
    let approval_bytes = read_small(approval)?;
    let release = FitRelease {
        schema_version: 1,
        kind: "representation_fit_release".into(),
        purpose: "representation_fit_only".into(),
        dataset_sha256: id,
        approval_document: String::from_utf8(approval_bytes.clone())
            .map_err(|_| AcquisitionError::Invalid("approval must be UTF-8".into()))?,
        topography_version: env!("CARGO_PKG_VERSION").into(),
    };
    validate(runtime, &release)?;
    ensure(read_small(approval)? == approval_bytes, "approval changed")?;
    let bytes = serde_json::to_vec_pretty(&release)?;
    ensure(bytes.len() <= 1024 * 1024, "fitting release exceeds 1 MiB")?;
    let path = storage(runtime, "datasets")?
        .join(digest(&bytes))
        .join("manifest.json");
    write_new(&path, &bytes)?;
    verify_fit(runtime, &path)?;
    Ok(path)
}

/// Check immutable child and all current geographic/source gates of its evaluation parent.
pub fn verify_fit(runtime: &Path, path: &Path) -> Result<String> {
    let bytes = read_small(path)?;
    let id = digest(&bytes);
    ensure(
        path.parent()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
            == Some(&id),
        "fitting manifest integrity mismatch",
    )?;
    let release: FitRelease = serde_json::from_slice(&bytes)?;
    let members = validate(runtime, &release)?;
    ensure(read_small(path)? == bytes, "fitting manifest changed")?;
    let regions: Vec<_> = members
        .into_iter()
        .map(|m| {
            json!({
                "prepared_sha256":m.id,"group":m.group,"role":m.role,"grid":m.grid
            })
        })
        .collect();
    Ok(serde_json::to_string_pretty(&json!({
        "schema_version":1,"kind":"representation_fit_status","manifest_sha256":id,
        "dataset_sha256":release.dataset_sha256,"current":true,
        "purpose":"representation_fit_only","trial":"scalar_codec_fit_v1",
        "maximum_steps":200,"threads":2,"timeout_seconds":1800,
        "address_space_bytes":6442450944_u64,"artifact_disk_limit_bytes":536870912,
        "gpu_used":false,"regions":regions,
        "production_training_admission":"not admitted"
    }))?)
}
