//! The backend the effects run against: the vault, the transfer engine, the session
//! manager (with the sessions' terminal shells), the remote edits and the prompt broker,
//! behind [`BackendApi`].

pub mod agent;
pub mod broker;
pub mod edit;
#[cfg(test)]
pub mod fake;
pub mod mapping;
pub mod ops;
pub mod sessions;
pub mod shells;
pub mod vault;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::stream::BoxStream;
use hoprs_core::{
    ConnectionState, Error, RemoteEntry, RemotePath, Result, SavedTab, Site, SiteId, Workspace,
    WorkspaceId,
};
use hoprs_import::{ImportOptions, ImportReport};
use hoprs_remote::{CompareOptions, CompareSignals, Progress, RemoteFs, Summary};
pub use hoprs_ssh::known_hosts::KnownHostEntry;
pub use hoprs_ssh::{AuthMethod, ShellOutput};
use hoprs_transfer::{
    ConflictPolicy, EngineConfig, Event, QueueStore, Resolution, TransferEngine, TransferId,
    TransferItem, TransferSpec,
};
use hoprs_vault::{
    AppSecretStore, BackupImportReport, ExportParams, ExportReport, ExportScope, ImportMode,
    MemoryStore, SecretStore, Vault,
};
use secrecy::SecretString;
use tokio::sync::{broadcast, mpsc, watch};

pub use broker::{BrokerEvent, PromptAnswer, PromptBroker, PromptId};
pub use edit::{
    EditChoice, EditEvent, EditId, EditManager, EditSession, EditState, EditSummary, KeptEdit,
    UploadMode,
};
pub use ops::DeleteTarget;
pub use sessions::{
    ConnectError, ConnectResult, Connected, SessionConfig, SessionId, SessionManager,
};
pub use shells::ShellId;
pub use vault::{LazyKeyringStore, ProfileSnapshot, VaultHandle};

use crate::agent::server::AgentServer;
use crate::agent::service::Service;
use crate::agent::{AgentEvent, Limits};
use crate::drive::{ConnectEvent, Drive, DriveAccount, DriveFile, Pages, Sealed};
use crate::settings::{self, Settings, SettingsLoad};
use crate::update::Release;
use crate::update::download::{self, DownloadEvent, DownloadJob};
use crate::update::http::{self, Client};

/// Which pane of a files tab.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Local,
    Remote,
}

/// What to do with a transfer item.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ControlOp {
    Pause,
    Resume,
    Cancel,
    Retry,
    Resolve(Resolution),
}

/// The transfer queue as the engine has it, for the queue panel to start (or start over)
/// from.
#[derive(Clone, Debug, PartialEq)]
pub struct TransferSnapshot {
    /// The `seq` of the last engine event the items reflect (see
    /// [`TransferEngine::snapshot_with_seq`]).
    pub seq: u64,
    pub items: Vec<TransferItem>,
    /// Transfer workers per site.
    pub workers: usize,
}

/// A workspace's saved tabs, by site (see [`BackendApi::save_workspace_tabs`]).
pub type WorkspaceTabs = Vec<(SiteId, Vec<SavedTab>)>;

/// What `effects::run` may call. Each later task adds the methods its effects need.
#[async_trait]
pub trait BackendApi: Send + Sync {
    // Test convenience: the app reads through `profiles` and `connect_batch`.
    #[cfg(test)]
    async fn sites(&self) -> Result<Vec<Site>>;
    #[cfg(test)]
    async fn workspaces(&self) -> Result<Vec<Workspace>>;
    /// Sites, workspaces and unreadable entries, read together.
    async fn profiles(&self) -> Result<ProfileSnapshot>;
    /// The directory holding `sites.toml` ("Open sites.toml folder").
    fn profile_dir(&self) -> PathBuf;
    /// Saves a site edited in the UI. Fields the backend owns are kept from the stored
    /// copy: the FTPS pin and `has_password`/`has_key_passphrase` (see
    /// [`VaultHandle::save_site`]).
    async fn save_site(&self, site: Site) -> Result<()>;
    /// Stores (`Some`) or deletes (`None`) the site's password in the keyring, which also
    /// sets or clears its `has_password`.
    async fn set_password(&self, site: SiteId, password: Option<SecretString>) -> Result<()>;
    /// Also removes the site from every workspace and deletes its secrets.
    async fn delete_site(&self, id: SiteId) -> Result<()>;
    async fn save_workspace(&self, workspace: Workspace) -> Result<()>;
    async fn delete_workspace(&self, id: WorkspaceId) -> Result<()>;
    /// Replaces the saved tabs of the workspace's sites named in `tabs` (the others keep
    /// theirs), reading the workspace as stored now: a form saved meanwhile is not undone.
    /// One atomic write of `workspaces.toml`; `NotFound` if the workspace is gone.
    async fn save_workspace_tabs(&self, workspace: WorkspaceId, tabs: WorkspaceTabs) -> Result<()>;

    /// Connects a saved site; see [`SessionManager::connect`].
    #[cfg(test)]
    async fn connect(&self, site: SiteId) -> std::result::Result<Connected, ConnectError>;
    /// Open workspace: connects `sites` in parallel, reading their secrets as one batch
    /// (at most one keyring unlock prompt), and sends each site's result on `results` as
    /// soon as it is known. Returns when every site is done; see
    /// [`SessionManager::connect_batch`].
    async fn connect_batch(
        &self,
        sites: Vec<SiteId>,
        results: mpsc::UnboundedSender<(SiteId, ConnectResult)>,
    );
    /// Retries a `Disconnected` session; see [`SessionManager::reconnect`].
    async fn reconnect(&self, session: SessionId) -> Result<()>;
    async fn close(&self, session: SessionId);
    /// Watches a session's connection state; `None` once it is closed. The sender goes
    /// away with the session, which ends the watch.
    fn session_state(&self, session: SessionId) -> Option<watch::Receiver<ConnectionState>>;
    /// How an SSH session logged in last; `None` for FTP, or before the first connection.
    fn session_auth(&self, session: SessionId) -> Option<AuthMethod>;
    /// The window is closing: shuts the transfer engine down (its running items keep their
    /// stored state, so the next start reopens them paused), then closes every session.
    async fn shutdown(&self);

    // Terminal shells, for the terminal tabs.
    /// Opens a terminal (PTY + shell, `cols` x `rows`) on an SSH session; see
    /// [`shells`]. Not connected: `Network`, "Not connected". Past the connection's channel
    /// budget: `Protocol`, "Too many channels on this connection (limit N)". An FTP session
    /// has no terminal (`Protocol`).
    async fn open_shell(&self, session: SessionId, cols: u16, rows: u16) -> Result<ShellId>;
    /// Sends keyboard input to the shell. Ended or unknown: `Network`, "Shell closed".
    async fn shell_input(&self, shell: ShellId, data: Vec<u8>) -> Result<()>;
    /// The terminal is now `cols` x `rows`.
    async fn shell_resize(&self, shell: ShellId, cols: u16, rows: u16) -> Result<()>;
    /// Closes the shell without waiting for the server (its output then yields `Closed`)
    /// and forgets it. Unknown ids are ignored.
    fn close_shell(&self, shell: ShellId);
    /// The shell's output (`Data`..., maybe `Exit`, then `Closed`), handed out once, for
    /// the tab's subscription; `None` afterwards and for unknown shells.
    fn shell_output(&self, shell: ShellId) -> Option<BoxStream<'static, ShellOutput>>;

    // Remote editing (see [`edit`]).
    /// Downloads the remote file `remote` of `session` to the private edit directory and
    /// opens it with `editor` (the settings' command: split with shell quoting rules, run
    /// without a shell, the path as its last argument). Every save goes back up after
    /// [`edit::DEBOUNCE`] of quiet, unless the server's copy changed meanwhile
    /// ([`EditEvent::RemoteChanged`]); a save while the session is down is held and goes up
    /// on its next `Connected`. A file already being edited opens again (same id).
    async fn start_edit(
        &self,
        session: SessionId,
        remote: RemotePath,
        editor: String,
    ) -> Result<EditId>;
    /// The user's answer to [`EditEvent::RemoteChanged`] (see [`EditChoice`]).
    async fn resolve_edit(&self, id: EditId, choice: EditChoice) -> Result<()>;
    /// The session's edits holding something not on the server yet.
    async fn edits_pending(&self, session: SessionId) -> Vec<EditSummary>;
    /// Ends the session's edits and deletes their local files, unless some hold unsent
    /// changes and `discard` is false: then nothing changes and those are returned.
    async fn finish_edits(&self, session: SessionId, discard: bool) -> Result<Vec<EditSummary>>;
    /// "Retry" on a failed upload: tries the edit again now (with the stamp check).
    async fn retry_edit(&self, id: EditId) -> Result<()>;
    /// "Upload pending changes": sends the session's unsent edits, waiting at most
    /// `within` for the session to be back; returns what is still unsent (conflicts are
    /// never overwritten here).
    async fn flush_edits(&self, session: SessionId, within: Duration) -> Vec<EditSummary>;
    /// "Keep local copies": saves each unsent edit as a recovered copy and ends the
    /// session's edits, deleting nothing (see [`EditManager::keep`]).
    async fn keep_edits(&self, session: SessionId) -> Result<Vec<KeptEdit>>;
    /// A new receiver of the edits' events.
    fn subscribe_edits(&self) -> broadcast::Receiver<EditEvent>;
    /// Where "keep" saves the recovered copies (the close questions say so).
    fn recovered_dir(&self) -> PathBuf;
    /// Saves every unsent edit as a recovered copy now, locally only (a forced exit, the
    /// start of a shutdown); see [`EditManager::preserve_unsent`].
    async fn preserve_edits(&self) -> Vec<KeptEdit>;

    /// Lists `path` on the local disk, or on the remote side of `session` (an error
    /// without one).
    async fn list(
        &self,
        session: Option<SessionId>,
        side: Side,
        path: RemotePath,
    ) -> Result<Vec<RemoteEntry>>;
    /// Creates the folder `path` (its parent must exist) on the side's filesystem.
    async fn mkdir(&self, session: Option<SessionId>, side: Side, path: RemotePath) -> Result<()>;
    /// Renames `from` to `to`; fails with `Conflict` if something is already at `to`
    /// (see [`ops::rename_no_replace`]).
    async fn rename(
        &self,
        session: Option<SessionId>,
        side: Side,
        from: RemotePath,
        to: RemotePath,
    ) -> Result<()>;
    /// Deletes `targets`, folders with everything in them (see [`ops::delete_all`]),
    /// sending the running count of removed items on `progress`. Returns the count.
    async fn delete(
        &self,
        session: Option<SessionId>,
        side: Side,
        targets: Vec<DeleteTarget>,
        progress: mpsc::UnboundedSender<u64>,
    ) -> Result<u64>;
    async fn chmod(
        &self,
        session: Option<SessionId>,
        side: Side,
        path: RemotePath,
        mode: u32,
    ) -> Result<()>;

    /// Compares the local folder `local_root` with the remote folder `remote_root` of
    /// `session` (see [`hoprs_remote::compare`]), over the session's existing connection:
    /// listing concurrency 4 on SFTP, 1 on FTP/FTPS, 4 on the local side (the
    /// options' concurrency fields are overwritten). Differences stream on `progress`.
    /// A closed session fails with the coded "session closed" error. See
    /// [`CompareSignals`] for pausing and cancelling.
    async fn compare(
        &self,
        session: SessionId,
        local_root: RemotePath,
        remote_root: RemotePath,
        options: CompareOptions,
        progress: mpsc::UnboundedSender<Progress>,
        signals: CompareSignals,
    ) -> Result<Summary>;

