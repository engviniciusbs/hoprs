use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hoprs_core::{Error, ErrorKind, RemotePath, Result, SiteId};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::model::{
    ConflictInfo, ConflictPolicy, Direction, Origin, TransferId, TransferItem, TransferSpec,
    TransferState,
};

const MIGRATION_V1: &str = "
CREATE TABLE transfers (
  id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES transfers(id) ON DELETE CASCADE,
  site TEXT NOT NULL, direction TEXT NOT NULL, local_path TEXT NOT NULL, remote_path TEXT NOT NULL,
  is_dir INTEGER, size INTEGER, bytes_done INTEGER NOT NULL DEFAULT 0,
  state TEXT NOT NULL, state_detail TEXT, error_kind TEXT, error_message TEXT, error_detail TEXT,
  conflict_json TEXT, created_at INTEGER NOT NULL);
CREATE INDEX transfers_parent ON transfers(parent_id);
";

/// Adds `next_policy`: the conflict policy the next run of an unfinished
/// item must use. Moves the pending overrides that schema v1 kept in
/// `state_detail` there, and gives files that v1 would have continued on
/// resume (`active`, or paused from `active`) a `resume` policy.
const MIGRATION_V2: &str = "
ALTER TABLE transfers ADD COLUMN next_policy TEXT;
UPDATE transfers SET next_policy = substr(state_detail, 10), state_detail = NULL
  WHERE state = 'queued' AND state_detail LIKE 'override:%';
UPDATE transfers
  SET next_policy = substr(state_detail, instr(state_detail, ';override:') + 10),
      state_detail = substr(state_detail, 1, instr(state_detail, ';override:') - 1)
  WHERE state = 'paused' AND instr(state_detail, ';override:') > 0;
UPDATE transfers SET next_policy = 'resume'
  WHERE next_policy IS NULL AND is_dir IS NOT 1
    AND (state = 'active' OR (state = 'paused' AND state_detail = 'active'));
";

/// Adds the stable code of a failed item's error and its arguments (a JSON
/// object of strings), so the app can show the error in the user's language.
/// Older rows have neither and show their English message.
const MIGRATION_V3: &str = "
ALTER TABLE transfers ADD COLUMN error_code TEXT;
ALTER TABLE transfers ADD COLUMN error_args TEXT;
";

/// Adds the conflict policy an item's tree was enqueued with and who enqueued
/// it (`'agent'`; `NULL` is the user). Older rows are the user's, with no tree
/// policy. hoprs 1.5.0 (schema 3) refuses a file at this version.
const MIGRATION_V4: &str = "
ALTER TABLE transfers ADD COLUMN tree_policy TEXT;
ALTER TABLE transfers ADD COLUMN origin TEXT;
";

/// The schema version this code reads and writes (`PRAGMA user_version`).
const SCHEMA_VERSION: i64 = 4;

const SELECT: &str = "SELECT id, parent_id, site, direction, local_path, remote_path, is_dir, size, \
    bytes_done, state, state_detail, error_kind, error_message, error_detail, conflict_json, \
    error_code, error_args, tree_policy, origin FROM transfers";

/// The state a `Paused` item was paused from, so `resume` can treat it correctly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PausedFrom {
    Queued,
    Active,
    AwaitingDecision,
}

impl PausedFrom {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(PausedFrom::Queued),
            "active" => Some(PausedFrom::Active),
            "awaiting_decision" => Some(PausedFrom::AwaitingDecision),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            PausedFrom::Queued => "queued",
            PausedFrom::Active => "active",
            PausedFrom::AwaitingDecision => "awaiting_decision",
        }
    }
}

fn policy_name(policy: ConflictPolicy) -> &'static str {
    match policy {
        ConflictPolicy::Ask => "ask",
        ConflictPolicy::Overwrite => "overwrite",
        ConflictPolicy::Skip => "skip",
        ConflictPolicy::OverwriteIfNewer => "overwrite_if_newer",
        ConflictPolicy::Resume => "resume",
    }
}

fn parse_policy(s: &str) -> Option<ConflictPolicy> {
    match s {
        "ask" => Some(ConflictPolicy::Ask),
        "overwrite" => Some(ConflictPolicy::Overwrite),
        "skip" => Some(ConflictPolicy::Skip),
        "overwrite_if_newer" => Some(ConflictPolicy::OverwriteIfNewer),
        "resume" => Some(ConflictPolicy::Resume),
        _ => None,
    }
}

/// The stored form of an origin: `NULL` for the user.
fn origin_name(origin: Origin) -> Option<&'static str> {
    match origin {
        Origin::User => None,
        Origin::Agent => Some("agent"),
    }
}

fn parse_origin(s: Option<&str>) -> Option<Origin> {
    match s {
        None => Some(Origin::User),
        Some("agent") => Some(Origin::Agent),
        Some(_) => None,
    }
}

/// The pause origin recorded in `state_detail` of a `paused` row.
fn paused_origin(state: &str, detail: Option<&str>) -> Option<PausedFrom> {
    match (state, detail) {
        ("paused", Some(d)) => PausedFrom::parse(d),
        _ => None,
    }
}

/// A child item for `QueueStore::insert_children`.
#[derive(Clone, Debug, PartialEq)]
pub struct NewChild {
    pub spec: TransferSpec,
    pub is_dir: bool,
    pub size: Option<u64>,
    /// Initial state, normally `Queued`.
    pub state: TransferState,
    /// The policy of the child's tree (its folder's `tree_policy`).
    pub tree_policy: Option<ConflictPolicy>,
    /// The origin of the child's tree.
    pub origin: Origin,
}

/// SQLite-backed persistent transfer queue. Synchronous: operations are short.
pub struct QueueStore {
    conn: Connection,
}

fn db_err(e: impl std::fmt::Display) -> Error {
    Error::coded(
        ErrorKind::Io,
        "queue-database-error",
        "Queue database error",
    )
    .with_detail(e.to_string())
}

fn corrupt(what: &str) -> Error {
    Error::coded(
        ErrorKind::Io,
        "queue-database-error",
        "Queue database error",
    )
    .with_detail(format!("corrupt row: {what}"))
}

trait Db<T> {
    fn db(self) -> Result<T>;
}

