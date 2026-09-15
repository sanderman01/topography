use super::{
    AcquisitionError, Result, digest, ensure, hash_file, http, io_context, raster, read_small,
    valid_digest, write_new,
};
use crate::{
    catalog::source::{SourceRecord, SourceSummary, SurfaceType, validate_source},
    config,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;

/// Immutable receipt. Object paths are derived from checked SHA-256 values.
/// A raw import records neither prepared-data admission nor human acceptance.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawReceipt {
    pub schema_version: u32,
    pub record_id: String,
    pub asset_sha256: String,
    pub asset_bytes: u64,
    pub source_sha256: String,
    pub original_metadata_sha256: String,
    pub inspection_sha256: String,
    pub method: String,
    pub http_etag: Option<String>,
    pub gdal_version: String,
    pub topography_version: String,
    pub training_permission_recorded: bool,
    pub synthetic: bool,
    pub review_state: String,
    pub unresolved_metadata: Vec<String>,
}

fn roots(runtime: &Path) -> Result<(PathBuf, PathBuf)> {
    let config = config::inspect_runtime(runtime)?;
    let path = |name| {
        config
            .paths
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, p)| p.clone())
            .ok_or_else(|| AcquisitionError::Invalid(format!("missing runtime path: {name}")))
    };
    Ok((path("raw")?, path("staging")?))
}

fn source(path: &Path) -> Result<(SourceRecord, SourceSummary, Vec<u8>, Vec<u8>)> {
    let bytes = read_small(path)?;
    let input = std::str::from_utf8(&bytes)
        .map_err(|_| AcquisitionError::Invalid("source must be UTF-8 TOML".into()))?;
    let record: SourceRecord =
        toml::from_str(input).map_err(|e| crate::contracts::ContractError::Parse {
            path: path.into(),
            source: Box::new(e),
        })?;
    let summary = validate_source(&record)?;
    ensure(
        matches!(record.spatial.surface_type, SurfaceType::BareEarthDem),
        "raw DEM import requires an explicit bare_earth_dem source declaration",
    )?;
    let metadata_path = path
        .parent()
        .unwrap_or(Path::new("."))
        .join(&record.original_metadata_path);
    let metadata = read_small(&metadata_path)?;
    Ok((record, summary, bytes, metadata))
}

fn publish_asset(temp: NamedTempFile, raw: &Path, sha: &str, bytes: u64) -> Result<()> {
    let path = raw.join("objects").join(sha);
    match temp.persist_noclobber(&path) {
        Ok(_) => {
            io_context(
                "sync object directory",
                fs::File::open(raw.join("objects")).and_then(|f| f.sync_all()),
            )?;
            Ok(())
        }
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => ensure(
            hash_file(&path)? == (sha.into(), bytes),
            "existing raw object is corrupt; it was not replaced",
        ),
        Err(e) => Err(AcquisitionError::Io {
            context: "publish raw asset".into(),
            source: e.error,
        }),
    }
}

fn finish(
    temp: NamedTempFile,
    source_data: (SourceRecord, SourceSummary, Vec<u8>, Vec<u8>),
    raw: &Path,
    method: &str,
    etag: Option<String>,
) -> Result<PathBuf> {
    let (record, summary, source_bytes, metadata_bytes) = source_data;
    let (asset_sha256, asset_bytes) = hash_file(temp.path())?;
    if let Some(expected) = &record.asset_sha256 {
        ensure(
            expected.eq_ignore_ascii_case(&asset_sha256),
            "asset SHA-256 mismatch; raw storage was not published",
        )?;
    }
    let (inspection, gdal_version) = raster::inspect(temp.path())?;
    let inspection_bytes = serde_json::to_vec_pretty(&inspection)?;
    let receipt = RawReceipt {
        schema_version: 1,
        record_id: record.record_id,
        asset_sha256: asset_sha256.clone(),
        asset_bytes,
        source_sha256: digest(&source_bytes),
        original_metadata_sha256: digest(&metadata_bytes),
        inspection_sha256: digest(&inspection_bytes),
        method: method.into(),
        http_etag: etag,
        gdal_version,
        topography_version: env!("CARGO_PKG_VERSION").into(),
        training_permission_recorded: summary.training_permission_recorded,
        synthetic: summary.synthetic,
        review_state: "unreviewed".into(),
        unresolved_metadata: summary
            .unresolved_fields
            .into_iter()
            .map(String::from)
            .collect(),
    };
    io_context("sync raw asset", temp.as_file().sync_all())?;
    publish_asset(temp, raw, &asset_sha256, asset_bytes)?;
    for (sha, bytes) in [
        (&receipt.source_sha256, &source_bytes),
        (&receipt.original_metadata_sha256, &metadata_bytes),
        (&receipt.inspection_sha256, &inspection_bytes),
    ] {
        write_new(&raw.join("objects").join(sha), bytes)?;
    }
    let bytes = serde_json::to_vec_pretty(&receipt)?;
    let path = raw.join("records").join(format!("{}.json", digest(&bytes)));
    write_new(&path, &bytes)?;
    Ok(path)
}