    async fn enqueue(&self, spec: TransferSpec) -> Result<TransferId>;
    /// Like [`enqueue`](Self::enqueue), with a conflict policy for this item
    /// alone (persisted, so it survives a restart).
    async fn enqueue_with_policy(
        &self,
        spec: TransferSpec,
        policy: ConflictPolicy,
    ) -> Result<TransferId>;
    async fn control(&self, id: TransferId, op: ControlOp) -> Result<()>;
    /// A new receiver of the engine's events. Subscribe first, then take
    /// [`transfer_snapshot`](Self::transfer_snapshot): events at or below its `seq` are
    /// already in it.
    fn transfer_events(&self) -> broadcast::Receiver<Event>;
    async fn transfer_snapshot(&self) -> TransferSnapshot;
    /// Removes the finished top-level items (and their children). Emits no event: take a
    /// new snapshot afterwards.
    async fn clear_finished(&self) -> Result<usize>;

    /// The `known_hosts` lines [`remove_known_host`](Self::remove_known_host) would delete
    /// for `host:port`, to show before it does (a line that also covers other hosts).
    async fn known_host_entries(&self, host: String, port: u16) -> Result<Vec<KnownHostEntry>>;
    /// Deletes every `known_hosts` line for `host:port` (the user's "Remove the old entry"
    /// after a changed host key), if they are still exactly `expected` (what the user saw
    /// and confirmed); `Conflict` otherwise, nothing removed. Returns how many.
    async fn remove_known_host(
        &self,
        host: String,
        port: u16,
        expected: Vec<KnownHostEntry>,
    ) -> Result<usize>;

    /// Delivers the UI's answer to a dialog (closing it is [`PromptAnswer::Cancel`]).
    async fn answer_prompt(&self, id: PromptId, answer: PromptAnswer);
    /// The dialogs to show, for the UI's subscription. Only the first call gets it.
    fn prompt_events(&self) -> Option<mpsc::UnboundedReceiver<BrokerEvent>>;

    /// "Import from FileZilla": reads and parses the `sitemanager.xml` at `path`, then
    /// applies it to the vault (sites, passwords to the keyring, optionally folders as
    /// workspaces). An unreadable or invalid file is an `Err` and changes nothing.
    async fn import_filezilla(&self, path: PathBuf, options: ImportOptions)
    -> Result<ImportReport>;
    /// Deletes the imported `sitemanager.xml` (see [`hoprs_import::delete_source`]: only a
    /// regular file that still looks like FileZilla's).
    async fn delete_sitemanager(&self, path: PathBuf) -> Result<()>;
    /// Writes the encrypted backup of what `scope` names (one site, one workspace and its
    /// sites, or everything), with their secrets, to `path`. Slow on purpose (Argon2id).
    async fn export_backup(
        &self,
        scope: ExportScope,
        path: PathBuf,
        password: SecretString,
    ) -> Result<ExportReport>;
    /// Restores a backup made by [`export_backup`](Self::export_backup).
    async fn restore_backup(
        &self,
        path: PathBuf,
        password: SecretString,
        mode: ImportMode,
    ) -> Result<BackupImportReport>;

    /// The settings read when the backend opened (the engine and the sessions started
    /// with them), with the error if the file was unusable.
    fn settings(&self) -> SettingsLoad;
    /// Why the persistent transfer queue could not be opened, if it could not: this run
    /// keeps its transfers in memory.
    fn queue_error(&self) -> Option<Error>;
    /// Writes `settings.toml` (atomically). Does not apply anything.
    async fn save_settings(&self, settings: Settings) -> Result<()>;
    /// The conflict policy of the transfers dispatched from now on.
    async fn set_policy(&self, policy: ConflictPolicy);
    /// Transfer workers per site for the connections made from now on (`1..=16`).
    fn set_workers(&self, workers: usize);
    /// Keepalive (seconds, `10..=300`) of the SSH and FTP connections made from now on.
    fn set_keepalive(&self, secs: u64);
    /// The master switch of Settings › Agents (MCP): whether agents may connect. A
    /// `settings.toml` saved afterwards sets it too (the stored setting wins).
    async fn set_agents_enabled(&self, on: bool);
    /// What the agent side reports (relays connected, the channel's state, the sites an
    /// agent holds, the sessions an agent brought up), for the UI's subscription. Only
    /// the first call gets it. Bounded while nobody reads (see
    /// [`crate::agent::latest_events`]): of `Relays`, `Sites` and `Channel` the latest
    /// value always arrives; of `Connected`, at least the latest of each site.
    // TODO(Task 14): remove
    #[allow(dead_code)]
    fn agent_events(&self) -> Option<mpsc::Receiver<AgentEvent>>;

    /// The latest stable release (the update check). `update-dev-build` in a debug build
    /// without `HOPRS_UPDATE_URL` (see [`http::api_url`]).
    async fn latest_release(&self) -> Result<Release>;
    /// Downloads and verifies `job` into the Downloads folder (see [`download::run`]). The
    /// stream ends with exactly one [`DownloadEvent::Done`]. While a download of the same
    /// asset runs (even one whose stream was dropped), another is refused at once with
    /// `update-download-busy`; nothing is started twice.
    fn download_update(&self, job: DownloadJob) -> BoxStream<'static, DownloadEvent>;

    // Google Drive (see [`crate::drive`]). Without an OAuth client in the build every call
    // fails with `drive-not-configured`.
    /// The connected account: the connection in memory, else the keyring entry (read
    /// once). `None`: not connected.
    async fn drive_status(&self) -> Result<Option<DriveAccount>>;
    /// Signs in through the browser (see [`Drive::connect`]): `Waiting { url }`, then
    /// exactly one `Done`, last. `pages` are the texts the browser sees afterwards.
    fn drive_connect(&self, pages: Pages) -> BoxStream<'static, ConnectEvent>;
    /// Ends the wait of a running connect (its `Done` is `drive-connect-cancelled`).
    fn drive_cancel(&self);
    /// Revokes the connection at Google (a failure is only logged) and deletes it here.
    async fn drive_disconnect(&self) -> Result<()>;
    /// Seals the encrypted backup of `scope` in memory (like
    /// [`export_backup`](Self::export_backup), without writing it). Slow on purpose.
    async fn seal_export(
        &self,
        scope: ExportScope,
        password: SecretString,
    ) -> Result<(ExportReport, Sealed)>;
    /// Uploads a sealed file as `name` into the `hoprs` folder of the connected Drive.
    async fn drive_upload(&self, name: String, sealed: Sealed) -> Result<DriveFile>;
    /// Writes a sealed file to `path` (atomically, `0o600` on Unix).
    async fn save_sealed(&self, path: PathBuf, sealed: Sealed) -> Result<()>;
}

/// The concurrent listings per side for a session's protocol: `(local, remote)`.
pub fn compare_concurrency(protocol: hoprs_core::Protocol) -> (usize, usize) {
    match protocol {
        hoprs_core::Protocol::Ssh => (4, 4),
        hoprs_core::Protocol::Ftp
        | hoprs_core::Protocol::FtpsExplicit
        | hoprs_core::Protocol::FtpsImplicit => (4, 1),
    }
}

fn not_connected() -> Error {
    Error::coded(
        hoprs_core::ErrorKind::NotFound,
        "net-not-connected",
        "Not connected",
    )
}

/// Runs file work on the blocking pool.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(background_failed)?
}

/// The update client, built on first use (reading the system roots is not free).
type UpdateClient = Arc<OnceLock<Option<Arc<Client>>>>;

fn update_client(cell: &OnceLock<Option<Arc<Client>>>) -> Result<Arc<Client>> {
    cell.get_or_init(|| http::api_url().map(|url| Arc::new(Client::system(url))))
        .clone()
        .ok_or_else(|| {
            Error::coded(
                hoprs_core::ErrorKind::NotFound,
                "update-dev-build",
                "Update checks are off in development builds (set HOPRS_UPDATE_URL)",
            )
        })
}

fn background_failed(err: tokio::task::JoinError) -> Error {
    Error::coded(
        hoprs_core::ErrorKind::Io,
        "app-background-task-failed",
        "Background task failed",
    )
    .with_detail(err.to_string())
}

/// Runs `work` on the blocking pool, streaming what it sends; the stream ends when the
/// worker does. A panicking worker ends it with `failed(app-background-task-failed)`.
/// Starts on the stream's first poll, inside the runtime.
fn blocking_stream<E: Send + 'static>(
    work: impl FnOnce(&mut dyn FnMut(E)) + Send + 'static,
    failed: impl FnOnce(Error) -> E + Send + 'static,
) -> BoxStream<'static, E> {
    use futures_util::StreamExt;
    futures_util::stream::once(async move {
        let (tx, rx) = mpsc::unbounded_channel();
        let worker = tokio::task::spawn_blocking(move || {
            work(&mut |event| {
                let _ = tx.send(event);
            });
        });
        futures_util::stream::unfold(Some((rx, worker, failed)), |state| async move {
            let (mut rx, worker, failed) = state?;
            if let Some(event) = rx.recv().await {
                return Some((event, Some((rx, worker, failed))));
            }
            // The channel closed: after the last event, or because the worker panicked.
            let err = worker.await.err()?;
            Some((failed(background_failed(err)), None))
        })
    })
    .flatten()
    .boxed()
}

/// The asset names being downloaded (see [`BackendApi::download_update`]).
type Downloading = Arc<Mutex<HashSet<String>>>;

/// Holds an asset name in [`Downloading`] until the worker ends.
struct InFlight {
    set: Downloading,
    name: String,
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.set
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.name);
    }
}

/// Runs `work` on the blocking pool unless `name` is already downloading, streaming what
/// it sends; always ends with one `Done` (a panicking worker gives
/// `app-background-task-failed`). Starts on the stream's first poll, inside the runtime.
fn start_download(
    downloading: Downloading,
    name: String,
    work: impl FnOnce(&mut dyn FnMut(DownloadEvent)) + Send + 'static,
) -> BoxStream<'static, DownloadEvent> {
    use futures_util::StreamExt;
    futures_util::stream::once(async move {
        let claimed = downloading
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.clone());
        if !claimed {
            let busy = Error::coded(
                hoprs_core::ErrorKind::Conflict,
                "update-download-busy",
                "This update is already downloading",
            );
            return futures_util::stream::iter([DownloadEvent::Done(Err(busy))]).left_stream();
        }
        let guard = InFlight {
            set: downloading,
            name,
        };
        blocking_stream(
            move |send| {
                let _guard = guard;
                work(send);
            },
            |err| DownloadEvent::Done(Err(err)),
        )
        .right_stream()
    })
    .flatten()
    .boxed()
}

/// Where the backend keeps its files.
#[derive(Clone, Debug)]
pub struct BackendConfig {
    /// `sites.toml` and `workspaces.toml` (see [`hoprs_vault::default_dir`]).
    pub profile_dir: PathBuf,
    /// The persistent transfer queue; `None` keeps it in memory.
    pub queue_path: Option<PathBuf>,
    /// The workers here are replaced by `settings.toml`'s.
    pub sessions: SessionConfig,
    /// Argon2id costs of the backups this backend writes.
    pub backup_params: ExportParams,
    /// The local copies of remote files being edited (see [`edit::default_edit_dir`]).
    pub edit_dir: PathBuf,
    /// The user's versions saved when a remote edit takes the server's (see
    /// [`edit::default_recovered_dir`]).
    pub recovered_dir: PathBuf,
    /// The directory of the agent channel, protected from agents' transfers like the
    /// others; `None` until the run knows it.
    pub agent_dir: Option<PathBuf>,
}

