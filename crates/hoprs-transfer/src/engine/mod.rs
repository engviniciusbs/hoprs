//! The transfer engine: a single actor task that owns the queue and
//! dispatches per-site workers. `TransferEngine` is a cheap, cloneable handle.

mod actor;
mod control;
mod dirs;
mod recovery;
mod worker;

use std::fmt;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use hoprs_core::{Error, ErrorKind, RemotePath, Result, SiteId};
use hoprs_remote::RemoteFs;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::model::{ConflictPolicy, Origin, TransferId, TransferItem, TransferSpec, TransferState};
use crate::progress::ProgressUpdate;
use crate::store::QueueStore;

use actor::Command;
use control::Control;

const EVENT_CAPACITY: usize = 1024;
const COMMAND_CAPACITY: usize = 64;

/// Engine tuning knobs.
#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub buf_size: usize,
    pub progress_interval: Duration,
    pub default_policy: ConflictPolicy,
    /// Checks the local path of every item of [`Origin::Agent`] before the
    /// item touches a filesystem. `None`: agent items are not checked.
    pub agent_guard: Option<LocalGuard>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            buf_size: 256 * 1024,
            progress_interval: Duration::from_millis(100),
            default_policy: ConflictPolicy::Ask,
            agent_guard: None,
        }
    }
}

/// The future a [`LocalGuard`] check returns.
type GuardFuture = Pin<Box<dyn Future<Output = Result<()>> + Send>>;

/// A check of the local path of an agent's item, run by the engine before
/// any filesystem call of the item (each run, files and folders alike). An
/// error fails the item with that error; nothing is listed, created or
/// written. Items of [`Origin::User`] never reach it.
#[derive(Clone)]
pub struct LocalGuard(Arc<dyn Fn(RemotePath) -> GuardFuture + Send + Sync>);

impl LocalGuard {
    pub fn new(check: impl Fn(RemotePath) -> GuardFuture + Send + Sync + 'static) -> LocalGuard {
        LocalGuard(Arc::new(check))
    }

    pub(crate) fn check(&self, local: RemotePath) -> GuardFuture {
        (self.0)(local)
    }
}

impl fmt::Debug for LocalGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LocalGuard")
    }
}

/// How [`TransferEngine::enqueue_with`] enqueues a transfer.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct EnqueueOptions {
    /// The conflict policy of the whole tree: the enqueued item's own run,
    /// and every item under it that has no continuation of its own.
    /// `None`: the global policy.
    pub policy: Option<ConflictPolicy>,
    /// Who asked; every item of the tree records it.
    pub origin: Origin,
}

/// Filesystems and worker count of one site. `workers` is clamped to `1..=16`.
#[derive(Clone)]
pub struct SiteEndpoints {
    pub local: Arc<dyn RemoteFs>,
    pub remote: Arc<dyn RemoteFs>,
    pub workers: usize,
}

/// User answer to a conflict.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ResolveAction {
    Overwrite,
    Skip,
    Resume,
}

/// A conflict resolution, optionally applied to every pending conflict.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    pub action: ResolveAction,
    pub apply_to_all: bool,
}

/// A notification broadcast by the engine.
///
/// `seq` numbers the events of one engine from 1, increasing by one per
/// event emitted. A subscriber that lags (`RecvError::Lagged`) or starts
/// late takes a [`snapshot_with_seq`](TransferEngine::snapshot_with_seq)
/// and drops every event whose `seq` is at most the snapshot's: the
/// snapshot already reflects them.
#[derive(Clone, Debug)]
pub struct Event {
    pub seq: u64,
    pub kind: EventKind,
}

/// What an [`Event`] reports.
#[derive(Clone, Debug)]
pub enum EventKind {
    Added(TransferItem),
    StateChanged {
        id: TransferId,
        state: TransferState,
    },
    /// Byte counts, at most one event per `EngineConfig::progress_interval`
    /// for the whole engine. A run's final count (rate 0) comes before the
    /// state change that ends it when no `Progress` went out in the last
    /// interval, otherwise with the next one, after that state change.
    Progress(Vec<ProgressUpdate>),
    SitePaused {
        site: SiteId,
        error: Error,
    },
}

/// Handle to the engine actor.
#[derive(Clone)]
pub struct TransferEngine {
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<Event>,
}

fn stopped() -> Error {
    Error::coded(
        ErrorKind::Io,
        "transfer-engine-stopped",
        "Transfer engine stopped",
    )
}

