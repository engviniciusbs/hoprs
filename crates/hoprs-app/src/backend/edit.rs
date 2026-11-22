//! Remote editing (Plan 6, Task 7): a remote file is downloaded to a private local
//! directory, opened in the user's editor, watched, and uploaded again on every save.
//!
//! **Where.** `<root>/<session>/<remote path>`, the root being `$XDG_RUNTIME_DIR/hoprs/edit`
//! (else the user's cache directory, see [`default_edit_dir`]). On Unix the directories are
//! `0700` and the files `0600`. The remote path is mapped component by component and
//! refused if any component is empty, `.`, `..`, holds a separator or NUL, or would make
//! the path leave the session directory ([`local_path_for`]). A session directory found
//! on disk before this run's first edit of that session (a crash left it; ids restart
//! each run) is moved aside as `<session>.stale-<timestamp>`, never deleted.
//!
//! **Editor.** The setting `system` opens the OS's default editor (`xdg-open`, `open -t`,
//! `ShellExecuteW`). Any other command is split with shell quoting rules ([`shlex`]; on
//! Windows, command-line rules) but no shell runs: no expansion, no substitution. The path
//! is the last argument, as one argument. The editor is spawned detached (its own process
//! group, null stdio) and never waited on by the caller; a thread reaps it when it exits
//! ([`launch_editor`]).
//!
//! **Watching.** One `notify` watcher per session directory, watching the directory of
//! each edited file (added once it exists) and not the files themselves: editors that save by renaming a temporary file over the
//! original, or by removing and recreating it, are seen like plain writes (the event names
//! the edited path either way). Access and metadata events are ignored (our own reads
//! would otherwise wake us). Each session has a worker task that debounces every file
//! separately ([`DEBOUNCE`], tokio time) and then syncs it. A sync uploads only if the
//! local bytes differ from what was last downloaded or uploaded (a hash), so our own
//! writes and editors that touch without changing anything upload nothing.
//!
//! **Upload.** Before uploading, the remote file is `stat`ed: if its size, or its mtime to
//! the second (FTP's `MDTM` resolution; both stamps come from the same server and the
//! same command, so they compare exactly at that resolution), differ from the
//! [`RemoteStamp`] of the last download or upload, nothing is sent and
//! [`EditEvent::RemoteChanged`] asks the user ([`EditChoice`]). Otherwise the bytes go up
//! by [`UploadMode`]: SFTP writes a temporary file beside the original and renames it over
//! (OpenSSH's `posix-rename` replaces atomically; a server without it refuses to rename
//! over an existing file, and the upload then falls back to writing in place); FTP writes
//! in place (`RNTO` over an existing file is refused by many servers). After the upload
//! the file is `stat`ed again for the new stamp.
//!
//! **Down.** A save while the session is not `Connected`, or one whose upload fails with
//! a network error, is held as [`EditState::Pending`] ([`EditEvent::Pending`]). The worker
//! watches the session's state and syncs every pending edit when the session is
//! `Connected` again (Review Focus 3): on every observed state change whose current value
//! is `Connected`, whatever came before (a `Reconnecting`/`Connected` pair can arrive as
//! one change while the worker is busy). A save held while the session still says
//! `Connected` (the link died first) is also retried with a backoff ([`RETRY_MIN`] doubling
//! to [`RETRY_MAX`]).
//!
//! **A cut in-place write** may have truncated the remote copy (the events say so, with
//! `remote_may_be_truncated`, and a `warn` is logged). Cut by the network, the first retry
//! that reaches the server skips the stamp check once (the change was ours). Cut by
//! anything else (quota, permission), the remote file is `stat`ed at once and that stamp
//! becomes the baseline: later saves are checked as usual. A temporary upload file left
//! behind by a cut is removed (best effort) after the next successful upload or connect.
//! Temporary names, local and remote, carry a random suffix.
//!
//! **Recovered copies.** "Keep remote" and "Save a copy" first save the local version to
//! `<recovered>/<site>/<remote path>.local-<timestamp>` (`$XDG_DATA_HOME/hoprs/recovered`,
//! durable, `0700`/`0600`, never overwriting another copy), then replace the local file by
//! the server's version ([`EditEvent::ReplacedByRemote`]).
//!
//! **A cut in-place write** stays flagged (`remote_may_be_truncated` on every `Pending`
//! and `Failed` event) until an upload succeeds, and its bytes are sent again even if
//! the user reverts the file to what was last synced.
//!
//! **Closing.** Once a session is closed its worker stops retrying (later saves are held
//! without touching the network) until the UI's close flow decides:
//! [`EditManager::flush`] uploads what is unsent within a limit, [`EditManager::keep`]
//! saves the unsent edits as recovered copies and ends them without deleting anything,
//! and [`EditManager::finish`] removes a session's local files only when none of them
//! holds unsent changes (or the user said to discard them). Each waits for an upload in
//! flight first. [`EditManager::end_all`] (the window closes) finishes what is all sent
//! and keeps the rest. Recovered copies live elsewhere and are never removed. A "Keep
//! remote" or "Save a copy" that fails after saving the user's version says where it is
//! ([`EditEvent::Failed`] with `recovered`).
//!
//! Two `start`s of the same file at once make one edit (a lock per session and path).
//!
//! Logs name paths and sizes, never contents.

use std::collections::HashMap;
use std::ffi::OsString;
use std::hash::{BuildHasher, DefaultHasher, Hasher};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hoprs_core::{ConnectionState, EntryKind, Error, ErrorKind, RemoteEntry, RemotePath, Result};
use hoprs_remote::RemoteFs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::{SessionId, blocking};

/// Quiet time after the last change of a file before it is uploaded.
pub const DEBOUNCE: Duration = Duration::from_millis(500);
/// First retry of a save held while the session said `Connected`.
pub const RETRY_MIN: Duration = Duration::from_secs(1);
/// Longest wait between such retries.
pub const RETRY_MAX: Duration = Duration::from_secs(30);

/// One remote file being edited locally.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EditId(pub u64);

/// What the edits tell the UI ([`EditManager::subscribe`]).
#[derive(Clone, Debug, PartialEq)]
pub enum EditEvent {
    /// A save reached the server.
    Uploaded { id: EditId, at: SystemTime },
    /// The server's copy changed since the last download or upload: the save was not
    /// sent; the user picks an [`EditChoice`].
    RemoteChanged { id: EditId },
    /// The session is down: the save is held and goes up on the next `Connected`.
    /// `remote_may_be_truncated`: an in-place write was cut, the server's copy may be
    /// incomplete until the retry.
    Pending {
        id: EditId,
        remote_may_be_truncated: bool,
    },
    /// The upload failed for another reason; the next save tries again (see `Pending` for
    /// the flag).
    Failed {
        id: EditId,
        error: Error,
        remote_may_be_truncated: bool,
        /// "Keep remote" or "Save a copy" failed after saving the user's version here.
        recovered: Option<PathBuf>,
    },
    /// "Keep remote" or "Save a copy": the local file now holds the server's version; the
    /// user's version was saved at `recovered` (`None`: there was no local file). With
    /// `reload_editor`, the editor must reload the file.
    ReplacedByRemote {
        id: EditId,
        recovered: Option<PathBuf>,
        reload_editor: bool,
    },
    /// The edit ended ([`EditManager::finish`]); its local file is gone.
    Closed { id: EditId },
}

/// The user's answer to [`EditEvent::RemoteChanged`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EditChoice {
    /// Upload the local version over the server's.
    Overwrite,
    /// Save the local version as a recovered copy (see the module docs), then download
    /// the server's version over the local file.
    KeepRemote,
    /// Like `KeepRemote`, and also upload the local version beside the original as
    /// `<name>.hoprs-<timestamp>`; later saves still target the original.
    SaveCopy,
}

/// Where an edit stands.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EditState {
    /// The local file is what was last downloaded or uploaded.
    Synced,
    /// Saved locally, not sent yet (the debounce, or a save not seen yet).
    Modified,
    /// Held while the session is down.
    Pending,
    /// Held because the server's copy changed.
    Conflict,
    /// The last upload failed.
    Failed,
}

/// An edit as [`EditManager::pending`] and [`EditManager::finish`] report it.
#[derive(Clone, Debug, PartialEq)]
pub struct EditSummary {
    pub id: EditId,
    pub session: SessionId,
    pub remote: RemotePath,
    pub local: PathBuf,
    pub state: EditState,
}

