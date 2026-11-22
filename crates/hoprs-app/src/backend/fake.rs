//! An in-memory [`BackendApi`] for the UI tests of Tasks 5–11 (ruling R3: test-only).
//!
//! Sites and workspaces live in memory. Every site has its own [`MemFs`] as its remote
//! side, and one shared `MemFs` stands for the local disk. Transfers, controls, answers and
//! closes are recorded, and dialogs are pushed by the test with
//! [`FakeBackend::push_prompt`].
//!
//! Connects follow the real [`SessionManager`](super::SessionManager) rules:
//! - one session per site: connecting a site with an open session returns that session;
//! - each connect attempt takes the next scripted outcome of the site (default success);
//! - a failed first connect keeps the session (its id comes with the error); connecting
//!   the site again, or `reconnect`, retries it with the next outcome, while a connected
//!   session is returned as is;
//! - with the keyring "locked" ([`FakeBackend::lock_keyring`]), sessions of sites with a
//!   stored secret report `keyring_unavailable`;
//! - secrets are read like the real `SecretBatch`: every site with a stored secret costs
//!   one keyring read ([`FakeBackend::keyring_reads`]), except that within one
//!   `connect_batch` a locked keyring is read once and the remaining sites are marked
//!   without asking it again;
//! - `connect_batch` runs the sites concurrently and sends each result as it finishes; a
//!   site's connect can be held back with [`FakeBackend::hold_connect`].
//!
//! Dialogs: [`FakeBackend::push_prompt`] remembers each request, so an answer to a
//! password or passphrase prompt with `save: true` is held for its session like the real
//! broker does, and stored (as `set_password` would) only when that session's connect
//! succeeds (the fake assumes the typed secret is what logged in: the "accepted only"
//! rule is tested against the real broker and session manager); a failed connect or a
//! close drops it. `known_hosts` is a scripted list
//! ([`FakeBackend::set_known_host_entries`]); removals are recorded.
//!
//! Each session has a connection state watch ([`BackendApi::session_state`]): `Connecting`
//! when started, then what each attempt gives; [`FakeBackend::drop_connection`] makes it
//! `Disconnected` and a close drops it (ending the watch). [`FakeBackend::calls`] records
//! tab saves, reconnects, shutdown and closes in order; a shutdown closes every session,
//! or hangs forever after [`FakeBackend::hang_shutdown`] (panics after
//! [`FakeBackend::panic_shutdown`]); [`FakeBackend::hang_save_tabs`] makes tab saves hang.
//!
//! `save_site` keeps the backend-owned fields of a stored site (`has_*`, and the FTPS pin
//! by the vault's [`merged_pin`](super::vault::merged_pin) rule), and
//! `set_password` records the password and sets `has_password`, like the vault.
//!
//! Shells are scripted: input echoes back as output, [`FakeBackend::push_shell_output`]
//! adds output, [`FakeBackend::end_shell`] ends a shell from the server's side, and
//! [`FakeBackend::fail_next_open_shell`] refuses the next open. They follow the real
//! lifecycle (see [`super::shells`]): opening needs a connected session; closing the session
//! closes its shells; a dropped connection ends them and a reconnect does not reopen them.
//! Resizes are kept per shell ([`FakeBackend::shell_sizes`]); opens and shell closes are
//! recorded in [`FakeBackend::calls`].
//!
//! Remote edits run on the real [`EditManager`](super::EditManager) over the site's `MemFs`
//! and the session's state watch, in a temporary directory, without a file watcher and
//! without an editor: [`FakeBackend::edits_launched`] records what would have opened, and
//! [`FakeBackend::save_edit`] stands for the editor's save (the debounce then runs as in the
//! real backend). Starting an edit needs a connected session.
//!
//! The FileZilla import, the delete of its file, the backup export and restore and the
//! settings calls are recorded in order ([`FakeBackend::backup_calls`]) and succeed with
//! empty reports.
//!
//! Google Drive is scripted: `drive_status` answers [`FakeBackend::with_drive`] (or the
//! error of [`FakeBackend::fail_drive_status`]), `drive_connect` streams the events of
//! [`FakeBackend::set_drive_connect`] (once; nothing when none are set), `seal_export`
//! answers `b"sealed"` with the scope's report (unless [`FakeBackend::fail_next_seal`]),
//! uploads succeed unless [`FakeBackend::fail_next_upload`] says otherwise; statuses,
//! uploads, cancels, disconnects and sealed saves are counted or recorded. The fake
//! follows its own calls as the real backend does: a connect that ends connected makes
//! `drive_status` answer that account, `drive_disconnect` forgets it, and an upload with
//! no account fails with `drive-not-connected`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream::{BoxStream, FuturesUnordered};
use hoprs_core::{
    ConnectionState, Error, ErrorKind, RemoteEntry, RemotePath, Result, Site, SiteId, Workspace,
    WorkspaceId,
};
use hoprs_import::{ImportOptions, ImportReport};
use hoprs_remote::{CompareOptions, CompareSignals, MemFs, Progress, RemoteFs, Summary};
use hoprs_transfer::{ConflictPolicy, Event, TransferId, TransferItem, TransferSpec};
use hoprs_vault::{BackupImportReport, ExportReport, ExportScope, ImportMode, UnreadableEntry};
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use super::broker::{PromptKind, PromptRequest};
use super::edit::{EditManager, EditSession, Launcher, UploadMode};
use super::shells::{ShellControl, Shells, shell_closed};
use super::{
    AuthMethod, BackendApi, BrokerEvent, ConnectError, ConnectResult, Connected, ControlOp,
    DeleteTarget, EditChoice, EditEvent, EditId, EditSummary, KeptEdit, KnownHostEntry,
    ProfileSnapshot, PromptAnswer, PromptId, SessionId, ShellId, ShellOutput, Side,
    TransferSnapshot, WorkspaceTabs,
};
use crate::drive::{ConnectEvent, DriveAccount, DriveFile, Pages, Sealed};
use crate::settings::{Settings, SettingsLoad};
use crate::update::Release;
use crate::update::download::{DownloadEvent, DownloadJob};
use hoprs_vault::SecretKind;

/// An import, backup or settings call, in [`FakeBackend::backup_calls`]. The test
/// fake keeps passwords in clear, to check what reached it.
#[derive(Clone, Debug, PartialEq)]
pub enum BackupCall {
    Import {
        path: PathBuf,
        folders_as_workspaces: bool,
    },
    DeleteSitemanager(PathBuf),
    Export {
        scope: ExportScope,
        path: PathBuf,
        password: String,
    },
    Restore {
        path: PathBuf,
        password: String,
        mode: ImportMode,
    },
    /// `seal_export`.
    Seal {
        scope: ExportScope,
        password: String,
    },
    SaveSettings(Settings),
    SetPolicy(ConflictPolicy),
    SetWorkers(usize),
    SetKeepalive(u64),
}

/// A recorded backend call, in [`FakeBackend::calls`].
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    SaveTabs(WorkspaceId),
    Reconnect(SessionId),
    Shutdown,
    Close(SessionId),
    OpenShell(SessionId),
    CloseShell(ShellId),
    FlushEdits(SessionId),
    KeepEdits(SessionId),
}

/// A scripted shell (see the module docs).
struct FakeShell {
    session: SessionId,
    output: mpsc::UnboundedSender<ShellOutput>,
    ended: AtomicBool,
    /// The opening size, then each resize.
    sizes: Mutex<Vec<(u16, u16)>>,
}

impl FakeShell {
    /// Ends the shell: `Closed`, once.
    fn end(&self) {
        if !self.ended.swap(true, Ordering::SeqCst) {
            let _ = self.output.send(ShellOutput::Closed);
        }
    }

