//! Bounded raw DEM acquisition, integrity verification, and GDAL review crops.

pub mod batch;
pub mod cover;
mod http;
pub(crate) mod raster;
mod store;
mod usgs;

pub use raster::create_review;
pub use store::{RawReceipt, acquire, import, verify};
pub use usgs::{DiscoverySummary, discover_usgs};

use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fmt, fs,
    io::{self, Read},
    path::Path,
};

use crate::contracts::ContractError;

#[derive(Debug)]
pub enum AcquisitionError {
    Contract(ContractError),
    Io { context: String, source: io::Error },
    Json(serde_json::Error),
    Catalog(rusqlite::Error),
    Transfer { message: String, transient: bool },
    Invalid(String),
}

impl fmt::Display for AcquisitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(e) => e.fmt(f),
            Self::Io { context, source } => write!(f, "{context}: {source}"),
            Self::Json(e) => write!(f, "invalid JSON: {e}"),
            Self::Catalog(e) => write!(f, "catalog: {e}"),
            Self::Transfer { message, .. } => f.write_str(message),
            Self::Invalid(message) => f.write_str(message),
        }
    }
}
impl Error for AcquisitionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Contract(e) => Some(e),
            Self::Io { source, .. } => Some(source),
            Self::Json(e) => Some(e),
            Self::Catalog(e) => Some(e),
            Self::Invalid(_) | Self::Transfer { .. } => None,
        }
    }
}
impl From<ContractError> for AcquisitionError {
    fn from(e: ContractError) -> Self {
        Self::Contract(e)
    }
}
impl From<serde_json::Error> for AcquisitionError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
impl From<rusqlite::Error> for AcquisitionError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Catalog(e)
    }
}
pub(crate) type Result<T> = std::result::Result<T, AcquisitionError>;

fn bounded_output(command: &mut std::process::Command) -> Result<std::process::Output> {
    use std::{io::Read, process::Stdio};
    const LIMIT: u64 = 1024 * 1024;
    let mut child = io_context(
        "start external tool",
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn(),
    )?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AcquisitionError::Invalid("missing tool stdout pipe".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AcquisitionError::Invalid("missing tool stderr pipe".into()))?;
    let errors = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let mut bytes = Vec::new();
    let read = stdout.take(LIMIT + 1).read_to_end(&mut bytes);
    if read.is_err() || bytes.len() as u64 > LIMIT {
        let _ = child.kill();
    }
    let status = io_context("wait for external tool", child.wait());
    let errors = errors
        .join()
        .map_err(|_| AcquisitionError::Invalid("external-tool stderr reader failed".into()))?;
    let stderr = io_context("read external-tool stderr", errors)?;
    io_context("read external-tool stdout", read)?;
    ensure(
        bytes.len() as u64 <= LIMIT && stderr.len() as u64 <= LIMIT,
        "external-tool output exceeds 1 MiB",
    )?;
    Ok(std::process::Output {
        status: status?,
        stdout: bytes,
        stderr,
    })
}

pub(crate) fn io_context<T>(context: impl Into<String>, result: io::Result<T>) -> Result<T> {
    result.map_err(|source| AcquisitionError::Io {
        context: context.into(),
        source,
    })
}
pub(crate) fn ensure(condition: bool, message: impl Into<String>) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(AcquisitionError::Invalid(message.into()))
    }
}
pub(crate) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(crate) fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

pub(crate) fn read_small(path: &Path) -> Result<Vec<u8>> {
    let file = io_context(format!("open {}", path.display()), fs::File::open(path))?;
    let mut bytes = Vec::new();
    io_context(
        "read metadata",
        file.take(1024 * 1024 + 1).read_to_end(&mut bytes),
    )?;
    ensure(bytes.len() <= 1024 * 1024, "metadata exceeds 1 MiB")?;
    Ok(bytes)
}
pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&read_small(path)?)?)
}

pub(crate) fn hash_file(path: &Path) -> Result<(String, u64)> {
    let mut file = io_context(format!("open {}", path.display()), fs::File::open(path))?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let count = io_context("hash file", file.read(&mut buffer))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        total += count as u64;
    }
    Ok((format!("{:x}", hasher.finalize()), total))
}

pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or_else(|| AcquisitionError::Invalid("output needs a parent".into()))?;
    io_context("create output parent", fs::create_dir_all(parent))?;
    let mut temp = io_context("stage metadata", tempfile::NamedTempFile::new_in(parent))?;
    io_context("write metadata", temp.write_all(bytes))?;
    io_context("sync metadata", temp.as_file().sync_all())?;
    match temp.persist_noclobber(path) {
        Ok(_) => {
            io_context(
                "sync output directory",
                fs::File::open(parent).and_then(|f| f.sync_all()),
            )?;
            Ok(())
        }
        Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => ensure(
            read_small(path)? == bytes,
            format!("existing file differs: {}", path.display()),
        ),
        Err(e) => Err(AcquisitionError::Io {
            context: format!("publish {}", path.display()),
            source: e.error,
        }),
    }
}