/// An unsent edit [`EditManager::keep`] saved as a recovered copy.
#[derive(Clone, Debug, PartialEq)]
pub struct KeptEdit {
    pub id: EditId,
    pub remote: RemotePath,
    pub recovered: PathBuf,
    /// The server's copy may be incomplete: an upload of this edit was under way when it
    /// was kept (the close may cut it), or an earlier cut in-place write was never
    /// repaired. Said in the kept notice; never left silent.
    pub remote_may_be_truncated: bool,
}

/// How a save reaches the server (see the module docs).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UploadMode {
    /// Temporary file beside the original, renamed over it; in place if the rename is
    /// refused (SFTP).
    TempRename,
    /// Straight into the file (FTP).
    InPlace,
}

/// The server's copy as last seen: what a save is checked against.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RemoteStamp {
    pub size: u64,
    pub mtime: Option<SystemTime>,
}

impl RemoteStamp {
    fn of(entry: &RemoteEntry) -> Self {
        Self {
            size: entry.size,
            mtime: entry.modified,
        }
    }

    /// Same size, and the same mtime to the second when both are known.
    fn matches(&self, other: &RemoteStamp) -> bool {
        let secs = |t: SystemTime| t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs());
        self.size == other.size
            && match (self.mtime, other.mtime) {
                (Some(a), Some(b)) => secs(a) == secs(b),
                _ => true,
            }
    }
}

/// The session an edit belongs to, as [`EditManager::start`] needs it.
pub struct EditSession {
    pub session: SessionId,
    /// Names the session's folder of recovered copies (the site id).
    pub site: String,
    pub fs: Arc<dyn RemoteFs>,
    /// The session's connection state (the edits of a session follow the first one's).
    pub state: watch::Receiver<ConnectionState>,
    pub mode: UploadMode,
}

/// Opens a local file in the editor: `(editor command, path)`. Runs on the blocking pool.
pub type Launcher = Arc<dyn Fn(&str, &Path) -> Result<()> + Send + Sync>;

/// See the module docs. Cheap to clone.
#[derive(Clone)]
pub struct EditManager {
    inner: Arc<Inner>,
}

/// Held by a `start` of one session and remote path.
type StartLock = Arc<tokio::sync::Mutex<()>>;

struct Inner {
    root: PathBuf,
    recovered: PathBuf,
    watch_files: bool,
    launcher: Launcher,
    next_id: AtomicU64,
    events: broadcast::Sender<EditEvent>,
    edits: Mutex<HashMap<EditId, Arc<Edit>>>,
    sessions: Mutex<HashMap<SessionId, SessionEdits>>,
    /// Serializes the creation of session directories.
    setup: tokio::sync::Mutex<()>,
    /// One `start` at a time per session and remote path.
    starting: Mutex<HashMap<(SessionId, RemotePath), StartLock>>,
    /// Serializes [`Inner::preserve`].
    preserving: tokio::sync::Mutex<()>,
}

/// A session's directory, watcher and worker.
struct SessionEdits {
    dir: PathBuf,
    /// Keeps the worker's channel open (the watcher holds a clone); tests and the fake
    /// backend report saves through it ([`EditManager::file_event`]).
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "only read by the test-only `file_event`")
    )]
    files: mpsc::UnboundedSender<PathBuf>,
    worker: JoinHandle<()>,
    /// Watches the directory of each edited file; dropping it stops it.
    watcher: Option<notify::RecommendedWatcher>,
}

struct Edit {
    id: EditId,
    session: SessionId,
    site: String,
    remote: RemotePath,
    local: PathBuf,
    fs: Arc<dyn RemoteFs>,
    mode: UploadMode,
    state: watch::Receiver<ConnectionState>,
    /// One sync or resolution at a time.
    turn: Arc<tokio::sync::Mutex<()>>,
    sync: Mutex<SyncState>,
    /// `keep`/`finish` is ending this edit: no sync may start a write any more (checked
    /// once the turn is taken, and again right before the upload).
    ending: AtomicBool,
}

struct SyncState {
    stamp: RemoteStamp,
    /// Hash of the bytes last downloaded or uploaded.
    synced: u64,
    state: EditState,
    /// The next attempt that reaches the server skips the stamp check, once: the user
    /// chose "Overwrite", or the network cut an in-place upload of ours.
    skip_check: bool,
    /// Temporary upload files a cut may have left on the server.
    orphans: Vec<RemotePath>,
    /// The last unsent version saved as a recovered copy (its hash, the copy), so the
    /// same bytes are not copied twice.
    preserved: Option<(u64, PathBuf)>,
    /// An in-place write of ours was cut: the server's copy may be incomplete until an
    /// upload succeeds (every `Pending`/`Failed` event says so meanwhile).
    truncated: bool,
}

impl SyncState {
    fn synced(stamp: RemoteStamp, synced: u64, orphans: Vec<RemotePath>) -> Self {
        Self {
            stamp,
            synced,
            state: EditState::Synced,
            skip_check: false,
            orphans,
            preserved: None,
            truncated: false,
        }
    }
}

impl Edit {
    /// The session says `Connected` and is not closed (a closed session's last state
    /// may still read `Connected`).
    fn connected(&self) -> bool {
        self.state.has_changed().is_ok()
            && matches!(*self.state.borrow(), ConnectionState::Connected)
    }

    fn sync_state(&self) -> MutexGuard<'_, SyncState> {
        lock(&self.sync)
    }

    fn set_state(&self, state: EditState) {
        self.sync_state().state = state;
    }

    fn summary(&self, state: EditState) -> EditSummary {
        EditSummary {
            id: self.id,
            session: self.session,
            remote: self.remote.clone(),
            local: self.local.clone(),
            state,
        }
    }
}

/// How a sync ended.
#[derive(Debug)]
enum Outcome {
    Unchanged,
    Uploaded,
    Pending,
    Conflict,
    Failed(Error),
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn digest(data: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write(data);
    hasher.finish()
}

/// 16 random hex digits, for temporary names.
fn nonce() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    hasher.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    hasher.write_u32(std::process::id());
    format!("{:016x}", hasher.finish())
}

/// `YYYYmmdd-HHMMSS`, local time, for the names of kept copies.
fn timestamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

fn no_edit() -> Error {
    Error::coded(ErrorKind::NotFound, "edit-unknown", "No such edit")
}

impl EditManager {
    /// Edits under `root`, recovered copies under `recovered`, watched with `notify`,
    /// opened with [`launch_editor`].
    pub fn new(root: PathBuf, recovered: PathBuf) -> Self {
        Self::with_options(root, recovered, true, Arc::new(launch_editor))
    }