/// Copy a local, self-contained Float32 GeoTIFF with a bounded streaming copy.
/// Original metadata and source declarations are captured as separate raw objects.
pub fn import(
    source_path: &Path,
    asset_path: &Path,
    runtime: &Path,
    max_bytes: u64,
) -> Result<PathBuf> {
    ensure(
        max_bytes > 0 && max_bytes < u64::MAX,
        "max-bytes must be positive and less than UInt64 max",
    )?;
    let source = source(source_path)?;
    let metadata = io_context("inspect local asset", fs::metadata(asset_path))?;
    ensure(
        metadata.is_file() && metadata.len() > 0 && metadata.len() <= max_bytes,
        "local asset must be a nonempty regular file within max-bytes",
    )?;
    let (raw, _) = roots(runtime)?;
    io_context(
        "create raw object directory",
        fs::create_dir_all(raw.join("objects")),
    )?;
    let mut temp = io_context(
        "stage raw import",
        NamedTempFile::new_in(raw.join("objects")),
    )?;
    let input = io_context("open local asset", fs::File::open(asset_path))?;
    let copied = io_context(
        "copy local asset",
        std::io::copy(&mut input.take(max_bytes + 1), &mut temp),
    )?;
    ensure(
        copied == metadata.len() && copied <= max_bytes,
        "local asset changed size during import",
    )?;
    io_context("flush imported asset", temp.flush())?;
    ensure(
        hash_file(asset_path)? == hash_file(temp.path())?,
        "local asset changed during import; no raw receipt was published",
    )?;
    finish(temp, source, &raw, "manual_import", None)
}

/// Acquire the first source URL, requiring an exact strong-ETag range identity.
/// Completed staging chunks survive interruption and are verified before reuse.
pub fn acquire(source_path: &Path, runtime: &Path, max_bytes: u64) -> Result<PathBuf> {
    ensure(max_bytes > 0, "max-bytes must be positive")?;
    let source = source(source_path)?;
    let identity = http::identify(&source.0.source_urls[0], max_bytes)?;
    let (raw, staging) = roots(runtime)?;
    io_context(
        "create raw object directory",
        fs::create_dir_all(raw.join("objects")),
    )?;
    let temp = io_context(
        "stage acquired asset",
        NamedTempFile::new_in(raw.join("objects")),
    )?;
    http::download(&identity, &staging, temp.path())?;
    ensure(
        hash_file(temp.path())?.1 == identity.bytes,
        "assembled asset size mismatch",
    )?;
    finish(temp, source, &raw, "https_ranges", Some(identity.etag))
}

/// Rehash every referenced object. This is integrity verification, not source acceptance.
pub fn verify(receipt_path: &Path) -> Result<RawReceipt> {
    let bytes = read_small(receipt_path)?;
    let receipt: RawReceipt = serde_json::from_slice(&bytes)?;
    ensure(
        receipt.schema_version == 1 && receipt.review_state == "unreviewed",
        "unsupported raw receipt version or review state",
    )?;
    ensure(
        receipt_path.file_stem().and_then(|s| s.to_str()) == Some(digest(&bytes).as_str()),
        "receipt filename/checksum mismatch",
    )?;
    let raw = receipt_path
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| AcquisitionError::Invalid("receipt must be under raw/records".into()))?;
    for sha in [
        &receipt.asset_sha256,
        &receipt.source_sha256,
        &receipt.original_metadata_sha256,
        &receipt.inspection_sha256,
    ] {
        ensure(valid_digest(sha), "invalid object SHA-256 in receipt")?;
        let (actual, length) = hash_file(&raw.join("objects").join(sha))?;
        ensure(
            &actual == sha,
            format!("raw object integrity mismatch: {sha}"),
        )?;
        if sha == &receipt.asset_sha256 {
            ensure(length == receipt.asset_bytes, "raw asset length mismatch")?;
        }
    }
    Ok(receipt)
}

pub(super) fn asset_path(receipt_path: &Path, receipt: &RawReceipt) -> Result<PathBuf> {
    let raw = receipt_path
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| AcquisitionError::Invalid("receipt must be under raw/records".into()))?;
    ensure(valid_digest(&receipt.asset_sha256), "invalid asset digest")?;
    Ok(raw.join("objects").join(&receipt.asset_sha256))
}
