//! Sequential gallery jobs. Curation remains authoritative; cached status is advisory.
use super::{bundle, gallery::Gallery};
use crate::{
    acquisition::{Result, digest, ensure, io_context, read_json, read_small},
    catalog::curation,
    config,
    preparation::automatic,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

pub(crate) fn package_root(runtime: &Path, review: &str, profile: &str) -> Result<PathBuf> {
    let root = config::inspect_runtime(runtime)?
        .paths
        .into_iter()
        .find(|(key, _)| *key == "review")
        .ok_or_else(|| {
            crate::acquisition::AcquisitionError::Invalid("review storage missing".into())
        })?
        .1;
    Ok(root
        .join("automatic-preparation")
        .join(review)
        .join(profile))
}

#[derive(Clone)]
struct Status {
    head: Option<String>,
    value: Value,
    parents: Vec<(String, String)>,
}
impl Status {
    fn new(head: Option<String>, state: &str, message: &str) -> Self {
        Self {
            head,
            value: json!({"state":state,"message":message,"manifest":null}),
            parents: vec![],
        }
    }
    fn ready(head: Option<String>, manifest: &Path) -> Result<Self> {
        let body: Value = read_json(manifest)?;
        let mut result = if body["schema_version"] == 2 && automatic::current_cover(manifest) {
            Self::new(
                head,
                "ready",
                "Prepared terrain and surface masks are ready.",
            )
        } else {
            Self::new(
                head,
                "assessment_pending",
                "Existing terrain is prepared. Game water, snow/ice and vegetation are pending; your choice is saved.",
            )
        };
        result.value["manifest"] = json!(manifest);
        for parent in body["parents"].as_array().ok_or_else(|| {
            crate::acquisition::AcquisitionError::Invalid("prepared parents missing".into())
        })? {
            result.parents.push((
                parent["review_sha256"].as_str().unwrap_or("").into(),
                parent["curation_record_sha256"]
                    .as_str()
                    .unwrap_or("")
                    .into(),
            ));
        }
        Ok(result)
    }
}

pub(crate) struct Worker {
    states: Arc<Mutex<Vec<Status>>>,
    stop: Arc<AtomicBool>,
    runtime: PathBuf,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
impl Worker {
    pub(crate) fn state(
        &self,
        index: usize,
        selection: &Value,
        heads: &BTreeMap<String, Option<String>>,
    ) -> Value {
        if selection["decision"] == "reject" {
            return Status::new(None, "not_selected", "Rejected; source data is preserved.").value;
        }
        if let Ok(states) = self.states.lock() {
            let state = &states[index];
            let head = selection["head"].as_str();
            let parents_current = parents_current(&self.runtime, &state.parents, heads);
            if state.head.as_deref() == head && parents_current {
                return state.value.clone();
            }
        }
        Status::new(
            None,
            if selection["decision"] == "accept" {
                "preparing"
            } else {
                "checking"
            },
            "Queued for technical checks.",
        )
        .value
    }
}

fn parents_current(
    runtime: &Path,
    parents: &[(String, String)],
    heads: &BTreeMap<String, Option<String>>,
) -> bool {
    parents.iter().all(|(review, expected)| {
        if let Some(actual) = heads.get(review) {
            actual.as_deref() == Some(expected.as_str())
        } else {
            curation::current_head(runtime, review)
                .is_ok_and(|actual| actual.as_deref() == Some(expected.as_str()))
        }
    })
}

fn publish(states: &Mutex<Vec<Status>>, index: usize, status: Status, path: &Path) {
    let bytes = serde_json::to_vec_pretty(
        &json!({"schema_version":1,"head":status.head,"preparation":status.value}),
    );
    if let Ok(mut states) = states.lock() {
        states[index] = status;
    }
    let saved = (|| -> Result<()> {
        let parent = path.parent().unwrap_or(Path::new("."));
        io_context("create job status storage", fs::create_dir_all(parent))?;
        let mut temp = io_context("stage job status", tempfile::NamedTempFile::new_in(parent))?;
        io_context("write job status", temp.write_all(&bytes?))?;
        io_context("sync job status", temp.as_file().sync_all())?;
        temp.persist(path)
            .map_err(|e| crate::acquisition::AcquisitionError::Io {
                context: "publish job status".into(),
                source: e.error,
            })?;
        Ok(())
    })();
    if let Err(error) = saved {
        eprintln!("Preparation status: {error}");
    }
}

pub(crate) fn start(
    runtime: &Path,
    legend: &Path,
    gallery: Arc<Gallery>,
    snapshots: Arc<Vec<bundle::Bundle>>,
) -> Result<Worker> {
    let states = Arc::new(Mutex::new(
        (0..gallery.tiles.len())
            .map(|_| Status::new(None, "checking", "Queued for technical checks."))
            .collect::<Vec<_>>(),
    ));
    let stop = Arc::new(AtomicBool::new(false));
    let worker = Worker {
        states: states.clone(),
        stop: stop.clone(),
        runtime: runtime.to_path_buf(),
    };
    let runtime = runtime.to_path_buf();
    let legend = legend.to_path_buf();
    io_context(
        "start preparation worker",
        thread::Builder::new()
            .name("terrain-preparation".into())
            .spawn(move || {
                let result = work(&runtime, &legend, &gallery, &snapshots, &states, &stop);
                if let Err(error) = result {
                    eprintln!("Preparation worker: {error}");
                    if let Ok(mut states) = states.lock() {
                        for state in states.iter_mut() {
                            *state = Status::new(
                                state.head.clone(),
                                "needs_attention",
                                &error.to_string(),
                            );
                        }
                    }
                }
            }),
    )?;
    Ok(worker)
}

fn work(
    runtime: &Path,
    legend: &Path,
    gallery: &Gallery,
    snapshots: &[bundle::Bundle],
    states: &Mutex<Vec<Status>>,
    stop: &AtomicBool,
) -> Result<()> {
    let mut finished: Vec<Option<String>> = vec![None; gallery.tiles.len()];
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let selections = gallery
            .tiles
            .iter()
            .zip(snapshots)
            .map(|(tile, snapshot)| curation::selection(runtime, &tile.review_sha256, snapshot))
            .collect::<Result<Vec<_>>>()?;
        let heads: BTreeMap<_, _> = gallery
            .tiles
            .iter()
            .zip(&selections)
            .map(|(tile, selection)| {
                (
                    tile.review_sha256.clone(),
                    selection["head"].as_str().map(str::to_owned),
                )
            })
            .collect();
        let mut worked = false;
        // Accepted work takes priority over checks for undecided tiles.
        let mut order = (0..gallery.tiles.len()).collect::<Vec<_>>();
        order.sort_by_key(|index| selections[*index]["decision"] != "accept");
        for index in order {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            let tile = &gallery.tiles[index];
            let selection = &selections[index];
            let head = selection["head"].as_str().map(str::to_owned);
            let key = serde_json::to_string(selection)?;
            let parents_current = states
                .lock()
                .map(|s| parents_current(runtime, &s[index].parents, &heads))
                .unwrap_or(false);
            if finished[index].as_deref() == Some(&key) && parents_current {
                continue;
            }
            let profile_id = tile
                .preparation_profile
                .as_ref()
                .map(|p| p.sha256.as_str())
                .unwrap_or("unconfigured");
            let root = package_root(runtime, &tile.review_sha256, profile_id)?;
            let status_path = root.join("status.json");
            if selection["decision"] == "reject" {
                publish(
                    states,
                    index,
                    Status::new(head, "not_selected", "Rejected; source data is preserved."),
                    &status_path,
                );
                finished[index] = Some(key);
                continue;
            }
            let accepted = selection["decision"] == "accept";
            publish(
                states,
                index,
                Status::new(
                    head.clone(),
                    if accepted { "preparing" } else { "checking" },
                    "Checking source evidence and native terrain.",
                ),
                &status_path,
            );
            let result = (|| -> Result<Status> {
                let cache = root.parent();
                // Build the current game cover product from an older product (or none),
                // keeping the accepted vote; then report the latest choice.
                let upgrade = |legacy: Option<(PathBuf, String)>| -> Result<Status> {
                    let (legacy, expected) = match legacy {
                        Some((path, head)) => (Some(path), head),
                        None => (None, head.clone().unwrap_or_default()),
                    };
                    let manifest = automatic::upgrade(
                        runtime,
                        &tile.raw,
                        &tile.review,
                        legacy.as_deref(),
                        &expected,
                        &root.join("cover-upgrade"),
                        cache,
                    )?;
                    let latest =
                        curation::selection(runtime, &tile.review_sha256, &snapshots[index])?;
                    ensure(
                        latest["decision"] == "accept",
                        "Selection changed during preparation.",
                    )?;
                    Status::ready(latest["head"].as_str().map(str::to_owned), &manifest)
                };
                if accepted {
                    if let Some(manifest) = automatic::existing(runtime, &tile.review_sha256)?
                        && automatic::current_cover(&manifest)
                    {
                        return Status::ready(head.clone(), &manifest);
                    }
                    // Older products gain game cover without a profile or a new vote.
                    if let Some(legacy) = automatic::legacy(runtime, &tile.review_sha256)? {
                        return upgrade(Some(legacy));
                    }
                }
                let Some(profile) = tile.preparation_profile.as_ref() else {
                    if accepted {
                        return upgrade(None).map_err(|e| {
                            crate::acquisition::AcquisitionError::Invalid(format!(
                                "Source assessment is not configured for this tile: {e}"
                            ))
                        });
                    }
                    return Err(crate::acquisition::AcquisitionError::Invalid(
                        "Source assessment is not configured for this tile.".into(),
                    ));
                };
                ensure(
                    digest(&read_small(&profile.path)?) == profile.sha256,
                    "Source assessment changed; publish an updated gallery.",
                )?;
                let package = automatic::stage(&tile.raw, &tile.review, &profile.path, &root)?;
                if !accepted {
                    return Ok(Status::new(
                        head.clone(),
                        "awaiting_acceptance",
                        "Technical checks passed; waiting for your choice.",
                    ));
                }
                let manifest =
                    automatic::prepare(runtime, &package, legend, head.as_deref().unwrap_or(""))?;
                if !automatic::current_cover(&manifest) {
                    // A profile with its own cover assessment still gets game cover.
                    let legacy =
                        automatic::legacy(runtime, &tile.review_sha256)?.ok_or_else(|| {
                            crate::acquisition::AcquisitionError::Invalid(
                                "Prepared product has no game cover successor path.".into(),
                            )
                        })?;
                    return upgrade(Some(legacy));
                }
                let latest = curation::selection(runtime, &tile.review_sha256, &snapshots[index])?;
                ensure(
                    latest["decision"] == "accept",
                    "Selection changed during preparation.",
                )?;
                Status::ready(latest["head"].as_str().map(str::to_owned), &manifest)
            })();
            let latest = curation::selection(runtime, &tile.review_sha256, &snapshots[index])?;
            let latest_head = latest["head"].as_str().map(str::to_owned);
            let status = match result {
                Ok(status) if status.head == latest_head => status,
                Ok(_) => {
                    finished[index] = None;
                    continue;
                }
                Err(_) if latest["decision"] == "reject" => Status::new(
                    latest_head,
                    "not_selected",
                    "Rejected; source data is preserved.",
                ),
                Err(error) => Status::new(latest_head, "needs_attention", &error.to_string()),
            };
            publish(states, index, status, &status_path);
            finished[index] = Some(serde_json::to_string(&latest)?);
            worked = true;
            break;
        }
        if !worked {
            thread::sleep(Duration::from_millis(500));
        }
    }
}