    /// `watch_files: false` leaves the file events to [`file_event`](Self::file_event)
    /// (tests, the fake backend).
    pub fn with_options(
        root: PathBuf,
        recovered: PathBuf,
        watch_files: bool,
        launcher: Launcher,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                root,
                recovered,
                watch_files,
                launcher,
                next_id: AtomicU64::new(1),
                events: broadcast::channel(256).0,
                edits: Mutex::new(HashMap::new()),
                sessions: Mutex::new(HashMap::new()),
                setup: tokio::sync::Mutex::new(()),
                starting: Mutex::new(HashMap::new()),
                preserving: tokio::sync::Mutex::new(()),
            }),
        }
    }

    /// Downloads `remote` into the session's directory, remembers its [`RemoteStamp`] and
    /// opens it with `editor`. A file already being edited is not downloaded again (its
    /// local copy may hold unsent changes): the editor opens it again, same id.
    pub async fn start(
        &self,
        target: EditSession,
        remote: RemotePath,
        editor: &str,
    ) -> Result<EditId> {
        let EditSession {
            session,
            site,
            fs,
            state,
            mode,
        } = target;
        let key = (session, remote.clone());
        let starting = lock(&self.inner.starting)
            .entry(key.clone())
            .or_default()
            .clone();
        let result = {
            let _starting = starting.lock().await;
            let target = EditSession {
                session,
                site,
                fs,
                state,
                mode,
            };
            self.start_locked(target, remote, editor).await
        };
        // The lock goes once no other `start` holds or waits for it (they clone it under
        // the map's lock, so the count cannot grow behind this check).
        let mut map = lock(&self.inner.starting);
        if Arc::strong_count(&starting) <= 2 {
            map.remove(&key);
        }
        result
    }

    /// [`start`](Self::start), under the lock of its session and path.
    async fn start_locked(
        &self,
        target: EditSession,
        remote: RemotePath,
        editor: &str,
    ) -> Result<EditId> {
        let EditSession {
            session,
            site,
            fs,
            state,
            mode,
        } = target;
        let existing = lock(&self.inner.edits)
            .values()
            .find(|edit| edit.session == session && edit.remote == remote)
            .cloned();
        if let Some(edit) = existing {
            self.launch(editor, &edit.local).await?;
            return Ok(edit.id);
        }
        let entry = fs.stat(&remote).await?;
        if entry.kind != EntryKind::File {
            return Err(
                Error::coded(ErrorKind::Io, "edit-only-files", "Only files can be edited")
                    .with_detail(remote.to_string()),
            );
        }
        let dir = self.session_dir(session, &state).await?;
        let local = local_path_for(&dir, remote.as_str())?;
        let data = fetch(fs.as_ref(), &remote).await?;
        let size = data.len();
        let synced = digest(&data);
        {
            let local = local.clone();
            blocking(move || write_private(&local, &data)).await?;
        }
        if let Some(parent) = local.parent() {
            self.watch_dir(session, parent)?;
        }
        let id = EditId(self.inner.next_id.fetch_add(1, Ordering::SeqCst));
        let edit = Arc::new(Edit {
            id,
            session,
            site,
            remote: remote.clone(),
            local: local.clone(),
            fs,
            mode,
            state,
            turn: Arc::default(),
            ending: AtomicBool::new(false),
            sync: Mutex::new(SyncState::synced(
                RemoteStamp::of(&entry),
                synced,
                Vec::new(),
            )),
        });
        lock(&self.inner.edits).insert(id, edit);
        tracing::info!(edit = id.0, remote = %remote, local = %local.display(), size, "remote edit started");
        if let Err(error) = self.launch(editor, &local).await {
            lock(&self.inner.edits).remove(&id);
            let _ = blocking(move || {
                let _ = std::fs::remove_file(&local);
                Ok(())
            })
            .await;
            return Err(error);
        }
        Ok(id)
    }

    /// Settles an [`EditEvent::RemoteChanged`] (see [`EditChoice`]). "Overwrite" while the
    /// session is down is held like a save ([`EditEvent::Pending`]); the other two need
    /// the session.
    pub async fn resolve(&self, id: EditId, choice: EditChoice) -> Result<()> {
        let edit = self.inner.edit(id).ok_or_else(no_edit)?;
        match choice {
            EditChoice::Overwrite => match self.inner.sync(&edit, true).await {
                Outcome::Failed(error) => Err(error),
                Outcome::Conflict => Err(Error::coded(
                    ErrorKind::Conflict,
                    "edit-remote-changed",
                    "The remote file changed",
                )),
                Outcome::Unchanged | Outcome::Uploaded | Outcome::Pending => Ok(()),
            },
            EditChoice::KeepRemote | EditChoice::SaveCopy => {
                let _turn = edit.turn.lock().await;
                if edit.ending.load(Ordering::SeqCst) {
                    return Err(no_edit());
                }
                let local = edit.local.clone();
                let data = blocking(move || read_local(&local)).await?;
                // The user's version is safe on disk before anything else happens.
                let recovered = match &data {
                    Some(data) => Some(self.inner.recover(&edit, data.clone()).await?),
                    None => None,
                };
                let replaced = async {
                    if choice == EditChoice::SaveCopy {
                        let data = data.ok_or_else(|| {
                            Error::coded(ErrorKind::NotFound, "edit-local-gone", "The local file is gone")
                        })?;
                        let name = edit.remote.file_name().unwrap_or("file");
                        let parent = edit.remote.parent().unwrap_or_else(RemotePath::root);
                        let copy = parent.join(&format!("{name}.hoprs-{}", timestamp()));
                        if edit.fs.stat(&copy).await.is_ok() {
                            return Err(Error::coded(ErrorKind::Conflict, "edit-copy-exists", "The copy already exists")
                                .with_detail(copy.to_string()));
                        }
                        write_remote(edit.fs.as_ref(), &copy, &data).await?;
                        tracing::info!(edit = id.0, copy = %copy, size = data.len(), "saved the local version beside the remote file");
                    }
                    self.inner.adopt_remote(&edit).await
                }
                .await;
                if let Err(error) = replaced {
                    // Half way: the edit still waits for a choice, and the user's version
                    // is safe at `recovered` (the event says where).
                    tracing::warn!(edit = id.0, remote = %edit.remote, %error, "the conflict could not be settled");
                    self.inner.emit(EditEvent::Failed {
                        id,
                        error: error.clone(),
                        remote_may_be_truncated: edit.sync_state().truncated,
                        recovered,
                    });
                    return Err(error);
                }
                self.inner.emit(EditEvent::ReplacedByRemote {
                    id,
                    recovered,
                    reload_editor: true,
                });
                Ok(())
            }
        }
    }

    /// The session's edits with something not on the server yet (anything but
    /// [`EditState::Synced`]; a synced edit whose local file differs counts as
    /// [`EditState::Modified`]).
    pub async fn pending(&self, session: SessionId) -> Vec<EditSummary> {
        let edits = self.inner.session_edits(session);
        let mut pending = Vec::new();
        for edit in edits {
            if let Some(summary) = unsent(&edit).await {
                pending.push(summary);
            }
        }
        pending
    }

    /// Ends the session's edits: with nothing unsent (or `discard`), stops its watcher and
    /// worker, deletes its local directory and sends [`EditEvent::Closed`] for each edit,
    /// returning an empty list. Otherwise changes nothing and returns the unsent edits.
    /// An upload in flight finishes first.
    pub async fn finish(&self, session: SessionId, discard: bool) -> Result<Vec<EditSummary>> {
        let edits = self.inner.session_edits(session);
        for edit in &edits {
            edit.ending.store(true, Ordering::SeqCst);
        }
        let mut turns = Vec::with_capacity(edits.len());
        for edit in &edits {
            turns.push(edit.turn.clone().lock_owned().await);
        }
        if !discard {
            let mut pending = Vec::new();
            for edit in &edits {
                if let Some(summary) = unsent(edit).await {
                    pending.push(summary);
                }
            }
            if !pending.is_empty() {
                // Not ending after all: they go on as before.
                self.unend(&edits);
                return Ok(pending);
            }
        }
        let record = lock(&self.inner.sessions).remove(&session);
        if let Some(record) = &record {
            record.worker.abort();
        }
        {
            let mut all = lock(&self.inner.edits);
            for edit in &edits {
                all.remove(&edit.id);
            }
        }
        if let Some(record) = record {
            let dir = record.dir.clone();
            drop(record);
            blocking(move || match std::fs::remove_dir_all(&dir) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(Error::from(e).with_detail(dir.display().to_string())),
            })
            .await?;
        }
        drop(turns);
        let mut ids: Vec<EditId> = edits.iter().map(|edit| edit.id).collect();
        ids.sort();
        tracing::info!(
            session = session.0,
            edits = ids.len(),
            discard,
            "remote edits finished"
        );
        for id in ids {
            self.inner.emit(EditEvent::Closed { id });
        }
        Ok(Vec::new())
    }

    /// "Retry" on a failed upload: syncs the edit now (with the usual stamp check).
    pub async fn retry(&self, id: EditId) -> Result<()> {
        let edit = self.inner.edit(id).ok_or_else(no_edit)?;
        match self.inner.sync(&edit, false).await {
            Outcome::Failed(error) => Err(error),
            Outcome::Conflict => Err(Error::coded(
                ErrorKind::Conflict,
                "edit-remote-changed",
                "The remote file changed",
            )),
            Outcome::Unchanged | Outcome::Uploaded | Outcome::Pending => Ok(()),
        }
    }

    /// "Upload pending changes": syncs every unsent edit of the session now, and again
    /// whenever the session is `Connected`, for at most `within`; returns what is still
    /// unsent. A conflict is never overwritten here (it is reported at once), and a closed
    /// session ends the wait at once.
    pub async fn flush(&self, session: SessionId, within: Duration) -> Vec<EditSummary> {
        let deadline = Instant::now() + within;
        let Some(mut state) = self
            .inner
            .session_edits(session)
            .first()
            .map(|edit| edit.state.clone())
        else {
            return Vec::new();
        };
        loop {
            state.borrow_and_update();
            let edits = self.inner.session_edits(session);
            if edits.iter().any(|edit| edit.connected()) {
                for edit in &edits {
                    // The deadline is checked before each sync, never during one: an
                    // upload under way finishes (or fails) under the protocol's own
                    // timeouts, so a cut in-place write is always recorded as such.
                    // Stopped ("keep" ended the edits): no further sync.
                    if Instant::now() >= deadline || edit.ending.load(Ordering::SeqCst) {
                        break;
                    }
                    let unsent = unsent(edit).await;
                    if unsent.is_some_and(|s| s.state != EditState::Conflict) {
                        self.inner.sync(edit, false).await;
                    }
                }
            }
            let left = self.pending(session).await;
            let closed = state.has_changed().is_err()
                || self
                    .inner
                    .session_edits(session)
                    .iter()
                    .any(|edit| edit.ending.load(Ordering::SeqCst));
            if left.iter().all(|s| s.state == EditState::Conflict)
                || closed
                || Instant::now() >= deadline
            {
                return left;
            }
            // Wait for the session to change (a reconnect), else retry a little later.
            let retry = (Instant::now() + RETRY_MIN).min(deadline);
            tokio::select! {
                _ = state.changed() => {}
                () = tokio::time::sleep_until(retry) => {}
            }
        }
    }

    /// "Keep local copies": saves each unsent edit of the session as a recovered copy
    /// (see the module docs), then ends the session's edits (worker and watcher stopped,
    /// [`EditEvent::Closed`] for each) **without deleting anything**: the edit directory
    /// stays as it is. An upload in flight finishes first. If a copy cannot be written,
    /// nothing is ended (the error says why).
    pub async fn keep(&self, session: SessionId) -> Result<Vec<KeptEdit>> {
        let edits = self.inner.session_edits(session);
        // From here no sync of these edits starts a write (a flush's next sync, a
        // debounce), and a flush stops between syncs.
        for edit in &edits {
            edit.ending.store(true, Ordering::SeqCst);
        }
        // An upload already under way is not waited for: its turn is taken if free, and
        // a busy one marks the copy "the server's may be incomplete".
        let mut turns = Vec::with_capacity(edits.len());
        let mut busy = Vec::new();
        for edit in &edits {
            match edit.turn.clone().try_lock_owned() {
                Ok(turn) => turns.push(turn),
                Err(_) => busy.push(edit.id),
            }
        }
        let mut kept = Vec::new();
        for edit in &edits {
            let in_flight = busy.contains(&edit.id);
            if !in_flight && unsent(edit).await.is_none() {
                continue;
            }
            let local = edit.local.clone();
            let data = match blocking(move || read_local(&local)).await {
                Ok(Some(data)) => data,
                Ok(None) => continue,
                Err(error) => {
                    self.unend(&edits);
                    return Err(error);
                }
            };
            let recovered = match self.inner.preserve(edit, data).await {
                Ok(path) => path,
                Err(error) => {
                    // Nothing is ended when a copy fails.
                    self.unend(&edits);
                    return Err(error);
                }
            };
            let remote_may_be_truncated = in_flight || edit.sync_state().truncated;
            if remote_may_be_truncated {
                tracing::warn!(edit = edit.id.0, remote = %edit.remote, "kept while an upload was under way; the server's copy may be incomplete");
            }
            kept.push(KeptEdit {
                id: edit.id,
                remote: edit.remote.clone(),
                recovered,
                remote_may_be_truncated,
            });
        }
        if let Some(record) = lock(&self.inner.sessions).remove(&session) {
            record.worker.abort();
        }
        {
            let mut all = lock(&self.inner.edits);
            for edit in &edits {
                all.remove(&edit.id);
            }
        }
        drop(turns);
        tracing::info!(
            session = session.0,
            edits = edits.len(),
            kept = kept.len(),
            "remote edits ended; local copies kept"
        );
        for edit in &edits {
            self.inner.emit(EditEvent::Closed { id: edit.id });
        }
        Ok(kept)
    }

    /// A `keep` that failed leaves the edits as they were.
    fn unend(&self, edits: &[Arc<Edit>]) {
        for edit in edits {
            edit.ending.store(false, Ordering::SeqCst);
        }
    }

    /// The window closes: ends every session's edits. A session with nothing unsent loses
    /// its edit directory ([`finish`](Self::finish)); unsent edits are kept as recovered
    /// copies ([`keep`](Self::keep)) and nothing of theirs is deleted. Failures are logged.
    pub async fn end_all(&self) -> Vec<KeptEdit> {
        let mut sessions: Vec<SessionId> = lock(&self.inner.edits)
            .values()
            .map(|edit| edit.session)
            .collect();
        sessions.sort();
        sessions.dedup();
        let mut kept = Vec::new();
        for session in sessions {
            let ended = match self.finish(session, false).await {
                Ok(unsent) if unsent.is_empty() => Ok(Vec::new()),
                Ok(_) => self.keep(session).await,
                Err(error) => Err(error),
            };
            match ended {
                Ok(copies) => kept.extend(copies),
                Err(error) => {
                    tracing::warn!(session = session.0, %error, "cannot end the remote edits; their files stay");
                }
            }
        }
        kept
    }

    /// The window closes (or is forced to): saves each unsent edit of every session as a
    /// recovered copy **now**, locally only. It neither waits for an upload in flight
    /// nor touches the network, so it runs first, before the shutdown's slower steps.
    /// Each version is saved once: a later [`keep`](Self::keep) of the same bytes reuses
    /// the copy. Failures are logged.
    pub async fn preserve_unsent(&self) -> Vec<KeptEdit> {
        let mut edits: Vec<Arc<Edit>> = lock(&self.inner.edits).values().cloned().collect();
        edits.sort_by_key(|edit| edit.id);
        let mut kept = Vec::new();
        for edit in edits {
            if unsent(&edit).await.is_none() {
                continue;
            }
            let local = edit.local.clone();
            let data = match blocking(move || read_local(&local)).await {
                Ok(Some(data)) => data,
                Ok(None) => continue,
                Err(error) => {
                    tracing::warn!(edit = edit.id.0, %error, "cannot read an unsent edit to keep it");
                    continue;
                }
            };
            match self.inner.preserve(&edit, data).await {
                Ok(recovered) => kept.push(KeptEdit {
                    id: edit.id,
                    remote: edit.remote.clone(),
                    recovered,
                    // An upload under way may be cut by the exit.
                    remote_may_be_truncated: edit.turn.try_lock().is_err()
                        || edit.sync_state().truncated,
                }),
                Err(error) => {
                    tracing::warn!(edit = edit.id.0, %error, "cannot keep an unsent edit; its local file stays");
                }
            }
        }
        kept
    }

    /// How many `start` locks are held (tests: they go once the starts are done).
    #[cfg(test)]
    fn starting_len(&self) -> usize {
        lock(&self.inner.starting).len()
    }

    /// The edits' events, from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<EditEvent> {
        self.inner.events.subscribe()
    }

    /// Where recovered copies go (see the module docs).
    pub fn recovered_dir(&self) -> &Path {
        &self.inner.recovered
    }

    /// The local copy of an edit, while it lasts.
    #[cfg(test)]
    pub fn local_path(&self, id: EditId) -> Option<PathBuf> {
        self.inner.edit(id).map(|edit| edit.local.clone())
    }

    /// The watcher's report that `path` (in the session's directory) changed.
    #[cfg(test)]
    pub fn file_event(&self, session: SessionId, path: PathBuf) {
        if let Some(record) = lock(&self.inner.sessions).get(&session) {
            let _ = record.files.send(path);
        }
    }

    /// Adds `dir` (an edited file's directory, which exists now) to the session's watcher.
    /// Watching each directory once it exists, rather than the session directory
    /// recursively, cannot miss a directory created just before a recursive watch would
    /// have reached it.
    fn watch_dir(&self, session: SessionId, dir: &Path) -> Result<()> {
        use notify::Watcher;
        let mut sessions = lock(&self.inner.sessions);
        match sessions
            .get_mut(&session)
            .and_then(|record| record.watcher.as_mut())
        {
            Some(watcher) => watcher
                .watch(dir, notify::RecursiveMode::NonRecursive)
                .map_err(watch_error),
            None => Ok(()),
        }
    }

    async fn launch(&self, editor: &str, path: &Path) -> Result<()> {
        let launcher = self.inner.launcher.clone();
        let (editor, path) = (editor.to_owned(), path.to_owned());
        blocking(move || launcher(&editor, &path)).await
    }

    /// The session's directory, created (with its watcher and worker) on first use.
    async fn session_dir(
        &self,
        session: SessionId,
        state: &watch::Receiver<ConnectionState>,
    ) -> Result<PathBuf> {
        let _setup = self.inner.setup.lock().await;
        if let Some(record) = lock(&self.inner.sessions).get(&session) {
            return Ok(record.dir.clone());
        }
        let root = self.inner.root.clone();
        let dir = blocking(move || prepare_session_dir(&root, session)).await?;
        let (files, rx) = mpsc::unbounded_channel();
        let watcher = if self.inner.watch_files {
            Some(start_watcher(files.clone())?)
        } else {
            None
        };
        let worker = tokio::spawn(run_worker(
            Arc::downgrade(&self.inner),
            session,
            rx,
            state.clone(),
        ));
        lock(&self.inner.sessions).insert(
            session,
            SessionEdits {
                dir: dir.clone(),
                files,
                worker,
                watcher,
            },
        );
        Ok(dir)
    }
}