    fn check_open(&self) -> Result<()> {
        if self.ended.load(Ordering::SeqCst) {
            Err(shell_closed())
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl ShellControl for FakeShell {
    async fn input(&self, data: Vec<u8>) -> Result<()> {
        self.check_open()?;
        let _ = self.output.send(ShellOutput::Data(data));
        Ok(())
    }

    async fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.check_open()?;
        self.sizes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((cols, rows));
        Ok(())
    }

    fn close(&self) {
        self.end();
    }
}

/// The output of a fake shell: everything sent, ending after `Closed`.
fn fake_output(rx: mpsc::UnboundedReceiver<ShellOutput>) -> BoxStream<'static, ShellOutput> {
    futures_util::stream::unfold(Some(rx), |rx| async move {
        let mut rx = rx?;
        let item = rx.recv().await?;
        let rest = (item != ShellOutput::Closed).then_some(rx);
        Some((item, rest))
    })
    .boxed()
}

/// See the module docs.
pub struct FakeBackend {
    /// Shared with the streams that change it (a connect that ends connected).
    state: Arc<Mutex<State>>,
    next_session: AtomicU64,
    next_transfer: AtomicI64,
    events: mpsc::UnboundedSender<BrokerEvent>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<BrokerEvent>>>,
    /// Engine events the test sends with [`FakeBackend::send_transfer_event`].
    transfer_events: broadcast::Sender<Event>,
    shells: Shells,
    /// The edits and their directory, made on first use.
    edits: OnceLock<(tempfile::TempDir, EditManager)>,
    /// `(editor command, local path)` of every editor the edits opened.
    launched: Arc<Mutex<Vec<(String, PathBuf)>>>,
}

#[derive(Default)]
struct State {
    sites: Vec<Site>,
    workspaces: Vec<Workspace>,
    unreadable: Vec<UnreadableEntry>,
    /// Stored passwords, by site.
    passwords: HashMap<SiteId, SecretString>,
    /// Keyring reads made to fetch connect secrets.
    keyring_reads: u32,
    /// Connects of these sites wait until the sender is used or dropped.
    holds: HashMap<SiteId, oneshot::Receiver<()>>,
    /// Scripted outcomes of the next connects of a site; empty means success.
    connects: HashMap<SiteId, VecDeque<std::result::Result<(), Error>>>,
    sessions: HashMap<SessionId, SiteId>,
    /// Sessions whose connection is up (the others failed and wait for a retry).
    connected: HashSet<SessionId>,
    /// Sessions started while the keyring was locked.
    keyring_unavailable: HashSet<SessionId>,
    keyring_locked: bool,
    remotes: HashMap<SiteId, MemFs>,
    local: MemFs,
    enqueued: Vec<(TransferSpec, Option<ConflictPolicy>)>,
    controls: Vec<(TransferId, ControlOp)>,
    /// What `transfer_snapshot` returns, as the test set it.
    queue: (u64, Vec<TransferItem>),
    clears: u32,
    answers: Vec<(PromptId, PromptAnswer)>,
    closed: Vec<SessionId>,
    /// The requests pushed to the UI, by id.
    prompts: HashMap<PromptId, PromptRequest>,
    /// Typed secrets answered with "save", waiting for their session to connect.
    pending_secrets: HashMap<SessionId, Vec<(SecretKind, SecretString)>>,
    /// Stored key passphrases, by site.
    passphrases: HashMap<SiteId, SecretString>,
    /// What `known_host_entries` returns.
    known_hosts: Vec<KnownHostEntry>,
    known_host_removals: Vec<(String, u16)>,
    /// Each open session's connection state.
    watches: HashMap<SessionId, watch::Sender<ConnectionState>>,
    /// How each session logged in, as the test set it.
    auth: HashMap<SessionId, AuthMethod>,
    calls: Vec<Call>,
    /// The next `save_workspace_tabs` calls fail with these.
    save_tab_failures: VecDeque<Error>,
    hang_shutdown: bool,
    hang_save_tabs: bool,
    panic_shutdown: bool,
    backup_calls: Vec<BackupCall>,
    /// The next `export_backup` calls fail with these.
    export_failures: VecDeque<Error>,
    /// The next `seal_export` calls fail with these.
    seal_failures: VecDeque<Error>,
    /// What `settings` returns.
    settings: SettingsLoad,
    /// What `queue_error` returns.
    queue_error: Option<Error>,
    /// The scripted shells, by id (also in `FakeBackend::shells` while open).
    fake_shells: HashMap<ShellId, Arc<FakeShell>>,
    /// The next `open_shell` calls fail with these.
    open_shell_failures: VecDeque<Error>,
    /// The scripted outcomes of the next `compare` calls (none: a real walk over the
    /// `MemFs`es).
    compare_scripts: VecDeque<CompareScript>,
    /// Every `compare` call: session, local root, remote root.
    compare_calls: Vec<(SessionId, RemotePath, RemotePath)>,
    /// What `latest_release` returns; `None` is a failed check.
    release: Option<Result<Release>>,
    releases_asked: u32,
    /// What the next `download_update` streams.
    download: Vec<DownloadEvent>,
    downloads: Vec<DownloadJob>,
    /// What `drive_status` answers (unless `drive_status_error`).
    drive_account: Option<DriveAccount>,
    drive_status_error: Option<Error>,
    drive_status_reads: u32,
    /// What the next `drive_connect` streams.
    drive_connect: Vec<ConnectEvent>,
    /// The next `drive_upload` calls fail with these.
    upload_failures: VecDeque<Error>,
    /// `(name, byte count)` of every `drive_upload`.
    drive_uploads: Vec<(String, usize)>,
    drive_cancels: u32,
    drive_disconnects: u32,
    sealed_saves: Vec<PathBuf>,
}

/// What a scripted `compare` does: sends `progress`, optionally waits on `hold`
/// (until its sender is used or dropped, or a stop), then returns `result`.
struct CompareScript {
    progress: Vec<Progress>,
    hold: Option<watch::Receiver<()>>,
    result: Result<Summary>,
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeBackend {
    pub fn new() -> Self {
        let (events, receiver) = mpsc::unbounded_channel();
        Self {
            state: Arc::new(Mutex::new(State::default())),
            next_session: AtomicU64::new(1),
            next_transfer: AtomicI64::new(1),
            events,
            receiver: Mutex::new(Some(receiver)),
            transfer_events: broadcast::channel(16).0,
            shells: Shells::default(),
            edits: OnceLock::new(),
            launched: Arc::default(),
        }
    }

    /// The edits (see the module docs).
    fn edits(&self) -> &EditManager {
        let (_, edits) = self.edits.get_or_init(|| {
            let dir = tempfile::tempdir().expect("a temporary edit directory");
            let launched = self.launched.clone();
            let launcher: Launcher = Arc::new(move |command: &str, path: &std::path::Path| {
                launched
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push((command.to_owned(), path.to_owned()));
                Ok(())
            });
            let edits = EditManager::with_options(
                dir.path().join("edit"),
                dir.path().join("recovered"),
                false,
                launcher,
            );
            (dir, edits)
        });
        edits
    }

    /// `(editor command, local path)` of every editor the edits opened, in order.
    pub fn edits_launched(&self) -> Vec<(String, PathBuf)> {
        self.launched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The local copy of a live edit.
    pub fn edit_local(&self, id: EditId) -> Option<PathBuf> {
        self.edits().local_path(id)
    }

    /// The editor saves `data` into the edit's local file (and the watcher reports it).
    pub fn save_edit(&self, session: SessionId, id: EditId, data: &[u8]) {
        let path = self.edits().local_path(id).expect("a live edit");
        std::fs::write(&path, data).expect("the local copy is writable");
        self.edits().file_event(session, path);
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The import, backup and settings calls, in order.
    pub fn backup_calls(&self) -> Vec<BackupCall> {
        self.state().backup_calls.clone()
    }

    fn record(&self, call: BackupCall) {
        self.state().backup_calls.push(call);
    }

    /// What `latest_release` returns (by default a failed check).
    pub fn with_release(self, release: Result<Release>) -> Self {
        self.state().release = Some(release);
        self
    }

    /// The events the next `download_update` streams.
    pub fn set_download(&self, events: Vec<DownloadEvent>) {
        self.state().download = events;
    }

    /// How many times `latest_release` was called.
    pub fn releases_asked(&self) -> u32 {
        self.state().releases_asked
    }

    /// Every `download_update` job, in order.
    pub fn downloads(&self) -> Vec<DownloadJob> {
        self.state().downloads.clone()
    }

    /// What `drive_status` answers (by default `None`: not connected).
    pub fn with_drive(self, account: Option<DriveAccount>) -> Self {
        self.state().drive_account = account;
        self
    }

    /// From now on `drive_status` fails with `error`.
    pub fn fail_drive_status(&self, error: Error) {
        self.state().drive_status_error = Some(error);
    }

    /// The events the next `drive_connect` streams.
    pub fn set_drive_connect(&self, events: Vec<ConnectEvent>) {
        self.state().drive_connect = events;
    }

    /// The next `drive_upload` fails with `error`.
    pub fn fail_next_upload(&self, error: Error) {
        self.state().upload_failures.push_back(error);
    }

    /// How many times `drive_status` was called.
    pub fn drive_status_reads(&self) -> u32 {
        self.state().drive_status_reads
    }

    /// `(name, byte count)` of every `drive_upload`, in order (failed ones too).
    pub fn drive_uploads(&self) -> Vec<(String, usize)> {
        self.state().drive_uploads.clone()
    }

    pub fn drive_cancels(&self) -> u32 {
        self.state().drive_cancels
    }

    pub fn drive_disconnects(&self) -> u32 {
        self.state().drive_disconnects
    }

    /// The path of every `save_sealed`, in order.
    pub fn sealed_saves(&self) -> Vec<PathBuf> {
        self.state().sealed_saves.clone()
    }

    pub fn with_site(self, site: Site) -> Self {
        self.state().sites.push(site);
        self
    }

    pub fn with_workspace(self, workspace: Workspace) -> Self {
        self.state().workspaces.push(workspace);
        self
    }

    /// The queue database "could not be opened" with `error`.
    pub fn with_queue_error(self, error: Error) -> Self {
        self.state().queue_error = Some(error);
        self
    }

    pub fn with_unreadable(self, entry: UnreadableEntry) -> Self {
        self.state().unreadable.push(entry);
        self
    }

    /// Keyring reads made so far to fetch connect secrets.
    pub fn keyring_reads(&self) -> u32 {
        self.state().keyring_reads
    }

    /// The stored password of `site`, if any.
    pub fn password(&self, site: &SiteId) -> Option<SecretString> {
        self.state().passwords.get(site).cloned()
    }

    /// The next connect of `site` waits until the returned sender is used or dropped.
    pub fn hold_connect(&self, site: &SiteId) -> oneshot::Sender<()> {
        let (release, held) = oneshot::channel();
        self.state().holds.insert(site.clone(), held);
        release
    }

    /// The next connect of `site` fails with `error` (scripts queue up in order).
    pub fn fail_next_connect(&self, site: &SiteId, error: Error) {
        self.state()
            .connects
            .entry(site.clone())
            .or_default()
            .push_back(Err(error));
    }

    /// From now on, new sessions of sites with a stored secret report the keyring as
    /// unavailable.
    pub fn lock_keyring(&self) {
        self.state().keyring_locked = true;
    }

    /// Scripts the next `compare` call: `progress` is sent, then the call returns
    /// `result`. With a hold, the call waits first until the returned sender sends (or
    /// is dropped), or until it is stopped (then it returns a stopped summary).
    pub fn script_compare(
        &self,
        progress: Vec<Progress>,
        result: Result<Summary>,
        hold: bool,
    ) -> Option<watch::Sender<()>> {
        let (tx, rx) = watch::channel(());
        self.state().compare_scripts.push_back(CompareScript {
            progress,
            hold: hold.then_some(rx),
            result,
        });
        hold.then_some(tx)
    }

    /// The `compare` calls made: session, local root, remote root.
    pub fn compare_calls(&self) -> Vec<(SessionId, RemotePath, RemotePath)> {
        self.state().compare_calls.clone()
    }

    /// The remote side of `site` (shared with the backend: fill it in the test).
    pub fn remote_fs(&self, site: &SiteId) -> MemFs {
        self.state()
            .remotes
            .entry(site.clone())
            .or_default()
            .clone()
    }

    /// The local side (shared with the backend).
    pub fn local_fs(&self) -> MemFs {
        self.state().local.clone()
    }

    /// Sends a broker event to the UI's subscription.
    pub fn push_prompt(&self, event: BrokerEvent) {
        if let BrokerEvent::Request(request) = &event {
            self.state().prompts.insert(request.id, request.clone());
        }
        let _ = self.events.send(event);
    }

    /// The stored key passphrase of `site`, if any.
    pub fn passphrase(&self, site: &SiteId) -> Option<SecretString> {
        self.state().passphrases.get(site).cloned()
    }

    /// What `known_host_entries` returns from now on.
    pub fn set_known_host_entries(&self, entries: Vec<KnownHostEntry>) {
        self.state().known_hosts = entries;
    }

    /// The `host:port` of each `remove_known_host` call.
    pub fn known_host_removals(&self) -> Vec<(String, u16)> {
        self.state().known_host_removals.clone()
    }

    /// The session connected: its typed secrets are stored, like the real session
    /// manager does.
    fn save_pending(state: &mut State, session: SessionId) {
        let Some(secrets) = state.pending_secrets.remove(&session) else {
            return;
        };
        let Some(site) = state.sessions.get(&session).cloned() else {
            return;
        };
        for (kind, secret) in secrets {
            let Some(slot) = state.sites.iter_mut().find(|s| s.id == site) else {
                return;
            };
            match kind {
                SecretKind::Password => {
                    slot.has_password = true;
                    state.passwords.insert(site.clone(), secret);
                }
                SecretKind::KeyPassphrase => {
                    slot.has_key_passphrase = true;
                    state.passphrases.insert(site.clone(), secret);
                }
            }
        }
    }

    /// One connection attempt's end settles the session's typed secrets.
    fn settle(state: &mut State, session: SessionId, connected: bool) {
        if connected {
            Self::save_pending(state, session);
        } else {
            state.pending_secrets.remove(&session);
        }
    }

    pub fn enqueued(&self) -> Vec<TransferSpec> {
        self.state()
            .enqueued
            .iter()
            .map(|(spec, _)| spec.clone())
            .collect()
    }

    /// Every enqueue with the per-item policy it carried (`None`: plain `enqueue`).
    pub fn enqueued_with_policy(&self) -> Vec<(TransferSpec, Option<ConflictPolicy>)> {
        self.state().enqueued.clone()
    }

    pub fn controls(&self) -> Vec<(TransferId, ControlOp)> {
        self.state().controls.clone()
    }

    /// Sets what the next snapshots return.
    pub fn set_queue(&self, seq: u64, items: Vec<TransferItem>) {
        self.state().queue = (seq, items);
    }

    /// Sends `event` to every `transfer_events` receiver (16 deep: more unread events
    /// make a receiver lag).
    pub fn send_transfer_event(&self, event: Event) {
        let _ = self.transfer_events.send(event);
    }

    pub fn clears(&self) -> u32 {
        self.state().clears
    }

    pub fn answers(&self) -> Vec<(PromptId, PromptAnswer)> {
        self.state().answers.clone()
    }

    pub fn closed(&self) -> Vec<SessionId> {
        self.state().closed.clone()
    }

    /// The tab saves, reconnects, shutdown and closes, in order.
    pub fn calls(&self) -> Vec<Call> {
        self.state().calls.clone()
    }

    /// The shell writes `data`, as if the server sent it.
    pub fn push_shell_output(&self, shell: ShellId, data: &[u8]) {
        if let Some(fake) = self.state().fake_shells.get(&shell) {
            let _ = fake.output.send(ShellOutput::Data(data.to_vec()));
        }
    }

    /// The shell's output yields `event` (an `Exit`, or the pump's `Overflowed`), as the
    /// real pump would before `Closed`.
    pub fn push_shell_event(&self, shell: ShellId, event: ShellOutput) {
        if let Some(fake) = self.state().fake_shells.get(&shell) {
            let _ = fake.output.send(event);
        }
    }

    /// The server ends the shell: its output yields `Closed`, input then fails.
    pub fn end_shell(&self, shell: ShellId) {
        if let Some(fake) = self.state().fake_shells.get(&shell) {
            fake.end();
        }
    }

    /// The next `open_shell` fails with `error`.
    pub fn fail_next_open_shell(&self, error: Error) {
        self.state().open_shell_failures.push_back(error);
    }

    /// The shell's opening size, then each resize.
    pub fn shell_sizes(&self, shell: ShellId) -> Vec<(u16, u16)> {
        self.state()
            .fake_shells
            .get(&shell)
            .map(|fake| {
                fake.sizes
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone()
            })
            .unwrap_or_default()
    }

    /// The session's connection is lost for good: `Disconnected` with `error`. Its shells
    /// end.
    pub fn drop_connection(&self, session: SessionId, error: Error) {
        let mut state = self.state();
        state.connected.remove(&session);
        for fake in state.fake_shells.values() {
            if fake.session == session {
                fake.end();
            }
        }
        if let Some(watch) = state.watches.get(&session) {
            watch.send_replace(ConnectionState::Disconnected(error));
        }
    }

    /// What `session_auth` returns for `session`.
    pub fn set_auth(&self, session: SessionId, auth: AuthMethod) {
        self.state().auth.insert(session, auth);
    }

    /// The next `save_workspace_tabs` fails with `error`.
    pub fn fail_next_save_tabs(&self, error: Error) {
        self.state().save_tab_failures.push_back(error);
    }

    /// The report of an export of `scope` (`NotFound` for an unknown id).
    fn export_report(state: &State, scope: &ExportScope) -> Result<ExportReport> {
        let (sites, workspaces) = match scope {
            ExportScope::All => (state.sites.len(), state.workspaces.len()),
            ExportScope::Site(id) => {
                if !state.sites.iter().any(|s| &s.id == id) {
                    return Err(Self::not_found("Unknown site"));
                }
                (1, 0)
            }
            ExportScope::Workspace(id) => {
                let workspace = state
                    .workspaces
                    .iter()
                    .find(|w| &w.id == id)
                    .ok_or_else(|| Self::not_found("Unknown workspace"))?;
                let held = workspace
                    .sites
                    .iter()
                    .filter(|entry| state.sites.iter().any(|s| s.id == entry.site))
                    .count();
                (held, 1)
            }
        };
        Ok(ExportReport {
            sites,
            workspaces,
            secrets: 0,
            skipped_unreadable: Vec::new(),
            missing_secrets: Vec::new(),
        })
    }

    /// The next `export_backup` fails with `error`.
    pub fn fail_next_export(&self, error: Error) {
        self.state().export_failures.push_back(error);
    }

    /// The next `seal_export` fails with `error`.
    pub fn fail_next_seal(&self, error: Error) {
        self.state().seal_failures.push_back(error);
    }

    /// From now on `save_workspace_tabs` never returns.
    pub fn hang_save_tabs(&self) {
        self.state().hang_save_tabs = true;
    }

    /// From now on `shutdown` panics.
    pub fn panic_shutdown(&self) {
        self.state().panic_shutdown = true;
    }

    /// From now on `shutdown` never returns.
    pub fn hang_shutdown(&self) {
        self.state().hang_shutdown = true;
    }

    /// The site a session belongs to, while it is open.
    pub fn session_site(&self, session: SessionId) -> Option<SiteId> {
        self.state().sessions.get(&session).cloned()
    }

    /// One connection attempt of `session`: a connected session stays connected;
    /// otherwise the site's next scripted outcome decides.
    fn attempt(state: &mut State, session: SessionId, site: &SiteId) -> Result<()> {
        if state.connected.contains(&session) {
            return Ok(());
        }
        let outcome = state
            .connects
            .get_mut(site)
            .and_then(VecDeque::pop_front)
            .unwrap_or(Ok(()));
        if outcome.is_ok() {
            state.connected.insert(session);
        }
        if let Some(watch) = state.watches.get(&session) {
            watch.send_replace(match &outcome {
                Ok(()) => ConnectionState::Connected,
                Err(error) => ConnectionState::Disconnected(error.clone()),
            });
        }
        outcome
    }

    async fn connect_one(&self, site: SiteId, keyring_down: &mut bool) -> ConnectResult {
        let started = self.start(&site, keyring_down)?;
        self.finish(&site, started).await
    }

    /// Reads the site's secrets (see the module docs) and finds or starts its session.
    fn start(
        &self,
        site: &SiteId,
        keyring_down: &mut bool,
    ) -> std::result::Result<(SessionId, bool), ConnectError> {
        let mut state = self.state();
        let Some(found) = state.sites.iter().find(|s| &s.id == site) else {
            return Err(ConnectError {
                session: None,
                error: Self::not_found("Unknown site"),
                keyring_unavailable: false,
            });
        };
        let has_secret = found.has_password || found.has_key_passphrase;
        let mut unavailable = false;
        if has_secret {
            if *keyring_down {
                unavailable = true;
            } else {
                state.keyring_reads += 1;
                if state.keyring_locked {
                    *keyring_down = true;
                    unavailable = true;
                }
            }
        }
        let existing = state
            .sessions
            .iter()
            .find(|(_, s)| *s == site)
            .map(|(id, _)| *id);
        let id = match existing {
            Some(id) => id,
            None => {
                let id = SessionId(self.next_session.fetch_add(1, Ordering::SeqCst));
                state.sessions.insert(id, site.clone());
                state
                    .watches
                    .insert(id, watch::channel(ConnectionState::Connecting).0);
                state.remotes.entry(site.clone()).or_default();
                if unavailable {
                    state.keyring_unavailable.insert(id);
                }
                id
            }
        };
        let keyring_unavailable = state.keyring_unavailable.contains(&id);
        Ok((id, keyring_unavailable))
    }

    /// Waits for a hold on the site, then makes the connection attempt.
    async fn finish(
        &self,
        site: &SiteId,
        (id, keyring_unavailable): (SessionId, bool),
    ) -> ConnectResult {
        let held = self.state().holds.remove(site);
        if let Some(held) = held {
            let _ = held.await;
        }
        let mut state = self.state();
        let attempt = Self::attempt(&mut state, id, site);
        Self::settle(&mut state, id, attempt.is_ok());
        match attempt {
            Ok(()) => Ok(Connected {
                session: id,
                keyring_unavailable,
            }),
            Err(error) => Err(ConnectError {
                session: Some(id),
                error,
                keyring_unavailable,
            }),
        }
    }

    /// The filesystem of one side: the shared local `MemFs`, or the remote one of the
    /// session's site.
    fn fs(&self, session: Option<SessionId>, side: Side) -> Result<MemFs> {
        match side {
            Side::Local => Ok(self.local_fs()),
            Side::Remote => {
                let site = session
                    .and_then(|session| self.session_site(session))
                    .ok_or_else(|| Self::not_found("Unknown session"))?;
                Ok(self.remote_fs(&site))
            }
        }
    }

    fn not_found(what: &str) -> Error {
        Error::new(ErrorKind::NotFound, what.to_owned())
    }
}

#[async_trait]
impl BackendApi for FakeBackend {
    async fn sites(&self) -> Result<Vec<Site>> {
        Ok(self.state().sites.clone())
    }

    async fn workspaces(&self) -> Result<Vec<Workspace>> {
        Ok(self.state().workspaces.clone())
    }

    async fn profiles(&self) -> Result<ProfileSnapshot> {
        let state = self.state();
        Ok(ProfileSnapshot {
            sites: state.sites.clone(),
            workspaces: state.workspaces.clone(),
            unreadable: state.unreadable.clone(),
        })
    }

    fn profile_dir(&self) -> PathBuf {
        PathBuf::from("fake-profiles")
    }

    async fn save_site(&self, mut site: Site) -> Result<()> {
        let mut state = self.state();
        match state.sites.iter_mut().find(|s| s.id == site.id) {
            Some(slot) => {
                site.ftp.pinned_fingerprint = super::vault::merged_pin(Some(slot), &site);
                site.has_password = slot.has_password;
                site.has_key_passphrase = slot.has_key_passphrase;
                *slot = site;
            }
            None => {
                site.ftp.pinned_fingerprint = None;
                site.has_password = false;
                site.has_key_passphrase = false;
                state.sites.push(site);
            }
        }
        Ok(())
    }

    async fn set_password(&self, site: SiteId, password: Option<SecretString>) -> Result<()> {
        let mut state = self.state();
        let slot = state
            .sites
            .iter_mut()
            .find(|s| s.id == site)
            .ok_or_else(|| Self::not_found("Unknown site"))?;
        slot.has_password = password.is_some();
        match password {
            Some(password) => state.passwords.insert(site, password),
            None => state.passwords.remove(&site),
        };
        Ok(())
    }

    async fn delete_site(&self, id: SiteId) -> Result<()> {
        let mut state = self.state();
        let before = state.sites.len();
        state.sites.retain(|s| s.id != id);
        if state.sites.len() == before {
            return Err(Self::not_found("Unknown site"));
        }
        for workspace in &mut state.workspaces {
            workspace.sites.retain(|s| s.site != id);
        }
        Ok(())
    }

    async fn save_workspace(&self, workspace: Workspace) -> Result<()> {
        let mut state = self.state();
        match state.workspaces.iter_mut().find(|w| w.id == workspace.id) {
            Some(slot) => *slot = workspace,
            None => state.workspaces.push(workspace),
        }
        Ok(())
    }

    async fn delete_workspace(&self, id: WorkspaceId) -> Result<()> {
        let mut state = self.state();
        let before = state.workspaces.len();
        state.workspaces.retain(|w| w.id != id);
        if state.workspaces.len() == before {
            return Err(Self::not_found("Unknown workspace"));
        }
        Ok(())
    }

    async fn save_workspace_tabs(&self, workspace: WorkspaceId, tabs: WorkspaceTabs) -> Result<()> {
        let hang = {
            let mut state = self.state();
            state.calls.push(Call::SaveTabs(workspace.clone()));
            state.hang_save_tabs
        };
        if hang {
            std::future::pending::<()>().await;
        }
        let mut state = self.state();
        if let Some(error) = state.save_tab_failures.pop_front() {
            return Err(error);
        }
        let stored = state
            .workspaces
            .iter_mut()
            .find(|w| w.id == workspace)
            .ok_or_else(|| Self::not_found("Unknown workspace"))?;
        for (site, saved) in tabs {
            if let Some(entry) = stored.sites.iter_mut().find(|e| e.site == site) {
                entry.tabs = saved;
            }
        }
        Ok(())
    }

    async fn connect(&self, site: SiteId) -> ConnectResult {
        let mut keyring_down = false;
        self.connect_one(site, &mut keyring_down).await
    }

    async fn connect_batch(
        &self,
        sites: Vec<SiteId>,
        results: mpsc::UnboundedSender<(SiteId, ConnectResult)>,
    ) {
        // One secret batch: the reads all happen first, as in `sites_with_secrets`.
        let mut keyring_down = false;
        let prepared: Vec<_> = sites
            .into_iter()
            .map(|site| {
                let start = self.start(&site, &mut keyring_down);
                (site, start)
            })
            .collect();
        let mut pending: FuturesUnordered<_> = prepared
            .into_iter()
            .map(|(site, start)| async move {
                let result = match start {
                    Ok(started) => self.finish(&site, started).await,
                    Err(error) => Err(error),
                };
                (site, result)
            })
            .collect();
        while let Some(result) = pending.next().await {
            let _ = results.send(result);
        }
    }

    async fn reconnect(&self, session: SessionId) -> Result<()> {
        let mut state = self.state();
        state.calls.push(Call::Reconnect(session));
        let site = state
            .sessions
            .get(&session)
            .cloned()
            .ok_or_else(|| Self::not_found("Unknown session"))?;
        let attempt = Self::attempt(&mut state, session, &site);
        Self::settle(&mut state, session, attempt.is_ok());
        attempt
    }

    async fn close(&self, session: SessionId) {
        let mut state = self.state();
        if state.sessions.remove(&session).is_some() {
            self.shells.close_session(session);
            state.fake_shells.retain(|_, fake| fake.session != session);
            state.connected.remove(&session);
            state.keyring_unavailable.remove(&session);
            state.pending_secrets.remove(&session);
            state.watches.remove(&session);
            state.auth.remove(&session);
            state.closed.push(session);
            state.calls.push(Call::Close(session));
        }
    }

    fn session_state(&self, session: SessionId) -> Option<watch::Receiver<ConnectionState>> {
        self.state()
            .watches
            .get(&session)
            .map(watch::Sender::subscribe)
    }

    fn session_auth(&self, session: SessionId) -> Option<AuthMethod> {
        self.state().auth.get(&session).cloned()
    }

    async fn shutdown(&self) {
        let (hang, panic) = {
            let mut state = self.state();
            state.calls.push(Call::Shutdown);
            (state.hang_shutdown, state.panic_shutdown)
        };
        // As the real backend: the unsent edits are kept first, locally.
        if let Some((_, edits)) = self.edits.get() {
            edits.preserve_unsent().await;
        }
        assert!(!panic, "the fake's shutdown panics, as asked");
        if hang {
            std::future::pending::<()>().await;
        }
        let mut sessions: Vec<SessionId> = self.state().sessions.keys().copied().collect();
        sessions.sort();
        for session in sessions {
            self.close(session).await;
        }
        if let Some((_, edits)) = self.edits.get() {
            edits.end_all().await;
        }
    }

    async fn open_shell(&self, session: SessionId, cols: u16, rows: u16) -> Result<ShellId> {
        let mut state = self.state();
        state.calls.push(Call::OpenShell(session));
        if let Some(error) = state.open_shell_failures.pop_front() {
            return Err(error);
        }
        if !state.sessions.contains_key(&session) {
            return Err(Self::not_found("Session closed"));
        }
        if !state.connected.contains(&session) {
            return Err(Error::new(ErrorKind::Network, "Not connected"));
        }
        let (output, rx) = mpsc::unbounded_channel();
        let fake = Arc::new(FakeShell {
            session,
            output,
            ended: AtomicBool::new(false),
            sizes: Mutex::new(vec![(cols, rows)]),
        });
        let id = self.shells.insert(session, fake.clone(), fake_output(rx));
        state.fake_shells.insert(id, fake);
        Ok(id)
    }

    async fn shell_input(&self, shell: ShellId, data: Vec<u8>) -> Result<()> {
        self.shells.input(shell, data).await
    }

    async fn shell_resize(&self, shell: ShellId, cols: u16, rows: u16) -> Result<()> {
        self.shells.resize(shell, cols, rows).await
    }

    fn close_shell(&self, shell: ShellId) {
        let mut state = self.state();
        state.calls.push(Call::CloseShell(shell));
        state.fake_shells.remove(&shell);
        self.shells.close(shell);
    }

    fn shell_output(&self, shell: ShellId) -> Option<BoxStream<'static, ShellOutput>> {
        self.shells.output(shell)
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
        super::ops::rename_no_replace(&fs, &from, &to).await
    }

    async fn delete(
        &self,
        session: Option<SessionId>,
        side: Side,
        targets: Vec<DeleteTarget>,
        progress: mpsc::UnboundedSender<u64>,
    ) -> Result<u64> {
        let fs = self.fs(session, side)?;
        super::ops::delete_all(&fs, targets, &progress).await
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

    async fn compare(
        &self,
        session: SessionId,
        local_root: RemotePath,
        remote_root: RemotePath,
        options: CompareOptions,
        progress: mpsc::UnboundedSender<Progress>,
        signals: CompareSignals,
    ) -> Result<Summary> {
        let site = self
            .session_site(session)
            .ok_or_else(super::sessions::closed)?;
        let script = {
            let mut state = self.state();
            state
                .compare_calls
                .push((session, local_root.clone(), remote_root.clone()));
            state.compare_scripts.pop_front()
        };
        if let Some(script) = script {
            for item in script.progress {
                let _ = progress.send(item);
            }
            if let Some(mut hold) = script.hold {
                // Waits for the test to resume or stop (like a paused or running walk).
                let mut stop = signals.stop.clone();
                tokio::select! {
                    _ = hold.changed() => {}
                    _ = stop.wait_for(|v| *v) => {
                        return Ok(Summary { identical: 0, differences: 0, partial: true, stopped: true });
                    }
                }
            }
            return script.result;
        }
        hoprs_remote::compare(
            Arc::new(self.local_fs()),
            local_root,
            Arc::new(self.remote_fs(&site)),
            remote_root,
            options,
            progress,
            signals,
        )
        .await
    }

    async fn enqueue(&self, spec: TransferSpec) -> Result<TransferId> {
        self.state().enqueued.push((spec, None));
        Ok(TransferId(
            self.next_transfer.fetch_add(1, Ordering::SeqCst),
        ))
    }

    async fn enqueue_with_policy(
        &self,
        spec: TransferSpec,
        policy: ConflictPolicy,
    ) -> Result<TransferId> {
        self.state().enqueued.push((spec, Some(policy)));
        Ok(TransferId(
            self.next_transfer.fetch_add(1, Ordering::SeqCst),
        ))
    }

    async fn control(&self, id: TransferId, op: ControlOp) -> Result<()> {
        self.state().controls.push((id, op));
        Ok(())
    }

    fn transfer_events(&self) -> broadcast::Receiver<Event> {
        self.transfer_events.subscribe()
    }

    async fn transfer_snapshot(&self) -> TransferSnapshot {
        let (seq, items) = self.state().queue.clone();
        TransferSnapshot {
            seq,
            items,
            workers: 4,
        }
    }

    /// Removes the terminal top-level items (and their children) from the snapshot.
    async fn clear_finished(&self) -> Result<usize> {
        let mut state = self.state();
        state.clears += 1;
        let items = &mut state.queue.1;
        let finished: HashSet<TransferId> = items
            .iter()
            .filter(|i| i.parent.is_none() && i.state.is_terminal())
            .map(|i| i.id)
            .collect();
        items.retain(|i| {
            !finished.contains(&i.id) && !i.parent.is_some_and(|p| finished.contains(&p))
        });
        Ok(finished.len())
    }

    async fn known_host_entries(&self, host: String, port: u16) -> Result<Vec<KnownHostEntry>> {
        let _ = (host, port);
        Ok(self.state().known_hosts.clone())
    }

    /// Like the real one: refused (`Conflict`) unless `expected` is the scripted list.
    async fn remove_known_host(
        &self,
        host: String,
        port: u16,
        expected: Vec<KnownHostEntry>,
    ) -> Result<usize> {
        let mut state = self.state();
        if state.known_hosts != expected {
            return Err(Error::new(
                ErrorKind::Conflict,
                "known_hosts changed; review the entries again",
            ));
        }
        state.known_host_removals.push((host, port));
        Ok(std::mem::take(&mut state.known_hosts).len())
    }

    async fn answer_prompt(&self, id: PromptId, answer: PromptAnswer) {
        let mut state = self.state();
        // Like the broker: a typed secret is held for its session, if the user asked to
        // save it (an answer without "save" drops an earlier one of the same kind).
        if let (Some(request), PromptAnswer::Secret { secret, save }) =
            (state.prompts.get(&id).cloned(), &answer)
        {
            let kind = match request.kind {
                PromptKind::Password { .. } => Some(SecretKind::Password),
                PromptKind::Passphrase { .. } => Some(SecretKind::KeyPassphrase),
                _ => None,
            };
            if let Some(kind) = kind {
                let pending = state.pending_secrets.entry(request.session).or_default();
                pending.retain(|(k, _)| *k != kind);
                if *save {
                    pending.push((kind, secret.clone()));
                }
            }
        }
        state.answers.push((id, answer));
    }

    fn prompt_events(&self) -> Option<mpsc::UnboundedReceiver<BrokerEvent>> {
        self.receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    async fn import_filezilla(
        &self,
        path: PathBuf,
        options: ImportOptions,
    ) -> Result<ImportReport> {
        self.record(BackupCall::Import {
            path,
            folders_as_workspaces: options.folders_as_workspaces,
        });
        Ok(ImportReport::default())
    }

    async fn delete_sitemanager(&self, path: PathBuf) -> Result<()> {
        self.record(BackupCall::DeleteSitemanager(path));
        Ok(())
    }

    async fn export_backup(
        &self,
        scope: ExportScope,
        path: PathBuf,
        password: SecretString,
    ) -> Result<ExportReport> {
        self.record(BackupCall::Export {
            scope: scope.clone(),
            path,
            password: password.expose_secret().to_owned(),
        });
        let mut state = self.state();
        if let Some(error) = state.export_failures.pop_front() {
            return Err(error);
        }
        Self::export_report(&state, &scope)
    }

    async fn seal_export(
        &self,
        scope: ExportScope,
        password: SecretString,
    ) -> Result<(ExportReport, Sealed)> {
        self.record(BackupCall::Seal {
            scope: scope.clone(),
            password: password.expose_secret().to_owned(),
        });
        let mut state = self.state();
        if let Some(error) = state.seal_failures.pop_front() {
            return Err(error);
        }
        let report = Self::export_report(&state, &scope)?;
        Ok((report, Sealed::new(b"sealed".to_vec())))
    }

    async fn drive_status(&self) -> Result<Option<DriveAccount>> {
        let mut state = self.state();
        state.drive_status_reads += 1;
        match &state.drive_status_error {
            Some(error) => Err(error.clone()),
            None => Ok(state.drive_account.clone()),
        }
    }

    fn drive_connect(&self, _pages: Pages) -> BoxStream<'static, ConnectEvent> {
        // As the real backend: a connect that ends connected is what `drive_status`
        // answers from then on (from memory: the keyring is not asked again).
        let state = self.state.clone();
        futures_util::stream::iter(std::mem::take(&mut self.state().drive_connect))
            .inspect(move |event| {
                if let ConnectEvent::Done(Ok(account)) = event {
                    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
                    state.drive_account = Some(account.clone());
                    state.drive_status_error = None;
                }
            })
            .boxed()
    }

    fn drive_cancel(&self) {
        self.state().drive_cancels += 1;
    }

    async fn drive_disconnect(&self) -> Result<()> {
        let mut state = self.state();
        state.drive_disconnects += 1;
        state.drive_account = None;
        Ok(())
    }

    async fn drive_upload(&self, name: String, sealed: Sealed) -> Result<DriveFile> {
        let mut state = self.state();
        state
            .drive_uploads
            .push((name.clone(), sealed.bytes().len()));
        if let Some(error) = state.upload_failures.pop_front() {
            return Err(error);
        }
        // As the real backend: nothing to upload with.
        if state.drive_account.is_none() {
            return Err(crate::drive::not_connected());
        }
        Ok(DriveFile {
            name,
            link: Some("https://drive.google.com/file/d/fake/view".into()),
        })
    }

    async fn save_sealed(&self, path: PathBuf, _sealed: Sealed) -> Result<()> {
        self.state().sealed_saves.push(path);
        Ok(())
    }

    async fn restore_backup(
        &self,
        path: PathBuf,
        password: SecretString,
        mode: ImportMode,
    ) -> Result<BackupImportReport> {
        self.record(BackupCall::Restore {
            path,
            password: password.expose_secret().to_owned(),
            mode,
        });
        Ok(BackupImportReport::default())
    }

    fn settings(&self) -> SettingsLoad {
        self.state().settings.clone()
    }

    fn queue_error(&self) -> Option<Error> {
        self.state().queue_error.clone()
    }

    async fn save_settings(&self, settings: Settings) -> Result<()> {
        self.record(BackupCall::SaveSettings(settings));
        Ok(())
    }

    async fn set_policy(&self, policy: ConflictPolicy) {
        self.record(BackupCall::SetPolicy(policy));
    }

    fn set_workers(&self, workers: usize) {
        self.record(BackupCall::SetWorkers(workers));
    }

    fn set_keepalive(&self, secs: u64) {
        self.record(BackupCall::SetKeepalive(secs));
    }

    async fn latest_release(&self) -> Result<Release> {
        let mut state = self.state();
        state.releases_asked += 1;
        state.release.clone().unwrap_or_else(|| {
            Err(Error::coded(
                ErrorKind::Network,
                "update-check-failed",
                "Could not check for updates",
            )
            .with_detail("the fake has no release"))
        })
    }

    fn download_update(&self, job: DownloadJob) -> BoxStream<'static, DownloadEvent> {
        let mut state = self.state();
        state.downloads.push(job);
        futures_util::stream::iter(std::mem::take(&mut state.download)).boxed()
    }

    async fn start_edit(
        &self,
        session: SessionId,
        remote: RemotePath,
        editor: String,
    ) -> Result<EditId> {
        let target = {
            let state = self.state();
            if !state.connected.contains(&session) {
                return Err(Error::new(ErrorKind::Network, "Not connected"));
            }
            let site = state
                .sessions
                .get(&session)
                .ok_or_else(|| Self::not_found("Unknown session"))?;
            let fs: Arc<dyn RemoteFs> = Arc::new(state.remotes[site].clone());
            let watch = state.watches[&session].subscribe();
            EditSession {
                session,
                site: site.to_string(),
                fs,
                state: watch,
                mode: UploadMode::TempRename,
            }
        };
        self.edits().start(target, remote, &editor).await
    }

    async fn resolve_edit(&self, id: EditId, choice: EditChoice) -> Result<()> {
        self.edits().resolve(id, choice).await
    }

    async fn edits_pending(&self, session: SessionId) -> Vec<EditSummary> {
        self.edits().pending(session).await
    }

    async fn finish_edits(&self, session: SessionId, discard: bool) -> Result<Vec<EditSummary>> {
        self.edits().finish(session, discard).await
    }

    async fn retry_edit(&self, id: EditId) -> Result<()> {
        self.edits().retry(id).await
    }

    async fn flush_edits(
        &self,
        session: SessionId,
        within: std::time::Duration,
    ) -> Vec<EditSummary> {
        self.state().calls.push(Call::FlushEdits(session));
        self.edits().flush(session, within).await
    }

    async fn keep_edits(&self, session: SessionId) -> Result<Vec<KeptEdit>> {
        self.state().calls.push(Call::KeepEdits(session));
        self.edits().keep(session).await
    }

    fn subscribe_edits(&self) -> broadcast::Receiver<EditEvent> {
        self.edits().subscribe()
    }

    fn recovered_dir(&self) -> PathBuf {
        self.edits().recovered_dir().to_owned()
    }

    async fn preserve_edits(&self) -> Vec<KeptEdit> {
        self.edits().preserve_unsent().await
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use hoprs_core::Protocol;
    use hoprs_transfer::Direction;

    use super::*;

    /// The update calls: a scripted release (a failed check by default), scripted
    /// download events, both recorded.
    #[tokio::test]
    async fn fake_backend_update_calls() {
        let fake = FakeBackend::new();
        assert_eq!(
            fake.latest_release().await.unwrap_err().code,
            Some("update-check-failed")
        );
        let release = Release {
            version: semver::Version::new(9, 9, 9),
            page: "https://github.com/a/b/releases/tag/v9.9.9".into(),
            assets: Vec::new(),
        };
        let fake = fake.with_release(Ok(release.clone()));
        assert_eq!(fake.latest_release().await.unwrap(), release);
        assert_eq!(fake.releases_asked(), 2);
        let job = DownloadJob {
            asset: crate::update::Asset {
                name: "a.deb".into(),
                url: "https://github.com/a.deb".into(),
                size: 1,
            },
            sums_url: "https://github.com/SHA256SUMS".into(),
        };
        fake.set_download(vec![
            DownloadEvent::Verifying,
            DownloadEvent::Done(Ok(PathBuf::from("a.deb"))),
        ]);
        let events: Vec<_> = fake.download_update(job.clone()).collect().await;
        assert!(matches!(
            events.as_slice(),
            [DownloadEvent::Verifying, DownloadEvent::Done(Ok(_))]
        ));
        assert_eq!(fake.downloads().len(), 1);
        assert_eq!(fake.downloads()[0].asset, job.asset);
    }

    /// The Drive calls follow the script: status, connect events, uploads, disconnects,
    /// cancels and sealed saves, all recorded.
    #[tokio::test]
    async fn fake_drive_follows_its_script() {
        use crate::drive::{ConnectEvent, DriveAccount, DriveFile, Pages, Sealed};
        let pages = || Pages {
            connected: "c".into(),
            failed: "f".into(),
        };
        let fake = FakeBackend::new();
        assert_eq!(fake.drive_status().await.unwrap(), None);
        let account = DriveAccount {
            email: Some("me@example.com".into()),
            stored: true,
        };
        let fake = fake.with_drive(Some(account.clone()));
        assert_eq!(fake.drive_status().await.unwrap(), Some(account.clone()));
        fake.fail_drive_status(Error::new(ErrorKind::Io, "keyring"));
        assert_eq!(fake.drive_status().await.unwrap_err().message, "keyring");
        assert_eq!(fake.drive_status_reads(), 3);

        // Nothing scripted: the stream ends at once.
        assert!(
            fake.drive_connect(pages())
                .collect::<Vec<_>>()
                .await
                .is_empty()
        );
        fake.set_drive_connect(vec![
            ConnectEvent::Waiting {
                url: "https://example.com/auth".into(),
            },
            ConnectEvent::Done(Ok(account.clone())),
        ]);
        let events: Vec<_> = fake.drive_connect(pages()).collect().await;
        assert!(matches!(
            events.as_slice(),
            [ConnectEvent::Waiting { .. }, ConnectEvent::Done(Ok(_))]
        ));
        fake.drive_cancel();
        assert_eq!(fake.drive_cancels(), 1);

        let scope = ExportScope::All;
        let (report, sealed) = fake
            .seal_export(scope.clone(), SecretString::from("pw"))
            .await
            .unwrap();
        assert_eq!(report.sites, 0);
        assert_eq!(sealed.bytes(), b"sealed");
        assert_eq!(
            fake.backup_calls(),
            vec![BackupCall::Seal {
                scope,
                password: "pw".into()
            }]
        );
        let file = fake
            .drive_upload("a.hoprs".into(), sealed.clone())
            .await
            .unwrap();
        assert_eq!(
            file,
            DriveFile {
                name: "a.hoprs".into(),
                link: Some("https://drive.google.com/file/d/fake/view".into())
            }
        );
        fake.fail_next_upload(Error::new(ErrorKind::Network, "down"));
        let err = fake
            .drive_upload("b.hoprs".into(), Sealed::new(vec![0; 3]))
            .await
            .unwrap_err();
        assert_eq!(err.message, "down");
        assert_eq!(
            fake.drive_uploads(),
            vec![("a.hoprs".to_owned(), 6), ("b.hoprs".to_owned(), 3)]
        );
        fake.drive_upload("c.hoprs".into(), sealed.clone())
            .await
            .unwrap();
        fake.save_sealed(PathBuf::from("x.hoprs"), sealed)
            .await
            .unwrap();
        assert_eq!(fake.sealed_saves(), vec![PathBuf::from("x.hoprs")]);
        fake.drive_disconnect().await.unwrap();
        assert_eq!(fake.drive_disconnects(), 1);
    }

    /// Edits run on the real edit manager: a save while the connection is down is held,
    /// and goes up once the session reconnects.
    #[tokio::test(start_paused = true)]
    async fn fake_backend_edits_wait_for_reconnect() {
        let site = Site::new("s", "h", "u", Protocol::Ssh);
        let fake = FakeBackend::new().with_site(site.clone());
        let session = fake.connect(site.id.clone()).await.unwrap().session;
        let remote = fake.remote_fs(&site.id);
        remote.add_file("/f.txt", b"one", SystemTime::UNIX_EPOCH);
        let mut events = fake.subscribe_edits();

        // Not a connected session: no edit.
        let err = fake
            .start_edit(
                SessionId(99),
                RemotePath::new("/f.txt").unwrap(),
                "ed".into(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Network);

        let id = fake
            .start_edit(session, RemotePath::new("/f.txt").unwrap(), "ed -s".into())
            .await
            .unwrap();
        let launched = fake.edits_launched();
        assert_eq!(launched.len(), 1);
        assert_eq!(launched[0].0, "ed -s");
        assert_eq!(std::fs::read(&launched[0].1).unwrap(), b"one");

        fake.drop_connection(session, Error::new(ErrorKind::Network, "lost"));
        fake.save_edit(session, id, b"two");
        tokio::time::advance(super::super::edit::DEBOUNCE).await;
        assert_eq!(
            events.recv().await.unwrap(),
            EditEvent::Pending {
                id,
                remote_may_be_truncated: false
            }
        );
        assert_eq!(fake.edits_pending(session).await.len(), 1);
        assert_eq!(remote.read_file("/f.txt").unwrap(), b"one");

        fake.reconnect(session).await.unwrap();
        assert!(matches!(
            events.recv().await.unwrap(),
            EditEvent::Uploaded { id: got, .. } if got == id
        ));
        assert_eq!(remote.read_file("/f.txt").unwrap(), b"two");
        assert!(fake.finish_edits(session, false).await.unwrap().is_empty());
        assert_eq!(events.recv().await.unwrap(), EditEvent::Closed { id });
    }

    #[tokio::test]
    async fn fake_backend_connects_lists_and_records() {
        let site = Site::new("s", "h", "u", Protocol::Ssh);
        let ws = Workspace {
            id: WorkspaceId::new(),
            name: "w".into(),
            sites: vec![hoprs_core::WorkspaceSite {
                site: site.id.clone(),
                tabs: Vec::new(),
            }],
        };
        let fake = FakeBackend::new()
            .with_site(site.clone())
            .with_workspace(ws.clone());
        fake.remote_fs(&site.id)
            .add_file("/srv/a.txt", b"abc", SystemTime::UNIX_EPOCH);
        fake.local_fs().add_dir("/home/me");

        // Scripted failure first, then success; unknown sites are NotFound.
        fake.fail_next_connect(&site.id, Error::new(ErrorKind::Auth, "Denied"));
        let err = fake.connect(site.id.clone()).await.unwrap_err();
        assert_eq!(err.error.kind, ErrorKind::Auth);
        let failed = err.session.unwrap();
        assert!(fake.reconnect(failed).await.is_ok());
        let id = fake.connect(site.id.clone()).await.unwrap().session;
        assert_eq!(id, failed, "the kept session was retried, not replaced");
        let missing = fake.connect(SiteId::new()).await.unwrap_err();
        assert_eq!(missing.session, None);

        let remote = fake
            .list(Some(id), Side::Remote, RemotePath::new("/srv").unwrap())
            .await
            .unwrap();
        assert_eq!(remote[0].name, "a.txt");
        let local = fake
            .list(None, Side::Local, RemotePath::new("/home").unwrap())
            .await
            .unwrap();
        assert_eq!(local[0].name, "me");

        let spec = TransferSpec {
            site: site.id.clone(),
            direction: Direction::Upload,
            local: RemotePath::new("/home/me").unwrap(),
            remote: RemotePath::new("/srv").unwrap(),
        };
        let tid = fake.enqueue(spec.clone()).await.unwrap();
        fake.control(tid, ControlOp::Pause).await.unwrap();
        fake.answer_prompt(PromptId(7), PromptAnswer::Decision(true))
            .await;
        fake.close(id).await;
        assert_eq!(fake.enqueued(), vec![spec.clone()]);
        fake.enqueue_with_policy(spec.clone(), ConflictPolicy::Overwrite)
            .await
            .unwrap();
        assert_eq!(
            fake.enqueued_with_policy(),
            vec![
                (spec.clone(), None),
                (spec, Some(ConflictPolicy::Overwrite))
            ]
        );
        assert_eq!(fake.controls(), vec![(tid, ControlOp::Pause)]);
        assert!(matches!(
            fake.answers()[..],
            [(PromptId(7), PromptAnswer::Decision(true))]
        ));
        assert_eq!(fake.closed(), vec![id]);
        assert!(fake.session_site(id).is_none());

        let mut events = fake.prompt_events().unwrap();
        assert!(fake.prompt_events().is_none());
        fake.push_prompt(BrokerEvent::Withdrawn(PromptId(1)));
        assert_eq!(
            events.recv().await,
            Some(BrokerEvent::Withdrawn(PromptId(1)))
        );

        // Sites and workspaces.
        let (tx, mut rx) = mpsc::unbounded_channel();
        fake.connect_batch(vec![site.id.clone()], tx).await;
        let (_, result) = rx.recv().await.unwrap();
        fake.close(result.unwrap().session).await;
        fake.save_workspace(ws.clone()).await.unwrap();
        assert_eq!(fake.workspaces().await.unwrap().len(), 1);
        fake.delete_site(site.id.clone()).await.unwrap();
        assert!(fake.workspaces().await.unwrap()[0].sites.is_empty());
        fake.delete_workspace(ws.id).await.unwrap();
        assert!(fake.sites().await.unwrap().is_empty());
    }

    /// Like the broker and session manager: a secret answered with "save" is stored only
    /// when its session's connect succeeds, a passphrase as a passphrase.
    #[tokio::test]
    async fn fake_backend_saves_typed_secrets_after_connect() {
        use super::super::broker::PromptRequest;
        use secrecy::ExposeSecret;

        let site = Site::new("s", "h", "u", Protocol::Ssh);
        let fake = FakeBackend::new().with_site(site.clone());
        fake.fail_next_connect(&site.id, Error::new(ErrorKind::Auth, "Denied"));
        let ask = |id: u64, kind: PromptKind| {
            fake.push_prompt(BrokerEvent::Request(PromptRequest {
                id: PromptId(id),
                session: SessionId(1),
                site: site.id.clone(),
                key: "h:22".into(),
                kind,
            }));
        };
        let answer = |value: &str| PromptAnswer::Secret {
            secret: SecretString::from(value),
            save: true,
        };
        let passphrase = PromptKind::Passphrase { path: "/k".into() };
        // The first connect fails: nothing is stored.
        ask(1, passphrase.clone());
        fake.answer_prompt(PromptId(1), answer("wrong")).await;
        let failed = fake.connect(site.id.clone()).await.unwrap_err();
        assert_eq!(failed.session, Some(SessionId(1)));
        assert!(fake.passphrase(&site.id).is_none());
        // The retry connects: the new answer is stored.
        ask(2, passphrase);
        fake.answer_prompt(PromptId(2), answer("right")).await;
        fake.connect(site.id.clone()).await.unwrap();
        assert_eq!(fake.passphrase(&site.id).unwrap().expose_secret(), "right");
        assert!(fake.password(&site.id).is_none());
        assert!(fake.sites().await.unwrap()[0].has_key_passphrase);
    }

    /// The fake follows the real `SessionManager` rules (see the module docs).
    #[tokio::test]
    async fn fake_backend_follows_session_manager_rules() {
        let site = Site::new("s", "h", "u", Protocol::Ftp);
        let mut secret = Site::new("p", "h", "u", Protocol::Ftp);
        secret.has_password = true;
        let fake = FakeBackend::new()
            .with_site(site.clone())
            .with_site(secret.clone());

        // A second connect of an open site returns the same session, without an attempt.
        let first = fake.connect(site.id.clone()).await.unwrap();
        assert!(!first.keyring_unavailable);
        fake.fail_next_connect(&site.id, Error::new(ErrorKind::Network, "down"));
        assert_eq!(fake.connect(site.id.clone()).await.unwrap(), first);
        // The scripted failure is still pending: a closed site starts a new session.
        fake.close(first.session).await;
        let err = fake.connect(site.id.clone()).await.unwrap_err();
        let kept = err
            .session
            .expect("a failed first connect keeps the session");
        assert!(kept > first.session);

        // Connecting again retries the kept session; so does reconnect.
        fake.fail_next_connect(&site.id, Error::new(ErrorKind::Network, "still down"));
        let again = fake.connect(site.id.clone()).await.unwrap_err();
        assert_eq!(again.session, Some(kept));
        assert_eq!(again.error.message, "still down");
        assert!(fake.reconnect(kept).await.is_ok());
        assert_eq!(fake.connect(site.id.clone()).await.unwrap().session, kept);
        // A connected session's reconnect consumes no script.
        fake.fail_next_connect(&site.id, Error::new(ErrorKind::Network, "unused"));
        assert!(fake.reconnect(kept).await.is_ok());
        assert!(fake.reconnect(SessionId(999)).await.is_err());

        // A locked keyring is reported for sites with a stored secret, on both paths.
        fake.lock_keyring();
        let ok = fake.connect(secret.id.clone()).await.unwrap();
        assert!(ok.keyring_unavailable);
        fake.close(ok.session).await;
        fake.fail_next_connect(&secret.id, Error::new(ErrorKind::Auth, "Denied"));
        let err = fake.connect(secret.id.clone()).await.unwrap_err();
        assert!(err.keyring_unavailable);
        let other = Site::new("o", "h", "u", Protocol::Ftp);
        let fake = fake.with_site(other.clone());
        assert!(!fake.connect(other.id).await.unwrap().keyring_unavailable);
    }

    async fn next(output: &mut BoxStream<'static, ShellOutput>) -> Option<ShellOutput> {
        tokio::time::timeout(std::time::Duration::from_secs(1), output.next())
            .await
            .expect("the shell output stalled")
    }

    /// Shells belong to their session (Review Focus 5): closing the session closes them
    /// and leaves the other sessions' shells alone; a dropped connection ends its shells
    /// and reconnecting does not reopen them.
    #[tokio::test]
    async fn shells_close_with_their_session() {
        let a = Site::new("a", "h", "u", Protocol::Ssh);
        let b = Site::new("b", "h", "u", Protocol::Ssh);
        let fake = FakeBackend::new().with_site(a.clone()).with_site(b.clone());
        let sa = fake.connect(a.id.clone()).await.unwrap().session;
        let sb = fake.connect(b.id.clone()).await.unwrap().session;

        let a1 = fake.open_shell(sa, 80, 24).await.unwrap();
        let a2 = fake.open_shell(sa, 120, 40).await.unwrap();
        let b1 = fake.open_shell(sb, 80, 24).await.unwrap();
        assert!(a1 != a2 && a2 != b1 && a1 != b1);
        let mut out_a1 = fake.shell_output(a1).expect("the output stream");
        assert!(fake.shell_output(a1).is_none(), "handed out once");
        let mut out_a2 = fake.shell_output(a2).unwrap();
        let mut out_b1 = fake.shell_output(b1).unwrap();

        // Scripted shells: input echoes back, the test adds output.
        fake.shell_input(a1, b"ls\n".to_vec()).await.unwrap();
        assert_eq!(
            next(&mut out_a1).await,
            Some(ShellOutput::Data(b"ls\n".to_vec()))
        );
        fake.push_shell_output(a1, b"$ ");
        assert_eq!(
            next(&mut out_a1).await,
            Some(ShellOutput::Data(b"$ ".to_vec()))
        );
        fake.shell_resize(a2, 100, 30).await.unwrap();
        assert_eq!(fake.shell_sizes(a2), vec![(120, 40), (100, 30)]);

        // Closing session a closes both of its shells, and only those.
        fake.close(sa).await;
        for out in [&mut out_a1, &mut out_a2] {
            assert_eq!(next(out).await, Some(ShellOutput::Closed));
            assert_eq!(next(out).await, None);
        }
        let e = fake.shell_input(a1, b"x".to_vec()).await.unwrap_err();
        assert_eq!(e.message, "Shell closed");
        assert!(fake.shell_resize(a2, 80, 24).await.is_err());
        assert!(
            fake.open_shell(sa, 80, 24).await.is_err(),
            "the session is gone"
        );
        fake.shell_input(b1, b"pwd\n".to_vec()).await.unwrap();
        assert_eq!(
            next(&mut out_b1).await,
            Some(ShellOutput::Data(b"pwd\n".to_vec()))
        );

        // A dropped connection ends the shell; reconnecting the session does not reopen it.
        fake.drop_connection(sb, Error::new(ErrorKind::Network, "Connection lost"));
        assert_eq!(next(&mut out_b1).await, Some(ShellOutput::Closed));
        assert_eq!(next(&mut out_b1).await, None);
        assert!(fake.open_shell(sb, 80, 24).await.is_err(), "not connected");
        fake.reconnect(sb).await.unwrap();
        assert!(fake.shell_input(b1, b"x".to_vec()).await.is_err());
        let b2 = fake.open_shell(sb, 80, 24).await.unwrap();
        assert_ne!(b2, b1, "a new shell, opened by the tab");
        let mut out_b2 = fake.shell_output(b2).unwrap();

        // The server ends a shell on command; an open can be refused on command.
        fake.end_shell(b2);
        assert_eq!(next(&mut out_b2).await, Some(ShellOutput::Closed));
        assert_eq!(next(&mut out_b2).await, None);
        let limit = Error::new(
            ErrorKind::Protocol,
            "Too many channels on this connection (limit 10)",
        );
        fake.fail_next_open_shell(limit.clone());
        assert_eq!(fake.open_shell(sb, 80, 24).await.unwrap_err(), limit);

        // `close_shell` ends one shell; the tab's own close is recorded.
        let b3 = fake.open_shell(sb, 80, 24).await.unwrap();
        let mut out_b3 = fake.shell_output(b3).unwrap();
        fake.close_shell(b3);
        assert_eq!(next(&mut out_b3).await, Some(ShellOutput::Closed));
        assert_eq!(next(&mut out_b3).await, None);
        assert!(fake.shell_output(b3).is_none());
        assert!(fake.calls().contains(&Call::CloseShell(b3)));
        fake.close_shell(b3);
    }
}
