//! The engine actor: owns the queue store, the in-memory items, the per-site
//! worker slots and the progress tracker. Never awaits I/O itself: workers
//! run as spawned tasks and report back through `WorkerMsg`.
//!
//! This module holds the actor's state, its loop, dispatch and the handling
//! of worker results. Control commands live in `control`, directory
//! bookkeeping in `dirs` and start-up recovery in `recovery`.
//!
//! Files are dispatched before directories, so a large tree starts copying
//! before it has been listed completely.
//!
//! A site has two pause flags and dispatches only while both are off:
//! `user_paused` (`pause_site`) and `network_paused`. A network error sets
//! `network_paused` on the item's whole site, emitting `SitePaused` when the
//! flag turns on (even while the site is also paused by the user): the item
//! goes back to `Queued` (a file continues from its partial destination
//! later). `network_restored` clears `network_paused`; `resume_site` clears
//! both. Other sites keep running.
//!
//! At start, items left unfinished by a previous run are loaded as `Paused`
//! (see `recovery::recover`); nothing resumes automatically.
//!
//! The conflict policy an item's next run needs (its *continuation*) is
//! persisted as the item's `next_policy` while it is unfinished: the
//! override a run consumed at dispatch, replaced by the policy that
//! continues its copy once the copy is decided (`Resume`, or `Overwrite`
//! when it replaces an existing file). Pausing, requeueing or recovering an
//! item carries it forward; an item with none runs under its tree policy
//! (the policy its tree was enqueued with, inherited by every item of a
//! folder), else the global policy (its destination was never opened for
//! writing). A cancelled item keeps its continuation too, and a failed one
//! that had one gets `Overwrite` instead (a failed write can leave stale
//! bytes or holes below the destination's size), so `retry` never skips or
//! appends to a partial destination.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::time::{Duration, Instant};

use hoprs_core::{Error, ErrorKind, RemoteEntry, RemotePath, Result, SiteId};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use super::control::Control;
use super::dirs::DirProgress;
use super::worker::{self, Prepared, Route, WorkerMsg};
use super::{EngineConfig, EnqueueOptions, Event, EventKind, LocalGuard, SiteEndpoints};
use crate::conflict::{Decision, decide};
use crate::copy::CopyOutcome;
use crate::model::{
    ConflictInfo, ConflictPolicy, Direction, Origin, TransferId, TransferItem, TransferSpec,
    TransferState,
};
use crate::progress::{ProgressTracker, ProgressUpdate};
use crate::store::QueueStore;

const MAX_WORKERS: usize = 16;
pub(super) const SYMLINK_SKIP: &str = "symlinks are not followed";
/// How long a stopping engine waits for its cancelled workers to report back.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

fn unknown() -> Error {
    Error::coded(ErrorKind::NotFound, "transfer-unknown", "Unknown transfer")
}

/// Appends a queue-store error to `e`'s technical detail.
fn with_store_detail(e: Error, store_err: &Error) -> Error {
    let store = match &store_err.detail {
        Some(d) => format!("queue store: {}: {d}", store_err.message),
        None => format!("queue store: {}", store_err.message),
    };
    let detail = match &e.detail {
        Some(d) => format!("{d}; {store}"),
        None => store,
    };
    e.with_detail(detail)
}

pub(super) enum Command {
    RegisterSite {
        site: SiteId,
        endpoints: SiteEndpoints,
        reply: oneshot::Sender<()>,
    },
    Enqueue {
        spec: TransferSpec,
        /// `policy`: for this item's own run (stored as its `next_policy`)
        /// and its tree's (stored as `tree_policy`).
        options: EnqueueOptions,
        reply: oneshot::Sender<Result<TransferId>>,
    },
    SetPolicy {
        policy: ConflictPolicy,
        reply: oneshot::Sender<()>,
    },
    Snapshot {
        reply: oneshot::Sender<(u64, Vec<TransferItem>)>,
    },
    ClearFinished {
        reply: oneshot::Sender<Result<usize>>,
    },
    Control {
        id: TransferId,
        op: Control,
        reply: oneshot::Sender<Result<()>>,
    },
    PauseSite {
        site: SiteId,
        reply: oneshot::Sender<()>,
    },
    ResumeSite {
        site: SiteId,
        reply: oneshot::Sender<()>,
    },
    NetworkRestored {
        site: SiteId,
        reply: oneshot::Sender<()>,
    },
    /// Stops the actor; `reply` is answered once it has stopped.
    Shutdown { reply: oneshot::Sender<()> },
}

/// What to do with an item holding a worker slot once its worker reports
/// back, set by a control command that arrived meanwhile.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum Intent {
    Pause,
    Cancel,
    /// Paused, then resumed before the worker stopped: back to `Queued`.
    Requeue,
}

/// An item holding a worker slot (preparing or copying).
pub(super) struct Running {
    pub(super) cancel: CancellationToken,
    pub(super) intent: Option<Intent>,
    /// Policy for the item's next run if this one is interrupted: the
    /// override this run consumed (unless it is only the item's tree
    /// policy), until the copy is decided; then the policy that continues
    /// that copy. Mirrors the stored `next_policy`.
    pub(super) retry: Option<ConflictPolicy>,
    /// The copy phase has started (`prepare` is over).
    pub(super) copying: bool,
}