impl Inner {
    fn emit(&self, event: EditEvent) {
        let _ = self.events.send(event);
    }

    fn edit(&self, id: EditId) -> Option<Arc<Edit>> {
        lock(&self.edits).get(&id).cloned()
    }

    fn session_edits(&self, session: SessionId) -> Vec<Arc<Edit>> {
        let mut edits: Vec<Arc<Edit>> = lock(&self.edits)
            .values()
            .filter(|edit| edit.session == session)
            .cloned()
            .collect();
        edits.sort_by_key(|edit| edit.id);
        edits
    }

    fn edit_at(&self, session: SessionId, path: &Path) -> Option<EditId> {
        lock(&self.edits)
            .values()
            .find(|edit| edit.session == session && edit.local == path)
            .map(|edit| edit.id)
    }

    /// Uploads the local file if it changed (see the module docs). `overwrite` skips the
    /// stamp check, now and, if this ends pending, on the retry.
    async fn sync(&self, edit: &Arc<Edit>, overwrite: bool) -> Outcome {
        let _turn = edit.turn.lock().await;
        // Ended while this sync waited (a flush holding an old handle, a debounce): it
        // writes nothing.
        if edit.ending.load(Ordering::SeqCst) {
            return Outcome::Unchanged;
        }
        let id = edit.id;
        let local = edit.local.clone();
        let data = match blocking(move || read_local(&local)).await {
            Ok(Some(data)) => data,
            // Mid-save (removed, not recreated yet): the creation brings another event.
            Ok(None) => return Outcome::Unchanged,
            Err(error) => return self.failed(edit, error),
        };
        let hash = digest(&data);
        let (stamp, skip_check) = {
            let mut st = edit.sync_state();
            if overwrite {
                st.skip_check = true;
            }
            // A server copy our cut may have truncated needs these bytes even if they are
            // the ones last synced (the user undid the change).
            if hash == st.synced && !overwrite && !st.truncated {
                st.state = EditState::Synced;
                return Outcome::Unchanged;
            }
            (st.stamp, st.skip_check)
        };
        if !edit.connected() {
            return self.pending(edit);
        }
        let before = match edit.fs.stat(&edit.remote).await {
            Ok(entry) => Some(entry),
            Err(error) if error.kind == ErrorKind::NotFound => None,
            Err(error) if error.kind == ErrorKind::Network => return self.pending(edit),
            Err(error) => return self.failed(edit, error),
        };
        if skip_check {
            // This attempt reached the server: the one pass is used.
            edit.sync_state().skip_check = false;
        }
        let unchanged = before
            .as_ref()
            .is_some_and(|entry| stamp.matches(&RemoteStamp::of(entry)));
        if !skip_check && !unchanged {
            edit.set_state(EditState::Conflict);
            tracing::warn!(edit = id.0, remote = %edit.remote, "the remote file changed during the edit; upload held");
            self.emit(EditEvent::RemoteChanged { id });
            return Outcome::Conflict;
        }
        // Last chance before writing: an edit `keep` ended meanwhile is left alone.
        if edit.ending.load(Ordering::SeqCst) {
            return Outcome::Unchanged;
        }
        let mode_bits = before.as_ref().and_then(|entry| entry.mode);
        if let Err(cut) = upload(edit.fs.as_ref(), &edit.remote, edit.mode, &data, mode_bits).await
        {
            if let Some(orphan) = cut.orphan {
                edit.sync_state().orphans.push(orphan);
            }
            if cut.clobbered {
                tracing::warn!(edit = id.0, remote = %edit.remote, error = %cut.error, "an in-place upload was cut; the remote file may be truncated");
                edit.sync_state().truncated = true;
            }
            if cut.error.kind == ErrorKind::Network {
                if cut.clobbered {
                    edit.sync_state().skip_check = true;
                }
                return self.pending(edit);
            }
            if cut.clobbered {
                // The damage is the new baseline: later saves are checked against it.
                if let Ok(entry) = edit.fs.stat(&edit.remote).await {
                    edit.sync_state().stamp = RemoteStamp::of(&entry);
                }
            }
            return self.failed(edit, cut.error);
        }
        let stamp = match edit.fs.stat(&edit.remote).await {
            Ok(entry) => RemoteStamp::of(&entry),
            // Uploaded, but the new mtime is unknown: check the size only next time.
            Err(_) => RemoteStamp {
                size: data.len() as u64,
                mtime: None,
            },
        };
        {
            let mut st = edit.sync_state();
            let orphans = std::mem::take(&mut st.orphans);
            *st = SyncState::synced(stamp, hash, orphans);
        }
        tracing::info!(edit = id.0, remote = %edit.remote, size = data.len(), "remote edit uploaded");
        self.emit(EditEvent::Uploaded {
            id,
            at: SystemTime::now(),
        });
        self.clean_orphans(edit).await;
        Outcome::Uploaded
    }

