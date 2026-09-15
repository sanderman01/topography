use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::PresetStatus;
use crate::contracts::{ContractError, read_toml, require, version};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeConfig {
    schema_version: u32,
    status: PresetStatus,
    paths: Paths,
    hardware: Hardware,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Paths {
    base_dir: PathBuf,
    catalog_db: PathBuf,
    raw: PathBuf,
    prepared: PathBuf,
    review: PathBuf,
    datasets: PathBuf,
    models: PathBuf,
    worlds: PathBuf,
    exports: PathBuf,
    cache: PathBuf,
    staging: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Hardware {
    gpu_vram_budget_gb: u8,
}

/// Resolved paths are not created, canonicalized, or checked for writability.
#[derive(Debug)]
pub struct RuntimeSummary {
    pub paths: Vec<(&'static str, PathBuf)>,
    pub gpu_vram_budget_gb: u8,
}

/// Resolve base_dir relative to the supplied file's parent, then storage paths
/// relative to base_dir. Absolute paths override either base. No overrides load implicitly.
pub fn inspect_runtime(path: &Path) -> Result<RuntimeSummary, ContractError> {
    let config: RuntimeConfig = read_toml(path)?;
    version(config.schema_version)?;
    let PresetStatus::Draft = config.status;
    require(
        (1..=16).contains(&config.hardware.gpu_vram_budget_gb),
        "hardware.gpu_vram_budget_gb",
        "must be in 1..=16; the pilot ceiling is 16 GB",
    )?;
    require(
        !config.paths.base_dir.as_os_str().is_empty(),
        "paths.base_dir",
        "must not be empty",
    )?;
    let absolute = std::path::absolute(path).map_err(|source| ContractError::Io {
        path: path.into(),
        source,
    })?;
    let parent = absolute.parent().ok_or_else(|| ContractError::Invalid {
        field: "path".into(),
        reason: "configuration must have a parent directory".into(),
    })?;
    let base = parent.join(config.paths.base_dir);
    let mut paths = Vec::new();
    for (name, value) in [
        ("catalog_db", config.paths.catalog_db),
        ("raw", config.paths.raw),
        ("prepared", config.paths.prepared),
        ("review", config.paths.review),
        ("datasets", config.paths.datasets),
        ("models", config.paths.models),
        ("worlds", config.paths.worlds),
        ("exports", config.paths.exports),
        ("cache", config.paths.cache),
        ("staging", config.paths.staging),
    ] {
        require(
            !value.as_os_str().is_empty(),
            &format!("paths.{name}"),
            "must not be empty",
        )?;
        paths.push((name, base.join(value)));
    }
    Ok(RuntimeSummary {
        paths,
        gpu_vram_budget_gb: config.hardware.gpu_vram_budget_gb,
    })
}