pub(super) struct Site {
    endpoints: SiteEndpoints,
    /// Worker slots currently held by items of this site.
    busy: usize,
    /// Set by `pause_site`, cleared by `resume_site`.
    user_paused: bool,
    /// Set by a network error, cleared by `network_restored` or `resume_site`.
    network_paused: bool,
}

impl Site {
    fn free(&self) -> usize {
        self.endpoints.workers.saturating_sub(self.busy)
    }

    /// Whether new items may start.
    fn dispatching(&self) -> bool {
        !self.user_paused && !self.network_paused
    }
}

/// `Queued` items of one site, in dispatch order: items not known to be
/// directories first, then directories, each by ascending id.
#[derive(Default)]
pub(super) struct SiteQueue {
    files: BTreeSet<TransferId>,
    dirs: BTreeSet<TransferId>,
}

impl SiteQueue {
    fn push(&mut self, id: TransferId, is_dir: bool) {
        if is_dir {
            self.dirs.insert(id);
        } else {
            self.files.insert(id);
        }
    }

    fn remove(&mut self, id: TransferId) {
        self.files.remove(&id);
        self.dirs.remove(&id);
    }

    /// Takes the first id, in dispatch order, that `busy` does not hold back; the
    /// ones it skips stay queued, in place.
    fn pop_eligible(&mut self, busy: impl Fn(TransferId) -> bool) -> Option<TransferId> {
        let id = self
            .files
            .iter()
            .chain(self.dirs.iter())
            .copied()
            .find(|id| !busy(*id))?;
        self.remove(id);
        Some(id)
    }
}

/// What a transfer writes: a path on one site's server (upload), or a local path,
/// shared by every site (download).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum DestKey {
    Remote(SiteId, RemotePath),
    Local(RemotePath),
}

/// The destination `spec` writes, as a key: two items with the same key never run
/// together.
pub(super) fn dest_key(spec: &TransferSpec) -> DestKey {
    match spec.direction {
        Direction::Upload => DestKey::Remote(spec.site.clone(), spec.remote.clone()),
        Direction::Download => DestKey::Local(spec.local.clone()),
    }
}

pub(super) struct Actor {
    pub(super) store: QueueStore,
    pub(super) buf_size: usize,
    pub(super) policy: ConflictPolicy,
    pub(super) events: broadcast::Sender<Event>,
    /// `seq` of the last event emitted (0 before the first).
    pub(super) seq: u64,
    pub(super) items: BTreeMap<TransferId, TransferItem>,
    pub(super) sites: HashMap<SiteId, Site>,
    /// Index of `Queued` items per site (registered or not).
    pub(super) queued: HashMap<SiteId, SiteQueue>,
    /// `Active` expanded directories that still have non-terminal children.
    pub(super) dirs: HashMap<TransferId, DirProgress>,
    /// Items holding a worker slot.
    pub(super) running: HashMap<TransferId, Running>,
    /// One-shot conflict policy for a `Queued` item's next run, consumed at
    /// dispatch. Equal to its stored `next_policy`, except that a `resolve`
    /// answer is only stored once the item is dispatched.
    pub(super) overrides: HashMap<TransferId, ConflictPolicy>,
    pub(super) tracker: ProgressTracker,
    /// The shortest time between two `Progress` events (spec §9: 10 Hz at most).
    pub(super) progress_interval: Duration,
    /// When the last `Progress` went out.
    pub(super) last_progress: Option<Instant>,
    /// Final counts of copy runs that ended since the last `Progress`; they go out
    /// with the next one.
    pub(super) final_progress: Vec<ProgressUpdate>,
    /// A `Progress` went out off the ticker: the next tick is one interval after it.
    pub(super) progress_off_tick: bool,
    /// Queued files under a folder this run created, never dispatched yet: their
    /// destination is missing, so their first `prepare` skips the conflict check.
    /// Leaving `Queued` (dispatch, pause, cancel) drops the mark: any later attempt
    /// (resume, retry, after a restart) checks.
    ///
    /// Only while one item owns that new subtree: a folder is not marked when another
    /// unfinished item's destination is the same path, above it or below it, and an item
    /// of another tree that is enqueued or put back in the queue (retry, resume, answer)
    /// there later drops the folder's marks (`drop_overlapping_fresh`). Accepted risk:
    /// another client creating a file in the new folder before the child is dispatched is
    /// not seen, and that file is overwritten.
    pub(super) fresh_dst: HashSet<TransferId>,
    /// The folders whose files are in `fresh_dst`, by folder: what `drop_overlapping_fresh`
    /// looks through (a few folders, not every marked file).
    pub(super) fresh_dirs: HashMap<TransferId, FreshDir>,
    /// The top-level item of every item added in this run (`root_of`).
    pub(super) roots: HashMap<TransferId, TransferId>,
    /// The item holding a worker slot that writes each destination. An item whose
    /// destination is taken waits in the queue: two items never write one file at
    /// once, and the second one's conflict check then sees the first one's file.
    pub(super) writing: HashMap<DestKey, TransferId>,
    /// The last item that started writing each destination. An item's stored
    /// continuation is dropped when someone else wrote its destination since.
    pub(super) last_writer: HashMap<DestKey, TransferId>,
    /// Checked before every run of an item of `Origin::Agent`.
    pub(super) agent_guard: Option<LocalGuard>,
    pub(super) worker_tx: mpsc::UnboundedSender<WorkerMsg>,
}