    /// Removes (best effort) the temporary upload files cuts left on the server.
    async fn clean_orphans(&self, edit: &Edit) {
        let orphans = std::mem::take(&mut edit.sync_state().orphans);
        let mut left = Vec::new();
        for orphan in orphans {
            match edit.fs.remove_file(&orphan).await {
                Ok(()) => {
                    tracing::info!(edit = edit.id.0, temp = %orphan, "removed a temporary upload file left by a cut");
                }
                Err(error) if error.kind == ErrorKind::NotFound => {}
                Err(_) => left.push(orphan),
            }
        }
        edit.sync_state().orphans.extend(left);
    }

    fn pending(&self, edit: &Edit) -> Outcome {
        let remote_may_be_truncated = {
            let mut st = edit.sync_state();
            st.state = EditState::Pending;
            st.truncated
        };
        tracing::info!(edit = edit.id.0, remote = %edit.remote, "session not connected; edit held until it reconnects");
        self.emit(EditEvent::Pending {
            id: edit.id,
            remote_may_be_truncated,
        });
        Outcome::Pending
    }

    fn failed(&self, edit: &Edit, error: Error) -> Outcome {
        let remote_may_be_truncated = {
            let mut st = edit.sync_state();
            st.state = EditState::Failed;
            st.truncated
        };
        tracing::warn!(edit = edit.id.0, remote = %edit.remote, %error, "remote edit upload failed");
        self.emit(EditEvent::Failed {
            id: edit.id,
            error: error.clone(),
            remote_may_be_truncated,
            recovered: None,
        });
        Outcome::Failed(error)
    }

    /// Saves `data` (the user's version) as a new recovered copy (see the module docs) and
    /// returns its path.
    async fn recover(&self, edit: &Edit, data: Vec<u8>) -> Result<PathBuf> {
        let root = self.recovered.clone();
        let site = if plain_component(&edit.site) {
            edit.site.clone()
        } else {
            "site".to_owned()
        };
        let remote = edit.remote.clone();
        let path = blocking(move || {
            create_private_dir_all(&root)?;
            set_private(&root, 0o700)?;
            let base = local_path_for(&root.join(site), remote.as_str())?;
            write_new_private(&base, &format!(".local-{}", timestamp()), &data)
        })
        .await?;
        tracing::info!(edit = edit.id.0, remote = %edit.remote, recovered = %path.display(), "saved the local version as a recovered copy");
        Ok(path)
    }