impl<T> Db<T> for rusqlite::Result<T> {
    fn db(self) -> Result<T> {
        self.map_err(db_err)
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn time_to_secs(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
        Err(e) => i64::try_from(e.duration().as_secs()).map_or(i64::MIN, |v| -v),
    }
}

fn secs_to_time(s: i64) -> Option<SystemTime> {
    if s >= 0 {
        UNIX_EPOCH.checked_add(Duration::from_secs(s as u64))
    } else {
        UNIX_EPOCH.checked_sub(Duration::from_secs(s.unsigned_abs()))
    }
}

fn opt_secs_json(t: Option<SystemTime>) -> String {
    t.map_or("null".to_owned(), |t| time_to_secs(t).to_string())
}

fn conflict_to_json(c: &ConflictInfo) -> String {
    format!(
        "{{\"src_size\":{},\"dst_size\":{},\"src_modified\":{},\"dst_modified\":{}}}",
        c.src_size,
        c.dst_size,
        opt_secs_json(c.src_modified),
        opt_secs_json(c.dst_modified)
    )
}

/// Extracts the raw value of `key` from the flat JSON object written by `conflict_to_json`.
fn json_field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\":");
    let start = json.find(&pat)? + pat.len();
    let rest = &json[start..];
    let end = rest.find([',', '}'])?;
    Some(rest[..end].trim())
}

fn conflict_from_json(json: &str) -> Result<ConflictInfo> {
    let num = |k: &str| -> Result<u64> {
        json_field(json, k)
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| corrupt("conflict json"))
    };
    let time = |k: &str| -> Result<Option<SystemTime>> {
        match json_field(json, k) {
            Some("null") => Ok(None),
            Some(v) => v
                .parse::<i64>()
                .ok()
                .and_then(secs_to_time)
                .map(Some)
                .ok_or_else(|| corrupt("conflict json")),
            None => Err(corrupt("conflict json")),
        }
    };
    Ok(ConflictInfo {
        src_size: num("src_size")?,
        dst_size: num("dst_size")?,
        src_modified: time("src_modified")?,
        dst_modified: time("dst_modified")?,
    })
}

/// A stored error code as the `&'static str` `Error::code` holds. Each distinct value read
/// from the file is leaked once (so the leak is bounded by the distinct codes the file
/// holds, not only those of this binary); a value that is not a kebab-case ASCII code of
/// at most 64 bytes reads as no code, which also bounds what a damaged file can leak.
fn intern_code(s: &str) -> Option<&'static str> {
    let valid = !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !valid {
        return None;
    }
    static CODES: Mutex<Option<HashMap<String, &'static str>>> = Mutex::new(None);
    let mut guard = CODES.lock().ok()?;
    let codes = guard.get_or_insert_with(HashMap::new);
    if let Some(code) = codes.get(s) {
        return Some(code);
    }
    let code: &'static str = Box::leak(s.to_owned().into_boxed_str());
    codes.insert(s.to_owned(), code);
    Some(code)
}

/// Argument names are `&'static str` too, interned the same way.
fn intern_name(s: &str) -> &'static str {
    static NAMES: Mutex<Option<HashMap<String, &'static str>>> = Mutex::new(None);
    let Ok(mut guard) = NAMES.lock() else {
        return Box::leak(s.to_owned().into_boxed_str());
    };
    let names = guard.get_or_insert_with(HashMap::new);
    names
        .entry(s.to_owned())
        .or_insert_with(|| Box::leak(s.to_owned().into_boxed_str()))
}