pub(super) async fn run(
    store: QueueStore,
    recovered: Vec<TransferItem>,
    config: EngineConfig,
    mut commands: mpsc::Receiver<Command>,
    events: broadcast::Sender<Event>,
) {
    let (worker_tx, mut worker_rx) = mpsc::unbounded_channel();
    let mut actor = Actor {
        store,
        buf_size: config.buf_size,
        policy: config.default_policy,
        events,
        seq: 0,
        items: recovered.into_iter().map(|i| (i.id, i)).collect(),
        sites: HashMap::new(),
        queued: HashMap::new(),
        dirs: HashMap::new(),
        running: HashMap::new(),
        overrides: HashMap::new(),
        tracker: ProgressTracker::new(),
        progress_interval: config.progress_interval.max(Duration::from_millis(1)),
        last_progress: None,
        final_progress: Vec::new(),
        progress_off_tick: false,
        fresh_dst: HashSet::new(),
        fresh_dirs: HashMap::new(),
        roots: HashMap::new(),
        writing: HashMap::new(),
        last_writer: HashMap::new(),
        agent_guard: config.agent_guard,
        worker_tx,
    };
    let mut ticker = tokio::time::interval(actor.progress_interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let shutdown = loop {
        tokio::select! {
            cmd = commands.recv() => match cmd {
                Some(Command::Shutdown { reply }) => break Some(reply),
                Some(cmd) => actor.handle_command(cmd),
                // Every handle was dropped.
                None => break None,
            },
            Some(msg) = worker_rx.recv() => actor.handle_worker(msg),
            _ = ticker.tick() => actor.tick(),
        }
        if std::mem::take(&mut actor.progress_off_tick) {
            ticker.reset();
        }
    };
    actor.stop_workers(&mut worker_rx).await;
    // Closes the store before acknowledging.
    drop(actor);
    if let Some(reply) = shutdown {
        let _ = reply.send(());
    }
}

impl Actor {
    /// Numbers `kind` with the next `seq` and broadcasts it.
    fn emit(&mut self, kind: EventKind) {
        self.seq += 1;
        let _ = self.events.send(Event {
            seq: self.seq,
            kind,
        });
    }

    fn handle_command(&mut self, cmd: Command) {
        match cmd {
            Command::RegisterSite {
                site,
                mut endpoints,
                reply,
            } => {
                endpoints.workers = endpoints.workers.clamp(1, MAX_WORKERS);
                match self.sites.get_mut(&site) {
                    Some(existing) => existing.endpoints = endpoints,
                    None => {
                        self.sites.insert(
                            site,
                            Site {
                                endpoints,
                                busy: 0,
                                user_paused: false,
                                network_paused: false,
                            },
                        );
                    }
                }
                let _ = reply.send(());
                self.dispatch();
            }
            Command::Enqueue {
                spec,
                options: EnqueueOptions { policy, origin },
                reply,
            } => {
                let id = match self.store.insert_with(None, &spec, policy, origin) {
                    Ok(id) => id,
                    Err(e) => {
                        let _ = reply.send(Err(e));
                        return;
                    }
                };
                self.drop_overlapping_fresh(id, destination(&spec));
                let key = dest_key(&spec);
                self.add_item(TransferItem {
                    id,
                    parent: None,
                    spec,
                    is_dir: None,
                    size: None,
                    bytes_done: 0,
                    state: TransferState::Queued,
                    origin,
                    tree_policy: policy,
                });
                if let Some(policy) = policy {
                    // Persisted like any continuation, so it survives a restart
                    // (recovery and `resume` read it back as the override).
                    if let Err(e) = self.store.set_next_policy(id, Some(policy)) {
                        self.transition(id, TransferState::Failed(e.clone()));
                        let _ = reply.send(Err(e));
                        return;
                    }
                    self.overrides.insert(id, policy);
                    // An explicit policy is not a continuation of an earlier
                    // copy by another item: make this item the destination's
                    // writer so dispatch does not drop it as superseded.
                    self.last_writer.insert(key, id);
                }
                let _ = reply.send(Ok(id));
                self.dispatch();
            }
            Command::SetPolicy { policy, reply } => {
                self.policy = policy;
                let _ = reply.send(());
            }
            Command::Snapshot { reply } => {
                let _ = reply.send((self.seq, self.items.values().cloned().collect()));
            }
            Command::ClearFinished { reply } => {
                let _ = reply.send(self.clear_finished());
            }
            Command::Control { id, op, reply } => {
                let result = match self.items.get(&id).map(|i| i.state.clone()) {
                    None => Err(unknown()),
                    Some(state) => self.control(id, &state, op),
                };
                let _ = reply.send(result);
                self.dispatch();
            }
            Command::PauseSite { site, reply } => {
                if let Some(site) = self.sites.get_mut(&site) {
                    site.user_paused = true;
                }
                let _ = reply.send(());
            }
            Command::ResumeSite { site, reply } => {
                if let Some(site) = self.sites.get_mut(&site) {
                    site.user_paused = false;
                    site.network_paused = false;
                }
                let _ = reply.send(());
                self.dispatch();
            }
            Command::NetworkRestored { site, reply } => {
                if let Some(site) = self.sites.get_mut(&site) {
                    site.network_paused = false;
                }
                let _ = reply.send(());
                self.dispatch();
            }
            // `run` takes it out of the command stream, since it ends the actor.
            Command::Shutdown { .. } => unreachable!("shutdown is handled by run"),
        }
    }

    /// Cancels every running worker and waits (at most `SHUTDOWN_GRACE`)
    /// until each has reported back, so that no cancelled copy is still
    /// writing its partial destination once the engine has stopped. Stored
    /// states are left as they are, for the next start to recover; but a
    /// copy that reports a non-network error stores `Overwrite` as its
    /// continuation (as `release` does for a failed item), so the recovered
    /// item never continues a destination that a failed write left with
    /// stale data below its size.
    async fn stop_workers(&mut self, worker_rx: &mut mpsc::UnboundedReceiver<WorkerMsg>) {
        for running in self.running.values() {
            running.cancel.cancel();
        }
        let drain = async {
            while !self.running.is_empty() {
                let Some(msg) = worker_rx.recv().await else {
                    break;
                };
                let id = match msg {
                    WorkerMsg::Copied {
                        id, result: Err(e), ..
                    } if e.kind != ErrorKind::Network => {
                        let _ = self.overwrite_on_retry(id, e);
                        id
                    }
                    WorkerMsg::Prepared { id, .. } | WorkerMsg::Copied { id, .. } => id,
                };
                self.running.remove(&id);
            }
        };
        let _ = tokio::time::timeout(SHUTDOWN_GRACE, drain).await;
    }

    /// The top-level item `id` belongs to: recorded when it was added, found through
    /// the parents for a recovered item.
    pub(super) fn root_of(&self, mut id: TransferId) -> TransferId {
        if let Some(root) = self.roots.get(&id) {
            return *root;
        }
        while let Some(parent) = self.items.get(&id).and_then(|item| item.parent) {
            id = parent;
        }
        id
    }

    /// Whether an unfinished item of another tree than `root` writes to `dst`, above it
    /// or below it.
    pub(super) fn destination_shared(&self, root: TransferId, dst: &RemotePath) -> bool {
        self.items.values().any(|item| {
            !item.state.is_terminal()
                && self.root_of(item.id) != root
                && paths_overlap(destination(&item.spec), dst)
        })
    }

    /// An item of the tree `root` is about to write to `dst`: the fresh folders of other
    /// trees at that path, above it or below it, lose their marks (the folder has another
    /// writer). Costs one look per fresh folder.
    pub(super) fn drop_overlapping_fresh(&mut self, root: TransferId, dst: &RemotePath) {
        let stale: Vec<TransferId> = self
            .fresh_dirs
            .iter()
            .filter(|(_, dir)| dir.root != root && paths_overlap(&dir.dst, dst))
            .map(|(id, _)| *id)
            .collect();
        for dir in stale {
            if let Some(dir) = self.fresh_dirs.remove(&dir) {
                for file in dir.files {
                    self.fresh_dst.remove(&file);
                }
            }
        }
    }

    /// Tracks a newly persisted item and emits `Added`.
    pub(super) fn add_item(&mut self, item: TransferItem) {
        let root = item.parent.map_or(item.id, |parent| self.root_of(parent));
        self.roots.insert(item.id, root);
        if item.state == TransferState::Queued {
            self.queued
                .entry(item.spec.site.clone())
                .or_default()
                .push(item.id, item.is_dir == Some(true));
        }
        self.items.insert(item.id, item.clone());
        self.emit(EventKind::Added(item));
    }

    /// Persists and applies `state`, then emits `StateChanged`. If persisting
    /// fails, the item becomes `Failed` with the store error instead.
    /// An item that becomes terminal is reported to its parent directory.
    /// Returns whether `state` itself was applied.
    pub(super) fn transition(&mut self, id: TransferId, state: TransferState) -> bool {
        // Put back in the queue (retry, resume, an answer, a network requeue): it may
        // write into another tree's fresh folder.
        if state == TransferState::Queued
            && let Some(item) = self.items.get(&id)
            && item.state != TransferState::Queued
        {
            let dst = destination(&item.spec).clone();
            self.drop_overlapping_fresh(self.root_of(id), &dst);
        }
        if state.is_terminal() {
            self.fresh_dirs.remove(&id);
        }
        let (state, applied) = match self.store.set_state(id, &state) {
            Ok(()) => (state, true),
            Err(_) if matches!(state, TransferState::Failed(_)) => (state, false),
            Err(e) => {
                let failed = TransferState::Failed(e);
                let _ = self.store.set_state(id, &failed);
                (failed, false)
            }
        };
        let mut finished_child_of = None;
        let mut reopened_child_of = None;
        if let Some(item) = self.items.get_mut(&id) {
            let was_queued = item.state == TransferState::Queued;
            let is_queued = state == TransferState::Queued;
            if !is_queued && state != TransferState::Active {
                self.fresh_dst.remove(&id);
            }
            if was_queued != is_queued {
                let queue = self.queued.entry(item.spec.site.clone()).or_default();
                if is_queued {
                    queue.push(id, item.is_dir == Some(true));
                } else {
                    queue.remove(id);
                }
            }
            if !item.state.is_terminal() && state.is_terminal() {
                finished_child_of = item.parent;
            }
            if item.state.is_terminal() && !state.is_terminal() {
                let was_failed = matches!(item.state, TransferState::Failed(_));
                reopened_child_of = item.parent.map(|p| (p, was_failed));
            }
            item.state = state.clone();
        }
        let failed = matches!(state, TransferState::Failed(_));
        self.emit(EventKind::StateChanged { id, state });
        if let Some(parent) = finished_child_of {
            self.child_finished(parent, failed);
        }
        if let Some((parent, was_failed)) = reopened_child_of {
            self.child_reopened(parent, was_failed);
        }
        applied
    }

    /// Starts `Queued` items while their site has free workers: per site,
    /// items not known to be directories first, then directories, each
    /// lowest id first.
    fn dispatch(&mut self) {
        let ready: Vec<SiteId> = self
            .sites
            .iter()
            .filter(|(_, s)| s.dispatching() && s.free() > 0)
            .map(|(id, _)| id.clone())
            .collect();
        for site_id in ready {
            while self.sites.get(&site_id).is_some_and(|s| s.free() > 0) {
                let (items, writing) = (&self.items, &self.writing);
                let busy = |id: TransferId| {
                    items
                        .get(&id)
                        .is_some_and(|item| writing.contains_key(&dest_key(&item.spec)))
                };
                let Some(id) = self
                    .queued
                    .get_mut(&site_id)
                    .and_then(|q| q.pop_eligible(busy))
                else {
                    break;
                };
                let (Some(item), Some(site)) = (self.items.get(&id), self.sites.get(&site_id))
                else {
                    continue;
                };
                let route = Route::new(&item.spec, &site.endpoints);
                let key = dest_key(&item.spec);
                let tree_policy = item.tree_policy;
                let guard = match (item.origin, &self.agent_guard) {
                    (Origin::Agent, Some(guard)) => Some((guard.clone(), item.spec.local.clone())),
                    _ => None,
                };
                if !self.transition(id, TransferState::Active) {
                    continue;
                }
                let mut policy_override = self.overrides.remove(&id);
                // A continuation (`Resume` from the partial file, or `Overwrite` to
                // start over) belongs to this item's own earlier copy. If another item
                // wrote the destination since, continuing would build on (or replace
                // without asking) that item's file: drop it, so this run gets a fresh
                // conflict check under its tree's policy, else the queue's.
                let superseded = self.last_writer.get(&key).is_some_and(|w| *w != id);
                let stored = if superseded && policy_override.is_some() {
                    policy_override = None;
                    self.store.set_next_policy(id, None)
                } else if policy_override.is_some() {
                    // Stored with the `Active` row, so that it survives an
                    // interruption before the copy is decided.
                    self.store.set_next_policy(id, policy_override)
                } else {
                    Ok(())
                };
                if let Err(e) = stored {
                    // No slot was taken yet: nothing to free.
                    self.transition(id, TransferState::Failed(e));
                    continue;
                }
                if let Some(site) = self.sites.get_mut(&site_id) {
                    site.busy += 1;
                }
                self.writing.insert(key, id);
                let cancel = CancellationToken::new();
                self.running.insert(
                    id,
                    Running {
                        cancel: cancel.clone(),
                        intent: None,
                        // The item's own tree policy is what it runs under
                        // anyway: not a continuation, so a failure before the
                        // copy is decided keeps it (a `Skip` never becomes
                        // `Overwrite`). `Resume` always is one.
                        retry: policy_override
                            .filter(|p| Some(*p) != tree_policy || *p == ConflictPolicy::Resume),
                        copying: false,
                    },
                );
                let policy = policy_override.or(tree_policy).unwrap_or(self.policy);
                let fresh_dst = self.fresh_dst.remove(&id);
                // A count left from the previous run must not follow this run's own.
                self.final_progress.retain(|u| u.id != id);
                worker::spawn_prepare(
                    id,
                    route,
                    policy,
                    fresh_dst,
                    guard,
                    cancel,
                    self.worker_tx.clone(),
                );
            }
        }
    }

    /// Frees the worker slot held by `id`.
    fn free_slot(&mut self, id: TransferId) {
        self.running.remove(&id);
        if let Some(item) = self.items.get(&id) {
            let key = dest_key(&item.spec);
            if self.writing.get(&key) == Some(&id) {
                self.writing.remove(&key);
            }
            if let Some(site) = self.sites.get_mut(&item.spec.site) {
                site.busy = site.busy.saturating_sub(1);
            }
        }
    }

    /// Moves an item that holds a worker slot to `state`, frees the slot and
    /// dispatches the next item. A failed item with a continuation keeps
    /// `Overwrite` as its continuation.
    fn release(&mut self, id: TransferId, state: TransferState) {
        let state = match state {
            TransferState::Failed(e) => TransferState::Failed(self.overwrite_on_retry(id, e)),
            state => state,
        };
        self.transition(id, state);
        self.free_slot(id);
        self.dispatch();
    }

    /// Before a running item fails: if its run held any continuation
    /// (`Running::retry`), stores `Overwrite` as its continuation, so that a
    /// retry rewrites the whole destination instead of trusting a partial
    /// one. That covers a copy that was decided (its destination was, or
    /// may have been, opened), and also a run that only consumed an
    /// override (a stored continuation, or a user `resolve`) and failed
    /// before the copy was decided. A run without one (also one whose
    /// override was only its tree policy) keeps its stored policy, so a
    /// retry uses its tree policy, else the global policy. (A `Cancelled` item
    /// is not handled here: it keeps its own continuation, `Resume`, or
    /// `Overwrite` when the run was replacing an existing file.) Returns
    /// `error`, with a store error in its detail.
    fn overwrite_on_retry(&mut self, id: TransferId, error: Error) -> Error {
        if !self.running.get(&id).is_some_and(|r| r.retry.is_some()) {
            return error;
        }
        match self
            .store
            .set_next_policy(id, Some(ConflictPolicy::Overwrite))
        {
            Ok(()) => error,
            Err(store_err) => with_store_detail(error, &store_err),
        }
    }

    /// Puts an item whose worker hit a network error back in the queue,
    /// under its continuation, and network-pauses its site. `SitePaused` is
    /// emitted only if the site was not network-paused already.
    fn network_requeue(&mut self, id: TransferId, error: Error) {
        let site_id = self.items.get(&id).map(|i| i.spec.site.clone());
        self.requeue(id);
        self.free_slot(id);
        if let Some(site_id) = site_id
            && let Some(site) = self.sites.get_mut(&site_id)
            && !site.network_paused
        {
            site.network_paused = true;
            self.emit(EventKind::SitePaused {
                site: site_id,
                error,
            });
        }
        self.dispatch();
    }

    /// Moves an item that holds a worker slot back to `Queued`, with its
    /// continuation (still stored, as `Active` keeps it) as the override of
    /// its next run. Does not free the slot.
    fn requeue(&mut self, id: TransferId) {
        let retry = self.running.get(&id).and_then(|r| r.retry);
        if self.transition(id, TransferState::Queued)
            && let Some(policy) = retry
        {
            self.overrides.insert(id, policy);
        }
    }

    /// Moves an item whose run was stopped by the control `intent` (or
    /// cancelled without one) to its next state, frees its slot and
    /// dispatches. A pause or requeue keeps its stored continuation.
    fn release_stopped(&mut self, id: TransferId, intent: Option<Intent>) {
        match intent {
            Some(Intent::Pause) => self.release(id, TransferState::Paused),
            Some(Intent::Requeue) => {
                self.requeue(id);
                self.free_slot(id);
                self.dispatch();
            }
            Some(Intent::Cancel) | None => self.release(id, TransferState::Cancelled),
        }
    }

    /// The pending control intent of the item holding a worker slot, if any.
    fn intent(&self, id: TransferId) -> Option<Intent> {
        self.running.get(&id).and_then(|r| r.intent)
    }

    fn handle_worker(&mut self, msg: WorkerMsg) {
        match msg {
            WorkerMsg::Prepared { id, route, result } => {
                let intent = self.intent(id);
                match (result.map(|p| *p), intent) {
                    (Ok(Prepared::Dir { entries, created }), intent) => {
                        self.free_slot(id);
                        self.expand(id, entries, created, intent);
                        self.dispatch();
                    }
                    // Stopped before copying anything: the prepare result is
                    // dropped and the next run prepares again, under the
                    // override this run consumed.
                    (_, Some(intent)) => self.release_stopped(id, Some(intent)),
                    (Ok(Prepared::File { src, dst, decision }), None) => {
                        self.on_file_prepared(id, route, src, dst, decision)
                    }
                    (Ok(Prepared::Symlink), None) => {
                        self.release(id, TransferState::Skipped(SYMLINK_SKIP.to_string()))
                    }
                    // Whatever its kind (never a network pause): the item failed.
                    (Ok(Prepared::Refused(e)), None) => self.release(id, TransferState::Failed(e)),
                    // Only a control command or shutdown cancels the token.
                    (Ok(Prepared::Stopped), None) => self.release_stopped(id, None),
                    (Err(e), None) if e.kind == ErrorKind::Network => self.network_requeue(id, e),
                    (Err(e), None) => self.release(id, TransferState::Failed(e)),
                }
            }
            WorkerMsg::Copied { id, bytes, result } => {
                let intent = self.intent(id);
                self.finish_progress(id, bytes);
                let state = match (result, self.store.set_bytes_done(id, bytes)) {
                    (Ok(CopyOutcome::Completed), Ok(())) => TransferState::Done,
                    (Ok(CopyOutcome::Cancelled), Ok(())) => {
                        self.release_stopped(id, intent);
                        return;
                    }
                    // A pending control command wins over the network error.
                    (Err(e), Ok(())) if e.kind == ErrorKind::Network && intent.is_some() => {
                        self.release_stopped(id, intent);
                        return;
                    }
                    (Err(e), Ok(())) if e.kind == ErrorKind::Network => {
                        self.network_requeue(id, e);
                        return;
                    }
                    (Ok(_), Err(store_err)) => TransferState::Failed(store_err),
                    (Err(e), Ok(())) => TransferState::Failed(e),
                    // Keep the copy's own error; the store error goes into its detail.
                    (Err(e), Err(store_err)) => {
                        TransferState::Failed(with_store_detail(e, &store_err))
                    }
                };
                self.release(id, state);
            }
        }
    }

    /// Ends the progress tracking of a copy run: records its final byte
    /// count and sends it in a `Progress` (rate 0, as the copy has stopped).
    ///
    /// `Progress` goes out at most once per interval, all workers together
    /// (spec §9): when the last one is at least an interval old, the final
    /// count goes out now, before the state change that ends the run, and the
    /// ticker restarts from it; otherwise it rides with the next tick, after
    /// the state change.
    fn finish_progress(&mut self, id: TransferId, bytes: u64) {
        self.tracker.unregister(id);
        let Some(item) = self.items.get_mut(&id) else {
            return;
        };
        item.bytes_done = bytes;
        let update = ProgressUpdate {
            id,
            bytes_done: bytes,
            total: item.size,
            bytes_per_sec: 0,
        };
        self.final_progress.retain(|u| u.id != id);
        self.final_progress.push(update);
        let now = Instant::now();
        let due = self
            .last_progress
            .is_none_or(|at| now.saturating_duration_since(at) >= self.progress_interval);
        if due {
            self.emit_progress(now);
            self.progress_off_tick = true;
        }
    }

    /// Removes finished top-level items and their descendants from the
    /// store and from every map of the actor; returns how many top-level
    /// items the store removed (finished trees that were never loaded
    /// included). A finished root with a descendant that is not finished
    /// yet (still stopping) is kept, in the store and in memory.
    fn clear_finished(&mut self) -> Result<usize> {
        let children = self.children_index();
        let mut keep = Vec::new();
        let mut prune = Vec::new();
        for root in self
            .items
            .values()
            .filter(|i| i.parent.is_none() && i.state.is_terminal())
        {
            let tree = Self::subtree(&children, root.id);
            let finished = tree.iter().all(|id| {
                !self.running.contains_key(id)
                    && self.items.get(id).is_some_and(|i| i.state.is_terminal())
            });
            if finished {
                prune.extend(tree);
            } else {
                keep.push(root.id);
            }
        }
        let removed = self.store.clear_finished_except(&keep)?;
        for id in prune {
            if let Some(item) = self.items.remove(&id)
                && let Some(queue) = self.queued.get_mut(&item.spec.site)
            {
                queue.remove(id);
            }
            self.dirs.remove(&id);
            self.overrides.remove(&id);
            self.tracker.unregister(id);
        }
        Ok(removed)
    }

    fn on_file_prepared(
        &mut self,
        id: TransferId,
        route: Route,
        src: RemoteEntry,
        dst: Option<RemoteEntry>,
        decision: Decision,
    ) {
        if let Err(e) = self.store.set_meta(id, false, Some(src.size)) {
            self.release(id, TransferState::Failed(e));
            return;
        }
        if let Some(item) = self.items.get_mut(&id) {
            item.is_dir = Some(false);
            item.size = Some(src.size);
        }
        // Prepared under `Ask` before the user chose a policy for all
        // conflicts: apply that policy instead of asking again.
        let decision = match decision {
            Decision::Ask if self.policy != ConflictPolicy::Ask => {
                decide(self.policy, &src, dst.as_ref())
            }
            decision => decision,
        };
        match decision {
            Decision::Write { offset } => {
                // The continuation of this copy, stored before the copy can
                // open the destination: the next run (after a pause, a
                // network error or a restart) continues from the partial
                // destination; but replacing an existing file starts over,
                // as the interruption may have come before the destination
                // was truncated.
                let retry = if offset == 0 && dst.is_some() {
                    ConflictPolicy::Overwrite
                } else {
                    ConflictPolicy::Resume
                };
                if let Err(e) = self.store.set_next_policy(id, Some(retry)) {
                    self.release(id, TransferState::Failed(e));
                    return;
                }
                if let Some(item) = self.items.get(&id) {
                    self.last_writer.insert(dest_key(&item.spec), id);
                }
                let cancel = match self.running.get_mut(&id) {
                    Some(running) => {
                        running.retry = Some(retry);
                        running.copying = true;
                        running.cancel.clone()
                    }
                    None => CancellationToken::new(),
                };
                let counter = self.tracker.register(id, Some(src.size), offset);
                worker::spawn_copy(
                    id,
                    route,
                    src,
                    offset,
                    self.buf_size,
                    counter,
                    cancel,
                    self.worker_tx.clone(),
                );
            }
            Decision::Skip(reason) => self.release(id, TransferState::Skipped(reason)),
            Decision::Ask => {
                let info = ConflictInfo {
                    src_size: src.size,
                    dst_size: dst.as_ref().map_or(0, |d| d.size),
                    src_modified: src.modified,
                    dst_modified: dst.as_ref().and_then(|d| d.modified),
                };
                self.release(id, TransferState::AwaitingDecision(info));
            }
        }
    }

    /// Emits the periodic `EventKind::Progress`.
    fn tick(&mut self) {
        self.emit_progress(Instant::now());
    }

    /// Emits one `EventKind::Progress`: the final counts of the copy runs
    /// that ended since the last one (see `finish_progress`), then a sample
    /// of the running ones; nothing when both are empty. Running counts are
    /// persisted best-effort here; a final count is persisted (strictly) at
    /// completion.
    fn emit_progress(&mut self, now: Instant) {
        let sampled = self.tracker.sample(now);
        if !sampled.is_empty() {
            let mut batch = Vec::with_capacity(sampled.len());
            for u in &sampled {
                if let Some(item) = self.items.get_mut(&u.id) {
                    item.bytes_done = u.bytes_done;
                }
                batch.push((u.id, u.bytes_done));
            }
            let _ = self.store.set_bytes_done_many(&batch);
        }
        let mut updates = std::mem::take(&mut self.final_progress);
        updates.extend(sampled);
        if updates.is_empty() {
            return;
        }
        self.last_progress = Some(now);
        self.emit(EventKind::Progress(updates));
    }
}

/// A folder this run created whose files were marked fresh: its tree and destination.
pub(super) struct FreshDir {
    pub(super) root: TransferId,
    pub(super) dst: RemotePath,
    pub(super) files: Vec<TransferId>,
}

/// Where `spec` writes.
pub(super) fn destination(spec: &TransferSpec) -> &RemotePath {
    match spec.direction {
        Direction::Upload => &spec.remote,
        Direction::Download => &spec.local,
    }
}

/// `a` and `b` are the same path, or one is below the other. Sites and sides are not
/// compared: a false overlap only costs a conflict check.
pub(super) fn paths_overlap(a: &RemotePath, b: &RemotePath) -> bool {
    let below = |inner: &str, outer: &str| {
        outer == "/"
            || inner
                .strip_prefix(outer)
                .is_some_and(|rest| rest.starts_with('/'))
    };
    let (a, b) = (a.as_str(), b.as_str());
    a == b || below(a, b) || below(b, a)
}

#[cfg(test)]
mod tests {
    use hoprs_core::ErrorKind;

    use super::*;

    #[test]
    fn pop_eligible_keeps_order_and_skipped_ids() {
        let mut q = SiteQueue::default();
        for (id, dir) in [(5, false), (2, false), (9, true), (3, false), (1, true)] {
            q.push(TransferId(id), dir);
        }
        // Files before folders, lowest id first; busy ids stay queued.
        let busy = |id: TransferId| id == TransferId(2) || id == TransferId(1);
        assert_eq!(q.pop_eligible(busy), Some(TransferId(3)));
        assert_eq!(q.pop_eligible(busy), Some(TransferId(5)));
        assert_eq!(q.pop_eligible(busy), Some(TransferId(9)));
        assert_eq!(q.pop_eligible(busy), None);
        // The skipped ones are still there, in order.
        assert_eq!(q.pop_eligible(|_| false), Some(TransferId(2)));
        assert_eq!(q.pop_eligible(|_| false), Some(TransferId(1)));
        assert_eq!(q.pop_eligible(|_| false), None);
    }

    #[test]
    fn dest_keys_separate_sites_for_uploads_only() {
        let up = |site: &str| TransferSpec {
            site: SiteId(site.into()),
            direction: Direction::Upload,
            local: RemotePath::new("/a/x").unwrap(),
            remote: RemotePath::new("/srv/x").unwrap(),
        };
        let down = |site: &str| TransferSpec {
            site: SiteId(site.into()),
            direction: Direction::Download,
            local: RemotePath::new("/dl/x").unwrap(),
            remote: RemotePath::new("/x").unwrap(),
        };
        assert_ne!(dest_key(&up("s1")), dest_key(&up("s2")));
        assert_eq!(dest_key(&up("s1")), dest_key(&up("s1")));
        assert_eq!(dest_key(&down("s1")), dest_key(&down("s2")));
    }

    #[test]
    fn store_error_goes_into_the_copy_error_detail() {
        let store_err = Error::new(ErrorKind::Io, "Queue database error").with_detail("disk full");
        let copy = Error::new(ErrorKind::Permission, "denied");
        let e = with_store_detail(copy, &store_err);
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ErrorKind::Permission, "denied")
        );
        assert_eq!(
            e.detail.as_deref(),
            Some("queue store: Queue database error: disk full")
        );

        let copy = Error::new(ErrorKind::Io, "read failed").with_detail("eof");
        let e = with_store_detail(copy, &store_err);
        assert_eq!(
            e.detail.as_deref(),
            Some("eof; queue store: Queue database error: disk full")
        );
    }
}
