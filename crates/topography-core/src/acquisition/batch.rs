//! Sequential, resumable acquisition queues; processing never selects terrain.
use super::{Result, digest, ensure, http, io_context, read_json, read_small, store, write_new};
use crate::{
    catalog::source::{SourceRecord, validate_source},
    review::bundle,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Queue {
    schema_version: u32,
    max_total_bytes: u64,
    sources: Vec<Source>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    id: String,
    source: PathBuf,
    metadata_url: Option<String>,
    existing_receipt: Option<PathBuf>,
    max_bytes: u64,
    crops: Vec<Crop>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Crop {
    id: String,
    title: String,
    window_cells: [u32; 4],
    primary_class_id: u16,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Completed {
    source_sha256: String,
    receipt: PathBuf,
    receipt_sha256: String,
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 80
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn source_record(path: &Path) -> Result<SourceRecord> {
    let bytes = read_small(path)?;
    let record: SourceRecord = toml::from_str(
        std::str::from_utf8(&bytes)
            .map_err(|_| super::AcquisitionError::Invalid("source must be UTF-8".into()))?,
    )
    .map_err(|e| super::AcquisitionError::Invalid(format!("invalid source: {e}")))?;
    validate_source(&record)?;
    Ok(record)
}
fn load(path: &Path) -> Result<(Queue, Value)> {
    let path = io_context("resolve queue", fs::canonicalize(path))?;
    let bytes = read_small(&path)?;
    let mut queue: Queue = serde_json::from_slice(&bytes)?;
    ensure(
        queue.schema_version == 1,
        "only queue version 1 is supported",
    )?;
    ensure(
        !queue.sources.is_empty() && queue.sources.len() <= 256,
        "queue needs 1..=256 sources",
    )?;
    let base = path.parent().unwrap_or(Path::new("."));
    let mut ids = BTreeSet::new();
    let mut budget = 0u64;
    let mut pinned = Vec::new();
    let mut crops = 0;
    for item in &mut queue.sources {
        ensure(identifier(&item.id), "invalid source identifier")?;
        ensure(ids.insert(&item.id), "duplicate source identifier")?;
        ensure(
            item.max_bytes > 0 && item.max_bytes < u64::MAX,
            "source byte limit must be positive and bounded",
        )?;
        budget = budget
            .checked_add(item.max_bytes)
            .ok_or_else(|| super::AcquisitionError::Invalid("source budget overflow".into()))?;
        item.source = io_context("resolve source", fs::canonicalize(base.join(&item.source)))?;
        let record = source_record(&item.source)?;
        http::validate_url(&record.source_urls[0])?;
        if let Some(url) = &item.metadata_url {
            http::validate_url(url)?;
        }
        if let Some(receipt) = &mut item.existing_receipt {
            *receipt = io_context("resolve receipt", fs::canonicalize(base.join(&receipt)))?;
        }
        let mut names = BTreeSet::new();
        for crop in &item.crops {
            ensure(identifier(&crop.id), "invalid crop identifier")?;
            ensure(names.insert(&crop.id), "duplicate crop identifier")?;
            ensure(
                !crop.title.trim().is_empty() && crop.title.len() <= 240,
                "crop title must contain 1..=240 bytes",
            )?;
            let [x, y, w, h] = crop.window_cells;
            ensure(
                w > 0
                    && h > 0
                    && w <= 2048
                    && h <= 2048
                    && x.checked_add(w).is_some()
                    && y.checked_add(h).is_some(),
                "invalid native crop window",
            )?;
            ensure(crop.primary_class_id > 0, "crop needs a primary class ID")?;
            crops += 1;
        }
        pinned.push(json!({"id": item.id, "source": item.source, "source_sha256": digest(&read_small(&item.source)?), "existing_receipt":item.existing_receipt}));
    }
    ensure(crops <= 2048, "queue exceeds 2048 crops")?;
    ensure(
        budget <= queue.max_total_bytes,
        "source limits exceed aggregate byte budget",
    )?;
    Ok((
        queue,
        json!({"schema_version":1,"queue_sha256":digest(&bytes),"queue":serde_json::from_slice::<Value>(&bytes)?,"sources":pinned}),
    ))
}
/// Validate all declarations without network requests or processing.
pub fn validate(path: &Path) -> Result<String> {
    let (queue, _) = load(path)?;
    Ok(format!(
        "Validated {} sources, {} crops; maximum source bytes {}",
        queue.sources.len(),
        queue.sources.iter().map(|s| s.crops.len()).sum::<usize>(),
        queue.max_total_bytes
    ))
}
fn initialize(queue: &Path, runtime: &Path, output: &Path) -> Result<Queue> {
    let (queue, mut pin) = load(queue)?;
    // A run cannot silently move its raw storage or catalog on resume.
    pin["runtime_sha256"] = json!(digest(&read_small(runtime)?));
    pin["runtime_path"] = json!(io_context("resolve runtime", fs::canonicalize(runtime))?);
    io_context("create batch directory", fs::create_dir_all(output))?;
    write_new(
        &output.join("request.json"),
        &serde_json::to_vec_pretty(&pin)?,
    )?;
    for item in &queue.sources {
        let metadata = metadata_path(item)?;
        let marker = output
            .join("metadata-inputs")
            .join(format!("{}.json", item.id));
        if !marker.exists() {
            let sha = if metadata.exists() {
                Some(digest(&read_small(&metadata)?))
            } else {
                None
            };
            write_new(&marker, &serde_json::to_vec(&json!({"sha256":sha}))?)?;
        }
        let initial: Value = read_json(&marker)?;
        if let Some(expected) = initial["sha256"].as_str() {
            ensure(
                digest(&read_small(&metadata)?) == expected,
                "original metadata differs from batch input",
            )?;
        }
        let fetched = output
            .join("metadata-fetched")
            .join(format!("{}.json", item.id));
        if fetched.exists() {
            let saved: Value = read_json(&fetched)?;
            ensure(
                saved["sha256"] == digest(&read_small(&metadata)?),
                "downloaded metadata differs from batch input",
            )?;
        }
    }
    Ok(queue)
}

fn metadata_path(item: &Source) -> Result<PathBuf> {
    Ok(item
        .source
        .parent()
        .unwrap_or(Path::new("."))
        .join(source_record(&item.source)?.original_metadata_path))
}
fn completed(item: &Source, marker: &Path) -> Result<PathBuf> {
    let saved: Completed = read_json(marker)?;
    ensure(
        saved.source_sha256 == digest(&read_small(&item.source)?)
            && saved.receipt_sha256 == digest(&read_small(&saved.receipt)?),
        "completed source identity mismatch",
    )?;
    let receipt = store::verify(&saved.receipt)?;
    let source = source_record(&item.source)?;
    let metadata = item
        .source
        .parent()
        .unwrap_or(Path::new("."))
        .join(source.original_metadata_path);
    ensure(
        receipt.source_sha256 == saved.source_sha256
            && receipt.original_metadata_sha256 == digest(&read_small(&metadata)?)
            && receipt.asset_bytes <= item.max_bytes,
        "completed source or metadata differs",
    )?;
    Ok(saved.receipt)
}
fn acquire_one(item: &Source, runtime: &Path, output: &Path) -> Result<PathBuf> {
    let marker = output.join("downloads").join(format!("{}.json", item.id));
    let source = source_record(&item.source)?;
    let metadata = item
        .source
        .parent()
        .unwrap_or(Path::new("."))
        .join(&source.original_metadata_path);
    let initial: Value = read_json(
        &output
            .join("metadata-inputs")
            .join(format!("{}.json", item.id)),
    )?;
    let fetched = output
        .join("metadata-fetched")
        .join(format!("{}.json", item.id));
    if initial["sha256"].is_null() && !fetched.exists() {
        let url = item.metadata_url.as_ref().ok_or_else(|| {
            super::AcquisitionError::Invalid(
                "missing original metadata; provide metadata_url or a local file".into(),
            )
        })?;
        http::metadata(url, &metadata)?;
        write_new(
            &fetched,
            &serde_json::to_vec(&json!({"sha256":digest(&read_small(&metadata)?)}))?,
        )?;
    }
    if marker.exists() {
        return completed(item, &marker);
    }
    let receipt = if let Some(path) = &item.existing_receipt {
        path.clone()
    } else {
        store::acquire(&item.source, runtime, item.max_bytes)?
    };
    let receipt = io_context("resolve acquired receipt", fs::canonicalize(receipt))?;
    let raw = store::verify(&receipt)?;
    ensure(
        raw.source_sha256 == digest(&read_small(&item.source)?)
            && raw.original_metadata_sha256 == digest(&read_small(&metadata)?)
            && raw.asset_bytes <= item.max_bytes,
        "acquired source or metadata differs",
    )?;
    let saved = Completed {
        source_sha256: raw.source_sha256,
        receipt_sha256: digest(&read_small(&receipt)?),
        receipt: receipt.clone(),
    };
    write_new(&marker, &serde_json::to_vec_pretty(&saved)?)?;
    Ok(receipt)
}
/// Atomically replace operational status; immutable artifacts use write_new instead.
fn status(path: &Path, value: &Value) -> Result<()> {
    let mut temp = io_context(
        "stage batch status",
        tempfile::NamedTempFile::new_in(path.parent().unwrap_or(Path::new("."))),
    )?;
    io_context(
        "write batch status",
        temp.write_all(&serde_json::to_vec_pretty(value)?),
    )?;
    io_context("sync batch status", temp.as_file().sync_all())?;
    temp.persist(path)
        .map_err(|e| super::AcquisitionError::Io {
            context: "publish batch status".into(),
            source: e.error,
        })?;
    Ok(())
}
fn report(
    output: &Path,
    stage: &str,
    completed: usize,
    errors: Vec<Value>,
) -> Result<(bool, String)> {
    let result = json!({"schema_version":1,"stage":stage,"completed":completed,"failed":errors.len(),"errors":errors,"output":output});
    status(&output.join(format!("{stage}-status.json")), &result)?;
    Ok((
        result["failed"] == 0,
        serde_json::to_string_pretty(&result)?,
    ))
}
/// Download each source once, continuing independent failures and retaining resumable chunks.
pub fn download(queue: &Path, runtime: &Path, output: &Path) -> Result<(bool, String)> {
    let queue = initialize(queue, runtime, output)?;
    let mut count = 0;
    let mut errors = Vec::new();
    for (index, item) in queue.sources.iter().enumerate() {
        eprintln!(
            "Download {}/{}: {}",
            index + 1,
            queue.sources.len(),
            item.id
        );
        match acquire_one(item, runtime, output) {
            Ok(_) => count += 1,
            Err(e) => errors.push(json!({"id":item.id,"error":e.to_string()})),
        }
        report(output, "download", count, errors.clone())?;
    }
    report(output, "download", count, errors)
}
/// Generate native crops from completed downloads. Reuse requires exact verified inputs.
pub fn review(queue: &Path, runtime: &Path, output: &Path) -> Result<(bool, String)> {
    let queue = initialize(queue, runtime, output)?;
    let mut count = 0;
    let mut errors = Vec::new();
    let mut tiles = Vec::new();
    for item in &queue.sources {
        let raw = completed(
            item,
            &output.join("downloads").join(format!("{}.json", item.id)),
        );
        for crop in &item.crops {
            let id = format!("{}--{}", item.id, crop.id);
            let result = (|| {
                let raw = raw
                    .as_ref()
                    .map_err(|e| super::AcquisitionError::Invalid(e.to_string()))?;
                let directory = output.join("reviews").join(&item.id).join(&crop.id);
                let manifest = directory.join("review.json");
                if !directory.exists() {
                    super::create_review(raw, &directory, crop.window_cells)?;
                }
                let verified = bundle::verify(raw, &manifest)?;
                ensure(
                    verified.manifest.window_cells == crop.window_cells,
                    "existing review window differs",
                )?;
                let review = io_context("resolve review", fs::canonicalize(manifest))?;
                Ok::<_, super::AcquisitionError>(
                    json!({"id":id,"title":crop.title,"raw":raw,"review":review,"primary_class_id":crop.primary_class_id}),
                )
            })();
            match result {
                Ok(tile) => {
                    count += 1;
                    tiles.push(tile)
                }
                Err(e) => errors.push(json!({"id":id,"error":e.to_string()})),
            }
        }
    }
    status(
        &output.join("gallery-input.json"),
        &json!({"schema_version":1,"title":"Terrain source review","tiles":tiles}),
    )?;
    report(output, "review", count, errors)
}