impl TransferEngine {
    /// Spawns the engine actor. Must be called inside a tokio runtime.
    ///
    /// Items left unfinished by a previous run are recovered as `Paused`
    /// (see `QueueStore::recover`); nothing resumes automatically. A store
    /// that cannot be read (a corrupt queue) is an error: the store is
    /// dropped without writing over it, and the caller decides what to do
    /// (for example, report it and run without a persistent queue).
    pub fn start(mut store: QueueStore, config: EngineConfig) -> Result<TransferEngine> {
        let recovered = recovery::recover(&mut store)?;
        let (commands, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        tokio::spawn(actor::run(
            store,
            recovered,
            config,
            command_rx,
            events.clone(),
        ));
        Ok(TransferEngine { commands, events })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// Registers (or replaces) the endpoints of a site; its queued items become dispatchable.
    pub async fn register_site(&self, site: SiteId, endpoints: SiteEndpoints) {
        self.request(|reply| Command::RegisterSite {
            site,
            endpoints,
            reply,
        })
        .await;
    }

    /// Adds a transfer of the user to the queue, under the global policy,
    /// and returns its id.
    pub async fn enqueue(&self, spec: TransferSpec) -> Result<TransferId> {
        self.enqueue_with(spec, EnqueueOptions::default()).await
    }

    /// Like `enqueue`, but the transfer runs under `policy` instead of the
    /// global one (see [`enqueue_with`](Self::enqueue_with)).
    pub async fn enqueue_with_policy(
        &self,
        spec: TransferSpec,
        policy: ConflictPolicy,
    ) -> Result<TransferId> {
        self.enqueue_with(
            spec,
            EnqueueOptions {
                policy: Some(policy),
                origin: Origin::User,
            },
        )
        .await
    }

    /// Adds a transfer to the queue and returns its id.
    ///
    /// With `options.policy`, the item's own conflict decision uses it
    /// instead of the global one: it is stored as the item's `next_policy`,
    /// so it also holds after a restart. It is also the item's tree policy:
    /// every item under an enqueued folder runs under it whenever it has no
    /// continuation of its own (its first run, a resume before its copy
    /// started, a retry of a run that never decided its copy, a run whose
    /// continuation another item's write superseded), so a folder's items
    /// never fall back to the global policy (which may be `Ask`). Items
    /// recovered at startup always come back `Paused`, so after a restart
    /// they run under these policies once resumed.
    ///
    /// `options.origin` is recorded on every item of the tree; the items of
    /// [`Origin::Agent`] pass the engine's `agent_guard` before each run.
    pub async fn enqueue_with(
        &self,
        spec: TransferSpec,
        options: EnqueueOptions,
    ) -> Result<TransferId> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(Command::Enqueue {
                spec,
                options,
                reply,
            })
            .await
            .map_err(|_| stopped())?;
        rx.await.map_err(|_| stopped())?
    }

    /// Pauses a transfer (a folder: with its pending descendants). An active
    /// file stops after its current block and keeps its partial destination.
    pub async fn pause(&self, id: TransferId) -> Result<()> {
        self.control(id, Control::Pause).await
    }

    /// Resumes a paused transfer (a folder: with its paused descendants). A
    /// file paused while copying continues from its partial destination (one
    /// that was replacing an existing file starts over); one paused before
    /// its copy started runs under its tree policy, else the global policy.
    pub async fn resume(&self, id: TransferId) -> Result<()> {
        self.control(id, Control::Resume).await
    }

    /// Cancels a transfer (a folder: with its pending descendants). Partial
    /// destination files are kept.
    pub async fn cancel(&self, id: TransferId) -> Result<()> {
        self.control(id, Control::Cancel).await
    }

    /// Requeues a failed or cancelled transfer (a folder: with its failed or
    /// cancelled descendants). Each item runs under its stored continuation
    /// when it has one, never skipped because of its partial destination:
    /// - a cancelled item keeps the continuation of the run it stopped:
    ///   `Resume` (it continues its partial destination), or `Overwrite`
    ///   when that run was replacing an existing file (it starts over);
    /// - a failed item whose run held any continuation (its copy had been
    ///   decided, or it ran under an override such as a user `resolve`)
    ///   gets `Overwrite`: it starts over;
    /// - any other item runs under its tree policy, else the global policy.
    ///
    /// A child that failed because its name repeats an earlier entry of
    /// its folder's listing is never retried: retrying it returns
    /// `Conflict`, and retrying the folder leaves it failed.
    pub async fn retry(&self, id: TransferId) -> Result<()> {
        self.control(id, Control::Retry).await
    }

    /// Answers the conflict of an item awaiting a decision.
    pub async fn resolve(&self, id: TransferId, resolution: Resolution) -> Result<()> {
        self.control(id, Control::Resolve(resolution)).await
    }

    async fn control(&self, id: TransferId, op: Control) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(Command::Control { id, op, reply })
            .await
            .map_err(|_| stopped())?;
        rx.await.map_err(|_| stopped())?
    }

