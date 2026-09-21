//! Single-user loopback gallery. Only explicit, same-origin selections mutate curation.
use super::gallery::{self, Gallery};
use crate::{
    acquisition::{AcquisitionError, Result, digest, ensure, hash_file, io_context},
    catalog::curation,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

struct Request {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}
fn request(stream: &mut TcpStream) -> Result<Request> {
    io_context(
        "set read timeout",
        stream.set_read_timeout(Some(Duration::from_secs(2))),
    )?;
    io_context(
        "set write timeout",
        stream.set_write_timeout(Some(Duration::from_secs(5))),
    )?;
    let start = Instant::now();
    let mut bytes = Vec::new();
    let mut part = [0u8; 1024];
    let split = loop {
        ensure(start.elapsed() < Duration::from_secs(5), "request timeout")?;
        let n = io_context("read request", stream.read(&mut part))?;
        ensure(n > 0, "incomplete request")?;
        bytes.extend_from_slice(&part[..n]);
        ensure(bytes.len() <= 16384, "request too large")?;
        if let Some(index) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break index + 4;
        }
    };
    ensure(split <= 8192, "headers too large")?;
    let text = std::str::from_utf8(&bytes[..split])
        .map_err(|_| AcquisitionError::Invalid("invalid request headers".into()))?;
    let mut lines = text.split("\r\n");
    let first = lines
        .next()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>();
    ensure(
        first.len() == 3 && first[2] == "HTTP/1.1",
        "HTTP/1.1 required",
    )?;
    let method = first[0].to_owned();
    let path = first[1].to_owned();
    let mut headers = BTreeMap::new();
    for line in lines.filter(|s| !s.is_empty()) {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| AcquisitionError::Invalid("malformed header".into()))?;
        ensure(
            !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
            "invalid header",
        )?;
        ensure(
            headers
                .insert(name.to_ascii_lowercase(), value.trim().to_owned())
                .is_none(),
            "duplicate header",
        )?;
    }
    ensure(
        !headers.contains_key("transfer-encoding"),
        "chunked requests unsupported",
    )?;
    let length = match headers.get("content-length") {
        Some(s) => s
            .parse::<usize>()
            .map_err(|_| AcquisitionError::Invalid("invalid content length".into()))?,
        None => 0,
    };
    ensure(length <= 4096, "request body too large")?;
    ensure(bytes.len() <= split + length, "extra request bytes")?;
    while bytes.len() < split + length {
        ensure(start.elapsed() < Duration::from_secs(5), "request timeout")?;
        let remaining = (split + length - bytes.len()).min(part.len());
        let n = io_context("read request body", stream.read(&mut part[..remaining]))?;
        ensure(n > 0, "incomplete body")?;
        bytes.extend_from_slice(&part[..n]);
    }
    Ok(Request {
        method,
        path,
        headers,
        body: bytes[split..].to_vec(),
    })
}
fn respond(stream: &mut TcpStream, code: u16, kind: &str, bytes: &[u8]) -> Result<()> {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        _ => "Error",
    };
    let headers = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src 'self' data:; frame-src 'self'; connect-src 'self'; frame-ancestors 'self'; object-src 'none'; base-uri 'none'\r\n\r\n",
        bytes.len()
    );
    io_context("write response", stream.write_all(headers.as_bytes()))?;
    io_context("write response body", stream.write_all(bytes))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Vote {
    index: usize,
    gallery_sha256: String,
    review_sha256: String,
    previous_record_sha256: Option<String>,
    decision: String,
}
struct Session {
    gallery: Gallery,
    snapshots: Vec<super::bundle::Bundle>,
    id: String,
    root: PathBuf,
    runtime: PathBuf,
    authority: String,
    token: String,
}
impl Session {
    fn state(&self) -> Result<Value> {
        let mut tiles = Vec::new();
        for (index, tile) in self.gallery.tiles.iter().enumerate() {
            let state =
                curation::selection(&self.runtime, &tile.review_sha256, &self.snapshots[index])?;
            tiles.push(json!({"index":index,"title":tile.title,"id":tile.id,"primary_class_id":tile.primary_class_id,"review_sha256":tile.review_sha256,"head":state["head"],"decision":state["decision"]}));
        }
        Ok(json!({"title":self.gallery.title,"gallery_sha256":self.id,"tiles":tiles}))
    }
    fn handle(&self, stream: &mut TcpStream, request: Request) -> Result<()> {
        ensure(
            request.headers.get("host") == Some(&self.authority),
            "invalid Host",
        )?;
        if request.method == "GET" && request.body.is_empty() {
            if request.path == "/" {
                let config = serde_json::to_string(&json!({"token":self.token}))?;
                let html = include_str!("gallery.html").replace("__GALLERY_CONFIG__", &config);
                return respond(stream, 200, "text/html; charset=utf-8", html.as_bytes());
            }
            if request.path == "/api/tiles" {
                return respond(
                    stream,
                    200,
                    "application/json",
                    &serde_json::to_vec(&self.state()?)?,
                );
            }
            for (prefix, name, kind) in [
                ("/preview/", "preview", "text/html; charset=utf-8"),
                ("/thumb/", "thumb", "image/png"),
            ] {
                if let Some(suffix) = request.path.strip_prefix(prefix) {
                    if let Ok(index) = suffix.parse::<usize>() {
                        if let Some(tile) = self.gallery.tiles.get(index) {
                            let ext = if name == "preview" { "html" } else { "png" };
                            let path = self.root.join(format!("{name}-{index}.{ext}"));
                            let (expected, limit) = if name == "preview" {
                                (&tile.preview_sha256, 128 * 1024 * 1024)
                            } else {
                                (&tile.thumbnail_sha256, 1024 * 1024)
                            };
                            ensure(
                                hash_file(&path)?.0 == *expected,
                                "gallery display integrity mismatch",
                            )?;
                            let file = io_context("open display", fs::File::open(path))?;
                            let mut bytes = Vec::new();
                            io_context(
                                "read display",
                                file.take(limit + 1).read_to_end(&mut bytes),
                            )?;
                            ensure(
                                bytes.len() as u64 <= limit && digest(&bytes) == *expected,
                                "gallery display changed",
                            )?;
                            if name == "preview" {
                                let identity = json!({"index":index,"review":tile.review_sha256});
                                let bridge = format!(
                                    r#"<style>body>.tag,body>h1,body>#identity,body>.note{{display:none}}body{{padding:12px}}</style><script>const galleryIdentity={identity};parent.postMessage({{...galleryIdentity,ready:true}},'*');addEventListener('keydown',e=>{{if(['y','n','ArrowLeft','ArrowRight'].includes(e.key)&&!e.repeat&&!e.ctrlKey&&!e.altKey&&!e.metaKey&&!['INPUT','TEXTAREA','SELECT'].includes(e.target.tagName)){{e.preventDefault();parent.postMessage({{...galleryIdentity,key:e.key}},'*');}}}});</script>"#
                                );
                                bytes.extend_from_slice(bridge.as_bytes());
                            }
                            return respond(stream, 200, kind, &bytes);
                        }
                    }
                }
            }
            return respond(stream, 404, "text/plain", b"Not found");
        }
        if request.method == "POST" && request.path == "/api/decision" {
            ensure(
                request.headers.get("origin") == Some(&format!("http://{}", self.authority))
                    && request.headers.get("x-topography-token") == Some(&self.token)
                    && request
                        .headers
                        .get("content-type")
                        .is_some_and(|s| s == "application/json"),
                "same-origin JSON and session token required",
            )?;
            let vote: Vote = serde_json::from_slice(&request.body)?;
            let result = (|| {
                ensure(vote.gallery_sha256 == self.id, "gallery identity mismatch")?;
                let tile = self
                    .gallery
                    .tiles
                    .get(vote.index)
                    .ok_or_else(|| AcquisitionError::Invalid("unknown tile".into()))?;
                ensure(
                    vote.review_sha256 == tile.review_sha256,
                    "review identity mismatch",
                )?;
                ensure(
                    hash_file(&self.root.join(format!("preview-{}.html", vote.index)))?.0
                        == tile.preview_sha256,
                    "reviewed preview changed",
                )?;
                ensure(
                    digest(&crate::acquisition::read_small(
                        &self.root.join("legend.toml"),
                    )?) == self.gallery.legend_sha256,
                    "gallery legend changed",
                )?;
                curation::select(
                    &self.runtime,
                    tile,
                    &self.root.join("legend.toml"),
                    vote.previous_record_sha256,
                    &vote.decision,
                    &self.id,
                    &self.snapshots[vote.index],
                )
            })();
            return match result {
                Ok(value) => respond(
                    stream,
                    200,
                    "application/json",
                    &serde_json::to_vec(&value)?,
                ),
                Err(e) => respond(
                    stream,
                    409,
                    "application/json",
                    &serde_json::to_vec(&json!({"error":e.to_string()}))?,
                ),
            };
        }
        respond(stream, 404, "text/plain", b"Not found")
    }
}
/// Serve a gallery until interrupted. Port zero chooses an available loopback port.
pub fn serve(runtime: &Path, manifest: &Path, port: u16) -> Result<()> {
    let (gallery, id, snapshots) = gallery::load(manifest)?;
    crate::config::inspect_runtime(runtime)?;
    let listener = io_context(
        "bind gallery loopback",
        TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)),
    )?;
    let authority = io_context("read gallery address", listener.local_addr())?.to_string();
    let mut entropy = [0u8; 32];
    io_context(
        "read gallery session entropy",
        fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut entropy)),
    )?;
    let session = Session {
        gallery,
        snapshots,
        id,
        root: manifest.parent().unwrap_or(Path::new(".")).into(),
        runtime: runtime.into(),
        authority,
        token: digest(&entropy),
    };
    // Fail before announcing a usable server if the existing catalog is damaged.
    session.state()?;
    println!("http://{}/", session.authority);
    io_context("flush gallery URL", std::io::stdout().flush())?;
    for stream in listener.incoming() {
        let mut stream = io_context("accept gallery connection", stream)?;
        let result = request(&mut stream).and_then(|r| session.handle(&mut stream, r));
        if let Err(error) = result {
            let _ = respond(
                &mut stream,
                400,
                "application/json",
                &serde_json::to_vec(&json!({"error":error.to_string()}))?,
            );
        }
    }
    Ok(())
}