impl BackendConfig {
    /// The standard locations: profiles in `~/.config/hoprs`, the queue in
    /// `$XDG_DATA_HOME/hoprs/queue.db` (spec §5). `None` without a config directory.
    pub fn standard() -> Option<Self> {
        Some(Self {
            profile_dir: hoprs_vault::default_dir()?,
            queue_path: dirs::data_dir().map(|dir| dir.join("hoprs").join("queue.db")),
            sessions: SessionConfig::default(),
            backup_params: ExportParams::default(),
            edit_dir: edit::default_edit_dir(),
            recovered_dir: edit::default_recovered_dir(),
            agent_dir: None,
        })
    }

    /// hoprs's own directories: the profiles, the queue's directory, the remote edits'
    /// copies, the recovered versions and the agent channel's directory.
    fn own_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![self.profile_dir.clone()];
        dirs.extend(
            self.queue_path
                .as_deref()
                .and_then(Path::parent)
                .map(Path::to_path_buf),
        );
        dirs.push(self.edit_dir.clone());
        dirs.push(self.recovered_dir.clone());
        dirs.extend(self.agent_dir.clone());
        dirs
    }
}

/// The real backend.
pub struct Backend {
    profile_dir: PathBuf,
    /// `settings.toml`, beside the profiles.
    settings_path: PathBuf,
    /// What `settings.toml` held when the backend opened.
    settings: SettingsLoad,
    /// Held across a write of `settings.toml` and the switch's update from it, so the
    /// switch always equals the file written last.
    settings_turn: tokio::sync::Mutex<()>,
    /// Why the queue database could not be opened (the engine runs in memory then).
    queue_error: Option<Error>,
    backup_params: ExportParams,
    vault: VaultHandle,
    engine: TransferEngine,
    sessions: SessionManager,
    edits: EditManager,
    broker: PromptBroker,
    update: UpdateClient,
    downloading: Downloading,
    /// The Google Drive connection (its keyring entry in the app secret store).
    drive: Arc<Drive>,
    /// The master switch for agents, started from `settings.toml`; the agent channel
    /// follows it.
    agents: tokio::sync::watch::Sender<bool>,
    /// What the agent service works through (see [`agent`]).
    agent: Arc<agent::RealHost>,
    /// The agent channel's server; `None` only while [`Backend::open`] builds it.
    agent_server: Option<AgentServer>,
    /// The agent side's events, until the UI takes them.
    agent_events: Mutex<Option<mpsc::Receiver<AgentEvent>>>,
}

impl Backend {
    /// Opens the vault and the queue and starts the engine. Must run inside the tokio
    /// runtime (iced's `boot` does). It reads the two small profile files and opens the
    /// queue database synchronously, once, before the first frame; the keyring is only
    /// reached later, on the blocking pool ([`LazyKeyringStore`]).
    ///
    /// A queue that cannot be opened or read is logged and replaced by an in-memory one,
    /// so the app still starts (the old file is left untouched);
    /// [`BackendApi::queue_error`] reports it.
    ///
    /// `settings.toml` (beside the profiles, also read here) gives the engine its conflict
    /// policy and the sessions their workers and keepalive; an unusable file means the defaults (it is
    /// logged, kept as is, and [`BackendApi::settings`] reports it).
    pub fn open(mut config: BackendConfig, secrets: Arc<dyn SecretStore>) -> Result<Backend> {
        let vault = VaultHandle::new(Vault::open(&config.profile_dir, secrets)?);
        let settings_path = config.profile_dir.join(settings::FILE_NAME);
        let loaded = settings::load(&settings_path);
        if let Some(error) = &loaded.error {
            tracing::warn!(%error, "settings.toml is unusable; this run uses the defaults");
        }
        // Before the engine starts: its guard of agent items needs them.
        let fixed = crate::agent::paths::fixed_roots(
            &config.own_dirs(),
            &config.sessions.known_hosts_path(),
        );
        let (engine, queue_error) = start_engine(
            config.queue_path.as_deref(),
            EngineConfig {
                default_policy: loaded.settings.default_policy,
                agent_guard: Some(agent::guard(vault.clone(), fixed.clone())),
                ..EngineConfig::default()
            },
        );
        let agents = tokio::sync::watch::Sender::new(loaded.settings.agents_enabled);
        config.sessions.workers = loaded.settings.workers_per_site;
        config.sessions.keepalive = std::time::Duration::from_secs(loaded.settings.keepalive_secs);
        let broker = PromptBroker::new(vault.clone());
        let sessions = SessionManager::new(
            engine.clone(),
            vault.clone(),
            broker.clone(),
            config.sessions,
        );
        let agent = Arc::new(agent::RealHost::new(
            vault.clone(),
            engine.clone(),
            sessions.clone(),
            broker.clone(),
            agents.subscribe(),
            fixed,
        ));
        let channel = config
            .agent_dir
            .as_deref()
            .map(crate::agent::channel::endpoint_in);
        let (agent_tx, agent_rx) = mpsc::unbounded_channel();
        let mut backend = Backend {
            edits: EditManager::new(config.edit_dir, config.recovered_dir),
            profile_dir: config.profile_dir,
            settings_path,
            settings: loaded,
            settings_turn: tokio::sync::Mutex::new(()),
            queue_error,
            backup_params: config.backup_params,
            vault,
            engine,
            sessions,
            broker,
            update: UpdateClient::default(),
            downloading: Downloading::default(),
            drive: Arc::new(Drive::new(Arc::new(MemoryStore::new()))),
            agents,
            agent,
            agent_server: None,
            agent_events: Mutex::new(Some(crate::agent::latest_events(agent_rx))),
        };
        // Its own task: the channel is opened there (never on this thread) when the
        // switch is on.
        let service = Service::new(backend.agent_host(), Limits::default(), agent_tx.clone());
        backend.agent_server = Some(AgentServer::start(
            service,
            channel,
            backend.agents.subscribe(),
            agent_tx,
        ));
        Ok(backend)
    }

    /// The agent service's host over this backend.
    pub fn agent_host(&self) -> Arc<agent::RealHost> {
        self.agent.clone()
    }

    /// Where the app's own secrets (the Google Drive connection) are kept; by default in
    /// memory only. Makes no call to `store`.
    pub fn with_app_secrets(mut self, store: Arc<dyn AppSecretStore>) -> Backend {
        self.drive = Arc::new(Drive::new(store));
        self
    }
}

impl Backend {
    /// The filesystem of one side: the local disk, or the remote side of `session` (an
    /// error without one).
    fn fs(&self, session: Option<SessionId>, side: Side) -> Result<Arc<dyn RemoteFs>> {
        match (side, session) {
            (Side::Local, _) => Ok(self.sessions.local()),
            (Side::Remote, Some(session)) => self.sessions.remote(session),
            (Side::Remote, None) => Err(not_connected()),
        }
    }
}

/// The engine over the queue at `path`, or over an in-memory queue if that fails (with
/// why).
fn start_engine(path: Option<&Path>, config: EngineConfig) -> (TransferEngine, Option<Error>) {
    let persistent = path.map(|path| {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(Error::from)?;
        }
        TransferEngine::start(QueueStore::open(path)?, config.clone())
    });
    let error = match persistent {
        Some(Ok(engine)) => return (engine, None),
        Some(Err(err)) => {
            tracing::error!(error = %err, "cannot open the transfer queue; this run keeps it in memory");
            Some(err)
        }
        None => None,
    };
    let store = QueueStore::open_in_memory().expect("an in-memory queue always opens");
    let engine = TransferEngine::start(store, config).expect("an empty queue always starts");
    (engine, error)
}

#[async_trait]
impl BackendApi for Backend {
    #[cfg(test)]
    async fn sites(&self) -> Result<Vec<Site>> {
        self.vault.sites().await
    }

    #[cfg(test)]
    async fn workspaces(&self) -> Result<Vec<Workspace>> {
        self.vault.workspaces().await
    }

    async fn profiles(&self) -> Result<ProfileSnapshot> {
        self.vault.snapshot().await
    }

    fn profile_dir(&self) -> PathBuf {
        self.profile_dir.clone()
    }

    async fn save_site(&self, site: Site) -> Result<()> {
        self.vault.save_site(site).await
    }

    async fn set_password(&self, site: SiteId, password: Option<SecretString>) -> Result<()> {
        self.vault.set_password(site, password).await
    }

    async fn delete_site(&self, id: SiteId) -> Result<()> {
        self.vault.remove_site(id).await
    }

    async fn save_workspace(&self, workspace: Workspace) -> Result<()> {
        self.vault.upsert_workspace(workspace).await
    }

    async fn delete_workspace(&self, id: WorkspaceId) -> Result<()> {
        self.vault.remove_workspace(id).await
    }

    async fn save_workspace_tabs(&self, workspace: WorkspaceId, tabs: WorkspaceTabs) -> Result<()> {
        self.vault.save_workspace_tabs(workspace, tabs).await
    }

    #[cfg(test)]
    async fn connect(&self, site: SiteId) -> std::result::Result<Connected, ConnectError> {
        self.sessions.connect(site).await
    }

    async fn connect_batch(
        &self,
        sites: Vec<SiteId>,
        results: mpsc::UnboundedSender<(SiteId, ConnectResult)>,
    ) {
        self.sessions.connect_batch(sites, results).await;
    }

    async fn reconnect(&self, session: SessionId) -> Result<()> {
        self.sessions.reconnect(session).await
    }

    async fn close(&self, session: SessionId) {
        self.sessions.close(session).await;
    }

    fn session_state(&self, session: SessionId) -> Option<watch::Receiver<ConnectionState>> {
        self.sessions.state(session)
    }

    fn session_auth(&self, session: SessionId) -> Option<AuthMethod> {
        self.sessions.auth(session)
    }

    /// The engine first: a session closed while the engine runs would pause its site,
    /// and an item paused that way reopens as the user's pause. Stopped first, the
    /// engine keeps each running item's stored state, which the next start recovers as
    /// paused.
    async fn shutdown(&self) {
        // Local and fast first: whatever is unsent is safe before any slow step.
        self.edits.preserve_unsent().await;
        // The relays are told hoprs quits and their calls dropped, so nothing of an
        // agent holds a session or a connection attempt when they close.
        if let Some(server) = &self.agent_server {
            server.shutdown().await;
        }
        self.engine.clone().shutdown().await;
        self.sessions.close_all().await;
        // The close flow settled the edits it knew of; anything unsent since is kept as a
        // recovered copy, never deleted.
        self.edits.end_all().await;
    }

    async fn open_shell(&self, session: SessionId, cols: u16, rows: u16) -> Result<ShellId> {
        self.sessions.open_shell(session, cols, rows).await
    }

    async fn shell_input(&self, shell: ShellId, data: Vec<u8>) -> Result<()> {
        self.sessions.shell_input(shell, data).await
    }

    async fn shell_resize(&self, shell: ShellId, cols: u16, rows: u16) -> Result<()> {
        self.sessions.shells().resize(shell, cols, rows).await
    }

    fn close_shell(&self, shell: ShellId) {
        self.sessions.shells().close(shell);
    }

