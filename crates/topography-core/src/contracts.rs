//! Shared errors and strict TOML decoding for small, versioned inputs.

use std::{
    error::Error,
    fmt,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

use serde::de::DeserializeOwned;

/// A contract failure preserves I/O and parser causes for callers.
#[derive(Debug)]
pub enum ContractError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Parse {
        path: PathBuf,
        source: Box<toml::de::Error>,
    },
    Invalid {
        field: String,
        reason: String,
    },
}

impl fmt::Display for ContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            Self::Parse { path, source } => write!(f, "cannot parse {}: {source}", path.display()),
            Self::Invalid { field, reason } => write!(f, "{field}: {reason}"),
        }
    }
}

impl Error for ContractError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source.as_ref()),
            Self::Invalid { .. } => None,
        }
    }
}

pub(crate) fn require(condition: bool, field: &str, reason: &str) -> Result<(), ContractError> {
    if condition {
        Ok(())
    } else {
        Err(ContractError::Invalid {
            field: field.into(),
            reason: reason.into(),
        })
    }
}

pub(crate) fn positive(value: f64, field: &str) -> Result<(), ContractError> {
    require(
        value.is_finite() && value > 0.0,
        field,
        "must be finite and greater than zero",
    )
}

pub(crate) fn nonempty(value: &str, field: &str) -> Result<(), ContractError> {
    require(!value.trim().is_empty(), field, "must not be empty")
}

pub(crate) fn version(value: u32) -> Result<(), ContractError> {
    require(value == 1, "schema_version", "only version 1 is supported")
}

pub(crate) fn read_toml<T: DeserializeOwned>(path: &Path) -> Result<T, ContractError> {
    // Contracts are small metadata inputs, never raster-sized payloads.
    const MAX_BYTES: u64 = 1024 * 1024;
    let mut input = String::new();
    File::open(path)
        .and_then(|file| file.take(MAX_BYTES + 1).read_to_string(&mut input))
        .map_err(|source| ContractError::Io {
            path: path.into(),
            source,
        })?;
    require(
        input.len() as u64 <= MAX_BYTES,
        "input",
        "contract exceeds the 1 MiB limit",
    )?;
    toml::from_str(&input).map_err(|source| ContractError::Parse {
        path: path.into(),
        source: Box::new(source),
    })
}