    /// Sets the conflict policy used by items dispatched from now on.
    pub async fn set_policy(&self, policy: ConflictPolicy) {
        self.request(|reply| Command::SetPolicy { policy, reply })
            .await;
    }

    /// Stops dispatching the items of `site` (a user pause); transfers
    /// already running continue. Has no effect on an unknown site. A network
    /// failure on a user-paused site still emits `SitePaused`.
    pub async fn pause_site(&self, site: SiteId) {
        self.request(|reply| Command::PauseSite { site, reply })
            .await;
    }

    /// Dispatches the items of `site` again: clears both its user pause and
    /// its network pause (an explicit "go" from the user; during a real
    /// outage the next network error pauses the site again). Has no effect
    /// on an unknown site.
    pub async fn resume_site(&self, site: SiteId) {
        self.request(|reply| Command::ResumeSite { site, reply })
            .await;
    }

    /// Clears the network pause of `site` (set by a network error, announced
    /// by `EventKind::SitePaused`). The site dispatches again unless the user
    /// also paused it (`pause_site`). Has no effect on an unknown site.
    ///
    /// Integration: run [`spawn_site_watcher`](crate::spawn_site_watcher) for every connected
    /// site (any protocol). It calls this when the connection returns with a new generation,
    /// after a grace period when a request failed but the connection stayed up, and when the
    /// event receiver lags.
    ///
    /// Calling it when the site is not network-paused does nothing, so the
    /// task never needs to know whether the site is paused.
    pub async fn network_restored(&self, site: SiteId) {
        self.request(|reply| Command::NetworkRestored { site, reply })
            .await;
    }

    /// Stops the engine: running transfers are cancelled, their items keep
    /// their stored state (so the next start recovers them), and this waits
    /// for the actor to finish. Other handles stop working.
    pub async fn shutdown(self) {
        self.request(|reply| Command::Shutdown { reply }).await;
    }

    /// Resolves once the engine actor has exited (after [`shutdown`](Self::shutdown) on any
    /// handle). Resolves at once if it already has. Long-lived helpers that hold an engine
    /// handle (such as the site watcher) use it to end with the engine, since their own
    /// handle keeps the event channel open.
    pub async fn stopped(&self) {
        // The actor owns the command receiver, so the channel closes when it exits.
        self.commands.closed().await;
    }

    /// Sends a command whose only reply is an acknowledgement, and waits for
    /// it. Does nothing if the engine has stopped.
    async fn request(&self, command: impl FnOnce(oneshot::Sender<()>) -> Command) {
        let (reply, rx) = oneshot::channel();
        if self.commands.send(command(reply)).await.is_ok() {
            let _ = rx.await;
        }
    }

    /// All items, ordered by id.
    pub async fn snapshot(&self) -> Vec<TransferItem> {
        self.snapshot_with_seq().await.1
    }

    /// All items, ordered by id, with the `seq` of the last event emitted
    /// before the snapshot was taken (0 if none). Events with a `seq` at
    /// most that value are already reflected in the items; apply only the
    /// later ones. Returns `(0, [])` if the engine has stopped.
    pub async fn snapshot_with_seq(&self) -> (u64, Vec<TransferItem>) {
        let (reply, rx) = oneshot::channel();
        if self
            .commands
            .send(Command::Snapshot { reply })
            .await
            .is_err()
        {
            return (0, Vec::new());
        }
        rx.await.unwrap_or_default()
    }

    /// Removes every finished (terminal) top-level item with all of its
    /// descendants, from the store and from memory, and returns how many
    /// top-level items were removed. Removed items no longer appear in
    /// snapshots; no event is emitted for them. A finished folder whose
    /// descendants are still stopping (a cancelled folder whose running
    /// child has not reported back yet) is kept until a later call.
    pub async fn clear_finished(&self) -> Result<usize> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(Command::ClearFinished { reply })
            .await
            .map_err(|_| stopped())?;
        rx.await.map_err(|_| stopped())?
    }
}