    fn shell_output(&self, shell: ShellId) -> Option<BoxStream<'static, ShellOutput>> {
        self.sessions.shells().output(shell)
    }

    /// SFTP replaces the file by a temporary one renamed over it; FTP writes in place
    /// (see [`edit`]).
    async fn start_edit(
        &self,
        session: SessionId,
        remote: RemotePath,
        editor: String,
    ) -> Result<EditId> {
        let fs = self.sessions.remote(session)?;
        let state = self.sessions.state(session).ok_or_else(|| {
            Error::coded(
                hoprs_core::ErrorKind::NotFound,
                "session-closed",
                "Session closed",
            )
        })?;
        let mode = match self.sessions.protocol(session) {
            Some(hoprs_core::Protocol::Ssh) => UploadMode::TempRename,
            _ => UploadMode::InPlace,
        };
        let site = self.sessions.site(session).ok_or_else(|| {
            Error::coded(
                hoprs_core::ErrorKind::NotFound,
                "session-closed",
                "Session closed",
            )
        })?;
        let target = EditSession {
            session,
            site: site.to_string(),
            fs,
            state,
            mode,
        };
        self.edits.start(target, remote, &editor).await
    }

    async fn resolve_edit(&self, id: EditId, choice: EditChoice) -> Result<()> {
        self.edits.resolve(id, choice).await
    }

    async fn edits_pending(&self, session: SessionId) -> Vec<EditSummary> {
        self.edits.pending(session).await
    }

    async fn finish_edits(&self, session: SessionId, discard: bool) -> Result<Vec<EditSummary>> {
        self.edits.finish(session, discard).await
    }

    async fn retry_edit(&self, id: EditId) -> Result<()> {
        self.edits.retry(id).await
    }

    async fn flush_edits(&self, session: SessionId, within: Duration) -> Vec<EditSummary> {
        self.edits.flush(session, within).await
    }

    async fn keep_edits(&self, session: SessionId) -> Result<Vec<KeptEdit>> {
        self.edits.keep(session).await
    }

    fn subscribe_edits(&self) -> broadcast::Receiver<EditEvent> {
        self.edits.subscribe()
    }

    fn recovered_dir(&self) -> PathBuf {
        self.edits.recovered_dir().to_owned()
    }

    async fn preserve_edits(&self) -> Vec<KeptEdit> {
        self.edits.preserve_unsent().await
    }

    async fn list(
        &self,
        session: Option<SessionId>,
        side: Side,
        path: RemotePath,
    ) -> Result<Vec<RemoteEntry>> {
        self.fs(session, side)?.list(&path).await
    }

    async fn mkdir(&self, session: Option<SessionId>, side: Side, path: RemotePath) -> Result<()> {
        self.fs(session, side)?.mkdir(&path).await
    }

    async fn rename(
        &self,
        session: Option<SessionId>,
        side: Side,
        from: RemotePath,
        to: RemotePath,
    ) -> Result<()> {
        let fs = self.fs(session, side)?;
        ops::rename_no_replace(fs.as_ref(), &from, &to).await
    }

    async fn delete(
        &self,
        session: Option<SessionId>,
        side: Side,
        targets: Vec<DeleteTarget>,
        progress: mpsc::UnboundedSender<u64>,
    ) -> Result<u64> {
        let fs = self.fs(session, side)?;
        ops::delete_all(fs.as_ref(), targets, &progress).await
    }

    async fn compare(
        &self,
        session: SessionId,
        local_root: RemotePath,
        remote_root: RemotePath,
        mut options: CompareOptions,
        progress: mpsc::UnboundedSender<Progress>,
        signals: CompareSignals,
    ) -> Result<Summary> {
        let remote = self.sessions.remote(session)?;
        let protocol = self
            .sessions
            .protocol(session)
            .unwrap_or(hoprs_core::Protocol::Ssh);
        (options.concurrency_a, options.concurrency_b) = compare_concurrency(protocol);
        hoprs_remote::compare(
            self.sessions.local(),
            local_root,
            remote,
            remote_root,
            options,
            progress,
            signals,
        )
        .await
    }

    async fn chmod(
        &self,
        session: Option<SessionId>,
        side: Side,
        path: RemotePath,
        mode: u32,
    ) -> Result<()> {
        self.fs(session, side)?.chmod(&path, mode).await
    }

    async fn enqueue(&self, spec: TransferSpec) -> Result<TransferId> {
        self.engine.enqueue(spec).await
    }

    async fn enqueue_with_policy(
        &self,
        spec: TransferSpec,
        policy: ConflictPolicy,
    ) -> Result<TransferId> {
        self.engine.enqueue_with_policy(spec, policy).await
    }

    async fn control(&self, id: TransferId, op: ControlOp) -> Result<()> {
        match op {
            ControlOp::Pause => self.engine.pause(id).await,
            ControlOp::Resume => self.engine.resume(id).await,
            ControlOp::Cancel => self.engine.cancel(id).await,
            ControlOp::Retry => self.engine.retry(id).await,
            ControlOp::Resolve(resolution) => self.engine.resolve(id, resolution).await,
        }
    }

    fn transfer_events(&self) -> broadcast::Receiver<Event> {
        self.engine.subscribe()
    }

    async fn transfer_snapshot(&self) -> TransferSnapshot {
        let (seq, items) = self.engine.snapshot_with_seq().await;
        TransferSnapshot {
            seq,
            items,
            // As the engine clamps it.
            workers: self.sessions.workers().clamp(1, 16),
        }
    }

    async fn clear_finished(&self) -> Result<usize> {
        self.engine.clear_finished().await
    }

    async fn known_host_entries(&self, host: String, port: u16) -> Result<Vec<KnownHostEntry>> {
        let path = self.sessions.known_hosts_path();
        blocking(move || hoprs_ssh::known_hosts::entries_for(&path, &host, port)).await
    }

    async fn remove_known_host(
        &self,
        host: String,
        port: u16,
        expected: Vec<KnownHostEntry>,
    ) -> Result<usize> {
        let path = self.sessions.known_hosts_path();
        let removed =
            blocking(move || hoprs_ssh::known_hosts::remove_entries(&path, &host, port, &expected))
                .await?;
        tracing::info!(removed, "removed known_hosts entries at the user's request");
        Ok(removed)
    }

    async fn answer_prompt(&self, id: PromptId, answer: PromptAnswer) {
        self.broker.answer(id, answer);
    }

    fn prompt_events(&self) -> Option<mpsc::UnboundedReceiver<BrokerEvent>> {
        self.broker.take_events()
    }

    async fn import_filezilla(
        &self,
        path: PathBuf,
        options: ImportOptions,
    ) -> Result<ImportReport> {
        // Parsed before the vault is locked: a bad file never holds up the vault.
        let parsed = blocking(move || hoprs_import::read_sitemanager(&path)).await?;
        let report = self
            .vault
            .call(move |vault| hoprs_import::apply(vault, parsed, &options))
            .await?;
        tracing::info!(
            imported = report.imported.len(),
            skipped = report.skipped.len(),
            failures = report.failures.len(),
            "imported FileZilla sites"
        );
        Ok(report)
    }

    async fn delete_sitemanager(&self, path: PathBuf) -> Result<()> {
        blocking(move || hoprs_import::delete_source(&path)).await?;
        tracing::info!("deleted the imported sitemanager.xml at the user's request");
        Ok(())
    }

    async fn export_backup(
        &self,
        scope: ExportScope,
        path: PathBuf,
        password: SecretString,
    ) -> Result<ExportReport> {
        let params = self.backup_params;
        let kind = scope_kind(&scope);
        let shown = path.display().to_string();
        let result = self
            .vault
            .call(move |vault| vault.export(&path, &scope, &password, params))
            .await;
        match &result {
            Ok(report) => tracing::info!(
                scope = kind,
                sites = report.sites,
                workspaces = report.workspaces,
                secrets = report.secrets,
                path = %shown,
                "exported to a file"
            ),
            Err(err) => {
                tracing::warn!(scope = kind, path = %shown, error = %err, "export to a file failed")
            }
        }
        result
    }

    async fn restore_backup(
        &self,
        path: PathBuf,
        password: SecretString,
        mode: ImportMode,
    ) -> Result<BackupImportReport> {
        self.vault
            .call(move |vault| vault.import_backup(&path, &password, mode))
            .await
    }

    fn settings(&self) -> SettingsLoad {
        self.settings.clone()
    }

    fn queue_error(&self) -> Option<Error> {
        self.queue_error.clone()
    }

    /// Once the file is written, its `agents_enabled` is the master switch: the stored
    /// setting wins over a [`set_agents_enabled`](BackendApi::set_agents_enabled) that
    /// reached the backend out of order. One save at a time, the write and the switch
    /// together: two saves racing leave the switch as the file written last.
    async fn save_settings(&self, settings: Settings) -> Result<()> {
        let _turn = self.settings_turn.lock().await;
        let path = self.settings_path.clone();
        let agents = settings.agents_enabled;
        blocking(move || settings::save(&path, &settings)).await?;
        self.agents
            .send_if_modified(|on| std::mem::replace(on, agents) != agents);
        Ok(())
    }

    async fn set_policy(&self, policy: ConflictPolicy) {
        self.engine.set_policy(policy).await;
    }

    async fn set_agents_enabled(&self, on: bool) {
        self.agents.send_replace(on);
    }

    fn agent_events(&self) -> Option<mpsc::Receiver<AgentEvent>> {
        self.agent_events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    fn set_workers(&self, workers: usize) {
        self.sessions.set_workers(settings::clamp_workers(workers));
    }

    fn set_keepalive(&self, secs: u64) {
        self.sessions
            .set_keepalive(std::time::Duration::from_secs(settings::clamp_keepalive(
                secs,
            )));
    }

    async fn latest_release(&self) -> Result<Release> {
        let cell = self.update.clone();
        blocking(move || update_client(&cell)?.latest()).await
    }

    fn download_update(&self, job: DownloadJob) -> BoxStream<'static, DownloadEvent> {
        let cell = self.update.clone();
        let name = job.asset.name.clone();
        start_download(self.downloading.clone(), name, move |send| {
            let ready =
                update_client(&cell).and_then(|client| Ok((client, download::download_dir()?)));
            match ready {
                Ok((client, dir)) => download::run(client.as_ref(), &job, &dir, send),
                Err(err) => send(DownloadEvent::Done(Err(err))),
            }
        })
    }

    async fn drive_status(&self) -> Result<Option<DriveAccount>> {
        let drive = self.drive.clone();
        blocking(move || drive.status()).await
    }

    fn drive_connect(&self, pages: Pages) -> BoxStream<'static, ConnectEvent> {
        let drive = self.drive.clone();
        blocking_stream(
            move |send| drive.connect(&pages, send),
            |err| ConnectEvent::Done(Err(err)),
        )
    }

    fn drive_cancel(&self) {
        self.drive.cancel();
    }

    async fn drive_disconnect(&self) -> Result<()> {
        let drive = self.drive.clone();
        blocking(move || drive.disconnect()).await
    }

    async fn seal_export(
        &self,
        scope: ExportScope,
        password: SecretString,
    ) -> Result<(ExportReport, Sealed)> {
        let params = self.backup_params;
        let kind = scope_kind(&scope);
        let sealed = self
            .vault
            .call(move |vault| vault.export_bytes(&scope, &password, params))
            .await;
        match sealed {
            Ok((report, bytes)) => {
                tracing::info!(
                    scope = kind,
                    sites = report.sites,
                    workspaces = report.workspaces,
                    secrets = report.secrets,
                    "export encrypted in memory"
                );
                Ok((report, Sealed::new(bytes)))
            }
            Err(err) => {
                tracing::warn!(scope = kind, error = %err, "export failed before it was encrypted");
                Err(err)
            }
        }
    }

    async fn drive_upload(&self, name: String, sealed: Sealed) -> Result<DriveFile> {
        let drive = self.drive.clone();
        let shown = name.clone();
        let result = blocking(move || drive.upload(&name, sealed.bytes())).await;
        match &result {
            Ok(file) => tracing::info!(name = %file.name, "export uploaded to Google Drive"),
            Err(err) => {
                tracing::warn!(name = %shown, error = %err, "export upload to Google Drive failed")
            }
        }
        result
    }

    async fn save_sealed(&self, path: PathBuf, sealed: Sealed) -> Result<()> {
        let shown = path.display().to_string();
        let result = blocking(move || hoprs_vault::write_atomic(&path, sealed.bytes())).await;
        match &result {
            Ok(()) => tracing::info!(path = %shown, "encrypted export saved to a file"),
            Err(err) => {
                tracing::warn!(path = %shown, error = %err, "saving the encrypted export failed")
            }
        }
        result
    }
}