    /// [`recover`](Self::recover), unless these exact bytes were saved already: then
    /// that copy.
    async fn preserve(&self, edit: &Edit, data: Vec<u8>) -> Result<PathBuf> {
        // One at a time: a forced exit racing a shutdown's pass never copies the same
        // bytes twice.
        let _one = self.preserving.lock().await;
        let hash = digest(&data);
        let done = edit
            .sync_state()
            .preserved
            .clone()
            .filter(|(h, path)| *h == hash && path.exists());
        if let Some((_, path)) = done {
            return Ok(path);
        }
        let path = self.recover(edit, data).await?;
        edit.sync_state().preserved = Some((hash, path.clone()));
        Ok(path)
    }

    /// Downloads the server's copy over the local file and takes its stamp. The caller
    /// holds the edit's turn.
    async fn adopt_remote(&self, edit: &Edit) -> Result<()> {
        let entry = edit.fs.stat(&edit.remote).await?;
        let data = fetch(edit.fs.as_ref(), &edit.remote).await?;
        let synced = digest(&data);
        let size = data.len();
        let local = edit.local.clone();
        blocking(move || write_private(&local, &data)).await?;
        {
            let mut st = edit.sync_state();
            let orphans = std::mem::take(&mut st.orphans);
            *st = SyncState::synced(RemoteStamp::of(&entry), synced, orphans);
        }
        tracing::info!(edit = edit.id.0, remote = %edit.remote, size, "remote version downloaded over the edit");
        Ok(())
    }
}

/// The edit's summary if something of it is not on the server.
async fn unsent(edit: &Arc<Edit>) -> Option<EditSummary> {
    let (state, synced) = {
        let st = edit.sync_state();
        (st.state, st.synced)
    };
    if state != EditState::Synced {
        return Some(edit.summary(state));
    }
    let local = edit.local.clone();
    let changed = blocking(move || Ok(read_local(&local)?.map(|data| digest(&data))))
        .await
        .ok()
        .flatten()
        .is_some_and(|hash| hash != synced);
    changed.then(|| edit.summary(EditState::Modified))
}

/// A session's worker: debounces file events per edit, syncs, and syncs the pending
/// edits when the session is connected again.
async fn run_worker(
    inner: Weak<Inner>,
    session: SessionId,
    mut files: mpsc::UnboundedReceiver<PathBuf>,
    mut state: watch::Receiver<ConnectionState>,
) {
    let mut due: HashMap<EditId, Instant> = HashMap::new();
    let mut backoff: HashMap<EditId, Duration> = HashMap::new();
    state.borrow_and_update();
    let mut state_open = true;
    loop {
        let next = due.values().min().copied();
        let wake = async move {
            match next {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            path = files.recv() => {
                let Some(path) = path else { break };
                let Some(inner) = inner.upgrade() else { break };
                if let Some(id) = inner.edit_at(session, &path) {
                    due.insert(id, Instant::now() + DEBOUNCE);
                }
            }
            changed = state.changed(), if state_open => {
                if changed.is_err() {
                    // The session is gone: no more retries; its held edits wait for the
                    // close flow (`flush`, `keep` or `finish`). Later saves are held
                    // without touching the network (`Edit::connected` is false now).
                    state_open = false;
                    due.clear();
                    backoff.clear();
                    continue;
                }
                // Whatever came before: a drop and a reconnect may arrive as one change.
                if !matches!(*state.borrow_and_update(), ConnectionState::Connected) {
                    continue;
                }
                let Some(inner) = inner.upgrade() else { break };
                for edit in inner.session_edits(session) {
                    inner.clean_orphans(&edit).await;
                    if edit.sync_state().state == EditState::Pending {
                        tracing::info!(edit = edit.id.0, remote = %edit.remote, "session connected; uploading the held edit");
                        let outcome = inner.sync(&edit, false).await;
                        schedule_retry(edit.id, edit.connected(), &outcome, &mut due, &mut backoff);
                    }
                }
            }
            () = wake => {
                let now = Instant::now();
                let mut ready: Vec<EditId> =
                    due.iter().filter(|(_, at)| **at <= now).map(|(id, _)| *id).collect();
                ready.sort();
                let Some(inner) = inner.upgrade() else { break };
                for id in ready {
                    due.remove(&id);
                    match inner.edit(id) {
                        Some(edit) => {
                            let outcome = inner.sync(&edit, false).await;
                            schedule_retry(edit.id, edit.connected(), &outcome, &mut due, &mut backoff);
                        }
                        None => {
                            backoff.remove(&id);
                        }
                    }
                }
            }
        }
    }
}

/// A save held while the session still says `Connected` (the link died first) is retried
/// after a growing delay; one held while it is down waits for `Connected`.
fn schedule_retry(
    id: EditId,
    connected: bool,
    outcome: &Outcome,
    due: &mut HashMap<EditId, Instant>,
    backoff: &mut HashMap<EditId, Duration>,
) {
    if matches!(outcome, Outcome::Pending) && connected {
        let wait = backoff
            .get(&id)
            .map_or(RETRY_MIN, |wait| (*wait * 2).min(RETRY_MAX));
        backoff.insert(id, wait);
        due.insert(id, Instant::now() + wait);
    } else {
        // Sent, settled, or held for `Connected`: no backoff to remember.
        backoff.remove(&id);
    }
}

/// A session's watcher, watching nothing yet (see [`EditManager::watch_dir`]): sends the
/// paths of saves.
fn start_watcher(files: mpsc::UnboundedSender<PathBuf>) -> Result<notify::RecommendedWatcher> {
    let watcher =
        notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
            Ok(event) if is_save(&event.kind) => {
                for path in event.paths {
                    let _ = files.send(path);
                }
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "file watcher error"),
        })
        .map_err(watch_error)?;
    Ok(watcher)
}

fn watch_error(error: notify::Error) -> Error {
    Error::coded(
        ErrorKind::Io,
        "edit-cannot-watch",
        "Cannot watch the edited files",
    )
    .with_detail(error.to_string())
}

/// Whether a watcher event may be a save: creations, removals (an editor recreating the
/// file), data and name changes, and a close after writing. Not reads, opens or metadata.
fn is_save(kind: &notify::EventKind) -> bool {
    use notify::EventKind;
    use notify::event::{AccessKind, AccessMode, ModifyKind};
    match kind {
        EventKind::Create(_) | EventKind::Remove(_) => true,
        EventKind::Modify(ModifyKind::Metadata(_)) => false,
        EventKind::Modify(_) => true,
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        _ => false,
    }
}

/// Reads a whole remote file.
async fn fetch(fs: &dyn RemoteFs, remote: &RemotePath) -> Result<Vec<u8>> {
    let mut reader = fs.open_read(remote, 0).await?;
    let mut data = Vec::new();
    reader.read_to_end(&mut data).await.map_err(Error::from)?;
    Ok(data)
}

/// Creates or replaces a remote file with `data`.
async fn write_remote(fs: &dyn RemoteFs, remote: &RemotePath, data: &[u8]) -> Result<()> {
    let mut writer = fs.open_write(remote, 0).await?;
    writer.write_all(data).await.map_err(Error::from)?;
    writer.shutdown().await.map_err(Error::from)?;
    Ok(())
}

/// Why [`put`] did not write the file.
#[derive(Debug)]
pub(crate) struct PutError {
    pub error: Error,
    /// The file itself was being written (in place) when it failed: what is on the
    /// server may now be incomplete. False when only a temporary file was cut, or nothing
    /// was written.
    pub may_be_incomplete: bool,
}

/// Writes `data` to `remote` the way a remote edit uploads (see the module docs): by
/// `mode`, keeping the mode bits of the file it replaces. An agent's `write_file`. A
/// temporary file a cut upload leaves behind is removed (best effort).
pub(crate) async fn put(
    fs: &dyn RemoteFs,
    remote: &RemotePath,
    mode: UploadMode,
    data: &[u8],
) -> std::result::Result<(), PutError> {
    let mode_bits = match fs.stat(remote).await {
        Ok(entry) => entry.mode,
        Err(error) if error.kind == ErrorKind::NotFound => None,
        Err(error) => {
            return Err(PutError {
                error,
                may_be_incomplete: false,
            });
        }
    };
    match upload(fs, remote, mode, data, mode_bits).await {
        Ok(()) => Ok(()),
        Err(cut) => {
            if let Some(orphan) = cut.orphan {
                let _ = fs.remove_file(&orphan).await;
            }
            Err(PutError {
                error: cut.error,
                may_be_incomplete: cut.clobbered,
            })
        }
    }
}

