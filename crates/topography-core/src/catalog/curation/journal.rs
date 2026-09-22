use super::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use std::time::Duration;

const APPLICATION_ID: i64 = 0x544f504f;
const MAX_RECORDS: usize = 4096;

pub(super) struct Store {
    pub db: PathBuf,
    root: PathBuf,
}

impl Store {
    pub fn object_path(&self, sha: &str) -> PathBuf {
        self.root.join("objects").join(sha)
    }
    pub fn locate(runtime: &Path) -> Result<Self> {
        let config = crate::config::inspect_runtime(runtime)?;
        let db = config
            .paths
            .into_iter()
            .find(|(key, _)| *key == "catalog_db")
            .map(|(_, path)| path)
            .ok_or_else(|| AcquisitionError::Invalid("catalog_db missing".into()))?;
        let mut root = db.as_os_str().to_os_string();
        root.push(".curation");
        let root = PathBuf::from(root);
        Ok(Self { db, root })
    }

    fn open(&self, write: bool) -> Result<Connection> {
        if write {
            io_context(
                "create catalog parent",
                fs::create_dir_all(self.db.parent().unwrap_or(Path::new("."))),
            )?;
        }
        let flags = if write {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        };
        let mut db = Connection::open_with_flags(&self.db, flags)?;
        db.busy_timeout(Duration::from_secs(5))?;
        if write {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let app: i64 = tx.pragma_query_value(None, "application_id", |r| r.get(0))?;
            let version: i64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
            let tables: i64 = tx.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )?;
            if app == 0 && version == 0 && tables == 0 {
                tx.execute_batch("CREATE TABLE records (id TEXT PRIMARY KEY, review_id TEXT NOT NULL, previous_id TEXT REFERENCES records(id), body BLOB NOT NULL, raw_path TEXT NOT NULL, review_path TEXT NOT NULL);
                    CREATE TABLE heads (review_id TEXT PRIMARY KEY, record_id TEXT NOT NULL UNIQUE REFERENCES records(id));
                    CREATE INDEX records_review ON records(review_id);
                    PRAGMA application_id=1414484047; PRAGMA user_version=1;")?;
            } else {
                ensure(
                    app == APPLICATION_ID && matches!(version, 1 | 2),
                    "unsupported or unrelated catalog database",
                )?;
            }
            tx.commit()?;
            db.pragma_update(None, "synchronous", "FULL")?;
        }
        ensure(
            db.pragma_query_value::<i64, _>(None, "application_id", |r| r.get(0))?
                == APPLICATION_ID
                && matches!(
                    db.pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0))?,
                    1 | 2
                ),
            "unsupported or unrelated catalog database",
        )?;
        db.pragma_update(None, "foreign_keys", true)?;
        Ok(db)
    }

    pub fn head(&self, review: &str) -> Result<Option<String>> {
        ensure(valid_digest(review), "invalid review digest")?;
        Self::head_in(&self.open(false)?, review)
    }

    pub fn register_prepared(
        &self,
        id: &str,
        body: &[u8],
        path: &Path,
        heads: &[(String, String)],
    ) -> Result<()> {
        ensure(
            valid_digest(id) && digest(body) == id && !heads.is_empty(),
            "invalid prepared identity",
        )?;
        let path = path
            .to_str()
            .ok_or_else(|| AcquisitionError::Invalid("prepared locator must be UTF-8".into()))?;
        let mut db = self.open(true)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version == 1 {
            tx.execute_batch("CREATE TABLE prepared_regions (id TEXT PRIMARY KEY, body BLOB NOT NULL, manifest_path TEXT NOT NULL); PRAGMA user_version=2;")?;
        }
        for (review, expected) in heads {
            ensure(
                Self::head_in(&tx, review)?.as_ref() == Some(expected),
                "stale curation head during prepared registration; output preserved unregistered",
            )?;
        }
        let existing: Option<Vec<u8>> = tx
            .query_row("SELECT body FROM prepared_regions WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(existing) = existing {
            ensure(existing == body, "prepared registry integrity mismatch")?;
        } else {
            let count: i64 =
                tx.query_row("SELECT count(*) FROM prepared_regions", [], |r| r.get(0))?;
            ensure(
                count < MAX_RECORDS as i64,
                "prepared catalog limit is 4096 regions",
            )?;
            tx.execute(
                "INSERT INTO prepared_regions (id, body, manifest_path) VALUES (?1, ?2, ?3)",
                params![id, body, path],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn prepared_registered(&self, id: &str, body: &[u8]) -> Result<bool> {
        let db = self.open(false)?;
        if db.pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0))? == 1 {
            return Ok(false);
        }
        let existing: Option<Vec<u8>> = db
            .query_row("SELECT body FROM prepared_regions WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(existing) = existing {
            ensure(existing == body, "prepared registry integrity mismatch")?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn head_in(db: &Connection, review: &str) -> Result<Option<String>> {
        Ok(db
            .query_row(
                "SELECT record_id FROM heads WHERE review_id=?1",
                [review],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn append(
        &self,
        record: &Record,
        objects: &BTreeMap<String, Vec<u8>>,
        raw: &Path,
        review: &Path,
    ) -> Result<String> {
        self.append_checked(record, objects, raw, review, None)
    }

    pub fn append_snapshot(
        &self,
        record: &Record,
        objects: &BTreeMap<String, Vec<u8>>,
        raw: &Path,
        review: &Path,
        snapshot: &bundle::Bundle,
    ) -> Result<String> {
        self.append_checked(record, objects, raw, review, Some(snapshot))
    }

    fn append_checked(
        &self,
        record: &Record,
        objects: &BTreeMap<String, Vec<u8>>,
        raw: &Path,
        review: &Path,
        snapshot: Option<&bundle::Bundle>,
    ) -> Result<String> {
        let bytes = json(record)?;
        let id = digest(&bytes);
        let mut db = self.open(true)?;
        let existing: Option<Vec<u8>> = db
            .query_row("SELECT body FROM records WHERE id=?1", [&id], |r| r.get(0))
            .optional()?;
        if let Some(existing) = existing {
            ensure(existing == bytes, "indexed record differs")?;
            self.history_checked(&record.target.review_sha256, snapshot)?;
            return Ok(id);
        }
        let head = Self::head_in(&db, &record.target.review_sha256)?;
        ensure(
            head == record.previous_record_sha256,
            "stale decision: create a fresh draft from the current head",
        )?;
        if head.is_some() {
            self.history_checked(&record.target.review_sha256, snapshot)?;
        }
        for (sha, content) in objects {
            write_new(&self.root.join("objects").join(sha), content)?;
        }
        // Files publish first. An interruption leaves unregistered records for check/retry.
        write_new(
            &self.root.join("records").join(format!("{id}.json")),
            &bytes,
        )?;
        let raw = raw.to_str().ok_or_else(|| {
            AcquisitionError::Invalid("catalog locator path must be UTF-8".into())
        })?;
        let review = review.to_str().ok_or_else(|| {
            AcquisitionError::Invalid("catalog locator path must be UTF-8".into())
        })?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count: i64 = tx.query_row("SELECT count(*) FROM records", [], |r| r.get(0))?;
        ensure(
            count < MAX_RECORDS as i64,
            "curation v1 catalog limit is 4096 records",
        )?;
        ensure(
            Self::head_in(&tx, &record.target.review_sha256)? == record.previous_record_sha256,
            "stale decision: head changed during publication; record remains unregistered",
        )?;
        tx.execute("INSERT INTO records (id,review_id,previous_id,body,raw_path,review_path) VALUES (?1,?2,?3,?4,?5,?6)",
            params![id, record.target.review_sha256, record.previous_record_sha256, bytes, raw, review])?;
        tx.execute("INSERT INTO heads (review_id,record_id) VALUES (?1,?2) ON CONFLICT(review_id) DO UPDATE SET record_id=excluded.record_id",
            params![record.target.review_sha256, id])?;
        tx.commit()?;
        Ok(id)
    }

    pub fn object(&self, id: &str) -> Result<Vec<u8>> {
        ensure(valid_digest(id), "invalid captured object digest")?;
        let bytes = read_small(&self.root.join("objects").join(id))?;
        ensure(digest(&bytes) == id, "captured object integrity mismatch")?;
        Ok(bytes)
    }

    pub fn record_locator(&self, id: &str, raw: bool) -> Result<PathBuf> {
        ensure(valid_digest(id), "invalid record digest")?;
        let column = if raw { "raw_path" } else { "review_path" };
        let path: String = self.open(false)?.query_row(
            &format!("SELECT {column} FROM records WHERE id=?1"),
            [id],
            |row| row.get(0),
        )?;
        Ok(PathBuf::from(path))
    }

    pub fn prepared_for_review(&self, review: &str) -> Result<Vec<PathBuf>> {
        ensure(valid_digest(review), "invalid review digest")?;
        let db = self.open(false)?;
        if db.pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0))? == 1 {
            return Ok(Vec::new());
        }
        let mut statement =
            db.prepare("SELECT body,manifest_path FROM prepared_regions ORDER BY id")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut paths = Vec::new();
        for row in rows {
            let (body, path) = row?;
            let value: serde_json::Value = serde_json::from_slice(&body)?;
            if value["parents"]
                .as_array()
                .is_some_and(|parents| parents.iter().any(|p| p["review_sha256"] == review))
            {
                paths.push(PathBuf::from(path));
            }
        }
        Ok(paths)
    }

    fn verified_record(
        &self,
        db: &Connection,
        id: &str,
        review: &str,
        snapshot: Option<&bundle::Bundle>,
    ) -> Result<(Record, Vec<u8>)> {
        ensure(valid_digest(id), "invalid indexed record digest")?;
        let (body, raw_path, review_path, previous, subject): (
            Vec<u8>,
            String,
            String,
            Option<String>,
            String,
        ) = db.query_row(
            "SELECT body,raw_path,review_path,previous_id,review_id FROM records WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
        ensure(
            body.len() <= 1024 * 1024
                && digest(&body) == id
                && read_small(&self.root.join("records").join(format!("{id}.json")))? == body,
            "record integrity mismatch",
        )?;
        let record: Record = serde_json::from_slice(&body)?;
        ensure(
            record.schema_version == 1
                && record.kind == "crop_curation"
                && record.admission == "not admitted"
                && record.target.review_sha256 == review
                && subject == review
                && record.previous_record_sha256 == previous,
            "indexed record identity mismatch",
        )?;
        for sha in [
            &record.target.raw_receipt_sha256,
            &record.target.review_sha256,
            &record.target.legend_sha256,
        ] {
            self.object(sha)?;
        }
        ensure(
            digest(&read_small(Path::new(&raw_path))?) == record.target.raw_receipt_sha256
                && digest(&read_small(Path::new(&review_path))?) == record.target.review_sha256,
            "local target digest mismatch",
        )?;
        let verified;
        let bundle = if let Some(snapshot) = snapshot {
            snapshot
        } else {
            verified = bundle::verify(Path::new(&raw_path), Path::new(&review_path))?;
            &verified
        };
        ensure(
            record.window_cells == bundle.manifest.window_cells
                && record.asset_sha256 == bundle.manifest.asset_sha256
                && record.target.raw_receipt_sha256 == bundle.manifest.raw_receipt_sha256,
            "curation crop identity mismatch",
        )?;
        let temp = io_context("stage captured legend", tempfile::tempdir())?;
        let legend_path = temp.path().join("legend.toml");
        write_new(&legend_path, &self.object(&record.target.legend_sha256)?)?;
        let legend = inspect_legend(&legend_path)?;
        ensure(
            legend.legend_version == record.legend_version,
            "legend version mismatch",
        )?;
        for evidence in &record.evidence {
            self.object(&evidence.sha256)?;
        }
        let [_, _, w, h] = record.window_cells;
        validate_claims(
            &record.claims,
            &record.evidence,
            &legend.classes,
            u64::from(w) * u64::from(h),
        )?;
        if let Some(automation) = &record.automation {
            ensure(
                matches!(
                    automation.kind.as_str(),
                    "supplier_qa_qualification_v1" | "supplier_qa_qualification_v2"
                ) && valid_digest(&automation.inherited_human_record_sha256)
                    && valid_digest(&automation.profile_sha256)
                    && valid_digest(&automation.quality_report_sha256)
                    && record.previous_record_sha256.is_some(),
                "invalid automatic qualification provenance",
            )?;
            let human_body: Vec<u8> = db.query_row(
                "SELECT body FROM records WHERE id=?1",
                [&automation.inherited_human_record_sha256],
                |row| row.get(0),
            )?;
            let human: Record = serde_json::from_slice(&human_body)?;
            let previous_id = record
                .previous_record_sha256
                .as_ref()
                .expect("checked predecessor");
            let previous_body: Vec<u8> = db.query_row(
                "SELECT body FROM records WHERE id=?1",
                [previous_id],
                |row| row.get(0),
            )?;
            let previous: Record = serde_json::from_slice(&previous_body)?;
            ensure(
                digest(&human_body) == automation.inherited_human_record_sha256
                    && human.automation.is_none()
                    && human.claims.actor.kind == ActorKind::Human
                    && human.claims.decision == Decision::Accept
                    && record.claims.actor.kind == ActorKind::Human
                    && record.claims.decision == Decision::Accept
                    && serde_json::to_vec(&record.claims.actor)?
                        == serde_json::to_vec(&human.claims.actor)?
                    && record.claims.reason == human.claims.reason
                    && record.claims.primary_class_id == human.claims.primary_class_id
                    && record.target.raw_receipt_sha256 == human.target.raw_receipt_sha256
                    && record.target.review_sha256 == human.target.review_sha256
                    && record.target.legend_sha256 == human.target.legend_sha256
                    && record.asset_sha256 == human.asset_sha256
                    && record.window_cells == human.window_cells
                    && record.legend_version == human.legend_version,
                "automatic qualification does not preserve its human acceptance",
            )?;
            ensure(
                previous_id == &automation.inherited_human_record_sha256
                    || (automation.kind == "supplier_qa_qualification_v2"
                        && previous.automation.as_ref().is_some_and(|value| {
                            matches!(
                                value.kind.as_str(),
                                "supplier_qa_qualification_v1" | "supplier_qa_qualification_v2"
                            ) && value.inherited_human_record_sha256
                                == automation.inherited_human_record_sha256
                        })),
                "automatic qualification predecessor has different human authority",
            )?;
            ensure(
                record.evidence.len() >= previous.evidence.len(),
                "automatic qualification changed captured human evidence",
            )?;
            for (current, prior) in record.evidence.iter().zip(&previous.evidence) {
                ensure(
                    serde_json::to_vec(current)? == serde_json::to_vec(prior)?,
                    "automatic qualification changed captured human evidence",
                )?;
            }
            for (name, finding) in &previous.claims.findings {
                if finding.state == FindingState::Fail {
                    let current = &record.claims.findings[name];
                    ensure(
                        current.state == FindingState::Fail
                            && current.note == finding.note
                            && current.evidence_ids == finding.evidence_ids,
                        "automatic qualification changed a failed finding",
                    )?;
                }
            }
            if matches!(
                previous.claims.license.training_permission,
                TrainingPermission::Restricted | TrainingPermission::Prohibited
            ) {
                ensure(
                    serde_json::to_vec(&record.claims.license)?
                        == serde_json::to_vec(&previous.claims.license)?,
                    "automatic qualification relaxed a permission restriction",
                )?;
            }
        }
        // Policy output is versioned with the record; do not silently reinterpret history.
        ensure(
            !record.preparation_blockers.is_empty(),
            "curation v1 cannot admit prepared data",
        )?;
        Ok((record, body))
    }

    pub fn history(&self, review: &str) -> Result<Vec<(String, Vec<u8>)>> {
        self.history_checked(review, None)
    }

    pub fn history_snapshot(
        &self,
        review: &str,
        snapshot: &bundle::Bundle,
    ) -> Result<Vec<(String, Vec<u8>)>> {
        self.history_checked(review, Some(snapshot))
    }

    fn history_checked(
        &self,
        review: &str,
        snapshot: Option<&bundle::Bundle>,
    ) -> Result<Vec<(String, Vec<u8>)>> {
        ensure(valid_digest(review), "invalid review digest")?;
        let mut db = self.open(false)?;
        let tx = db.transaction()?;
        let mut next = Self::head_in(&tx, review)?;
        let mut visited = BTreeSet::new();
        let mut records = Vec::new();
        while let Some(id) = next {
            ensure(
                records.len() < MAX_RECORDS && visited.insert(id.clone()),
                "history cycle or record limit exceeded",
            )?;
            let (record, body) = self.verified_record(&tx, &id, review, snapshot)?;
            next = record.previous_record_sha256;
            records.push((id, body));
        }
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM records WHERE review_id=?1",
            [review],
            |r| r.get(0),
        )?;
        ensure(
            count == records.len() as i64,
            "indexed records are detached from the current history",
        )?;
        records.reverse();
        Ok(records)
    }

    pub fn check(&self) -> Result<String> {
        let db = self.open(false)?;
        let integrity: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        ensure(integrity == "ok", "SQLite integrity check failed")?;
        let invalid_fk = db.prepare("PRAGMA foreign_key_check")?.exists([])?;
        ensure(!invalid_fk, "catalog foreign-key check failed")?;
        let mut statement = db.prepare(
            "SELECT DISTINCT review_id FROM records UNION SELECT review_id FROM heads LIMIT 4097",
        )?;
        let subjects = statement
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ensure(
            subjects.len() <= MAX_RECORDS,
            "catalog check exceeds 4096 crops",
        )?;
        let mut indexed = BTreeSet::new();
        let mut referenced = BTreeSet::new();
        for subject in subjects {
            for (id, bytes) in self.history(&subject)? {
                let record: Record = serde_json::from_slice(&bytes)?;
                referenced.extend(record.objects().into_iter().cloned());
                indexed.insert(id);
            }
        }
        let mut unregistered = Vec::new();
        let directory = self.root.join("records");
        if directory.exists() {
            for entry in io_context("list published records", fs::read_dir(directory))? {
                let entry = io_context("read published entry", entry)?;
                let path = entry.path();
                let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                ensure(
                    path.extension().is_some_and(|v| v == "json") && valid_digest(id),
                    "unexpected file in curation records",
                )?;
                if !indexed.contains(id) {
                    let bytes = read_small(&path)?;
                    ensure(
                        digest(&bytes) == id,
                        "unregistered record integrity mismatch",
                    )?;
                    let record: Record = serde_json::from_slice(&bytes)?;
                    for sha in record.objects() {
                        self.object(sha)?;
                        referenced.insert(sha.clone());
                    }
                    unregistered.push(id.to_string());
                }
                ensure(
                    indexed.len() + unregistered.len() <= MAX_RECORDS,
                    "catalog check exceeds 4096 records",
                )?;
            }
        }
        unregistered.sort();
        let mut unreferenced = Vec::new();
        let directory = self.root.join("objects");
        if directory.exists() {
            for entry in io_context("list captured objects", fs::read_dir(directory))? {
                let entry = io_context("read captured entry", entry)?;
                let id = entry.file_name().to_string_lossy().into_owned();
                self.object(&id)?;
                if !referenced.contains(&id) {
                    unreferenced.push(id);
                }
                ensure(
                    referenced.len() + unreferenced.len() <= MAX_RECORDS * 19,
                    "too many captured objects",
                )?;
            }
        }
        unreferenced.sort();
        let mut prepared_regions = 0;
        if db.pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0))? == 2 {
            let mut stmt =
                db.prepare("SELECT id, body, manifest_path FROM prepared_regions LIMIT 4097")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                let (id, body, path) = row?;
                prepared_regions += 1;
                ensure(
                    prepared_regions <= MAX_RECORDS
                        && digest(&body) == id
                        && read_small(Path::new(&path))? == body,
                    "prepared registry integrity mismatch",
                )?;
                crate::preparation::verify_files(Path::new(&path))?;
            }
        }
        Ok(serde_json::to_string_pretty(
            &serde_json::json!({"schema_version":1,"indexed_records":indexed.len(),
            "prepared_regions": prepared_regions,
            "unregistered_records":unregistered,"unreferenced_objects":unreferenced,
            "recovery":"retry original draft; never auto-promote or delete", "sqlite_version":rusqlite::version()}),
        )?)
    }

    pub fn export(&self, review: &str, output: &Path) -> Result<PathBuf> {
        ensure(!output.exists(), "export output already exists")?;
        let records = self.history(review)?;
        ensure(!records.is_empty(), "crop has no curation records")?;
        let output = if output.is_absolute() {
            output.to_path_buf()
        } else {
            io_context("read working directory", std::env::current_dir())?.join(output)
        };
        let parent = output
            .parent()
            .ok_or_else(|| AcquisitionError::Invalid("export needs parent directory".into()))?;
        io_context("create export parent", fs::create_dir_all(parent))?;
        let stage = io_context("stage portable curation", tempfile::tempdir_in(parent))?;
        let mut ids = Vec::new();
        for (id, bytes) in records {
            let record: Record = serde_json::from_slice(&bytes)?;
            write_new(
                &stage.path().join("records").join(format!("{id}.json")),
                &bytes,
            )?;
            let objects = [
                &record.target.raw_receipt_sha256,
                &record.target.review_sha256,
                &record.target.legend_sha256,
            ]
            .into_iter()
            .chain(record.evidence.iter().map(|e| &e.sha256));
            for sha in objects {
                write_new(&stage.path().join("objects").join(sha), &self.object(sha)?)?;
            }
            ids.push(id);
        }
        write_new(
            &stage.path().join("export.json"),
            &json(
                &serde_json::json!({"schema_version":1,"kind":"curation_export", "review_sha256":review,
            "records_oldest_first":ids, "includes_rasters":false, "admission":"not admitted"}),
            )?,
        )?;
        // Reserve the destination without clobbering another producer's output.
        io_context("reserve export output", fs::create_dir(&output))?;
        io_context("publish curation export", fs::rename(stage.path(), &output))?;
        io_context(
            "sync export parent",
            fs::File::open(parent).and_then(|f| f.sync_all()),
        )?;
        Ok(output.join("export.json"))
    }
}