/// The scope's kind for a log line: never a site's or a workspace's name.
fn scope_kind(scope: &ExportScope) -> &'static str {
    match scope {
        ExportScope::All => "all",
        ExportScope::Workspace(_) => "workspace",
        ExportScope::Site(_) => "site",
    }
}

#[cfg(test)]
mod tests {
    use hoprs_core::{ErrorKind, Protocol, WorkspaceSite};
    use hoprs_transfer::{Direction, ResolveAction};
    use hoprs_vault::MemoryStore;

    use super::*;

    /// Argon2id at its cheapest: the tests check the plumbing, not the cost.
    const CHEAP_BACKUP: ExportParams = ExportParams {
        m_cost: 8,
        t_cost: 1,
        p_cost: 1,
    };

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../hoprs-import/tests/fixtures")
            .join(name)
    }

    fn open(dir: &Path) -> Backend {
        open_with_agents(dir, None)
    }

    fn open_with_agents(dir: &Path, agent_dir: Option<PathBuf>) -> Backend {
        let config = BackendConfig {
            profile_dir: dir.join("profiles"),
            queue_path: Some(dir.join("data").join("queue.db")),
            sessions: SessionConfig::default(),
            backup_params: CHEAP_BACKUP,
            edit_dir: dir.join("edit"),
            recovered_dir: dir.join("recovered"),
            agent_dir,
        };
        Backend::open(config, Arc::new(MemoryStore::new())).unwrap()
    }

    #[tokio::test]
    async fn agents_switch_starts_from_the_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        let profiles = dir.path().join("profiles");
        std::fs::create_dir_all(&profiles).unwrap();
        std::fs::write(
            profiles.join(settings::FILE_NAME),
            "agents_enabled = true\n",
        )
        .unwrap();
        let backend = open(dir.path());
        assert!(*backend.agents.borrow());
        backend.set_agents_enabled(false).await;
        assert!(!*backend.agents.borrow());

        // No file: off.
        let other = tempfile::tempdir().unwrap();
        assert!(!*open(other.path()).agents.borrow());
    }

    /// A relay of the real backend, its channel in a temporary directory.
    struct Relay {
        reader: tokio::io::BufReader<tokio::io::ReadHalf<crate::agent::channel::Stream>>,
        writer: tokio::io::WriteHalf<crate::agent::channel::Stream>,
    }

    impl Relay {
        /// Connects (waiting for the channel: `Backend::open` returns before it exists)
        /// and says its hello.
        async fn connect(endpoint: &crate::agent::channel::Endpoint) -> Relay {
            let start = tokio::time::Instant::now();
            let stream = loop {
                match crate::agent::channel::connect(endpoint).await {
                    Ok(stream) => break stream,
                    Err(err) => {
                        assert!(start.elapsed() < LINE_WAIT, "no channel: {err}");
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }
            };
            let (read, writer) = tokio::io::split(stream);
            let mut relay = Relay {
                reader: tokio::io::BufReader::new(read),
                writer,
            };
            let version = env!("CARGO_PKG_VERSION");
            relay
                .send(&format!(r#"{{"hoprs_agent":1,"version":"{version}"}}"#))
                .await;
            assert_eq!(
                relay.line().await.as_deref(),
                Some(format!(r#"{{"hello":"ok","version":"{version}"}}"#).as_str())
            );
            relay
        }

        async fn send(&mut self, line: &str) {
            use tokio::io::AsyncWriteExt;
            self.writer
                .write_all(format!("{line}\n").as_bytes())
                .await
                .unwrap();
        }

        /// The next line; `None` at the end.
        async fn line(&mut self) -> Option<String> {
            tokio::time::timeout(LINE_WAIT, crate::agent::proto::read_line(&mut self.reader))
                .await
                .expect("a line in time")
                .ok()
                .flatten()
        }
    }

    const LINE_WAIT: Duration = Duration::from_secs(10);

    /// The master switch from `settings.toml` opens the channel, a relay's call goes
    /// through the real host, turning it off says so; a saved settings file is the
    /// switch too, and quitting says goodbye.
    #[tokio::test]
    async fn agent_channel_follows_the_settings() {
        let dir = tempfile::tempdir().unwrap();
        let profiles = dir.path().join("profiles");
        std::fs::create_dir_all(&profiles).unwrap();
        std::fs::write(
            profiles.join(settings::FILE_NAME),
            "agents_enabled = true\n",
        )
        .unwrap();
        let agent_dir = dir.path().join("run");
        let backend = open_with_agents(dir.path(), Some(agent_dir.clone()));
        let mut events = backend.agent_events().unwrap();
        let endpoint = crate::agent::channel::endpoint_in(&agent_dir);

        let mut relay = Relay::connect(&endpoint).await;
        relay
            .send(r#"{"call":1,"tool":"list_sites","args":{}}"#)
            .await;
        assert_eq!(
            relay.line().await.as_deref(),
            Some(r#"{"reply":1,"ok":true,"blocks":["{\"sites\":[]}"]}"#)
        );
        backend.set_agents_enabled(false).await;
        assert_eq!(relay.line().await.as_deref(), Some(r#"{"bye":"off"}"#));
        assert_eq!(relay.line().await, None);
        let mut seen = Vec::new();
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(200), events.recv()).await
        {
            seen.push(event);
        }
        assert!(seen.contains(&AgentEvent::Channel(None)), "{seen:?}");
        assert!(seen.contains(&AgentEvent::Relays(1)), "{seen:?}");
        let relays = seen
            .iter()
            .rev()
            .find(|e| matches!(e, AgentEvent::Relays(_)));
        assert_eq!(relays, Some(&AgentEvent::Relays(0)), "{seen:?}");
        // The reaper ran when the channel closed.
        assert_eq!(
            seen.last(),
            Some(&AgentEvent::Sites(Vec::new())),
            "{seen:?}"
        );

        // The stored setting is the switch: saved on, the channel opens again.
        let on = Settings {
            agents_enabled: true,
            ..Settings::default()
        };
        backend.save_settings(on).await.unwrap();
        let mut relay = Relay::connect(&endpoint).await;
        backend.shutdown().await;
        assert_eq!(relay.line().await.as_deref(), Some(r#"{"bye":"quit"}"#));
        assert_eq!(relay.line().await, None);
    }

    /// Saves racing each other: whatever order their writes and their continuations
    /// take, the switch ends equal to the file written last.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn the_switch_is_the_file_written_last() {
        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let path = backend.settings_path.clone();
        for round in 0..40 {
            let saves = (0..8).map(|i| {
                backend.save_settings(Settings {
                    agents_enabled: (i + round) % 2 == 0,
                    ..Settings::default()
                })
            });
            for saved in futures_util::future::join_all(saves).await {
                saved.unwrap();
            }
            let stored = settings::load(&path).settings.agents_enabled;
            assert_eq!(*backend.agents.borrow(), stored, "round {round}");
        }
    }

    #[tokio::test]
    async fn agent_events_are_handed_out_once() {
        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        assert!(backend.agent_events().is_some());
        assert!(backend.agent_events().is_none());

        let fake = fake::FakeBackend::new();
        let mut events = fake.agent_events().unwrap();
        assert!(fake.agent_events().is_none());
        fake.push_agent_event(AgentEvent::Relays(2));
        assert_eq!(events.recv().await, Some(AgentEvent::Relays(2)));
    }

    /// A debug build without `HOPRS_UPDATE_URL` never reaches the network: both update
    /// calls fail with `update-dev-build`.
    #[tokio::test]
    async fn update_calls_need_a_source_in_debug_builds() {
        use futures_util::StreamExt;
        if !cfg!(debug_assertions) || std::env::var_os("HOPRS_UPDATE_URL").is_some() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        assert_eq!(
            backend.latest_release().await.unwrap_err().code,
            Some("update-dev-build")
        );
        let job = DownloadJob {
            asset: crate::update::Asset {
                name: "a.deb".into(),
                url: "https://github.com/a.deb".into(),
                size: 1,
            },
            sums_url: "https://github.com/SHA256SUMS".into(),
        };
        let events: Vec<_> = backend.download_update(job).collect().await;
        assert!(
            matches!(
                events.as_slice(),
                [DownloadEvent::Done(Err(err))] if err.code == Some("update-dev-build")
            ),
            "{events:?}"
        );
    }

    /// (d) A second download of the same asset while one runs starts nothing and is
    /// refused at once; another asset, or the same one afterwards, runs.
    #[tokio::test]
    async fn one_download_per_asset_at_a_time() {
        use futures_util::StreamExt;
        let downloading = Downloading::default();
        let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
        let started = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let work = |gate: Option<std::sync::mpsc::Receiver<()>>| {
            let started = started.clone();
            move |send: &mut dyn FnMut(DownloadEvent)| {
                started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                send(DownloadEvent::Verifying);
                if let Some(gate) = gate {
                    let _ = gate.recv();
                }
                send(DownloadEvent::Done(Ok(PathBuf::from("a.deb"))));
            }
        };
        let mut first = start_download(downloading.clone(), "a.deb".into(), work(Some(gate_rx)));
        assert!(matches!(first.next().await, Some(DownloadEvent::Verifying)));
        let second: Vec<_> = start_download(downloading.clone(), "a.deb".into(), work(None))
            .collect()
            .await;
        assert!(
            matches!(
                second.as_slice(),
                [DownloadEvent::Done(Err(err))] if err.code == Some("update-download-busy")
            ),
            "{second:?}"
        );
        // Another asset is independent.
        let other: Vec<_> = start_download(downloading.clone(), "b.msi".into(), work(None))
            .collect()
            .await;
        assert!(matches!(other.as_slice(), [_, DownloadEvent::Done(Ok(_))]));
        // Dropping the first stream does not free the name: its worker still runs.
        drop(first);
        let again: Vec<_> = start_download(downloading.clone(), "a.deb".into(), work(None))
            .collect()
            .await;
        assert!(matches!(again.as_slice(), [DownloadEvent::Done(Err(_))]));
        assert_eq!(started.load(std::sync::atomic::Ordering::SeqCst), 2);
        // Once it ends, the name is free again.
        gate_tx.send(()).unwrap();
        for _ in 0..200 {
            if downloading.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let last: Vec<_> = start_download(downloading, "a.deb".into(), work(None))
            .collect()
            .await;
        assert!(matches!(last.as_slice(), [_, DownloadEvent::Done(Ok(_))]));
        assert_eq!(started.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    /// A queue database that can't be read: the backend still opens, on an in-memory
    /// queue, and reports why (the file is left as it was).
    #[tokio::test]
    async fn corrupt_queue_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let queue = dir.path().join("data").join("queue.db");
        std::fs::create_dir_all(queue.parent().unwrap()).unwrap();
        let garbage = vec![0x5a_u8; 8192];
        std::fs::write(&queue, &garbage).unwrap();
        let backend = open(dir.path());
        let error = backend.queue_error().expect("the failure is reported");
        assert!(!error.message.is_empty());
        assert_eq!(std::fs::read(&queue).unwrap(), garbage, "left untouched");
        // The in-memory queue works.
        backend
            .enqueue(TransferSpec {
                site: SiteId::new(),
                direction: hoprs_transfer::Direction::Download,
                local: RemotePath::new("/local-x").unwrap(),
                remote: RemotePath::new("/r").unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(backend.transfer_snapshot().await.items.len(), 1);

        // A queue that opens reports nothing.
        let fine = tempfile::tempdir().unwrap();
        assert_eq!(open(fine.path()).queue_error(), None);
    }

    #[tokio::test]
    async fn backend_saves_lists_and_queues() {
        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        assert!(dir.path().join("data").join("queue.db").exists());

        let site = Site::new("s", "h", "u", Protocol::Ssh);
        backend.save_site(site.clone()).await.unwrap();
        let workspace = Workspace {
            id: WorkspaceId::new(),
            name: "w".into(),
            sites: vec![WorkspaceSite {
                site: site.id.clone(),
                tabs: Vec::new(),
            }],
        };
        backend.save_workspace(workspace.clone()).await.unwrap();
        assert_eq!(backend.sites().await.unwrap(), vec![site.clone()]);
        assert_eq!(backend.workspaces().await.unwrap(), vec![workspace.clone()]);

        // Open workspace with a site that no longer exists: it fails alone, inline.
        let gone = SiteId::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        backend.connect_batch(vec![gone.clone()], tx).await;
        let (site_id, result) = rx.recv().await.unwrap();
        assert_eq!(site_id, gone);
        assert_eq!(result.unwrap_err().error.kind, ErrorKind::NotFound);
        assert!(rx.recv().await.is_none(), "one result per site");
        assert_eq!(backend.profile_dir(), dir.path().join("profiles"));
        assert_eq!(backend.profiles().await.unwrap().sites, vec![site.clone()]);
        assert_eq!(
            backend.reconnect(SessionId(99)).await.unwrap_err().kind,
            ErrorKind::NotFound
        );
        backend.close(SessionId(99)).await;

        backend.delete_workspace(workspace.id).await.unwrap();
        backend.delete_site(site.id.clone()).await.unwrap();
        assert!(backend.sites().await.unwrap().is_empty());
        assert!(backend.workspaces().await.unwrap().is_empty());

        // The remote side of an unknown session is an error, not a panic.
        let err = backend
            .list(Some(SessionId(0)), Side::Remote, RemotePath::root())
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        let err = backend
            .list(None, Side::Remote, RemotePath::root())
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);

        // Transfers reach the engine; every control is routed to it.
        let id = backend
            .enqueue(TransferSpec {
                site: site.id,
                direction: Direction::Download,
                local: RemotePath::new("/local-x").unwrap(),
                remote: RemotePath::new("/r").unwrap(),
            })
            .await
            .unwrap();
        backend.control(id, ControlOp::Pause).await.unwrap();
        backend.control(id, ControlOp::Resume).await.unwrap();
        backend.control(id, ControlOp::Cancel).await.unwrap();
        backend.control(id, ControlOp::Retry).await.unwrap();
        let resolve = ControlOp::Resolve(Resolution {
            action: ResolveAction::Skip,
            apply_to_all: false,
        });
        // Nothing awaits a decision: the engine refuses, and the error comes back.
        assert!(backend.control(id, resolve).await.is_err());
        assert!(
            backend
                .control(TransferId(-1), ControlOp::Pause)
                .await
                .is_err()
        );

        assert!(backend.prompt_events().is_some());
        assert!(backend.prompt_events().is_none());
        backend
            .answer_prompt(PromptId(1), PromptAnswer::Cancel)
            .await;
    }

    #[tokio::test]
    async fn compare_on_a_closed_session_is_the_session_closed_error() {
        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let (tx, _rx) = mpsc::unbounded_channel();
        let signals = CompareSignals {
            resume: watch::channel(false).1,
            stop: watch::channel(false).1,
        };
        let error = backend
            .compare(
                SessionId(99),
                RemotePath::new("/l").unwrap(),
                RemotePath::new("/r").unwrap(),
                CompareOptions::default(),
                tx,
                signals,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, Some("session-closed"));
    }

    #[test]
    fn compare_concurrency_is_4_on_sftp_and_1_on_ftp() {
        use hoprs_core::Protocol;
        assert_eq!(compare_concurrency(Protocol::Ssh), (4, 4));
        for ftp in [
            Protocol::Ftp,
            Protocol::FtpsExplicit,
            Protocol::FtpsImplicit,
        ] {
            assert_eq!(compare_concurrency(ftp), (4, 1));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn local_side_needs_no_session() {
        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let local = tempfile::tempdir().unwrap();
        std::fs::write(local.path().join("a.txt"), b"x").unwrap();
        let path = RemotePath::new(local.path().to_str().unwrap()).unwrap();
        let listed = backend.list(None, Side::Local, path).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "a.txt");
    }

    /// The file actions reach the local disk without a session; the remote side needs
    /// one.
    #[cfg(unix)]
    #[tokio::test]
    async fn local_file_actions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let local = tempfile::tempdir().unwrap();
        let root = RemotePath::new(local.path().to_str().unwrap()).unwrap();
        let disk = |name: &str| local.path().join(name);

        backend
            .mkdir(None, Side::Local, root.join("new"))
            .await
            .unwrap();
        assert!(disk("new").is_dir());
        std::fs::write(disk("new").join("a.txt"), b"a").unwrap();
        backend
            .rename(None, Side::Local, root.join("new"), root.join("old"))
            .await
            .unwrap();
        assert!(disk("old").join("a.txt").is_file());
        backend
            .chmod(None, Side::Local, root.join("old").join("a.txt"), 0o600)
            .await
            .unwrap();
        let mode = std::fs::metadata(disk("old").join("a.txt"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);

        // A link to a folder is removed as a link; its target stays.
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("keep.txt"), b"k").unwrap();
        std::os::unix::fs::symlink(outside.path(), disk("old").join("link")).unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let removed = backend
            .delete(
                None,
                Side::Local,
                vec![DeleteTarget {
                    path: root.join("old"),
                    kind: hoprs_core::EntryKind::Dir,
                }],
                tx,
            )
            .await
            .unwrap();
        assert_eq!(removed, 3, "a.txt, link, old/");
        assert!(!disk("old").exists());
        assert!(outside.path().join("keep.txt").is_file());
        let mut last = 0;
        while let Some(n) = rx.recv().await {
            last = n;
        }
        assert_eq!(last, 3);

        let err = backend
            .mkdir(None, Side::Remote, root.join("x"))
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
    }

    /// The queue's feed: events numbered after the snapshot's `seq`, the workers per site,
    /// and "Clear finished" (no event: only a new snapshot shows it).
    #[tokio::test]
    async fn transfer_feed_snapshot_and_clear() {
        use hoprs_transfer::{EventKind, TransferState};

        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let mut events = backend.transfer_events();
        let empty = backend.transfer_snapshot().await;
        assert_eq!((empty.seq, empty.items.len(), empty.workers), (0, 0, 4));

        let id = backend
            .enqueue(TransferSpec {
                site: SiteId::new(),
                direction: Direction::Upload,
                local: RemotePath::new("/l").unwrap(),
                remote: RemotePath::new("/r").unwrap(),
            })
            .await
            .unwrap();
        let added = events.recv().await.unwrap();
        assert!(matches!(&added.kind, EventKind::Added(item) if item.id == id));
        backend.control(id, ControlOp::Cancel).await.unwrap();
        let cancelled = events.recv().await.unwrap();
        assert_eq!(cancelled.seq, added.seq + 1);
        let snapshot = backend.transfer_snapshot().await;
        assert_eq!(snapshot.seq, cancelled.seq);
        assert_eq!(snapshot.items[0].state, TransferState::Cancelled);

        assert_eq!(backend.clear_finished().await.unwrap(), 1);
        let after = backend.transfer_snapshot().await;
        assert_eq!((after.seq, after.items.len()), (snapshot.seq, 0));
    }

    /// "Remove the old entry": the preview lists what removing deletes, from the
    /// session manager's `known_hosts`.
    #[tokio::test]
    async fn known_host_entries_then_remove() {
        let dir = tempfile::tempdir().unwrap();
        let known_hosts = dir.path().join("known_hosts");
        let config = BackendConfig {
            profile_dir: dir.path().join("profiles"),
            queue_path: None,
            sessions: SessionConfig {
                known_hosts: Some(known_hosts.clone()),
                workers: 2,
                ..SessionConfig::default()
            },
            backup_params: CHEAP_BACKUP,
            edit_dir: dir.path().join("edit"),
            recovered_dir: dir.path().join("recovered"),
            agent_dir: None,
        };
        let backend = Backend::open(config, Arc::new(MemoryStore::new())).unwrap();
        let key = "AAAAC3NzaC1lZDI1NTE5AAAAIGrhflDkrs18RwIFvHFCsoXLdUL+0cVr81616Tzk8aBj";
        std::fs::write(
            &known_hosts,
            format!(
                "[h]:2222 ssh-ed25519 {key}\nother ssh-ed25519 {key}\n[h]:2222,other ssh-ed25519 {key}\n"
            ),
        )
        .unwrap();

        let entries = backend.known_host_entries("h".into(), 2222).await.unwrap();
        let lines: Vec<_> = entries
            .iter()
            .map(|e| (e.line, e.covers_other_hosts))
            .collect();
        assert_eq!(lines, [(1, false), (3, true)]);
        // Not what is in the file now (a stale preview): refused, nothing removed.
        let stale = entries[..1].to_vec();
        let err = backend
            .remove_known_host("h".into(), 2222, stale)
            .await
            .unwrap_err();
        assert_eq!(err.kind, hoprs_core::ErrorKind::Conflict);
        assert_eq!(
            backend.known_host_entries("h".into(), 2222).await.unwrap(),
            entries
        );
        assert_eq!(
            backend
                .remove_known_host("h".into(), 2222, entries)
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            std::fs::read_to_string(&known_hosts).unwrap(),
            format!("other ssh-ed25519 {key}\n")
        );
        assert!(
            backend
                .known_host_entries("h".into(), 2222)
                .await
                .unwrap()
                .is_empty()
        );
    }

    /// Review Focus 5: closing stops the engine and the next start finds the unfinished
    /// items paused (nothing resumes on its own).
    #[tokio::test]
    async fn shutdown_stops_the_engine_and_items_reopen_paused() {
        use hoprs_transfer::{Direction, TransferState};

        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let id = backend
            .enqueue(TransferSpec {
                site: SiteId::new(),
                direction: Direction::Upload,
                local: RemotePath::new("/l").unwrap(),
                remote: RemotePath::new("/r").unwrap(),
            })
            .await
            .unwrap();
        backend.shutdown().await;
        tokio::time::timeout(std::time::Duration::from_secs(5), backend.engine.stopped())
            .await
            .expect("the engine stopped");
        // Calls after the shutdown neither hang nor panic.
        backend.close(SessionId(1)).await;
        drop(backend);

        let reopened = open(dir.path());
        let items = reopened.transfer_snapshot().await.items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, id);
        assert_eq!(items[0].state, TransferState::Paused);
    }

    /// The tabs of the named sites are replaced in the stored workspace; the rest of it
    /// (a name changed meanwhile, other sites' tabs) is kept.
    #[tokio::test]
    async fn save_workspace_tabs_updates_only_the_named_sites() {
        use hoprs_core::{SavedTab, TabKind};

        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let a = Site::new("a", "h", "u", Protocol::Ssh);
        let b = Site::new("b", "h", "u", Protocol::Ftp);
        backend.save_site(a.clone()).await.unwrap();
        backend.save_site(b.clone()).await.unwrap();
        let kept = SavedTab {
            kind: TabKind::Files,
            remote_dir: Some("/b".into()),
            local_dir: None,
        };
        let mut workspace = Workspace {
            id: WorkspaceId::new(),
            name: "w".into(),
            sites: vec![
                WorkspaceSite {
                    site: a.id.clone(),
                    tabs: Vec::new(),
                },
                WorkspaceSite {
                    site: b.id.clone(),
                    tabs: vec![kept.clone()],
                },
            ],
        };
        backend.save_workspace(workspace.clone()).await.unwrap();
        // Renamed in a form after the app read it.
        workspace.name = "renamed".into();
        backend.save_workspace(workspace.clone()).await.unwrap();

        let saved = SavedTab {
            kind: TabKind::Files,
            remote_dir: Some("/var/www".into()),
            local_dir: Some(PathBuf::from("/home/me")),
        };
        backend
            .save_workspace_tabs(
                workspace.id.clone(),
                vec![(a.id.clone(), vec![saved.clone()])],
            )
            .await
            .unwrap();
        let stored = &backend.workspaces().await.unwrap()[0];
        assert_eq!(stored.name, "renamed");
        assert_eq!(stored.sites[0].tabs, vec![saved.clone()]);
        assert_eq!(stored.sites[1].tabs, vec![kept]);
        // On disk too (a fresh vault reads it back).
        drop(backend);
        let reopened = open(dir.path());
        assert_eq!(
            reopened.workspaces().await.unwrap()[0].sites[0].tabs,
            vec![saved.clone()]
        );

        let err = reopened
            .save_workspace_tabs(WorkspaceId::new(), vec![(a.id, vec![saved])])
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn unreadable_queue_falls_back_to_memory() {
        let dir = tempfile::tempdir().unwrap();
        let queue = dir.path().join("queue.db");
        std::fs::write(&queue, b"not a database").unwrap();
        let config = BackendConfig {
            profile_dir: dir.path().join("profiles"),
            queue_path: Some(queue.clone()),
            sessions: SessionConfig::default(),
            backup_params: CHEAP_BACKUP,
            edit_dir: dir.path().join("edit"),
            recovered_dir: dir.path().join("recovered"),
            agent_dir: None,
        };
        let backend = Backend::open(config, Arc::new(MemoryStore::new())).unwrap();
        assert!(backend.engine.snapshot().await.is_empty());
        // The broken file is left as it was.
        assert_eq!(std::fs::read(&queue).unwrap(), b"not a database");
    }

    /// A real import of hoprs-import's FileZilla fixture into a temporary profile, with
    /// folders as workspaces; then the delete of the source file.
    #[tokio::test]
    async fn filezilla_import_into_a_temp_profile() {
        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let options = ImportOptions {
            folders_as_workspaces: true,
        };

        // A file that is not there: an error, nothing written.
        let err = backend
            .import_filezilla(dir.path().join("missing.xml"), options)
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        let err = backend
            .import_filezilla(fixture("sitemanager-broken.xml"), options)
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
        assert!(backend.sites().await.unwrap().is_empty());

        // A copy, so the delete below has something of its own to remove.
        let source = dir.path().join("sitemanager.xml");
        std::fs::copy(fixture("sitemanager-3.40-folders.xml"), &source).unwrap();
        let report = backend
            .import_filezilla(source.clone(), options)
            .await
            .unwrap();
        let names: Vec<_> = report.imported.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "Beta", "Gamma", "Top level"]);
        assert_eq!(
            report.workspaces_created,
            [
                "Work",
                "Work/Clientes de Produção",
                "Work/Clientes de Produção/Deep"
            ]
        );
        assert!(report.failures.is_empty());
        let profiles = backend.profiles().await.unwrap();
        assert_eq!(profiles.sites.len(), 4);
        assert_eq!(profiles.workspaces.len(), 3);
        // The password went to the keyring, not to sites.toml.
        let alpha = profiles.sites.iter().find(|s| s.name == "Alpha").unwrap();
        assert!(alpha.has_password);
        let toml = std::fs::read_to_string(dir.path().join("profiles/sites.toml")).unwrap();
        assert!(!toml.contains("hunter2"));

        // Again: everything is already there.
        let again = backend
            .import_filezilla(source.clone(), options)
            .await
            .unwrap();
        assert!(again.imported.is_empty());
        assert_eq!(again.skipped.len(), 4);
        assert_eq!(backend.sites().await.unwrap().len(), 4);

        // Deleting the source: refused for a file that is not FileZilla's.
        let other = dir.path().join("notes.xml");
        std::fs::write(&other, "<?xml version=\"1.0\"?><notes/>").unwrap();
        let err = backend.delete_sitemanager(other.clone()).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
        assert!(other.exists());
        backend.delete_sitemanager(source.clone()).await.unwrap();
        assert!(!source.exists());
    }

    /// One site exported alone restores as one site with Merge, and never with Replace.
    #[tokio::test]
    async fn export_of_one_site_restores_as_one_site() {
        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let a = Site::new("a", "ha", "u", Protocol::Ssh);
        let b = Site::new("b", "hb", "u", Protocol::Ftp);
        backend.save_site(a.clone()).await.unwrap();
        backend.save_site(b).await.unwrap();
        let path = dir.path().join("one.hoprs");
        let report = backend
            .export_backup(
                ExportScope::Site(a.id),
                path.clone(),
                SecretString::from("pw"),
            )
            .await
            .unwrap();
        assert_eq!((report.sites, report.workspaces), (1, 0));

        let other_dir = tempfile::tempdir().unwrap();
        let other = open(other_dir.path());
        let restored = other
            .restore_backup(path.clone(), SecretString::from("pw"), ImportMode::Merge)
            .await
            .unwrap();
        assert_eq!(restored.added_sites, 1);
        assert_eq!(other.sites().await.unwrap().len(), 1);

        let err = other
            .restore_backup(path, SecretString::from("pw"), ImportMode::Replace)
            .await
            .unwrap_err();
        assert_eq!(err.code, Some("backup-partial-no-replace"));
        assert_eq!(other.sites().await.unwrap().len(), 1);
    }

    /// A site sealed in memory, then saved: the file restores with Merge elsewhere, and
    /// its header says partial (version 2).
    #[tokio::test]
    async fn seal_then_save_restores() {
        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let a = Site::new("a", "ha", "u", Protocol::Ssh);
        backend.save_site(a.clone()).await.unwrap();
        backend
            .save_site(Site::new("b", "hb", "u", Protocol::Ftp))
            .await
            .unwrap();
        let (report, sealed) = backend
            .seal_export(ExportScope::Site(a.id.clone()), SecretString::from("pw"))
            .await
            .unwrap();
        assert_eq!((report.sites, report.workspaces), (1, 0));
        let path = dir.path().join("out").join("a.hoprs");
        backend
            .save_sealed(path.clone(), sealed.clone())
            .await
            .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes, sealed.bytes());
        assert_eq!(&bytes[..8], b"HOPRSVLT");
        assert_eq!(&bytes[8..10], &2u16.to_be_bytes(), "version 2: partial");
        assert_eq!(
            format!("{sealed:?}"),
            format!("Sealed({} bytes)", bytes.len())
        );

        let other_dir = tempfile::tempdir().unwrap();
        let other = open(other_dir.path());
        let restored = other
            .restore_backup(path, SecretString::from("pw"), ImportMode::Merge)
            .await
            .unwrap();
        assert_eq!(restored.added_sites, 1);
        assert_eq!(other.sites().await.unwrap()[0].id, a.id);
        // An empty password seals nothing.
        let err = backend
            .seal_export(ExportScope::All, SecretString::from(""))
            .await
            .unwrap_err();
        assert_eq!(err.code, Some("backup-password-empty"));
    }

    /// A connect worker that panics still ends its stream with one `Done`.
    #[tokio::test]
    async fn a_panicking_connect_worker_ends_with_done() {
        use futures_util::StreamExt;
        let events: Vec<ConnectEvent> = blocking_stream(
            |send: &mut dyn FnMut(ConnectEvent)| {
                send(ConnectEvent::Waiting { url: "u".into() });
                panic!("the worker fell over");
            },
            |err| ConnectEvent::Done(Err(err)),
        )
        .collect()
        .await;
        assert!(
            matches!(
                events.as_slice(),
                [ConnectEvent::Waiting { .. }, ConnectEvent::Done(Err(err))]
                    if err.code == Some("app-background-task-failed")
            ),
            "{events:?}"
        );
    }

    /// Without an OAuth client in the build, the Drive calls answer at once, without a
    /// keyring call (a debug build may have one in its environment: then nothing to check).
    #[tokio::test]
    async fn drive_calls_without_a_client() {
        use futures_util::StreamExt;
        if crate::drive::client_configured() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let code = Some("drive-not-configured");
        assert_eq!(backend.drive_status().await.unwrap_err().code, code);
        assert_eq!(backend.drive_disconnect().await.unwrap_err().code, code);
        let pages = crate::drive::Pages {
            connected: "c".into(),
            failed: "f".into(),
        };
        let events: Vec<_> = backend.drive_connect(pages).collect().await;
        assert!(
            matches!(events.as_slice(), [crate::drive::ConnectEvent::Done(Err(err))] if err.code == code),
            "{events:?}"
        );
        let err = backend
            .drive_upload("n".into(), crate::drive::Sealed::new(vec![1]))
            .await
            .unwrap_err();
        assert_eq!(err.code, code);
        backend.drive_cancel();
    }

    /// Every export outcome reaches the log (INFO done, WARN failed, with the scope kind
    /// and the path), even when nobody is left on the export screen to see it; the
    /// password, the secrets and the site's name never do.
    #[test]
    fn export_outcomes_are_logged() {
        let _lock = crate::subscriber_test_lock();
        let (buffer, capture) = crate::capture_log(&crate::log_filter(Some("info")));
        let dir = tempfile::tempdir().unwrap();
        capture.runtime().block_on(async {
            let backend = open(dir.path());
            let site = Site::new("Site-Name-Qv31", "h", "u", Protocol::Ssh);
            backend.save_site(site.clone()).await.unwrap();
            backend
                .set_password(site.id.clone(), Some(SecretString::from("secret-Kp83")))
                .await
                .unwrap();
            let path = dir.path().join("out.hoprs");
            backend
                .export_backup(
                    ExportScope::Site(site.id.clone()),
                    path.clone(),
                    SecretString::from("password-Zt57"),
                )
                .await
                .unwrap();
            backend
                .export_backup(ExportScope::All, path.clone(), SecretString::from(""))
                .await
                .unwrap_err();
            let (_, sealed) = backend
                .seal_export(ExportScope::All, SecretString::from("password-Zt57"))
                .await
                .unwrap();
            backend
                .seal_export(ExportScope::All, SecretString::from(""))
                .await
                .unwrap_err();
            let saved = dir.path().join("sealed.hoprs");
            backend
                .save_sealed(saved.clone(), sealed.clone())
                .await
                .unwrap();
            // A folder where the file should go: the write fails.
            let blocked = dir.path().join("blocked");
            std::fs::create_dir(&blocked).unwrap();
            backend.save_sealed(blocked, sealed).await.unwrap_err();
            crate::log_canaries().await;
        });
        drop(capture);
        let text = buffer.text();
        let path = dir.path().join("out.hoprs").display().to_string();
        for line in [
            "exported to a file",
            "export to a file failed",
            "export encrypted in memory",
            "export failed before it was encrypted",
            "encrypted export saved to a file",
            "saving the encrypted export failed",
            "scope=\"site\"",
            "scope=\"all\"",
            path.as_str(),
        ] {
            assert!(text.contains(line), "{line} missing: {text}");
        }
        crate::assert_no_secrets(&buffer, &["password-Zt57", "secret-Kp83", "Site-Name-Qv31"]);
    }

    /// Export, then restore (merge and replace), through the vault on the blocking pool.
    #[tokio::test]
    async fn backup_export_and_restore() {
        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let site = Site::new("prod", "h", "u", Protocol::Ssh);
        backend.save_site(site.clone()).await.unwrap();
        backend
            .set_password(site.id.clone(), Some(SecretString::from("s3cret")))
            .await
            .unwrap();
        let path = dir.path().join("out").join("hoprs.backup");

        let err = backend
            .export_backup(ExportScope::All, path.clone(), SecretString::from(""))
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
        assert!(!path.exists());
        let report = backend
            .export_backup(ExportScope::All, path.clone(), SecretString::from("pw"))
            .await
            .unwrap();
        assert_eq!((report.sites, report.workspaces, report.secrets), (1, 0, 1));
        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.windows(6).any(|w| w == b"s3cret"));

        // A wrong password changes nothing.
        backend.delete_site(site.id.clone()).await.unwrap();
        assert!(
            backend
                .restore_backup(path.clone(), SecretString::from("nope"), ImportMode::Merge)
                .await
                .is_err()
        );
        assert!(backend.sites().await.unwrap().is_empty());

        let restored = backend
            .restore_backup(path.clone(), SecretString::from("pw"), ImportMode::Merge)
            .await
            .unwrap();
        assert_eq!((restored.added_sites, restored.secrets_restored), (1, 1));
        assert!(restored.secret_errors.is_empty());
        let sites = backend.sites().await.unwrap();
        assert_eq!(sites.len(), 1);
        assert!(sites[0].has_password);

        // Replace: a site made since is gone.
        backend
            .save_site(Site::new("later", "h2", "u", Protocol::Ftp))
            .await
            .unwrap();
        backend
            .restore_backup(path, SecretString::from("pw"), ImportMode::Replace)
            .await
            .unwrap();
        let names: Vec<_> = backend
            .sites()
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, ["prod"]);
    }

    /// `settings.toml` is read at open (workers for the sessions, the policy for the
    /// engine), saved on request, and an unusable file is reported and left alone.
    #[tokio::test]
    async fn settings_apply_at_open_and_change() {
        use hoprs_remote::MemFs;
        use hoprs_transfer::{Direction, EventKind, SiteEndpoints, TransferState};

        let dir = tempfile::tempdir().unwrap();
        let profiles = dir.path().join("profiles");
        std::fs::create_dir_all(&profiles).unwrap();
        let file = profiles.join(settings::FILE_NAME);
        std::fs::write(
            &file,
            "workers_per_site = 3\ndefault_policy = \"skip\"\nkeepalive_secs = 60\n",
        )
        .unwrap();
        let backend = open(dir.path());
        let loaded = backend.settings();
        assert_eq!(loaded.error, None);
        assert_eq!(
            loaded.settings,
            Settings {
                workers_per_site: 3,
                default_policy: ConflictPolicy::Skip,
                keepalive_secs: 60,
                ..Settings::default()
            }
        );
        assert_eq!(backend.transfer_snapshot().await.workers, 3);
        assert_eq!(backend.sessions.workers(), 3);
        assert_eq!(
            backend.sessions.keepalive(),
            std::time::Duration::from_secs(60)
        );

        // The saved policy is the engine's from the start: an existing file is skipped,
        // never asked about.
        let site = SiteId::new();
        let local = MemFs::new();
        let remote = MemFs::new();
        let now = std::time::SystemTime::now();
        local.add_file("/l/a.txt", b"local", now);
        remote.add_file("/r/a.txt", b"remote", now);
        backend
            .engine
            .register_site(
                site.clone(),
                SiteEndpoints {
                    local: Arc::new(local),
                    remote: Arc::new(remote),
                    workers: 1,
                },
            )
            .await;
        let mut events = backend.transfer_events();
        let id = backend
            .enqueue(TransferSpec {
                site,
                direction: Direction::Download,
                local: RemotePath::new("/l/a.txt").unwrap(),
                remote: RemotePath::new("/r/a.txt").unwrap(),
            })
            .await
            .unwrap();
        let state = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let EventKind::StateChanged { id: changed, state } =
                    events.recv().await.unwrap().kind
                    && changed == id
                    && !matches!(state, TransferState::Queued | TransferState::Active)
                {
                    return state;
                }
            }
        })
        .await
        .expect("the transfer settles");
        assert!(matches!(state, TransferState::Skipped(_)), "{state:?}");

        // Workers change for the next connections, clamped.
        backend.set_workers(40);
        assert_eq!(backend.sessions.workers(), 16);
        // The keepalive: clamped to 10..=300 s.
        backend.set_keepalive(2);
        assert_eq!(
            backend.sessions.keepalive(),
            std::time::Duration::from_secs(10)
        );
        backend.set_keepalive(45);
        assert_eq!(
            backend.sessions.keepalive(),
            std::time::Duration::from_secs(45)
        );
        backend.set_workers(0);
        assert_eq!(backend.transfer_snapshot().await.workers, 1);

        let saved = Settings {
            workers_per_site: 9,
            default_policy: ConflictPolicy::Resume,
            ..Settings::default()
        };
        backend.save_settings(saved.clone()).await.unwrap();
        assert_eq!(settings::load(&file).settings, saved);
        drop(backend);

        // An unusable file: the defaults, the error, and the file as it was.
        std::fs::write(&file, "workers_per_site = [").unwrap();
        let backend = open(dir.path());
        let loaded = backend.settings();
        assert_eq!(loaded.settings, Settings::default());
        assert!(loaded.error.is_some());
        assert_eq!(backend.sessions.workers(), 4);
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "workers_per_site = ["
        );
    }

    /// Plan 6 Task 7, end to end against the Docker servers (sshd 127.0.0.1:2222 alice /
    /// alice-pass; vsftpd 127.0.0.1:2121 frank / frank-pass): the real backend, the real
    /// file watcher and a real editor process (`true`). A save goes up (SFTP by a
    /// temporary file renamed over the original, keeping its permissions; FTP in place), a
    /// change made by someone else holds the next save until "Overwrite", and finishing
    /// removes the local copy.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
    async fn edit_roundtrip_sftp_and_ftp() {
        use crate::backend::broker::PromptKind;
        use tokio::io::AsyncWriteExt;

        async fn put(fs: &dyn RemoteFs, path: &RemotePath, data: &[u8]) {
            let mut writer = fs.open_write(path, 0).await.unwrap();
            writer.write_all(data).await.unwrap();
            writer.shutdown().await.unwrap();
        }

        async fn get(fs: &dyn RemoteFs, path: &RemotePath) -> Vec<u8> {
            use tokio::io::AsyncReadExt;
            let mut data = Vec::new();
            let mut reader = fs.open_read(path, 0).await.unwrap();
            reader.read_to_end(&mut data).await.unwrap();
            data
        }

        async fn next(events: &mut broadcast::Receiver<EditEvent>) -> EditEvent {
            tokio::time::timeout(std::time::Duration::from_secs(20), events.recv())
                .await
                .expect("an edit event in time")
                .unwrap()
        }

        let dir = tempfile::tempdir().unwrap();
        let config = BackendConfig {
            profile_dir: dir.path().join("profiles"),
            queue_path: None,
            sessions: SessionConfig {
                known_hosts: Some(dir.path().join("known_hosts")),
                ..SessionConfig::default()
            },
            backup_params: CHEAP_BACKUP,
            edit_dir: dir.path().join("edit"),
            recovered_dir: dir.path().join("recovered"),
            agent_dir: None,
        };
        let backend = Arc::new(Backend::open(config, Arc::new(MemoryStore::new())).unwrap());
        let mut prompts = backend.prompt_events().unwrap();
        let answering = tokio::spawn({
            let backend = backend.clone();
            async move {
                while let Some(event) = prompts.recv().await {
                    if let BrokerEvent::Request(r) = event {
                        let answer = match r.kind {
                            PromptKind::HostKey { .. } | PromptKind::Certificate { .. } => {
                                PromptAnswer::Decision(true)
                            }
                            _ => PromptAnswer::Cancel,
                        };
                        backend.answer_prompt(r.id, answer).await;
                    }
                }
            }
        });

        let mut ssh = Site::new("docker ssh", "127.0.0.1", "alice", Protocol::Ssh);
        ssh.port = hoprs_ssh::testing::SSH_PORT;
        ssh.ssh.use_agent = false;
        let mut ftp = Site::new("docker ftp", "127.0.0.1", "frank", Protocol::Ftp);
        ftp.port = hoprs_remote::testing::FTP_PORT;
        let unique = std::process::id();
        for (site, password, home) in [
            (ssh, "alice-pass", "/home/alice"),
            (ftp, "frank-pass", "/home/frank"),
        ] {
            let ssh = site.protocol == Protocol::Ssh;
            backend.save_site(site.clone()).await.unwrap();
            backend
                .set_password(site.id.clone(), Some(password.to_owned().into()))
                .await
                .unwrap();
            let session = backend.connect(site.id.clone()).await.unwrap().session;
            let fs = backend.fs(Some(session), Side::Remote).unwrap();
            let home = RemotePath::new(home).unwrap();
            let path = home.join(&format!("hoprs-edit-{unique}.txt"));
            put(fs.as_ref(), &path, b"version 1\n").await;
            if ssh {
                fs.chmod(&path, 0o640).await.unwrap();
            }
            let mut events = backend.subscribe_edits();

            let id = backend
                .start_edit(session, path.clone(), "true".into())
                .await
                .unwrap();
            let local = backend.edits.local_path(id).unwrap();
            assert_eq!(std::fs::read(&local).unwrap(), b"version 1\n");

            // A save goes up.
            std::fs::write(&local, b"edited locally\n").unwrap();
            let event = next(&mut events).await;
            assert!(
                matches!(event, EditEvent::Uploaded { id: got, .. } if got == id),
                "{event:?}"
            );
            assert_eq!(get(fs.as_ref(), &path).await, b"edited locally\n");
            let names: Vec<String> = fs
                .list(&home)
                .await
                .unwrap()
                .into_iter()
                .map(|e| e.name)
                .collect();
            assert!(
                !names.iter().any(|n| n.contains(".hoprs-upload")),
                "{names:?}"
            );
            if ssh {
                assert_eq!(fs.stat(&path).await.unwrap().mode.unwrap() & 0o777, 0o640);
            }

            // Someone else changes it: the next save is held.
            put(fs.as_ref(), &path, b"changed by someone else\n").await;
            std::fs::write(&local, b"mine\n").unwrap();
            assert_eq!(next(&mut events).await, EditEvent::RemoteChanged { id });
            assert_eq!(get(fs.as_ref(), &path).await, b"changed by someone else\n");

            backend
                .resolve_edit(id, EditChoice::Overwrite)
                .await
                .unwrap();
            assert!(matches!(
                next(&mut events).await,
                EditEvent::Uploaded { .. }
            ));
            assert_eq!(get(fs.as_ref(), &path).await, b"mine\n");

            assert!(
                backend
                    .finish_edits(session, false)
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(next(&mut events).await, EditEvent::Closed { id });
            assert!(!local.exists());
            fs.remove_file(&path).await.unwrap();
            backend.close(session).await;
        }
        answering.abort();
    }
}