/// Why an upload did not finish.
struct Cut {
    error: Error,
    /// An in-place write started: the remote copy itself may be damaged.
    clobbered: bool,
    /// A temporary file that may be left on the server.
    orphan: Option<RemotePath>,
}

/// Uploads `data` by `mode` (see the module docs).
#[allow(clippy::result_large_err)] // `Cut` holds an `Error`, rare and short-lived
async fn upload(
    fs: &dyn RemoteFs,
    remote: &RemotePath,
    mode: UploadMode,
    data: &[u8],
    mode_bits: Option<u32>,
) -> std::result::Result<(), Cut> {
    if mode == UploadMode::TempRename
        && let (Some(name), Some(parent)) = (remote.file_name(), remote.parent())
    {
        let temp = parent.join(&format!(".{name}.{}.hoprs-upload", nonce()));
        let network = |error: Error, temp: &RemotePath| Cut {
            error,
            clobbered: false,
            orphan: Some(temp.clone()),
        };
        match write_remote(fs, &temp, data).await {
            Ok(()) => {
                if let Some(bits) = mode_bits {
                    // Best effort: keep the original's permissions on the replacement.
                    let _ = fs.chmod(&temp, bits & 0o7777).await;
                }
                match fs.rename(&temp, remote).await {
                    Ok(()) => return Ok(()),
                    Err(error) if error.kind == ErrorKind::Network => {
                        return Err(network(error, &temp));
                    }
                    Err(error) => {
                        tracing::debug!(remote = %remote, %error, "rename over the file refused; writing in place");
                        let _ = fs.remove_file(&temp).await;
                    }
                }
            }
            Err(error) if error.kind == ErrorKind::Network => return Err(network(error, &temp)),
            Err(error) => {
                tracing::debug!(remote = %remote, %error, "cannot write a temporary file; writing in place");
                let _ = fs.remove_file(&temp).await;
            }
        }
    }
    write_remote(fs, remote, data).await.map_err(|error| Cut {
        error,
        clobbered: true,
        orphan: None,
    })
}

/// The local file's bytes; `None` if it does not exist (right now).
fn read_local(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(data) => Ok(Some(data)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::from(e).with_detail(path.display().to_string())),
    }
}

/// `path` with `suffix` added to its file name.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().map(OsString::from).unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

/// Writes `data` to `path` (private, see [`create_private_dir_all`]), through a temporary
/// file renamed over it: the editor never sees a half-written file.
fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::coded(ErrorKind::Io, "edit-no-parent", "No parent directory"))?;
    create_private_dir_all(parent)?;
    let mut name = OsString::from(".");
    name.push(path.file_name().unwrap_or_default());
    name.push(format!(".{}.hoprs-dl", nonce()));
    let temp = parent.join(name);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options
        .open(&temp)
        .and_then(|mut file| file.write_all(data));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(Error::from(e).with_detail(temp.display().to_string()));
    }
    set_private(&temp, 0o600)?;
    std::fs::rename(&temp, path).map_err(|e| Error::from(e).with_detail(path.display().to_string()))
}

/// Writes `data` to a new file named `base` + `suffix`, or `base` + `suffix-N` if taken
/// (never replacing a file), `0600`, its directories `0700`. Returns its path. A file
/// whose write fails is removed (never left half written).
fn write_new_private(base: &Path, suffix: &str, data: &[u8]) -> Result<PathBuf> {
    write_new_private_with(base, suffix, data, |file, data| file.write_all(data))
}

/// [`write_new_private`] with the write itself given (tests make it fail).
fn write_new_private_with(
    base: &Path,
    suffix: &str,
    data: &[u8],
    write: impl Fn(&mut std::fs::File, &[u8]) -> std::io::Result<()>,
) -> Result<PathBuf> {
    let parent = base
        .parent()
        .ok_or_else(|| Error::coded(ErrorKind::Io, "edit-no-parent", "No parent directory"))?;
    create_private_dir_all(parent)?;
    for n in 0u32.. {
        let path = if n == 0 {
            sibling(base, suffix)
        } else {
            sibling(base, &format!("{suffix}-{n}"))
        };
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(mut file) => {
                if let Err(e) = write(&mut file, data).and_then(|()| file.sync_all()) {
                    drop(file);
                    let _ = std::fs::remove_file(&path);
                    return Err(Error::from(e).with_detail(path.display().to_string()));
                }
                set_private(&path, 0o600)?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Error::from(e).with_detail(path.display().to_string())),
        }
    }
    unreachable!("some numbered name is free")
}

/// Creates `dir` and its missing parents, `0700` on Unix.
fn create_private_dir_all(dir: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(dir)
        .map_err(|e| Error::from(e).with_detail(dir.display().to_string()))
}

/// Sets `mode` on Unix; nothing elsewhere (the user's profile directories are private
/// there already).
#[cfg(unix)]
fn set_private(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|e| Error::from(e).with_detail(path.display().to_string()))
}

