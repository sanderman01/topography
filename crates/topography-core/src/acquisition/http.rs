use super::{
    AcquisitionError, Result, digest, ensure, hash_file, io_context, read_json, write_new,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    process::Command,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
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

/// Fetch a small public JSON response from an HTTPS query URL, bounded to 1 MiB.
pub(super) fn query(url: &str) -> Result<Vec<u8>> {
    let authority = url
        .strip_prefix("https://")
        .and_then(|u| u.split(['/', '?']).next())
        .unwrap_or("");
    ensure(
        !authority.is_empty()
            && !authority.contains('@')
            && !url.contains('#')
            && url.bytes().all(|b| b.is_ascii_graphic()),
        "query URL must be HTTPS without credentials, fragments or whitespace",
    )?;
    let bytes = run(curl().args(["--max-filesize", "1048576", "--retry", "2", "--url", url]))?;
    ensure(!bytes.is_empty(), "empty query response")?;
    Ok(bytes)
}

/// Like [`query`], but a missing object (HTTP 403/404) is `None`.
pub(super) fn query_optional(url: &str) -> Result<Option<Vec<u8>>> {
    match query(url) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(AcquisitionError::Transfer { message, .. })
            if message.contains("returned error: 404")
                || message.contains("returned error: 403") =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// Fetch a public HTTPS query response of up to 64 MiB with retries and a User-Agent.
pub(super) fn query_large(url: &str, user_agent: &str, max_time_s: u64) -> Result<Vec<u8>> {
    ensure(
        url.starts_with("https://")
            && !url.contains('#')
            && url.bytes().all(|b| b.is_ascii_graphic()),
        "query URL must be HTTPS without fragments or whitespace",
    )?;
    let temp = io_context("stage query response", NamedTempFile::new())?;
    let time = max_time_s.to_string();
    run(curl()
        .args([
            "--max-filesize",
            "67108864",
            // Exponential backoff (honouring Retry-After) rides out rate limits.
            "--retry",
            "8",
            "--retry-connrefused",
            "--max-time",
            &time,
            "--user-agent",
            user_agent,
            "--output",
        ])
        .arg(temp.path())
        .args(["--url", url]))?;
    let bytes = io_context("read query response", fs::read(temp.path()))?;
    ensure(!bytes.is_empty(), "empty query response")?;
    Ok(bytes)
}

/// Final HTTPS URL after redirects, for example a dated file behind a "latest" link.
pub(super) fn resolve(url: &str) -> Result<String> {
    let mut current = url.to_owned();
    for _ in 0..4 {
        validate_url(&current)?;
        let out = run(curl().args([
            "--no-location",
            "--head",
            "--retry",
            "3",
            "--output",
            "/dev/null",
            "--write-out",
            "%{redirect_url}",
            "--url",
            &current,
        ]))?;
        let next = String::from_utf8_lossy(&out).trim().to_owned();
        if next.is_empty() {
            return Ok(current);
        }
        // Some mirrors redirect to plain HTTP on their own host; stay on HTTPS.
        let host = |u: &str| {
            u.split("://")
                .nth(1)
                .and_then(|r| r.split('/').next())
                .map(str::to_owned)
        };
        current = match next.strip_prefix("http://") {
            Some(rest) if host(&next) == host(&current) => format!("https://{rest}"),
            _ => next,
        };
    }
    Err(AcquisitionError::Invalid(format!(
        "too many redirects from {url}"
    )))
}

/// HEAD an HTTPS object with retries: missing (403/404) is `false`, other failures are errors.
pub(super) fn exists(url: &str) -> Result<bool> {
    validate_url(url)?;
    match run(curl().args(["--head", "--retry", "3", "--url", url])) {
        Ok(_) => Ok(true),
        Err(AcquisitionError::Transfer { message, .. })
            if message.contains("returned error: 404")
                || message.contains("returned error: 403") =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
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

/// Minimum spacing between requests to one host. Overpass is a shared volunteer
/// service that bans bursts; S3 buckets serve range requests at scale; loopback
/// test servers need none.
fn min_interval(host: &str) -> Duration {
    if host == "localhost" || host.starts_with("127.") {
        Duration::ZERO
    } else if host.contains("overpass") {
        Duration::from_secs(10)
    } else if host.ends_with(".amazonaws.com") {
        Duration::from_millis(100)
    } else {
        Duration::from_secs(1)
    }
}

/// Wait until this host's minimum interval has passed since the last request by any
/// topography process (a timestamp file per host in the temp directory; the rare
/// race between processes only costs one early request).
pub(super) fn pace(url: &str) {
    static LOCK: Mutex<()> = Mutex::new(());
    let Some(host) = url
        .strip_prefix("https://")
        .and_then(|u| u.split(['/', '?', ':']).next())
        .filter(|h| {
            !h.is_empty()
                && h.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-.".contains(&b))
        })
    else {
        return;
    };
    let _guard = LOCK.lock();
    let dir = std::env::temp_dir().join("topography-http-pace");
    let file = dir.join(host);
    let now = || {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64)
    };
    let last = fs::read_to_string(&file)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let wait = (last + min_interval(host).as_millis() as u64).saturating_sub(now());
    if wait > 0 {
        std::thread::sleep(Duration::from_millis(wait.min(60_000)));
    }
    let _ = fs::create_dir_all(&dir).and_then(|_| fs::write(&file, now().to_string()));
}

fn run(command: &mut Command) -> Result<Vec<u8>> {
    if let Some(url) = command
        .get_args()
        .filter_map(|a| a.to_str())
        .find(|a| a.starts_with("https://"))
    {
        pace(url);
    }
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
                        // An 8 MiB range can take over two minutes on a healthy slow link.
                        "--max-time",
                        "300",
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
