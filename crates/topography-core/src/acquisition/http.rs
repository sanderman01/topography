use super::{
    AcquisitionError, Result, digest, ensure, hash_file, io_context, read_json, write_new,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path, process::Command};
use tempfile::NamedTempFile;

const CHUNK_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Identity {
    pub url: String,
    pub etag: String,
    pub bytes: u64,
}

fn curl() -> Command {
    let mut command = Command::new("curl");
    command.args([
        "--disable",
        "--fail",
        "--silent",
        "--show-error",
        "--globoff",
        "--disallow-username-in-url",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--location",
        "--max-redirs",
        "3",
        "--connect-timeout",
        "15",
        "--max-time",
        "120",
        "--header",
        "Accept-Encoding: identity",
    ]);
    command
}

/// Fetch a small public metadata document without exposing per-file orchestration.
pub(super) fn metadata(url: &str, output: &Path) -> Result<()> {
    validate_url(url)?;
    let bytes = run(curl().args(["--max-filesize", "1048576", "--retry", "2", "--url", url]))?;
    ensure(!bytes.is_empty(), "empty original metadata")?;
    write_new(output, &bytes)
}

pub(super) fn validate_url(url: &str) -> Result<()> {
    let authority = url
        .strip_prefix("https://")
        .and_then(|u| u.split('/').next())
        .unwrap_or("");
    ensure(
        !authority.is_empty()
            && !authority.contains('@')
            && !url.contains(['?', '#'])
            && !url.chars().any(char::is_whitespace),
        "asset URL must be HTTPS without credentials, query parameters, fragments or whitespace",
    )
}

