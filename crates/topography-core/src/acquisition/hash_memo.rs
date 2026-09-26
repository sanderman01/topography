//! Opt-in memo of file digests for long-running local services. A digest is reused
//! only while the file keeps its inode, size, mtime and ctime; any write or
//! replacement changes ctime. `curate check` never enables it, so it still re-reads
//! every byte to catch silent corruption.

use serde_json::{Map, Value, json};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::Mutex,
};

type Identity = [i64; 6];

struct Memo {
    path: PathBuf,
    entries: HashMap<String, (Identity, String, u64)>,
    dirty: bool,
}

static MEMO: Mutex<Option<Memo>> = Mutex::new(None);

fn identity(meta: &fs::Metadata) -> Identity {
    [
        meta.ino() as i64,
        meta.size() as i64,
        meta.mtime(),
        meta.mtime_nsec(),
        meta.ctime(),
        meta.ctime_nsec(),
    ]
}

/// Enable the memo for this process, loading earlier digests from `path`.
pub fn enable(path: &Path) {
    let mut entries = HashMap::new();
    if let Some(Value::Object(saved)) = fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
    {
        for (file, entry) in saved {
            if let Ok((id, sha, len)) = serde_json::from_value(entry) {
                entries.insert(file, (id, sha, len));
            }
        }
    }
    if let Ok(mut memo) = MEMO.lock() {
        *memo = Some(Memo {
            path: path.to_path_buf(),
            entries,
            dirty: false,
        });
    }
}

/// Persist new digests. Failures only cost time on the next start.
pub fn save() {
    let Ok(mut guard) = MEMO.lock() else { return };
    let Some(memo) = guard.as_mut().filter(|m| m.dirty) else {
        return;
    };
    let map: Map<String, Value> = memo
        .entries
        .iter()
        .map(|(file, (id, sha, len))| (file.clone(), json!([id, sha, len])))
        .collect();
    let written = (|| -> std::io::Result<()> {
        let parent = memo.path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(&serde_json::to_vec(&Value::Object(map))?)?;
        temp.persist(&memo.path).map(|_| ()).map_err(|e| e.error)
    })();
    if written.is_ok() {
        memo.dirty = false;
    }
}

pub(super) fn lookup(path: &Path, meta: &fs::Metadata) -> Option<(String, u64)> {
    let guard = MEMO.lock().ok()?;
    let (id, sha, len) = guard.as_ref()?.entries.get(path.to_str()?)?;
    (*id == identity(meta)).then(|| (sha.clone(), *len))
}

pub(super) fn remember(path: &Path, meta: &fs::Metadata, sha: &str, len: u64) {
    let (Ok(mut guard), Some(file)) = (MEMO.lock(), path.to_str()) else {
        return;
    };
    if let Some(memo) = guard.as_mut() {
        memo.entries
            .insert(file.to_owned(), (identity(meta), sha.to_owned(), len));
        memo.dirty = true;
    }
}