#[cfg(not(unix))]
fn set_private(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

/// Creates `<root>/<session>` (and the root), `0700`, moving aside a directory left by an
/// earlier run. Returns its canonical path (what the watcher reports).
fn prepare_session_dir(root: &Path, session: SessionId) -> Result<PathBuf> {
    create_private_dir_all(root)?;
    set_private(root, 0o700)?;
    let dir = root.join(session.0.to_string());
    if std::fs::symlink_metadata(&dir).is_ok() {
        let stale = root.join(format!("{}.stale-{}", session.0, timestamp()));
        tracing::warn!(dir = %dir.display(), moved_to = %stale.display(), "an edit directory from an earlier run was moved aside");
        std::fs::rename(&dir, &stale)
            .map_err(|e| Error::from(e).with_detail(dir.display().to_string()))?;
    }
    create_private_dir_all(&dir)?;
    set_private(&dir, 0o700)?;
    std::fs::canonicalize(&dir).map_err(|e| Error::from(e).with_detail(dir.display().to_string()))
}

fn refused(remote: &str) -> Error {
    Error::coded(
        ErrorKind::Permission,
        "edit-path-refused",
        "This remote path cannot be edited locally",
    )
    .with_detail(format!("{remote:?}"))
}

/// Whether `part` is one plain file name (see [`local_path_for`]).
fn plain_component(part: &str) -> bool {
    let plain = !part.is_empty()
        && part != "."
        && part != ".."
        && !part.contains(['\\', '\0', '/'])
        && !(cfg!(windows) && part.contains(':'));
    let mut components = Path::new(part).components();
    plain
        && matches!(
            (components.next(), components.next()),
            (Some(Component::Normal(_)), None)
        )
}

/// The local path of `remote` under `session_dir`: each component of the absolute remote
/// path must be a plain name (not empty, `.` or `..`, without `\`, NUL or, on Windows,
/// `:`), and the result must stay inside `session_dir`.
pub fn local_path_for(session_dir: &Path, remote: &str) -> Result<PathBuf> {
    let relative = remote.strip_prefix('/').ok_or_else(|| refused(remote))?;
    if relative.is_empty() {
        return Err(refused(remote));
    }
    let mut path = session_dir.to_path_buf();
    for part in relative.split('/') {
        if !plain_component(part) {
            return Err(refused(remote));
        }
        path.push(part);
    }
    if !path.starts_with(session_dir) || path == session_dir {
        return Err(refused(remote));
    }
    Ok(path)
}

/// How [`launch_editor`] opens a file.
#[derive(Debug, PartialEq, Eq)]
pub enum EditorLaunch {
    /// The OS's default editor (the setting `system`): `xdg-open` on Linux, `open -t` on
    /// macOS, `ShellExecuteW` with the `open` verb on Windows.
    System,
    /// A command line: the program, its arguments, the file's path last.
    Argv(Vec<OsString>),
}

/// What the editor setting `command` runs for `path`: `system` (trimmed, any case) is
/// [`EditorLaunch::System`]; anything else is split ([`split_command`]) into program and
/// arguments, no shell, with `path` last.
pub fn editor_launch(command: &str, path: &Path) -> Result<EditorLaunch> {
    if crate::settings::is_system_editor(command) {
        return Ok(EditorLaunch::System);
    }
    let words = split_command(command).ok_or_else(|| {
        Error::coded(
            ErrorKind::Io,
            "edit-unmatched-quote",
            "The editor command has an unmatched quote",
        )
        .with_detail(command.to_owned())
    })?;
    if words.is_empty() {
        return Err(Error::coded(
            ErrorKind::Io,
            "edit-no-editor",
            "No editor command is set",
        ));
    }
    let mut argv: Vec<OsString> = words.into_iter().map(OsString::from).collect();
    argv.push(path.as_os_str().to_owned());
    Ok(EditorLaunch::Argv(argv))
}

/// `command` split into words, `None` on an unmatched quote: POSIX shell quoting
/// ([`shlex`]) on Unix, Windows command-line rules ([`split_windows`]) on Windows. No
/// shell runs either way.
fn split_command(command: &str) -> Option<Vec<String>> {
    #[cfg(windows)]
    {
        split_windows(command)
    }
    #[cfg(not(windows))]
    {
        shlex::split(command)
    }
}

/// `command` split with the rules of `CommandLineToArgvW`, except that an unmatched
/// double quote is refused (`None`) instead of closed at the end. The first word (the
/// program) ends at the next whitespace, or at the closing quote when it starts with
/// one, and its backslashes are literal. In the other words, whitespace outside quotes
/// separates; `2n` backslashes before a `"` are `n` backslashes and the quote toggles
/// quoting, `2n + 1` are `n` backslashes and a literal `"`; backslashes before anything
/// else are literal; `""` inside quotes is a literal `"`. Single quotes are plain
/// characters.
#[cfg(any(windows, test))]
fn split_windows(command: &str) -> Option<Vec<String>> {
    let is_space = |c: char| c == ' ' || c == '\t';
    let mut chars = command.trim_start_matches(is_space).chars().peekable();
    let mut words = Vec::new();
    if chars.peek().is_none() {
        return Some(words);
    }
    let mut program = String::new();
    if chars.next_if_eq(&'"').is_some() {
        loop {
            match chars.next()? {
                '"' => break,
                c => program.push(c),
            }
        }
    } else {
        while let Some(c) = chars.next_if(|&c| !is_space(c)) {
            program.push(c);
        }
    }
    words.push(program);

    let mut word = String::new();
    let mut in_word = false;
    let mut quoted = false;
    let mut backslashes = 0usize;
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                backslashes += 1;
                in_word = true;
            }
            '"' => {
                word.extend(std::iter::repeat_n('\\', backslashes / 2));
                // An escaped quote, or `""` inside quotes: a literal quote.
                if backslashes % 2 == 1 || (quoted && chars.next_if_eq(&'"').is_some()) {
                    word.push('"');
                } else {
                    quoted = !quoted;
                }
                backslashes = 0;
                in_word = true;
            }
            c if is_space(c) && !quoted => {
                word.extend(std::iter::repeat_n('\\', backslashes));
                backslashes = 0;
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            c => {
                word.extend(std::iter::repeat_n('\\', backslashes));
                backslashes = 0;
                word.push(c);
                in_word = true;
            }
        }
    }
    if quoted {
        return None;
    }
    word.extend(std::iter::repeat_n('\\', backslashes));
    if in_word {
        words.push(word);
    }
    Some(words)
}

/// The command line of the OS's default editor for `path` on Linux (`xdg-open`) and
/// macOS (`open -t`: the default *text* editor, so `.conf` or extensionless files open
/// in something useful). Windows uses `ShellExecuteW` instead.
#[cfg(not(windows))]
fn system_argv(path: &Path) -> Vec<OsString> {
    let mut argv: Vec<OsString> = if cfg!(target_os = "macos") {
        vec!["open".into(), "-t".into()]
    } else {
        vec!["xdg-open".into()]
    };
    argv.push(path.as_os_str().to_owned());
    argv
}

/// Launches the editor of the setting `command` (see [`editor_launch`]) on `path`,
/// detached, without waiting for it.
pub fn launch_editor(command: &str, path: &Path) -> Result<()> {
    match editor_launch(command, path)? {
        EditorLaunch::Argv(argv) => spawn_editor(&argv, path),
        EditorLaunch::System => open_with_system(path),
    }
}

/// The OS's default editor on Linux and macOS: [`system_argv`], spawned like a command.
#[cfg(not(windows))]
fn open_with_system(path: &Path) -> Result<()> {
    spawn_editor(&system_argv(path), path)
}

/// The OS's default editor on Windows: `ShellExecuteW` with the `open` verb (the file
/// type's association, no console window); a type without one (`SE_ERR_NOASSOC`) opens
/// in `notepad.exe`.
#[cfg(windows)]
fn open_with_system(path: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{S_FALSE, S_OK};
    use windows_sys::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
    };
    use windows_sys::Win32::UI::Shell::{SE_ERR_NOASSOC, ShellExecuteW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide = |s: &std::ffi::OsStr| -> Vec<u16> { s.encode_wide().chain([0]).collect() };
    let verb = wide(std::ffi::OsStr::new("open"));
    let file = wide(path.as_os_str());
    // Shell extensions behind an association may need COM, as `ShellExecuteW`'s
    // documentation says; this thread's own initialisation is undone afterwards.
    // SAFETY: the reserved pointer must be null; the flags are valid.
    let com = unsafe {
        CoInitializeEx(
            std::ptr::null(),
            (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
        )
    };
    // SAFETY: `verb` and `file` are NUL-terminated UTF-16 buffers that outlive the call;
    // the window, parameters and directory may be null.
    let code = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if com == S_OK || com == S_FALSE {
        // SAFETY: balances this thread's successful `CoInitializeEx` above.
        unsafe { CoUninitialize() };
    }
    if code > 32 {
        tracing::info!(editor = "system", path = %path.display(), "editor started");
        return Ok(());
    }
    if code == SE_ERR_NOASSOC as isize {
        tracing::debug!(path = %path.display(), "no file association: opening in notepad");
        return spawn_editor(&["notepad.exe".into(), path.as_os_str().to_owned()], path);
    }
    Err(Error::coded(
        ErrorKind::Io,
        "edit-cannot-start-editor",
        "Cannot start the editor",
    )
    .with_detail(format!("ShellExecuteW failed with code {code}")))
}

/// Spawns `argv` (program first, never empty) detached, without waiting for it: null
/// stdio; on Unix its own process group; on Windows its own process group and no
/// inherited console (`DETACHED_PROCESS`; a GUI editor still opens its window); reaped
/// by a thread when it exits.
fn spawn_editor(argv: &[OsString], path: &Path) -> Result<()> {
    let (program, args) = argv.split_first().expect("an editor argv is never empty");
    let mut child = std::process::Command::new(program);
    child
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        child.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // winbase.h; `CREATE_NO_WINDOW` stays unset (a GUI editor shows its window).
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        child.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    let mut child = child.spawn().map_err(|e| {
        Error::coded(
            ErrorKind::Io,
            "edit-cannot-start-editor",
            "Cannot start the editor",
        )
        .with_detail(format!("{}: {e}", program.to_string_lossy()))
    })?;
    tracing::info!(editor = %program.to_string_lossy(), path = %path.display(), "editor started");
    let reaper = std::thread::Builder::new()
        .name("hoprs-editor".to_owned())
        .spawn(move || {
            let _ = child.wait();
        });
    if let Err(error) = reaper {
        tracing::debug!(%error, "cannot start the editor's reaper thread");
    }
    Ok(())
}

/// `$XDG_RUNTIME_DIR/hoprs/edit`, else the cache directory's `hoprs/edit`, else the
/// temporary directory's.
pub fn default_edit_dir() -> PathBuf {
    dirs::runtime_dir()
        .or_else(dirs::cache_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("hoprs")
        .join("edit")
}

/// `$XDG_DATA_HOME/hoprs/recovered` (durable: recovered copies outlive the session and
/// the login), else the cache directory's, else the temporary directory's.
pub fn default_recovered_dir() -> PathBuf {
    dirs::data_dir()
        .or_else(dirs::cache_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("hoprs")
        .join("recovered")
}

#[cfg(test)]
mod tests;