fn run(command: &mut Command) -> Result<Vec<u8>> {
    let output = super::bounded_output(command)?;
    if !output.status.success() {
        let status = headers(&output.stdout)
            .map(|(status, _)| status)
            .unwrap_or(0);
        let transient = matches!(
            output.status.code(),
            Some(5 | 6 | 7 | 18 | 28 | 35 | 52 | 55 | 56)
        ) || matches!(status, 408 | 429 | 500 | 502 | 503 | 504);
        return Err(AcquisitionError::Transfer {
            transient,
            message: format!(
                "curl failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            ),
        });
    }
    ensure(
        output.stdout.len() <= 1024 * 1024,
        "HTTP headers exceed 1 MiB",
    )?;
    Ok(output.stdout)
}

fn headers(bytes: &[u8]) -> Result<(u16, BTreeMap<String, String>)> {
    let mut status = 0;
    let mut fields = BTreeMap::new();
    for line in String::from_utf8_lossy(bytes).lines() {
        if line.starts_with("HTTP/") {
            status = line
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            fields.clear();
        } else if let Some((key, value)) = line.split_once(':') {
            fields.insert(key.to_ascii_lowercase(), value.trim().into());
        }
    }
    ensure(status != 0, "missing HTTP response status")?;
    Ok((status, fields))
}

pub(super) fn identify(url: &str, max_bytes: u64) -> Result<Identity> {
    validate_url(url)?;
    let (status, fields) = headers(&run(curl().args(["--head", "--url", url]))?)?;
    ensure(status == 200, "asset HEAD did not return HTTP 200")?;
    let size = fields
        .get("content-length")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    ensure(
        size > 0 && size <= max_bytes,
        format!("asset size {size} bytes is missing, zero, or exceeds limit {max_bytes}"),
    )?;
    let etag = fields.get("etag").cloned().unwrap_or_default();
    ensure(
        etag.starts_with('"') && etag.ends_with('"') && !etag.contains(['\r', '\n']),
        "acquisition requires a strong HTTP ETag; use manual import for other servers",
    )?;
    Ok(Identity {
        url: url.into(),
        etag,
        bytes: size,
    })
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Chunk {
    sha256: String,
    bytes: u64,
}

/// Persist complete range responses only. Interrupted chunks are re-requested;
/// completed chunks are checked and reused under the exact URL/ETag/size identity.
pub(super) fn download(identity: &Identity, staging: &Path, destination: &Path) -> Result<()> {
    use std::io::Write;
    let identity_bytes = serde_json::to_vec(identity)?;
    let directory = staging.join("http").join(digest(&identity_bytes));
    io_context("create download staging", fs::create_dir_all(&directory))?;
    write_new(&directory.join("identity.json"), &identity_bytes)?;
    let mut assembled = io_context("create assembled asset", fs::File::create(destination))?;
    let mut start = 0;
    while start < identity.bytes {
        let count = CHUNK_BYTES.min(identity.bytes - start);
        let end = start + count - 1;
        let chunk_path = directory.join(format!("{start}.chunk"));
        let receipt_path = directory.join(format!("{start}.json"));
        if receipt_path.exists() {
            let receipt: Chunk = read_json(&receipt_path)?;
            let (sha, bytes) = hash_file(&chunk_path)?;
            ensure(
                receipt.bytes == count && bytes == count && receipt.sha256 == sha,
                format!(
                    "corrupt staged chunk at {}; preserve it and use a new staging root to retry",
                    chunk_path.display()
                ),
            )?;
        } else {
            let temp = io_context("stage range response", NamedTempFile::new_in(&directory))?;
            let mut failure = None;
            // Bounded retries use a fresh output file; curl never appends to unverified bytes.
            for attempt in 0..3 {
                let response = run(curl()
                    .args([
                        "--range",
                        &format!("{start}-{end}"),
                        "--header",
                        &format!("If-Match: {}", identity.etag),
                        "--max-filesize",
                        &count.to_string(),
                        "--dump-header",
                        "-",
                    ])
                    .arg("--output")
                    .arg(temp.path())
                    .args(["--url", &identity.url]));
                match response {
                    Ok(bytes) => {
                        let (status, fields) = headers(&bytes)?;
                        ensure(
                            status == 206
                                && fields.get("content-range")
                                    == Some(&format!("bytes {start}-{end}/{}", identity.bytes))
                                && fields.get("etag") == Some(&identity.etag),
                            "range response changed identity or did not exactly match the requested bytes",
                        )?;
                        failure = None;
                        break;
                    }
                    Err(error) => {
                        let transient = matches!(
                            &error,
                            AcquisitionError::Transfer {
                                transient: true,
                                ..
                            }
                        );
                        failure = Some(error);
                        if !transient {
                            break;
                        }
                        if attempt < 2 {
                            std::thread::sleep(std::time::Duration::from_secs(1 << attempt));
                        }
                    }
                }
            }
            if let Some(error) = failure {
                return Err(error);
            }
            let (sha256, bytes) = hash_file(temp.path())?;
            ensure(bytes == count, "incomplete range response")?;
            io_context("sync staged chunk", temp.as_file().sync_all())?;
            match temp.persist_noclobber(&chunk_path) {
                Ok(_) => (),
                Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {
                    ensure(
                        hash_file(&chunk_path)? == (sha256.clone(), bytes),
                        "existing staged chunk differs",
                    )?;
                }
                Err(e) => {
                    return Err(AcquisitionError::Io {
                        context: "publish staged chunk".into(),
                        source: e.error,
                    });
                }
            }
            write_new(
                &receipt_path,
                &serde_json::to_vec(&Chunk { sha256, bytes })?,
            )?;
        }
        let mut chunk = io_context("open verified chunk", fs::File::open(&chunk_path))?;
        io_context(
            "assemble verified chunk",
            std::io::copy(&mut chunk, &mut assembled),
        )?;
        start += count;
    }
    io_context("flush assembled asset", assembled.flush())?;
    io_context("sync assembled asset", assembled.sync_all())?;
    Ok(())
}

pub(super) fn discover(bounds: [f64; 4], limit: u32, output: &Path) -> Result<()> {
    let bbox = bounds.map(|v| v.to_string()).join(",");
    run(curl()
        .args([
            "--get",
            "--max-filesize",
            "1048576",
            "--retry",
            "2",
            "--retry-max-time",
            "45",
            "--url",
            "https://tnmaccess.nationalmap.gov/api/v1/products",
            "--data-urlencode",
            "datasets=Digital Elevation Model (DEM) 1 meter",
            "--data-urlencode",
            "prodFormats=GeoTIFF",
            "--data-urlencode",
            "outputFormat=JSON",
            "--data-urlencode",
            &format!("bbox={bbox}"),
            "--data-urlencode",
            &format!("max={limit}"),
        ])
        .arg("--output")
        .arg(output))?;
    Ok(())
}