fn args_to_json(args: &[(&'static str, String)]) -> Option<String> {
    if args.is_empty() {
        return None;
    }
    let map: serde_json::Map<String, serde_json::Value> = args
        .iter()
        .map(|(k, v)| ((*k).to_owned(), serde_json::Value::String(v.clone())))
        .collect();
    Some(serde_json::Value::Object(map).to_string())
}

/// Malformed JSON, or anything but an object of strings, reads as no arguments.
fn args_from_json(json: &str) -> Vec<(&'static str, String)> {
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str(json) else {
        return Vec::new();
    };
    let mut args = Vec::new();
    for (k, v) in map {
        let serde_json::Value::String(v) = v else {
            return Vec::new();
        };
        args.push((intern_name(&k), v));
    }
    args
}

fn item_from_row(row: &Row<'_>) -> Result<TransferItem> {
    let site: String = row.get(2).db()?;
    let direction = match row.get::<_, String>(3).db()?.as_str() {
        "upload" => Direction::Upload,
        "download" => Direction::Download,
        _ => return Err(corrupt("direction")),
    };
    let local =
        RemotePath::new(&row.get::<_, String>(4).db()?).map_err(|_| corrupt("local_path"))?;
    let remote =
        RemotePath::new(&row.get::<_, String>(5).db()?).map_err(|_| corrupt("remote_path"))?;
    let state_name: String = row.get(9).db()?;
    let detail: Option<String> = row.get(10).db()?;
    let state = match state_name.as_str() {
        "queued" => TransferState::Queued,
        "active" => TransferState::Active,
        "paused" => TransferState::Paused,
        "awaiting_decision" => {
            let json: Option<String> = row.get(14).db()?;
            TransferState::AwaitingDecision(conflict_from_json(
                &json.ok_or_else(|| corrupt("missing conflict"))?,
            )?)
        }
        "done" => TransferState::Done,
        "skipped" => TransferState::Skipped(detail.unwrap_or_default()),
        "failed" => {
            let kind: Option<String> = row.get(11).db()?;
            let kind = kind
                .as_deref()
                .and_then(ErrorKind::parse)
                .ok_or_else(|| corrupt("error_kind"))?;
            let message: Option<String> = row.get(12).db()?;
            let mut err = Error::new(kind, message.unwrap_or_default());
            err.detail = row.get(13).db()?;
            let code: Option<String> = row.get(15).db()?;
            if let Some(code) = code.as_deref().and_then(intern_code) {
                err.code = Some(code);
                let args: Option<String> = row.get(16).db()?;
                err.args = args.as_deref().map(args_from_json).unwrap_or_default();
            }
            TransferState::Failed(err)
        }
        "cancelled" => TransferState::Cancelled,
        _ => return Err(corrupt("state")),
    };
    let tree_policy = match row.get::<_, Option<String>>(17).db()? {
        None => None,
        Some(name) => Some(parse_policy(&name).ok_or_else(|| corrupt("tree_policy"))?),
    };
    let origin = parse_origin(row.get::<_, Option<String>>(18).db()?.as_deref())
        .ok_or_else(|| corrupt("origin"))?;
    Ok(TransferItem {
        id: TransferId(row.get(0).db()?),
        parent: row.get::<_, Option<i64>>(1).db()?.map(TransferId),
        spec: TransferSpec {
            site: SiteId(site),
            direction,
            local,
            remote,
        },
        is_dir: row.get::<_, Option<bool>>(6).db()?,
        size: row.get::<_, Option<i64>>(7).db()?.map(|s| s as u64),
        bytes_done: row.get::<_, i64>(8).db()? as u64,
        state,
        origin,
        tree_policy,
    })
}

fn insert_row(
    conn: &Connection,
    parent: Option<TransferId>,
    spec: &TransferSpec,
    tree_policy: Option<ConflictPolicy>,
    origin: Origin,
) -> Result<TransferId> {
    let direction = match spec.direction {
        Direction::Upload => "upload",
        Direction::Download => "download",
    };
    conn.execute(
        "INSERT INTO transfers (parent_id, site, direction, local_path, remote_path, state, created_at, \
         tree_policy, origin) VALUES (?1, ?2, ?3, ?4, ?5, 'queued', ?6, ?7, ?8)",
        params![
            parent.map(|p| p.0),
            spec.site.0,
            direction,
            spec.local.as_str(),
            spec.remote.as_str(),
            now_secs(),
            tree_policy.map(policy_name),
            origin_name(origin)
        ],
    ).db()?;
    Ok(TransferId(conn.last_insert_rowid()))
}

fn current_state(conn: &Connection, id: TransferId) -> Result<Option<(String, Option<String>)>> {
    conn.query_row(
        "SELECT state, state_detail FROM transfers WHERE id = ?1",
        [id.0],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .db()
}

/// Persists a new state. Moving to `Paused` records the state it was paused
/// from. `next_policy` is kept while the item is `Queued`, `Active`,
/// `Paused`, `Failed` or `Cancelled` (a stopped copy keeps its continuation
/// for a retry), and cleared by any other state.
fn write_state(conn: &Connection, id: TransferId, state: &TransferState) -> Result<()> {
    let mut detail: Option<String> = None;
    let (mut kind, mut message, mut err_detail, mut conflict) = (None, None, None, None);
    let (mut err_code, mut err_args): (Option<&str>, Option<String>) = (None, None);
    let mut keep_policy = false;
    let name = match state {
        TransferState::Queued => {
            keep_policy = true;
            "queued"
        }
        TransferState::Active => {
            keep_policy = true;
            "active"
        }
        TransferState::Paused => {
            keep_policy = true;
            if let Some((s, d)) = current_state(conn, id)? {
                let origin = PausedFrom::parse(&s).or_else(|| paused_origin(&s, d.as_deref()));
                detail = origin.map(|o| o.as_str().to_owned());
            }
            "paused"
        }
        TransferState::AwaitingDecision(c) => {
            conflict = Some(conflict_to_json(c));
            "awaiting_decision"
        }
        TransferState::Done => "done",
        TransferState::Skipped(reason) => {
            detail = Some(reason.clone());
            "skipped"
        }
        TransferState::Failed(e) => {
            keep_policy = true;
            kind = Some(e.kind.as_str());
            message = Some(e.message.clone());
            err_detail = e.detail.clone();
            err_code = e.code;
            err_args = args_to_json(&e.args);
            "failed"
        }
        TransferState::Cancelled => {
            keep_policy = true;
            "cancelled"
        }
    };
    conn.execute(
        "UPDATE transfers SET state = ?2, state_detail = ?3, error_kind = ?4, error_message = ?5, \
         error_detail = ?6, conflict_json = ?7, error_code = ?9, error_args = ?10, \
         next_policy = CASE WHEN ?8 THEN next_policy ELSE NULL END WHERE id = ?1",
        params![
            id.0,
            name,
            detail,
            kind,
            message,
            err_detail,
            conflict,
            keep_policy,
            err_code,
            err_args
        ],
    )
    .db()?;
    Ok(())
}

/// The `synchronous` setting for a database in `journal_mode`: `NORMAL` is
/// only safe in WAL mode; in any other mode a power loss during a commit can
/// corrupt the database unless every commit waits for the disk (`FULL`).
fn sync_mode_for(journal_mode: &str) -> &'static str {
    if journal_mode.eq_ignore_ascii_case("wal") {
        "NORMAL"
    } else {
        "FULL"
    }
}

impl QueueStore {
    /// Opens (creating if needed) the queue database at `path`, in WAL mode
    /// with `synchronous=NORMAL`: commits do not wait for an fsync, so a
    /// power loss may roll back the last few state changes, but an
    /// application crash loses nothing and the database never corrupts.
    /// (With the default `FULL`, every state change waits for the disk.)
    /// If the file system refuses WAL mode, it stays in its journal mode with
    /// `synchronous=FULL`.
    pub fn open(path: &Path) -> Result<QueueStore> {
        let conn = Connection::open(path).db()?;
        let mode: String = conn
            .pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))
            .db()?;
        conn.pragma_update(None, "synchronous", sync_mode_for(&mode))
            .db()?;
        Self::init(conn)
    }

    /// Opens a private in-memory queue database.
    pub fn open_in_memory() -> Result<QueueStore> {
        Self::init(Connection::open_in_memory().db()?)
    }

    /// Migrates the schema up to `SCHEMA_VERSION`, one transaction per
    /// version. Refuses a database written by a newer version.
    fn init(conn: Connection) -> Result<QueueStore> {
        conn.pragma_update(None, "foreign_keys", "ON").db()?;
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .db()?;
        if version > SCHEMA_VERSION {
            return Err(Error::coded(
                ErrorKind::Io,
                "queue-newer-version",
                "The transfer queue was saved by a newer version of hoprs",
            )
            .with_detail(format!(
                "queue schema version {version}, this version understands up to {SCHEMA_VERSION}"
            )));
        }
        for (target, migration) in [
            (1, MIGRATION_V1),
            (2, MIGRATION_V2),
            (3, MIGRATION_V3),
            (4, MIGRATION_V4),
        ] {
            if version < target {
                let tx = conn.unchecked_transaction().db()?;
                tx.execute_batch(migration).db()?;
                tx.pragma_update(None, "user_version", target).db()?;
                tx.commit().db()?;
            }
        }
        Ok(QueueStore { conn })
    }

    /// Inserts a new `Queued` item of the user, with no tree policy, and
    /// returns its id.
    pub fn insert(
        &mut self,
        parent: Option<TransferId>,
        spec: &TransferSpec,
    ) -> Result<TransferId> {
        insert_row(&self.conn, parent, spec, None, Origin::User)
    }

    /// Like [`insert`](Self::insert), with the item's tree policy and origin.
    pub fn insert_with(
        &mut self,
        parent: Option<TransferId>,
        spec: &TransferSpec,
        tree_policy: Option<ConflictPolicy>,
        origin: Origin,
    ) -> Result<TransferId> {
        insert_row(&self.conn, parent, spec, tree_policy, origin)
    }

    /// Inserts the children of `parent` (with their metadata and initial
    /// state) in one transaction and returns their ids, in order.
    pub fn insert_children(
        &mut self,
        parent: TransferId,
        children: &[NewChild],
    ) -> Result<Vec<TransferId>> {
        let tx = self.conn.transaction().db()?;
        let mut ids = Vec::with_capacity(children.len());
        for child in children {
            let id = insert_row(
                &tx,
                Some(parent),
                &child.spec,
                child.tree_policy,
                child.origin,
            )?;
            tx.execute(
                "UPDATE transfers SET is_dir = ?2, size = ?3 WHERE id = ?1",
                params![id.0, child.is_dir, child.size.map(|s| s as i64)],
            )
            .db()?;
            if child.state != TransferState::Queued {
                write_state(&tx, id, &child.state)?;
            }
            ids.push(id);
        }
        tx.commit().db()?;
        Ok(ids)
    }

    /// Persists a new state. Moving to `Paused` records the state it was paused from.
    pub fn set_state(&mut self, id: TransferId, state: &TransferState) -> Result<()> {
        write_state(&self.conn, id, state)
    }

    /// Returns what a `Paused` item was paused from, or `None` if it is not paused.
    pub fn paused_from(&self, id: TransferId) -> Result<Option<PausedFrom>> {
        Ok(current_state(&self.conn, id)?.and_then(|(s, d)| paused_origin(&s, d.as_deref())))
    }

    /// The conflict policy the next run of the item must use, if any (see
    /// `set_next_policy`).
    pub fn next_policy(&self, id: TransferId) -> Result<Option<ConflictPolicy>> {
        let stored: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT next_policy FROM transfers WHERE id = ?1",
                [id.0],
                |r| r.get(0),
            )
            .optional()
            .db()?;
        match stored.flatten() {
            None => Ok(None),
            Some(name) => parse_policy(&name)
                .map(Some)
                .ok_or_else(|| corrupt("next_policy")),
        }
    }

    /// Records the conflict policy the next run of an unfinished item must
    /// use instead of the global one (`None`: the global one). It is kept
    /// while the item is `Queued`, `Active`, `Paused`, `Failed` or
    /// `Cancelled` (including across recovery) and cleared by any other
    /// state. Does nothing unless the item is `Queued`, `Active` or `Paused`.
    pub fn set_next_policy(
        &mut self,
        id: TransferId,
        policy: Option<ConflictPolicy>,
    ) -> Result<()> {
        self.conn
            .execute(
                "UPDATE transfers SET next_policy = ?2 \
                 WHERE id = ?1 AND state IN ('queued', 'active', 'paused')",
                params![id.0, policy.map(policy_name)],
            )
            .db()?;
        Ok(())
    }

    pub fn set_meta(&mut self, id: TransferId, is_dir: bool, size: Option<u64>) -> Result<()> {
        self.conn
            .execute(
                "UPDATE transfers SET is_dir = ?2, size = ?3 WHERE id = ?1",
                params![id.0, is_dir, size.map(|s| s as i64)],
            )
            .db()?;
        Ok(())
    }

    pub fn set_bytes_done(&mut self, id: TransferId, bytes: u64) -> Result<()> {
        self.conn
            .execute(
                "UPDATE transfers SET bytes_done = ?2 WHERE id = ?1",
                params![id.0, bytes as i64],
            )
            .db()?;
        Ok(())
    }

    /// Persists several byte counts in one transaction.
    pub fn set_bytes_done_many(&mut self, updates: &[(TransferId, u64)]) -> Result<()> {
        let tx = self.conn.transaction().db()?;
        {
            let mut stmt = tx
                .prepare_cached("UPDATE transfers SET bytes_done = ?2 WHERE id = ?1")
                .db()?;
            for (id, bytes) in updates {
                stmt.execute(params![id.0, *bytes as i64]).db()?;
            }
        }
        tx.commit().db()
    }

    pub fn get(&self, id: TransferId) -> Result<Option<TransferItem>> {
        self.query(&format!("{SELECT} WHERE id = ?1"), params![id.0])
            .map(|mut v| v.pop())
    }

    /// All items ordered by id.
    pub fn list_all(&self) -> Result<Vec<TransferItem>> {
        self.query(&format!("{SELECT} ORDER BY id"), [])
    }

    /// Direct children of `parent`, ordered by id.
    pub fn children(&self, parent: TransferId) -> Result<Vec<TransferItem>> {
        self.query(
            &format!("{SELECT} WHERE parent_id = ?1 ORDER BY id"),
            params![parent.0],
        )
    }

    fn query(&self, sql: &str, params: impl rusqlite::Params) -> Result<Vec<TransferItem>> {
        let mut stmt = self.conn.prepare(sql).db()?;
        let mut rows = stmt.query(params).db()?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().db()? {
            out.push(item_from_row(row)?);
        }
        Ok(out)
    }

    /// Pauses `Active` and `AwaitingDecision` items (remembering the origin) and returns
    /// every non-terminal item.
    pub fn recover(&mut self) -> Result<Vec<TransferItem>> {
        self.conn
            .execute(
                "UPDATE transfers SET state = 'paused', state_detail = state, conflict_json = NULL \
             WHERE state IN ('active', 'awaiting_decision')",
                [],
            )
            .db()?;
        let all = self.query(&format!("{SELECT} ORDER BY id"), [])?;
        Ok(all.into_iter().filter(|i| !i.state.is_terminal()).collect())
    }

    /// Removes terminal root items and all their descendants (via `ON DELETE CASCADE`).
    /// Returns the number of root items removed (descendants are not counted).
    pub fn clear_finished(&mut self) -> Result<usize> {
        self.clear_finished_except(&[])
    }

    /// Like [`clear_finished`](Self::clear_finished), but keeps the roots in
    /// `keep` (with their descendants). One transaction.
    pub fn clear_finished_except(&mut self, keep: &[TransferId]) -> Result<usize> {
        let tx = self.conn.transaction().db()?;
        let roots: Vec<i64> = {
            let mut stmt = tx
                .prepare(
                    "SELECT id FROM transfers WHERE parent_id IS NULL \
                 AND state IN ('done', 'skipped', 'failed', 'cancelled')",
                )
                .db()?;
            let rows = stmt.query_map([], |r| r.get(0)).db()?;
            rows.collect::<rusqlite::Result<_>>().db()?
        };
        let mut removed = 0;
        for id in roots
            .into_iter()
            .filter(|id| !keep.contains(&TransferId(*id)))
        {
            removed += tx
                .execute("DELETE FROM transfers WHERE id = ?1", [id])
                .db()?;
        }
        tx.commit().db()?;
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(local: &str, remote: &str) -> TransferSpec {
        TransferSpec {
            site: SiteId("site".to_string()),
            direction: Direction::Upload,
            local: RemotePath::new(local).unwrap(),
            remote: RemotePath::new(remote).unwrap(),
        }
    }

    fn new_item(s: &mut QueueStore) -> TransferId {
        s.insert(None, &spec("/a", "/b")).unwrap()
    }

    #[test]
    fn every_state_round_trips() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let conflict = ConflictInfo {
            src_size: 10,
            dst_size: 20,
            src_modified: Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            dst_modified: None,
        };
        let states = vec![
            TransferState::Queued,
            TransferState::Active,
            TransferState::Paused,
            TransferState::AwaitingDecision(conflict),
            TransferState::Done,
            TransferState::Skipped("exists".to_string()),
            TransferState::Failed(
                Error::new(ErrorKind::Permission, "denied").with_detail("EACCES /x"),
            ),
            TransferState::Cancelled,
        ];
        for st in states {
            let id = new_item(&mut s);
            s.set_state(id, &st).unwrap();
            let item = s.get(id).unwrap().unwrap();
            assert_eq!(item.state, st);
            assert_eq!(item.spec, spec("/a", "/b"));
        }
        assert!(s.get(TransferId(999)).unwrap().is_none());
    }

    #[test]
    fn leaving_a_state_clears_its_columns() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let conflict = ConflictInfo {
            src_size: 1,
            dst_size: 2,
            src_modified: None,
            dst_modified: None,
        };
        let leaving = [
            TransferState::Failed(Error::new(ErrorKind::Io, "x").with_detail("d")),
            TransferState::AwaitingDecision(conflict),
            TransferState::Skipped("why".to_string()),
        ];
        for st in leaving {
            let id = new_item(&mut s);
            s.set_state(id, &st).unwrap();
            s.set_state(id, &TransferState::Queued).unwrap();
            let nulls: i64 = s
                .conn
                .query_row(
                    "SELECT error_kind IS NULL AND error_message IS NULL AND error_detail IS NULL \
                     AND conflict_json IS NULL AND state_detail IS NULL FROM transfers WHERE id = ?1",
                    [id.0],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(nulls, 1, "columns not cleared after leaving {st:?}");
        }
    }

    #[test]
    fn out_of_range_stored_timestamps_are_errors_not_panics() {
        for bad in [
            "-9223372036854775808",
            "9223372036854775807",
            "99999999999999999",
        ] {
            let mut s = QueueStore::open_in_memory().unwrap();
            let id = new_item(&mut s);
            s.set_state(
                id,
                &TransferState::AwaitingDecision(ConflictInfo {
                    src_size: 1,
                    dst_size: 2,
                    src_modified: None,
                    dst_modified: None,
                }),
            )
            .unwrap();
            let json = format!(
                "{{\"src_size\":1,\"dst_size\":2,\"src_modified\":{bad},\"dst_modified\":null}}"
            );
            s.conn
                .execute("UPDATE transfers SET conflict_json = ?1", [json])
                .unwrap();
            // Representable on some platforms (Linux i64 seconds), not on others (Windows):
            // either way it must be a value or an Io error, never a panic. Unparseable
            // numbers are an error everywhere.
            let got = s.get(id);
            let listed = s.list_all();
            let recovered = s.recover();
            if bad.parse::<i64>().is_err() {
                assert_eq!(got.unwrap_err().kind, ErrorKind::Io);
                assert_eq!(listed.unwrap_err().kind, ErrorKind::Io);
                assert_eq!(recovered.unwrap_err().kind, ErrorKind::Io);
            } else if let Err(e) = got {
                assert_eq!(e.kind, ErrorKind::Io);
            }
        }
    }

    #[test]
    fn meta_and_bytes_are_persisted() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let id = new_item(&mut s);
        s.set_meta(id, false, Some(42)).unwrap();
        s.set_bytes_done(id, 7).unwrap();
        let item = s.get(id).unwrap().unwrap();
        assert_eq!(
            (item.is_dir, item.size, item.bytes_done),
            (Some(false), Some(42), 7)
        );
    }

    #[test]
    fn set_bytes_done_many_updates_each_listed_item() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let (a, b, c) = (new_item(&mut s), new_item(&mut s), new_item(&mut s));
        s.set_bytes_done(c, 5).unwrap();
        s.set_bytes_done_many(&[(a, 100), (b, 200)]).unwrap();
        let bytes: Vec<u64> = s.list_all().unwrap().iter().map(|i| i.bytes_done).collect();
        assert_eq!(bytes, vec![100, 200, 5]);
        s.set_bytes_done_many(&[]).unwrap();
    }

    #[test]
    fn children_returns_only_children_of_the_parent() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let p1 = new_item(&mut s);
        let p2 = new_item(&mut s);
        let c1 = s.insert(Some(p1), &spec("/a/x", "/b/x")).unwrap();
        s.insert(Some(p2), &spec("/a/y", "/b/y")).unwrap();
        let kids = s.children(p1).unwrap();
        assert_eq!(kids.len(), 1);
        assert_eq!(kids[0].id, c1);
        assert_eq!(kids[0].parent, Some(p1));
        assert_eq!(s.list_all().unwrap().len(), 4);
    }

    #[test]
    fn insert_children_persists_meta_and_state() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let parent = new_item(&mut s);
        let other = new_item(&mut s);
        let children = vec![
            NewChild {
                spec: spec("/a/f", "/b/f"),
                is_dir: false,
                size: Some(42),
                state: TransferState::Queued,
                tree_policy: None,
                origin: Origin::User,
            },
            NewChild {
                spec: spec("/a/d", "/b/d"),
                is_dir: true,
                size: None,
                state: TransferState::Queued,
                tree_policy: None,
                origin: Origin::User,
            },
            NewChild {
                spec: spec("/a/l", "/b/l"),
                is_dir: false,
                size: None,
                state: TransferState::Skipped("symlinks are not followed".to_string()),
                tree_policy: None,
                origin: Origin::User,
            },
        ];
        let ids = s.insert_children(parent, &children).unwrap();
        assert_eq!(ids.len(), 3);
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
        assert!(ids[0] > other);

        let kids = s.children(parent).unwrap();
        assert_eq!(kids.iter().map(|k| k.id).collect::<Vec<_>>(), ids);
        for (kid, want) in kids.iter().zip(&children) {
            assert_eq!(kid.parent, Some(parent));
            assert_eq!(kid.spec, want.spec);
            assert_eq!(kid.is_dir, Some(want.is_dir));
            assert_eq!(kid.size, want.size);
            assert_eq!(kid.state, want.state);
            assert_eq!(kid.bytes_done, 0);
        }
        assert!(s.insert_children(parent, &[]).unwrap().is_empty());
    }

    #[test]
    fn insert_children_is_all_or_nothing() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let child = NewChild {
            spec: spec("/a/f", "/b/f"),
            is_dir: false,
            size: None,
            state: TransferState::Queued,
            tree_policy: None,
            origin: Origin::User,
        };
        // The parent does not exist: the foreign key rejects the batch.
        assert_eq!(
            s.insert_children(TransferId(999), &[child.clone(), child])
                .unwrap_err()
                .kind,
            ErrorKind::Io
        );
        assert!(s.list_all().unwrap().is_empty());
    }

    #[test]
    fn recover_pauses_active_and_awaiting_and_skips_terminal() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let active = new_item(&mut s);
        let awaiting = new_item(&mut s);
        let done = new_item(&mut s);
        let queued = new_item(&mut s);
        s.set_state(active, &TransferState::Active).unwrap();
        s.set_state(
            awaiting,
            &TransferState::AwaitingDecision(ConflictInfo {
                src_size: 1,
                dst_size: 2,
                src_modified: None,
                dst_modified: None,
            }),
        )
        .unwrap();
        s.set_state(done, &TransferState::Done).unwrap();

        let recovered = s.recover().unwrap();
        let ids: Vec<_> = recovered.iter().map(|i| i.id).collect();
        assert_eq!(ids, vec![active, awaiting, queued]);
        assert_eq!(recovered[0].state, TransferState::Paused);
        assert_eq!(recovered[1].state, TransferState::Paused);
        assert_eq!(recovered[2].state, TransferState::Queued);
        assert_eq!(s.get(done).unwrap().unwrap().state, TransferState::Done);
        assert_eq!(s.paused_from(active).unwrap(), Some(PausedFrom::Active));
        assert_eq!(
            s.paused_from(awaiting).unwrap(),
            Some(PausedFrom::AwaitingDecision)
        );
    }

    #[test]
    fn paused_from_tracks_the_origin_state() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let id = new_item(&mut s);
        assert_eq!(s.paused_from(id).unwrap(), None);
        s.set_state(id, &TransferState::Active).unwrap();
        s.set_state(id, &TransferState::Paused).unwrap();
        assert_eq!(s.paused_from(id).unwrap(), Some(PausedFrom::Active));
        // Pausing again keeps the original origin.
        s.set_state(id, &TransferState::Paused).unwrap();
        assert_eq!(s.paused_from(id).unwrap(), Some(PausedFrom::Active));
        s.set_state(id, &TransferState::Queued).unwrap();
        assert_eq!(s.paused_from(id).unwrap(), None);
        s.set_state(id, &TransferState::Paused).unwrap();
        assert_eq!(s.paused_from(id).unwrap(), Some(PausedFrom::Queued));
    }

    #[test]
    fn next_policy_is_kept_while_unfinished_and_cleared_when_finished() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.db");
        let mut s = QueueStore::open(&path).unwrap();
        let id = new_item(&mut s);
        assert_eq!(s.next_policy(id).unwrap(), None);
        s.set_next_policy(id, Some(ConflictPolicy::Resume)).unwrap();
        assert_eq!(s.get(id).unwrap().unwrap().state, TransferState::Queued);
        assert_eq!(s.next_policy(id).unwrap(), Some(ConflictPolicy::Resume));

        // Kept through every non-terminal state but `AwaitingDecision`,
        // alongside the pause origin.
        for st in [
            TransferState::Paused,
            TransferState::Queued,
            TransferState::Active,
            TransferState::Paused,
        ] {
            s.set_state(id, &st).unwrap();
            assert_eq!(s.next_policy(id).unwrap(), Some(ConflictPolicy::Resume));
        }
        assert_eq!(s.paused_from(id).unwrap(), Some(PausedFrom::Active));
        s.set_state(id, &TransferState::Queued).unwrap();
        s.set_state(id, &TransferState::Active).unwrap();
        s.set_next_policy(id, Some(ConflictPolicy::Overwrite))
            .unwrap();

        // Recovery after a restart keeps it.
        drop(s);
        let mut s = QueueStore::open(&path).unwrap();
        s.recover().unwrap();
        assert_eq!(s.paused_from(id).unwrap(), Some(PausedFrom::Active));
        assert_eq!(s.next_policy(id).unwrap(), Some(ConflictPolicy::Overwrite));
        s.set_next_policy(id, None).unwrap();
        assert_eq!(s.next_policy(id).unwrap(), None);
        assert_eq!(s.paused_from(id).unwrap(), Some(PausedFrom::Active));

        // Any other state but `Failed` and `Cancelled` clears it.
        let conflict = ConflictInfo {
            src_size: 1,
            dst_size: 2,
            src_modified: None,
            dst_modified: None,
        };
        for st in [
            TransferState::AwaitingDecision(conflict),
            TransferState::Done,
            TransferState::Skipped("exists".to_string()),
        ] {
            let other = new_item(&mut s);
            s.set_next_policy(other, Some(ConflictPolicy::Resume))
                .unwrap();
            s.set_state(other, &st).unwrap();
            assert_eq!(s.next_policy(other).unwrap(), None, "{st:?}");
            s.set_state(other, &TransferState::Queued).unwrap();
            assert_eq!(s.next_policy(other).unwrap(), None, "{st:?}");
        }
    }

    #[test]
    fn failed_and_cancelled_keep_next_policy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.db");
        let mut s = QueueStore::open(&path).unwrap();
        let cases = [
            (
                TransferState::Failed(Error::new(ErrorKind::Io, "x")),
                ConflictPolicy::Overwrite,
            ),
            (TransferState::Cancelled, ConflictPolicy::Resume),
        ];
        let mut ids = Vec::new();
        for (st, policy) in &cases {
            let id = new_item(&mut s);
            s.set_state(id, &TransferState::Active).unwrap();
            s.set_next_policy(id, Some(*policy)).unwrap();
            s.set_state(id, st).unwrap();
            assert_eq!(s.next_policy(id).unwrap(), Some(*policy), "{st:?}");
            ids.push(id);
        }
        // Without a continuation they stay without one.
        let plain = new_item(&mut s);
        s.set_state(plain, &TransferState::Cancelled).unwrap();
        assert_eq!(s.next_policy(plain).unwrap(), None);

        // Kept across a reopen, and into the retried run.
        drop(s);
        let mut s = QueueStore::open(&path).unwrap();
        s.recover().unwrap();
        for (id, (_, policy)) in ids.iter().zip(&cases) {
            assert_eq!(s.next_policy(*id).unwrap(), Some(*policy));
            s.set_state(*id, &TransferState::Queued).unwrap();
            assert_eq!(s.next_policy(*id).unwrap(), Some(*policy));
        }
    }

    #[test]
    fn sync_mode_follows_the_journal_mode() {
        assert_eq!(sync_mode_for("wal"), "NORMAL");
        assert_eq!(sync_mode_for("delete"), "FULL");
        assert_eq!(sync_mode_for("memory"), "FULL");
    }

    /// Creates a schema-v1 queue file with the given rows
    /// `(state, state_detail, is_dir)`.
    fn write_v1(path: &Path, rows: &[(&str, Option<&str>, Option<bool>)]) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(MIGRATION_V1).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        for (state, detail, is_dir) in rows {
            conn.execute(
                "INSERT INTO transfers (site, direction, local_path, remote_path, is_dir, \
                 state, state_detail, created_at) VALUES ('site', 'upload', '/a', '/b', ?1, ?2, ?3, 0)",
                params![is_dir, state, detail],
            )
            .unwrap();
        }
    }

    #[test]
    fn opening_a_v1_database_migrates_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.db");
        write_v1(
            &path,
            &[
                ("queued", Some("override:resume"), None),
                ("paused", Some("queued;override:overwrite"), Some(false)),
                ("paused", Some("active"), Some(false)),
                ("paused", Some("active"), Some(true)),
                ("paused", Some("awaiting_decision"), Some(false)),
                ("active", None, Some(false)),
                ("queued", None, None),
            ],
        );
        let expected = [
            (Some(ConflictPolicy::Resume), None),
            (Some(ConflictPolicy::Overwrite), Some(PausedFrom::Queued)),
            // Schema v1 continued every copy paused while active.
            (Some(ConflictPolicy::Resume), Some(PausedFrom::Active)),
            (None, Some(PausedFrom::Active)),
            (None, Some(PausedFrom::AwaitingDecision)),
            (Some(ConflictPolicy::Resume), None),
            (None, None),
        ];
        // Opening again leaves the migrated file as it is.
        for _ in 0..2 {
            let s = QueueStore::open(&path).unwrap();
            let version: i64 = s
                .conn
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap();
            assert_eq!(version, SCHEMA_VERSION);
            let items = s.list_all().unwrap();
            assert_eq!(items.len(), expected.len());
            for (item, (policy, origin)) in items.iter().zip(expected) {
                assert_eq!(s.next_policy(item.id).unwrap(), policy, "{item:?}");
                assert_eq!(s.paused_from(item.id).unwrap(), origin, "{item:?}");
            }
        }
    }

    /// A schema-v2 queue file with one failed row.
    fn write_v2_failed(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(MIGRATION_V1).unwrap();
        conn.execute_batch(MIGRATION_V2).unwrap();
        conn.pragma_update(None, "user_version", 2).unwrap();
        conn.execute(
            "INSERT INTO transfers (site, direction, local_path, remote_path, state, error_kind, \
             error_message, error_detail, created_at) VALUES ('site', 'upload', '/a', '/b', \
             'failed', 'permission', 'Permission denied', 'EACCES', 0)",
            [],
        )
        .unwrap();
    }

    #[test]
    fn opening_a_v2_database_migrates_to_v3() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.db");
        write_v2_failed(&path);
        for _ in 0..2 {
            let s = QueueStore::open(&path).unwrap();
            let version: i64 = s
                .conn
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap();
            assert_eq!(version, SCHEMA_VERSION);
            let items = s.list_all().unwrap();
            let TransferState::Failed(e) = &items[0].state else {
                panic!("{items:?}");
            };
            assert_eq!(e.kind, ErrorKind::Permission);
            assert_eq!(e.message, "Permission denied");
            assert_eq!(e.detail.as_deref(), Some("EACCES"));
            assert_eq!((e.code, e.args.len()), (None, 0));
        }
    }

    /// A schema-v3 queue file (hoprs 1.5.0) with a folder and its child.
    fn write_v3(path: &Path) {
        let conn = Connection::open(path).unwrap();
        for migration in [MIGRATION_V1, MIGRATION_V2, MIGRATION_V3] {
            conn.execute_batch(migration).unwrap();
        }
        conn.pragma_update(None, "user_version", 3).unwrap();
        conn.execute_batch(
            "INSERT INTO transfers (id, site, direction, local_path, remote_path, is_dir, state, \
             next_policy, created_at) VALUES (1, 'site', 'upload', '/a', '/b', 1, 'active', \
             'overwrite', 0);
             INSERT INTO transfers (id, parent_id, site, direction, local_path, remote_path, \
             is_dir, state, created_at) VALUES (2, 1, 'site', 'upload', '/a/x', '/b/x', 0, \
             'queued', 0);",
        )
        .unwrap();
    }

    #[test]
    fn v3_queue_opens_as_v4() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.db");
        write_v3(&path);
        for _ in 0..2 {
            let s = QueueStore::open(&path).unwrap();
            let version: i64 = s
                .conn
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap();
            assert_eq!(version, 4);
            let items = s.list_all().unwrap();
            assert_eq!(items.len(), 2);
            for item in &items {
                assert_eq!(item.origin, Origin::User, "{item:?}");
                assert_eq!(item.tree_policy, None, "{item:?}");
            }
            assert_eq!(items[1].parent, Some(items[0].id));
            assert_eq!(
                s.next_policy(items[0].id).unwrap(),
                Some(ConflictPolicy::Overwrite)
            );
        }
    }

    #[test]
    fn tree_policy_and_origin_round_trip() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let root = s
            .insert_with(
                None,
                &spec("/a", "/b"),
                Some(ConflictPolicy::Skip),
                Origin::Agent,
            )
            .unwrap();
        let ids = s
            .insert_children(
                root,
                &[NewChild {
                    spec: spec("/a/f", "/b/f"),
                    is_dir: false,
                    size: Some(1),
                    state: TransferState::Queued,
                    tree_policy: Some(ConflictPolicy::Skip),
                    origin: Origin::Agent,
                }],
            )
            .unwrap();
        let plain = new_item(&mut s);
        for id in [root, ids[0]] {
            let item = s.get(id).unwrap().unwrap();
            assert_eq!(
                (item.tree_policy, item.origin),
                (Some(ConflictPolicy::Skip), Origin::Agent)
            );
        }
        let item = s.get(plain).unwrap().unwrap();
        assert_eq!((item.tree_policy, item.origin), (None, Origin::User));
        // A user's row stores nothing in the new columns.
        let nulls: i64 = s
            .conn
            .query_row(
                "SELECT tree_policy IS NULL AND origin IS NULL FROM transfers WHERE id = ?1",
                [plain.0],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(nulls, 1);
    }

    #[test]
    fn coded_error_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.db");
        let id = {
            let mut s = QueueStore::open(&path).unwrap();
            let id = new_item(&mut s);
            let err = Error::coded(ErrorKind::Network, "net-test-code", "Cannot reach h")
                .arg("host", "h")
                .arg("port", "22")
                .with_detail("refused");
            s.set_state(id, &TransferState::Failed(err)).unwrap();
            id
        };
        let s = QueueStore::open(&path).unwrap();
        let TransferState::Failed(e) = s.get(id).unwrap().unwrap().state else {
            panic!("not failed");
        };
        assert_eq!(e.code, Some("net-test-code"));
        let mut args = e.args.clone();
        args.sort();
        assert_eq!(
            args,
            vec![("host", "h".to_owned()), ("port", "22".to_owned())]
        );
        assert_eq!(e.message, "Cannot reach h");
        assert_eq!(e.detail.as_deref(), Some("refused"));
    }

    #[test]
    fn invalid_stored_codes_read_as_no_code() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let id = new_item(&mut s);
        let err = Error::coded(ErrorKind::Io, "io-test-code", "Broken");
        s.set_state(id, &TransferState::Failed(err)).unwrap();
        let long = "a".repeat(65);
        for bad in ["Has Space", "UPPER", "", long.as_str(), "snow\u{2603}"] {
            s.conn
                .execute("UPDATE transfers SET error_code = ?1", [bad])
                .unwrap();
            let TransferState::Failed(e) = s.get(id).unwrap().unwrap().state else {
                panic!("not failed");
            };
            assert_eq!(e.code, None, "{bad}");
            assert_eq!(e.message, "Broken");
        }
    }

    #[test]
    fn malformed_error_args_reads_as_no_args() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let id = new_item(&mut s);
        let err = Error::coded(ErrorKind::Io, "io-test-code", "Broken").arg("a", "b");
        s.set_state(id, &TransferState::Failed(err)).unwrap();
        for bad in ["{not json", "[1,2]", "{\"a\": 3}", ""] {
            s.conn
                .execute("UPDATE transfers SET error_args = ?1", [bad])
                .unwrap();
            let TransferState::Failed(e) = s.get(id).unwrap().unwrap().state else {
                panic!("not failed");
            };
            assert_eq!(e.code, Some("io-test-code"), "{bad}");
            assert!(e.args.is_empty(), "{bad}");
            assert_eq!(e.message, "Broken");
        }
    }

    #[test]
    fn a_newer_schema_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.db");
        QueueStore::open(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        drop(conn);
        let err = QueueStore::open(&path).err().expect("newer schema refused");
        assert_eq!(err.kind, ErrorKind::Io);
        assert!(err.message.contains("newer"), "{err:?}");
    }

    #[test]
    fn file_databases_use_wal_with_normal_sync() {
        let dir = tempfile::tempdir().unwrap();
        let s = QueueStore::open(&dir.path().join("queue.db")).unwrap();
        let mode: String = s
            .conn
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        let sync: i64 = s
            .conn
            .pragma_query_value(None, "synchronous", |r| r.get(0))
            .unwrap();
        assert_eq!((mode.as_str(), sync), ("wal", 1), "1 = NORMAL");
    }

    #[test]
    fn items_persist_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.db");
        let id = {
            let mut s = QueueStore::open(&path).unwrap();
            new_item(&mut s)
        };
        let s = QueueStore::open(&path).unwrap();
        let items = s.list_all().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, id);
    }

    #[test]
    fn clear_finished_removes_terminal_roots_with_descendants() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let done_root = new_item(&mut s);
        let child = s.insert(Some(done_root), &spec("/a/x", "/b/x")).unwrap();
        let grandchild = s.insert(Some(child), &spec("/a/x/y", "/b/x/y")).unwrap();
        let queued_root = new_item(&mut s);
        s.set_state(done_root, &TransferState::Done).unwrap();

        assert_eq!(s.clear_finished().unwrap(), 1);
        assert!(s.get(done_root).unwrap().is_none());
        assert!(s.get(child).unwrap().is_none());
        assert!(s.get(grandchild).unwrap().is_none());
        assert!(s.get(queued_root).unwrap().is_some());
    }

    #[test]
    fn clear_finished_except_keeps_the_listed_roots() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let kept = new_item(&mut s);
        let kept_child = s.insert(Some(kept), &spec("/a/k", "/b/k")).unwrap();
        let cleared = new_item(&mut s);
        s.set_state(kept, &TransferState::Cancelled).unwrap();
        s.set_state(cleared, &TransferState::Done).unwrap();

        assert_eq!(s.clear_finished_except(&[kept]).unwrap(), 1);
        assert!(s.get(cleared).unwrap().is_none());
        assert!(s.get(kept).unwrap().is_some());
        assert!(s.get(kept_child).unwrap().is_some());
    }

    #[test]
    fn corrupt_state_is_an_error_not_a_panic() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let id = new_item(&mut s);
        s.conn
            .execute("UPDATE transfers SET state = 'bogus'", [])
            .unwrap();
        let err = s.get(id).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Io);
    }
}
