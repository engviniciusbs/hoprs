//! The session manager: connects saved sites, registers them with the transfer engine,
//! watches them and closes them.
//!
//! **Connect shape (ruling R5).** `SftpFs::new` needs a *connected* SSH session, so
//! [`SessionManager::connect`] starts the connection (`SshSession::start` /
//! `FtpFs::start`), keeps it under a fresh [`SessionId`] at once (its state is observable
//! and [`close`](SessionManager::close) can cancel it, prompts included), then waits for
//! the first `Connected`. Only then does it build the remote filesystem (`SftpFs` over the
//! session, or the `FtpFs` itself), register the site with the engine
//! (`LocalFs::new("/")` as the local side) and spawn its `spawn_site_watcher`. FTP takes the
//! same path for uniformity: `connect` returns once either protocol can list.
//!
//! If the first connection ends `Disconnected`, `connect` fails with a [`ConnectError`]
//! that still carries the session id: the session stays (so the UI shows its error) and
//! [`reconnect`](SessionManager::reconnect) retries it (`reconnect_now`, then the same
//! wait-and-register). [`close`](SessionManager::close) drops it.
//!
//! One session per site: `connect` on a site that already has one returns that session
//! (retrying it if it is `Disconnected`). The engine keys endpoints by site, so two live
//! sessions of one site would replace each other's endpoints.
//!
//! An SSH session also carries the terminal shells opened on it
//! ([`open_shell`](SessionManager::open_shell), see [`super::shells`]): `close` closes them,
//! and a reconnect does not reopen them.
//!
//! **Connection log and idle drops (Plan 6, Task 5).** Each session has a monitor task, the
//! one place that logs its connection: every state change at `info` (site name, protocol,
//! `host:port`, the new state, the reason as `ErrorKind` and message, and
//! `since_last_request_secs`), an unanswered keepalive at `warn`. The transports log
//! nothing. `since_last_request_secs` counts from the start of the last remote filesystem
//! call (listing, file operation, the opening of a transfer's stream) or shell input: a long
//! transfer still running counts from its start. When the transport gives up reconnecting
//! (a whole round of attempts failed, while nobody used the site, say), the monitor starts
//! **one** more round by itself, quietly (a network failure only, never host key,
//! authentication or TLS, never after `close`, and only for a session that was connected
//! before); the next such drop waits for the user (the app reconnects on their next action).
//! The budget comes back once the session is connected again.
//!
//! **Sessions for agents (Plan 17).** [`connect_for_agent`](SessionManager::connect_for_agent)
//! is the one-site connect an agent's call takes. A session it *starts* is `agent_only` and
//! its prompter is [`by_agent`](SessionPrompter::by_agent) (its dialogs say an agent asked);
//! a session the site already has is reused as it is (a tab's stays the tab's, and is not
//! retried for the agent if it is `Disconnected`: that is the user's to do). When the user
//! connects a site whose session is `agent_only` (`connect_batch`), the tab takes it over:
//! the flag is cleared, and the agent side never closes it
//! ([`close_agent_session`](SessionManager::close_agent_session) closes `agent_only`
//! sessions only). Nothing here opens a tab or reaches the UI: the app sees an agent's
//! session only through the dialogs and [`agent_sites`](SessionManager::agent_sites).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use futures_util::stream::FuturesUnordered;
use futures_util::{FutureExt, StreamExt};
use hoprs_core::{
    ConnectionState, Error, ErrorKind, Protocol, ReconnectPolicy, Result, Site, SiteId,
};
use hoprs_remote::{Connection, FtpFs, LocalFs, RemoteFs, SftpConfig, SftpFs};
use hoprs_ssh::{ExecLimits, ExecOutput, PtyRequest, SshSession};
use hoprs_transfer::{SiteEndpoints, TransferEngine, spawn_site_watcher};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use super::broker::{PromptBroker, SessionPrompter};
use super::mapping::{SiteSecrets, ftp_config, ssh_config};
use super::shells::{ShellId, Shells};
use super::vault::VaultHandle;

/// Identifies one open session. Monotonic: never reused within a run.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(pub u64);

/// A connected session.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Connected {
    pub session: SessionId,
    /// The keyring could not be reached when the session's secrets were read, so its
    /// stored password or passphrase (if any) was not used: the UI says so, which tells a
    /// locked keyring apart from no stored secret.
    pub keyring_unavailable: bool,
}

/// What connecting one site gives.
pub type ConnectResult = std::result::Result<Connected, ConnectError>;

/// A failed connect. `session` is `Some` when the session exists (its first connection
/// failed): [`SessionManager::reconnect`] retries it, [`SessionManager::close`] drops it.
#[derive(Clone, Debug, PartialEq)]
pub struct ConnectError {
    pub session: Option<SessionId>,
    pub error: Error,
    /// As in [`Connected::keyring_unavailable`]: the keyring was unreachable when the
    /// secrets were read (a likely reason for an authentication failure).
    pub keyring_unavailable: bool,
}

impl ConnectError {
    fn before_start(error: Error) -> Self {
        Self {
            session: None,
            error,
            keyring_unavailable: false,
        }
    }
}

/// Session manager options.
#[derive(Clone, Debug)]
pub struct SessionConfig {
    /// Replaces `~/.ssh/known_hosts` (tests).
    pub known_hosts: Option<PathBuf>,
    /// Transfer workers per site; also the SFTP stream channels and FTP transfer
    /// connections (both clamped to `1..=8`; the SFTP streams are also capped by the SSH
    /// channel budget, which keeps room for terminals). The starting value: see
    /// [`SessionManager::set_workers`].
    pub workers: usize,
    /// Keepalive of SSH and FTP sessions (Settings). The starting value: see
    /// [`SessionManager::set_keepalive`].
    pub keepalive: Duration,
    /// Replaces the protocols' connect timeout (tests).
    pub connect_timeout: Option<Duration>,
    /// Replaces the protocols' reconnect policy (tests).
    pub reconnect: Option<ReconnectPolicy>,
}

impl SessionConfig {
    /// The `known_hosts` file SSH sessions check: the configured one, else
    /// `~/.ssh/known_hosts`.
    pub fn known_hosts_path(&self) -> PathBuf {
        match &self.known_hosts {
            Some(path) => path.clone(),
            None => hoprs_ssh::SshConfig::new("", 22, "").known_hosts,
        }
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            known_hosts: None,
            workers: 4,
            keepalive: Duration::from_secs(30),
            connect_timeout: None,
            reconnect: None,
        }
    }
}

/// Who asks for a connection: the user (a tab, a workspace) or an agent's call.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Caller {
    User,
    Agent,
}

/// The live connection of a session.
#[derive(Clone)]
enum Link {
    Ssh(SshSession),
    Ftp(FtpFs),
    /// A test double that only reports state and counts `reconnect_now`.
    #[cfg(test)]
    Fake {
        conn: Arc<dyn Connection>,
        reconnects: Arc<AtomicUsize>,
    },
}

/// What the connection log says a session is.
#[derive(Clone, Debug)]
struct SessionLabel {
    name: String,
    protocol: Protocol,
    endpoint: String,
    /// Reconnection attempts per round, for "attempt n of m".
    attempts: u32,
}

/// When a session was last asked something (remote filesystem calls, shell input), and
/// whether `close` closed it.
#[derive(Debug)]
struct Activity {
    last_request: Mutex<Instant>,
    closed: std::sync::atomic::AtomicBool,
}

impl Activity {
    fn new() -> Self {
        Self {
            last_request: Mutex::new(Instant::now()),
            closed: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn touch(&self) {
        *lock(&self.last_request) = Instant::now();
    }

    fn since_last_request(&self) -> Duration {
        lock(&self.last_request).elapsed()
    }

    /// `close` is closing the session: its end is the user's, not the network's.
    fn mark_closed(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

struct Entry {
    site: SiteId,
    link: Link,
    label: SessionLabel,
    activity: Arc<Activity>,
    /// Logs the connection's states and reconnects once after an idle drop.
    monitor: Option<JoinHandle<()>>,
    prompter: SessionPrompter,
    /// The secrets were read while the keyring was unreachable.
    keyring_unavailable: bool,
    /// Set once the first connection is up and the site is registered.
    remote: Option<Arc<dyn RemoteFs>>,
    watcher: Option<JoinHandle<()>>,
    /// Started by an agent's call and not used by a tab since (see the module docs).
    agent_only: bool,
}

/// Connects, registers, watches and closes sessions. Cheap to clone.
#[derive(Clone)]
pub struct SessionManager {
    inner: Arc<Inner>,
}

struct Inner {
    engine: TransferEngine,
    vault: VaultHandle,
    broker: PromptBroker,
    config: SessionConfig,
    /// Transfer workers per site for the next connections (Settings).
    workers: AtomicUsize,
    /// Keepalive for the next connections (Settings).
    keepalive: Mutex<Duration>,
    local: Arc<dyn RemoteFs>,
    next_id: AtomicU64,
    sessions: Mutex<HashMap<SessionId, Entry>>,
    /// Serializes connect, reconnect and close per site.
    site_locks: Mutex<HashMap<SiteId, Arc<tokio::sync::Mutex<()>>>>,
    /// Sites `close` paused in the engine; the next connect resumes them.
    paused_by_close: Mutex<HashSet<SiteId>>,
    /// The terminal shells of the sessions. Locked after `sessions` when both are.
    shells: Shells,
    /// Remotes of the test sessions made by `attach`, until they are ready.
    #[cfg(test)]
    test_remotes: Mutex<HashMap<SessionId, Arc<dyn RemoteFs>>>,
}

impl Link {
    fn connection(&self) -> Arc<dyn Connection> {
        match self {
            Self::Ssh(session) => Arc::new(session.clone()),
            Self::Ftp(fs) => Arc::new(fs.clone()),
            #[cfg(test)]
            Self::Fake { conn, .. } => conn.clone(),
        }
    }

    fn state(&self) -> watch::Receiver<ConnectionState> {
        match self {
            Self::Ssh(session) => session.state(),
            Self::Ftp(fs) => Connection::state(fs),
            #[cfg(test)]
            Self::Fake { conn, .. } => conn.state(),
        }
    }

    async fn reconnect_now(&self) {
        match self {
            Self::Ssh(session) => session.reconnect_now().await,
            Self::Ftp(fs) => fs.reconnect_now().await,
            #[cfg(test)]
            Self::Fake { reconnects, .. } => {
                reconnects.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    async fn disconnect(&self) {
        match self {
            Self::Ssh(session) => session.disconnect().await,
            Self::Ftp(fs) => fs.disconnect().await,
            #[cfg(test)]
            Self::Fake { .. } => {}
        }
    }
}

pub(super) fn closed() -> Error {
    Error::coded(ErrorKind::NotFound, "session-closed", "Session closed")
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl SessionManager {
    pub fn new(
        engine: TransferEngine,
        vault: VaultHandle,
        broker: PromptBroker,
        config: SessionConfig,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                engine,
                vault,
                broker,
                workers: AtomicUsize::new(config.workers),
                keepalive: Mutex::new(config.keepalive),
                config,
                local: Arc::new(LocalFs::new("/")),
                next_id: AtomicU64::new(1),
                sessions: Mutex::new(HashMap::new()),
                site_locks: Mutex::new(HashMap::new()),
                paused_by_close: Mutex::new(HashSet::new()),
                shells: Shells::default(),
                #[cfg(test)]
                test_remotes: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Transfer workers per site for the connections made from now on.
    pub fn workers(&self) -> usize {
        self.inner.workers.load(Ordering::Relaxed)
    }

    /// Sets the workers per site (Settings). Sessions already registered keep theirs;
    /// the next connection of a site (a new session) uses this value.
    pub fn set_workers(&self, workers: usize) {
        self.inner.workers.store(workers, Ordering::Relaxed);
    }

    /// Keepalive of the sessions started from now on.
    pub fn keepalive(&self) -> Duration {
        *lock(&self.inner.keepalive)
    }

    /// Sets the keepalive of SSH and FTP sessions (Settings). Sessions already started keep
    /// theirs; the next connection of a site (a new session) uses this value.
    pub fn set_keepalive(&self, keepalive: Duration) {
        *lock(&self.inner.keepalive) = keepalive;
    }

    /// Connects the saved site `site` (see the module docs). The app connects through
    /// [`connect_batch`](Self::connect_batch); this is for the tests.
    #[cfg(test)]
    pub async fn connect(&self, site: SiteId) -> ConnectResult {
        let (_, result) = self
            .connect_many(vec![site])
            .await
            .pop()
            .expect("one result per site");
        result
    }

    /// Connects several sites at once and returns when all are done. Their secrets are
    /// read with one [`SecretBatch`](super::vault::SecretBatch): a locked keyring raises at
    /// most one unlock prompt, and the sites after it ask for their passwords instead.
    /// Results come in the order of `sites`; a site missing from the vault is `NotFound`.
    #[cfg(test)]
    pub async fn connect_many(&self, sites: Vec<SiteId>) -> Vec<(SiteId, ConnectResult)> {
        futures_util::future::join_all(self.site_connects(sites, Caller::User).await).await
    }

    /// Open workspace: like [`connect_many`](Self::connect_many) (one secret batch, every
    /// site in parallel), but each site's result is sent on `results` as soon as that site
    /// finishes, so one slow or failing site never holds back the others. Returns once
    /// every result was sent (or the receiver is gone).
    pub async fn connect_batch(
        &self,
        sites: Vec<SiteId>,
        results: mpsc::UnboundedSender<(SiteId, ConnectResult)>,
    ) {
        let mut pending: FuturesUnordered<_> = self
            .site_connects(sites, Caller::User)
            .await
            .into_iter()
            .collect();
        while let Some(result) = pending.next().await {
            // A send fails only once the receiver is gone. In the app the receiver and this
            // future live in the same stream, so they are dropped together: the remaining
            // connects are then cancelled with it (not finished). A session already started
            // stays registered, `Disconnected` or connecting, for the next connect or close.
            let _ = results.send(result);
        }
    }

    /// Connects `site` for an agent's call: the one-site path of
    /// [`connect_batch`](Self::connect_batch). A session it starts is `agent_only` and its
    /// dialogs say an agent asked; a session the site already has is reused as it is (see
    /// the module docs). Opens no tab and takes no focus.
    pub async fn connect_for_agent(&self, site: SiteId) -> ConnectResult {
        let (_, result) =
            futures_util::future::join_all(self.site_connects(vec![site], Caller::Agent).await)
                .await
                .pop()
                .expect("one result per site");
        result
    }

    /// Reads the secrets of `sites` as one batch, then one connect future per site.
    async fn site_connects(
        &self,
        sites: Vec<SiteId>,
        caller: Caller,
    ) -> Vec<BoxFuture<'_, (SiteId, ConnectResult)>> {
        let read = match self.inner.vault.sites_with_secrets(sites.clone()).await {
            Ok(read) => read,
            Err(error) => {
                return sites
                    .into_iter()
                    .map(|site| {
                        let result = Err(ConnectError::before_start(error.clone()));
                        futures_util::future::ready((site, result)).boxed()
                    })
                    .collect();
            }
        };
        read.into_iter()
            .map(|(id, found)| {
                async move {
                    let result = match found {
                        Ok((site, secrets)) => self.connect_site(site, secrets, caller).await,
                        Err(error) => Err(ConnectError::before_start(error)),
                    };
                    (id, result)
                }
                .boxed()
            })
            .collect()
    }

    async fn connect_site(
        &self,
        site: Site,
        secrets: SiteSecrets,
        caller: Caller,
    ) -> ConnectResult {
        let site_lock = self.site_lock(&site.id);
        let _turn = site_lock.lock().await;
        let existing = {
            let mut sessions = lock(&self.inner.sessions);
            sessions
                .iter_mut()
                .find(|(_, entry)| entry.site == site.id)
                .map(|(id, entry)| {
                    let agent_only = entry.agent_only;
                    if caller == Caller::User && entry.agent_only {
                        // A tab takes the agent's session over: it is the user's now.
                        entry.agent_only = false;
                        entry.prompter.taken_over();
                    }
                    (
                        *id,
                        entry.link.clone(),
                        entry.keyring_unavailable,
                        agent_only,
                    )
                })
        };
        // An existing session keeps the secrets (and the flag) it was started with.
        let (id, keyring_unavailable) = match existing {
            Some((id, link, keyring_unavailable, agent_only)) => {
                // Does nothing unless `Disconnected`. An agent never retries a tab's
                // session: that is the user's to do, in the tab.
                if caller == Caller::User || agent_only {
                    link.reconnect_now().await;
                }
                (id, keyring_unavailable)
            }
            None => (
                self.start(&site, &secrets, caller),
                secrets.keyring_unavailable,
            ),
        };
        match self.ready(id).await {
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

    /// Starts the connection of `site` under a new session id; an agent's is
    /// `agent_only` and its dialogs say so.
    fn start(&self, site: &Site, secrets: &SiteSecrets, caller: Caller) -> SessionId {
        let id = SessionId(self.inner.next_id.fetch_add(1, Ordering::SeqCst));
        let mut prompter = self
            .inner
            .broker
            .session(id, site.id.clone(), &site.host, site.port);
        if caller == Caller::Agent {
            prompter = prompter.by_agent();
        }
        let config = &self.inner.config;
        let keepalive = self.keepalive();
        let reconnect = config.reconnect.clone().unwrap_or_default();
        let link = match site.protocol {
            Protocol::Ssh => {
                let mut ssh = ssh_config(site, secrets);
                if let Some(known_hosts) = &config.known_hosts {
                    ssh.known_hosts = known_hosts.clone();
                }
                ssh.keepalive_interval = keepalive;
                ssh.reconnect = reconnect.clone();
                if let Some(timeout) = config.connect_timeout {
                    ssh.connect_timeout = timeout;
                }
                Link::Ssh(SshSession::start(ssh, Arc::new(prompter.clone())))
            }
            Protocol::Ftp | Protocol::FtpsExplicit | Protocol::FtpsImplicit => {
                let mut ftp = ftp_config(site, secrets);
                ftp.transfer_connections = self.workers().clamp(1, 8);
                ftp.keepalive_interval = keepalive;
                ftp.reconnect = reconnect.clone();
                if let Some(timeout) = config.connect_timeout {
                    ftp.connect_timeout = timeout;
                }
                Link::Ftp(FtpFs::start(ftp, Arc::new(prompter.clone())))
            }
        };
        let label = SessionLabel::of(site, reconnect.max_attempts);
        self.insert(
            id,
            site.id.clone(),
            link,
            label,
            prompter,
            secrets.keyring_unavailable,
            caller,
        );
        id
    }

    #[allow(clippy::too_many_arguments)] // one call per kind of session
    fn insert(
        &self,
        id: SessionId,
        site: SiteId,
        link: Link,
        label: SessionLabel,
        prompter: SessionPrompter,
        keyring_unavailable: bool,
        caller: Caller,
    ) {
        let activity = Arc::new(Activity::new());
        // Subscribed before the entry exists: the monitor sees every state from the first.
        let monitor = spawn_monitor(label.clone(), link.clone(), activity.clone());
        let entry = Entry {
            site,
            link,
            label,
            activity,
            monitor: Some(monitor),
            prompter,
            keyring_unavailable,
            remote: None,
            watcher: None,
            agent_only: caller == Caller::Agent,
        };
        lock(&self.inner.sessions).insert(id, entry);
    }

    /// Waits for the session's first `Connected` (or its failure), then, once per session,
    /// builds its remote filesystem, registers the site and spawns its watcher. Called
    /// with the site's lock held.
    ///
    /// The outcome also settles the secrets typed with "Save in the system keyring" (see
    /// [`SessionPrompter::take_pending_secrets`]): `Connected` writes them to the keyring,
    /// a failure drops them.
    async fn ready(&self, id: SessionId) -> Result<()> {
        let (link, site, built, prompter, activity) = {
            let sessions = lock(&self.inner.sessions);
            let entry = sessions.get(&id).ok_or_else(closed)?;
            (
                entry.link.clone(),
                entry.site.clone(),
                entry.remote.is_some(),
                entry.prompter.clone(),
                entry.activity.clone(),
            )
        };
        let mut state = link.state();
        let outcome = state
            .wait_for(|s| {
                matches!(
                    s,
                    ConnectionState::Connected | ConnectionState::Disconnected(_)
                )
            })
            .await
            .map(|s| s.clone())
            .map_err(|_| closed())?;
        if let ConnectionState::Disconnected(error) = outcome {
            prompter.discard_pending_secrets();
            return Err(error);
        }
        let typed = {
            let sessions = lock(&self.inner.sessions);
            if !sessions.contains_key(&id) {
                // Closed while connecting: `close` dropped the secrets already.
                return Err(closed());
            }
            // Taken under the lock `close` removes the session with: a close that comes
            // first leaves nothing to save; one that comes after finds them gone. Either
            // way a closed session never stores what it took.
            prompter.take_pending_secrets()
        };
        self.save_typed_secrets(&site, typed).await;
        if built {
            return Ok(());
        }

        let workers = self.workers();
        let remote: Arc<dyn RemoteFs> = match &link {
            Link::Ssh(session) => {
                let sftp = SftpConfig {
                    stream_channels: workers.clamp(1, 8),
                };
                Arc::new(SftpFs::new(session.clone(), sftp).await?)
            }
            Link::Ftp(fs) => Arc::new(fs.clone()),
            #[cfg(test)]
            Link::Fake { .. } => self.test_remote(id)?,
        };
        let remote: Arc<dyn RemoteFs> = Arc::new(ActiveFs {
            inner: remote,
            activity,
        });
        let engine = &self.inner.engine;
        let endpoints = SiteEndpoints {
            local: self.inner.local.clone(),
            remote: remote.clone(),
            workers,
        };
        engine.register_site(site.clone(), endpoints).await;
        if lock(&self.inner.paused_by_close).remove(&site) {
            engine.resume_site(site.clone()).await;
        }
        let watcher = spawn_site_watcher(engine.clone(), site.clone(), link.connection());

        {
            let mut sessions = lock(&self.inner.sessions);
            if let Some(entry) = sessions.get_mut(&id) {
                entry.remote = Some(remote);
                entry.watcher = Some(watcher);
                return Ok(());
            }
        }
        // Closed while this registered the site (`close` may have paused it before the
        // registration and resume above): undo, so the site ends paused. No other session
        // of the site can exist meanwhile: this runs under the site's lock.
        watcher.abort();
        lock(&self.inner.paused_by_close).insert(site.clone());
        engine.pause_site(site).await;
        Err(closed())
    }

    /// Writes the secrets the user typed with "Save in the system keyring" now that the
    /// session connected with them. A failure is logged: the session works anyway, and
    /// the next connect asks again.
    async fn save_typed_secrets(
        &self,
        site: &SiteId,
        typed: Vec<(hoprs_vault::SecretKind, secrecy::SecretString)>,
    ) {
        for (kind, secret) in typed {
            let saved = self
                .inner
                .vault
                .set_secret(site.clone(), kind, secret)
                .await;
            if let Err(err) = saved {
                tracing::warn!(site = %site, secret = kind.account(), error = %err, "cannot save the typed secret in the keyring");
            }
        }
    }

    /// Retries a session whose connection is `Disconnected` (first connection included)
    /// and waits for it like `connect`.
    ///
    /// The request and its outcome are logged at `info` (the states in between by the
    /// session's monitor).
    pub async fn reconnect(&self, id: SessionId) -> Result<()> {
        let (site, link, label) = {
            let sessions = lock(&self.inner.sessions);
            let entry = sessions.get(&id).ok_or_else(closed)?;
            (entry.site.clone(), entry.link.clone(), entry.label.clone())
        };
        tracing::info!(
            site = ?label.name,
            protocol = %protocol_name(label.protocol),
            endpoint = %label.endpoint,
            "reconnect requested"
        );
        let site_lock = self.site_lock(&site);
        let _turn = site_lock.lock().await;
        link.reconnect_now().await;
        let result = self.ready(id).await;
        let outcome = match &result {
            Ok(()) => "reconnected".to_owned(),
            Err(error) => format!("failed: {}", reason_text(error)),
        };
        tracing::info!(
            site = ?label.name,
            protocol = %protocol_name(label.protocol),
            endpoint = %label.endpoint,
            outcome = ?outcome,
            "reconnect finished"
        );
        result
    }

    /// Closes a session: closes its shells, refuses its prompts, aborts its site watcher,
    /// pauses its site in the engine (the next connect of the site resumes it), then
    /// disconnects and drops the connection. Unknown ids are ignored.
    ///
    /// It does not take the site's lock: a connect waiting on this session's prompts or
    /// first connection holds it, and closing is what ends that wait. A `ready` still
    /// running for this session notices the close and pauses the site again itself.
    pub async fn close(&self, id: SessionId) {
        self.close_where(id, |_| true).await;
    }

    /// Closes the session of `site` if it is `agent_only` (no tab uses it); a tab's
    /// session is left alone. Whether a session was closed.
    pub async fn close_agent_session(&self, site: &SiteId) -> bool {
        let Some(id) = self.session_of(site) else {
            return false;
        };
        // Checked again under the lock that removes it: a tab taking the session over in
        // between keeps it.
        self.close_where(id, |entry| entry.agent_only).await
    }

    /// [`close`](Self::close), if the session exists and `closes` says so (read under
    /// the sessions lock that removes it). Whether it was closed.
    async fn close_where(&self, id: SessionId, closes: impl FnOnce(&Entry) -> bool) -> bool {
        let (entry, replaced) = {
            let mut sessions = lock(&self.inner.sessions);
            if !sessions.get(&id).is_some_and(closes) {
                return false;
            }
            let Some(entry) = sessions.remove(&id) else {
                return false;
            };
            let replaced = sessions.values().any(|other| other.site == entry.site);
            (entry, replaced)
        };
        // A shell `open_shell` registers is registered under the sessions lock while the
        // session still exists, so none can appear after this.
        self.inner.shells.close_session(id);
        entry.prompter.cancel();
        entry.prompter.discard_pending_secrets();
        if let Some(watcher) = &entry.watcher {
            watcher.abort();
        }
        // Closed by the user: nothing to log or reconnect any more (the flag covers a
        // monitor that is running right now, the abort the rest).
        entry.activity.mark_closed();
        if let Some(monitor) = &entry.monitor {
            monitor.abort();
        }
        if !replaced {
            lock(&self.inner.paused_by_close).insert(entry.site.clone());
            self.inner.engine.pause_site(entry.site.clone()).await;
        }
        entry.link.disconnect().await;
        drop(entry);
        true
    }

    /// The sites whose session only an agent uses, sorted.
    pub fn agent_sites(&self) -> Vec<SiteId> {
        let mut sites: Vec<SiteId> = lock(&self.inner.sessions)
            .values()
            .filter(|entry| entry.agent_only)
            .map(|entry| entry.site.clone())
            .collect();
        sites.sort_by(|a, b| a.0.cmp(&b.0));
        sites.dedup();
        sites
    }

    /// The session of `site`, in any state.
    pub(crate) fn session_of(&self, site: &SiteId) -> Option<SessionId> {
        lock(&self.inner.sessions)
            .iter()
            .find(|(_, entry)| entry.site == *site)
            .map(|(id, _)| *id)
    }

    /// The session of `site` while it is connected and its remote filesystem is built.
    pub fn connected_session(&self, site: &SiteId) -> Option<SessionId> {
        lock(&self.inner.sessions)
            .iter()
            .find(|(_, entry)| {
                entry.site == *site
                    && entry.remote.is_some()
                    && matches!(*entry.link.state().borrow(), ConnectionState::Connected)
            })
            .map(|(id, _)| *id)
    }

    /// Runs `command` on its own exec channel of the SSH session `id` (see
    /// [`SshSession::exec`]); the session counts as used. Not for FTP sessions.
    pub async fn exec(
        &self,
        id: SessionId,
        command: String,
        limits: ExecLimits,
    ) -> Result<ExecOutput> {
        let link = {
            let sessions = lock(&self.inner.sessions);
            sessions.get(&id).ok_or_else(closed)?.link.clone()
        };
        let Link::Ssh(session) = link else {
            return Err(Error::new(ErrorKind::Protocol, "Commands need an SSH site"));
        };
        self.touch(id);
        let output = session.exec(&command, limits).await;
        // A long command: the session was in use until now.
        self.touch(id);
        output
    }

    /// Opens a terminal of `cols` x `rows` on the SSH session `id` (see
    /// [`BackendApi::open_shell`](super::BackendApi::open_shell)).
    pub async fn open_shell(&self, id: SessionId, cols: u16, rows: u16) -> Result<ShellId> {
        let link = {
            let sessions = lock(&self.inner.sessions);
            sessions.get(&id).ok_or_else(closed)?.link.clone()
        };
        let Link::Ssh(session) = link else {
            return Err(Error::coded(
                ErrorKind::Protocol,
                "session-terminal-needs-ssh",
                "Terminals need an SSH site",
            ));
        };
        let request = PtyRequest {
            cols: cols.into(),
            rows: rows.into(),
            ..PtyRequest::default()
        };
        self.touch(id);
        let (handle, output) = session.open_shell(request).await?.into_output();
        let sessions = lock(&self.inner.sessions);
        if !sessions.contains_key(&id) {
            // Closed while the shell opened.
            handle.close();
            return Err(closed());
        }
        Ok(self
            .inner
            .shells
            .insert(id, Arc::new(handle), output.boxed()))
    }

    /// The terminal shells of the sessions.
    pub fn shells(&self) -> &Shells {
        &self.inner.shells
    }

    /// Sends keyboard input to a shell; its session counts as used.
    pub async fn shell_input(&self, shell: ShellId, data: Vec<u8>) -> Result<()> {
        if let Some(session) = self.inner.shells.session_of(shell) {
            self.touch(session);
        }
        self.inner.shells.input(shell, data).await
    }

    /// The session was just used (it is not idle).
    fn touch(&self, id: SessionId) {
        if let Some(entry) = lock(&self.inner.sessions).get(&id) {
            entry.activity.touch();
        }
    }

    /// Closes every session (the window is closing).
    pub async fn close_all(&self) {
        let mut ids: Vec<SessionId> = lock(&self.inner.sessions).keys().copied().collect();
        ids.sort();
        futures_util::future::join_all(ids.into_iter().map(|id| self.close(id))).await;
    }

    /// How an SSH session logged in last; `None` for FTP and before the first connection.
    pub fn auth(&self, id: SessionId) -> Option<hoprs_ssh::AuthMethod> {
        match &lock(&self.inner.sessions).get(&id)?.link {
            Link::Ssh(session) => session.auth_method(),
            Link::Ftp(_) => None,
            #[cfg(test)]
            Link::Fake { .. } => None,
        }
    }

    /// The remote filesystem of a connected session.
    pub fn remote(&self, id: SessionId) -> Result<Arc<dyn RemoteFs>> {
        let sessions = lock(&self.inner.sessions);
        let entry = sessions.get(&id).ok_or_else(closed)?;
        entry
            .remote
            .clone()
            .ok_or_else(|| Error::coded(ErrorKind::Network, "net-not-connected", "Not connected"))
    }

    /// The site of a session.
    pub fn site(&self, id: SessionId) -> Option<SiteId> {
        lock(&self.inner.sessions)
            .get(&id)
            .map(|entry| entry.site.clone())
    }

    /// The protocol of a session's site.
    pub fn protocol(&self, id: SessionId) -> Option<Protocol> {
        lock(&self.inner.sessions)
            .get(&id)
            .map(|entry| entry.label.protocol)
    }

    /// The `known_hosts` file SSH sessions check: the configured one, else
    /// `~/.ssh/known_hosts`.
    pub fn known_hosts_path(&self) -> PathBuf {
        self.inner.config.known_hosts_path()
    }

    /// The local filesystem every session transfers with.
    pub fn local(&self) -> Arc<dyn RemoteFs> {
        self.inner.local.clone()
    }

    /// Watches a session's connection state.
    pub fn state(&self, id: SessionId) -> Option<watch::Receiver<ConnectionState>> {
        lock(&self.inner.sessions)
            .get(&id)
            .map(|entry| entry.link.state())
    }

    fn site_lock(&self, site: &SiteId) -> Arc<tokio::sync::Mutex<()>> {
        lock(&self.inner.site_locks)
            .entry(site.clone())
            .or_default()
            .clone()
    }

    /// Adds a session over a test connection and `remote`, then readies it like `connect`.
    #[cfg(test)]
    pub(crate) async fn attach(
        &self,
        site: SiteId,
        conn: Arc<dyn Connection>,
        remote: Arc<dyn RemoteFs>,
    ) -> Result<SessionId> {
        self.attach_as(site, conn, remote, Caller::User).await
    }

    /// [`attach`](Self::attach) as [`connect_for_agent`](Self::connect_for_agent) would
    /// start it: `agent_only`, its dialogs flagged.
    #[cfg(test)]
    pub(crate) async fn attach_for_agent(
        &self,
        site: SiteId,
        conn: Arc<dyn Connection>,
        remote: Arc<dyn RemoteFs>,
    ) -> Result<SessionId> {
        self.attach_as(site, conn, remote, Caller::Agent).await
    }

    #[cfg(test)]
    async fn attach_as(
        &self,
        site: SiteId,
        conn: Arc<dyn Connection>,
        remote: Arc<dyn RemoteFs>,
        caller: Caller,
    ) -> Result<SessionId> {
        let mut test_site = Site::new("test", "test", "u", Protocol::Ftp);
        test_site.id = site;
        test_site.port = 0;
        self.attach_site_as(&test_site, conn, remote, caller).await
    }

    /// [`attach`](Self::attach) for a given saved site (what the connection log names).
    #[cfg(test)]
    async fn attach_site(
        &self,
        site: &Site,
        conn: Arc<dyn Connection>,
        remote: Arc<dyn RemoteFs>,
    ) -> Result<SessionId> {
        self.attach_site_as(site, conn, remote, Caller::User).await
    }

    #[cfg(test)]
    async fn attach_site_as(
        &self,
        site: &Site,
        conn: Arc<dyn Connection>,
        remote: Arc<dyn RemoteFs>,
        caller: Caller,
    ) -> Result<SessionId> {
        let id = SessionId(self.inner.next_id.fetch_add(1, Ordering::SeqCst));
        let mut prompter = self
            .inner
            .broker
            .session(id, site.id.clone(), &site.host, site.port);
        if caller == Caller::Agent {
            prompter = prompter.by_agent();
        }
        let link = Link::Fake {
            conn,
            reconnects: Arc::new(AtomicUsize::new(0)),
        };
        let attempts = self.inner.config.reconnect.clone().unwrap_or_default();
        let label = SessionLabel::of(site, attempts.max_attempts);
        self.insert(id, site.id.clone(), link, label, prompter, false, caller);
        lock(&self.inner.test_remotes).insert(id, remote);
        let site_lock = self.site_lock(&site.id);
        let _turn = site_lock.lock().await;
        self.ready(id).await?;
        Ok(id)
    }

    /// How many times a test session's `reconnect_now` was called.
    #[cfg(test)]
    fn fake_reconnects(&self, id: SessionId) -> usize {
        match &lock(&self.inner.sessions).get(&id).map(|e| e.link.clone()) {
            Some(Link::Fake { reconnects, .. }) => reconnects.load(Ordering::SeqCst),
            _ => 0,
        }
    }

    /// The prompter of a session (tests answer its prompts directly).
    #[cfg(test)]
    pub(crate) fn prompter(&self, id: SessionId) -> Option<SessionPrompter> {
        lock(&self.inner.sessions)
            .get(&id)
            .map(|entry| entry.prompter.clone())
    }

    #[cfg(test)]
    fn test_remote(&self, id: SessionId) -> Result<Arc<dyn RemoteFs>> {
        lock(&self.inner.test_remotes)
            .remove(&id)
            .ok_or_else(closed)
    }
}

impl SessionLabel {
    fn of(site: &Site, attempts: u32) -> Self {
        let protocol = site.protocol;
        Self {
            name: site.name.clone(),
            protocol,
            endpoint: format!("{}:{}", site.host, site.port),
            attempts,
        }
    }
}

/// A session's remote filesystem that records when it is used (see [`Activity`]).
struct ActiveFs {
    inner: Arc<dyn RemoteFs>,
    activity: Arc<Activity>,
}

#[async_trait::async_trait]
impl RemoteFs for ActiveFs {
    async fn list(&self, path: &hoprs_core::RemotePath) -> Result<Vec<hoprs_core::RemoteEntry>> {
        self.activity.touch();
        self.inner.list(path).await
    }

    async fn stat(&self, path: &hoprs_core::RemotePath) -> Result<hoprs_core::RemoteEntry> {
        self.activity.touch();
        self.inner.stat(path).await
    }

    async fn size(&self, path: &hoprs_core::RemotePath) -> Result<u64> {
        self.activity.touch();
        self.inner.size(path).await
    }

    async fn open_read(
        &self,
        path: &hoprs_core::RemotePath,
        offset: u64,
    ) -> Result<hoprs_remote::BoxAsyncRead> {
        self.activity.touch();
        self.inner.open_read(path, offset).await
    }

    async fn open_write(
        &self,
        path: &hoprs_core::RemotePath,
        offset: u64,
    ) -> Result<hoprs_remote::BoxAsyncWrite> {
        self.activity.touch();
        self.inner.open_write(path, offset).await
    }

    async fn mkdir(&self, path: &hoprs_core::RemotePath) -> Result<()> {
        self.activity.touch();
        self.inner.mkdir(path).await
    }

    async fn rename(
        &self,
        from: &hoprs_core::RemotePath,
        to: &hoprs_core::RemotePath,
    ) -> Result<()> {
        self.activity.touch();
        self.inner.rename(from, to).await
    }

    async fn remove_file(&self, path: &hoprs_core::RemotePath) -> Result<()> {
        self.activity.touch();
        self.inner.remove_file(path).await
    }

    async fn remove_dir(&self, path: &hoprs_core::RemotePath) -> Result<()> {
        self.activity.touch();
        self.inner.remove_dir(path).await
    }

    async fn chmod(&self, path: &hoprs_core::RemotePath, mode: u32) -> Result<()> {
        self.activity.touch();
        self.inner.chmod(path, mode).await
    }

    async fn set_modified(
        &self,
        path: &hoprs_core::RemotePath,
        mtime: std::time::SystemTime,
    ) -> Result<()> {
        self.activity.touch();
        self.inner.set_modified(path, mtime).await
    }

    fn mtime_precision(&self) -> Duration {
        self.inner.mtime_precision()
    }
}

/// Starts the session's monitor (see the module docs): it logs every state change and
/// starts one more reconnection round, quietly, when the transport gives up on a session
/// that was connected (a network failure only).
///
/// It holds a clone of the session's link, which keeps the connection (and so its state
/// channel) alive: it does not end by itself. `close` aborts it (and `close_all` closes
/// every session when the window closes).
fn spawn_monitor(label: SessionLabel, link: Link, activity: Arc<Activity>) -> JoinHandle<()> {
    let mut state = link.state();
    let conn = link.connection();
    tokio::spawn(async move {
        let mut current = state.borrow_and_update().clone();
        log_transition(
            &label,
            &current,
            conn.last_error(),
            activity.since_last_request(),
        );
        // One quiet round per connection: spent by a retry, back once connected.
        let mut retry_left = true;
        while state.changed().await.is_ok() {
            let next = state.borrow_and_update().clone();
            if next == current {
                continue;
            }
            let previous = std::mem::replace(&mut current, next);
            let since = activity.since_last_request();
            log_transition(&label, &current, conn.last_error(), since);
            match &current {
                ConnectionState::Connected => retry_left = true,
                ConnectionState::Disconnected(error)
                    if retry_left
                        && conn.generation() > 0
                        && gave_up_on_network(&previous, error, &activity) =>
                {
                    retry_left = false;
                    tracing::info!(
                        site = ?label.name,
                        protocol = %protocol_name(label.protocol),
                        endpoint = %label.endpoint,
                        since_last_request_secs = since.as_secs(),
                        "reconnection given up; trying one more round"
                    );
                    // Without the site's lock: `reconnect_now` only starts a round when the
                    // session is `Disconnected` and is ignored while one runs, so racing
                    // `SessionManager::reconnect` (which holds the lock for its own wait)
                    // starts at most one round; neither waits on the other.
                    link.reconnect_now().await;
                }
                _ => {}
            }
        }
    })
}

/// Whether the transport gave up on the session for a network failure, worth one more
/// round: it was reconnecting (a whole round failed), the error is a network one (not a
/// host key, authentication or TLS refusal), and `close` is not closing it.
fn gave_up_on_network(previous: &ConnectionState, error: &Error, activity: &Activity) -> bool {
    matches!(previous, ConnectionState::Reconnecting { .. })
        && error.kind == ErrorKind::Network
        && !activity.is_closed()
}

/// The protocol as the connection log names it.
fn protocol_name(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Ssh => "ssh",
        Protocol::Ftp => "ftp",
        Protocol::FtpsExplicit => "ftps-explicit",
        Protocol::FtpsImplicit => "ftps-implicit",
    }
}

/// `kind: message (detail)`; never holds a secret (no error does).
fn reason_text(error: &Error) -> String {
    match &error.detail {
        Some(detail) => format!("{}: {} ({detail})", error.kind.as_str(), error.message),
        None => format!("{}: {}", error.kind.as_str(), error.message),
    }
}

/// Logs one state change of a session (see the module docs). `last_error` is the
/// transport's reason for a `Reconnecting` state; `Disconnected` carries its own.
fn log_transition(
    label: &SessionLabel,
    state: &ConnectionState,
    last_error: Option<Error>,
    since_last_request: Duration,
) {
    let (name, reason) = match state {
        ConnectionState::Connecting => ("connecting".to_owned(), None),
        ConnectionState::Connected => ("connected".to_owned(), None),
        ConnectionState::Reconnecting { attempt } => (
            format!("reconnecting {attempt}/{}", label.attempts),
            last_error,
        ),
        ConnectionState::WaitingForUser => ("waiting for the user".to_owned(), None),
        ConnectionState::Disconnected(error) => ("disconnected".to_owned(), Some(error.clone())),
    };
    let since_last_request_secs = since_last_request.as_secs();
    if matches!(state, ConnectionState::Reconnecting { attempt: 1 })
        && let Some(error) = &reason
        && error.detail.as_deref() == Some(hoprs_core::KEEPALIVE_UNANSWERED)
    {
        tracing::warn!(
            site = ?label.name,
            protocol = %protocol_name(label.protocol),
            endpoint = %label.endpoint,
            since_last_request_secs,
            "keepalive not answered"
        );
    }
    let reason = reason.as_ref().map_or_else(|| "-".to_owned(), reason_text);
    tracing::info!(
        site = ?label.name,
        protocol = %protocol_name(label.protocol),
        endpoint = %label.endpoint,
        state = ?name,
        reason = ?reason,
        since_last_request_secs,
        "connection state changed"
    );
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures_util::stream::BoxStream;
    use hoprs_core::{EntryKind, RemotePath};
    use hoprs_remote::MemFs;
    use hoprs_ssh::ShellOutput;
    use hoprs_transfer::{EngineConfig, QueueStore};
    use hoprs_vault::{MemoryStore, SecretKind, Vault};
    use secrecy::SecretString;

    use super::super::broker::{BrokerEvent, PromptAnswer, PromptKind};
    use super::*;

    #[tokio::test]
    async fn active_fs_forwards_the_mtime_precision() {
        struct Minutes(MemFs);
        #[async_trait::async_trait]
        impl RemoteFs for Minutes {
            async fn list(&self, p: &RemotePath) -> Result<Vec<hoprs_core::RemoteEntry>> {
                self.0.list(p).await
            }
            async fn stat(&self, p: &RemotePath) -> Result<hoprs_core::RemoteEntry> {
                self.0.stat(p).await
            }
            async fn open_read(
                &self,
                p: &RemotePath,
                o: u64,
            ) -> Result<hoprs_remote::BoxAsyncRead> {
                self.0.open_read(p, o).await
            }
            async fn open_write(
                &self,
                p: &RemotePath,
                o: u64,
            ) -> Result<hoprs_remote::BoxAsyncWrite> {
                self.0.open_write(p, o).await
            }
            async fn mkdir(&self, p: &RemotePath) -> Result<()> {
                self.0.mkdir(p).await
            }
            async fn rename(&self, a: &RemotePath, b: &RemotePath) -> Result<()> {
                self.0.rename(a, b).await
            }
            async fn remove_file(&self, p: &RemotePath) -> Result<()> {
                self.0.remove_file(p).await
            }
            async fn remove_dir(&self, p: &RemotePath) -> Result<()> {
                self.0.remove_dir(p).await
            }
            async fn chmod(&self, p: &RemotePath, m: u32) -> Result<()> {
                self.0.chmod(p, m).await
            }
            fn mtime_precision(&self) -> Duration {
                Duration::from_secs(60)
            }
        }
        let fs = ActiveFs {
            inner: Arc::new(Minutes(MemFs::new())),
            activity: Arc::new(Activity::new()),
        };
        assert_eq!(fs.mtime_precision(), Duration::from_secs(60));
    }

    struct FakeConn {
        state: watch::Sender<ConnectionState>,
        last_error: Mutex<Option<Error>>,
    }

    impl FakeConn {
        fn new(state: watch::Sender<ConnectionState>) -> Self {
            Self {
                state,
                last_error: Mutex::new(None),
            }
        }

        /// The connection ends with `error`: the transport starts reconnecting.
        fn lose(&self, error: Error) {
            *self.last_error.lock().unwrap() = Some(error);
            self.state
                .send_replace(ConnectionState::Reconnecting { attempt: 1 });
        }
    }

    impl Connection for FakeConn {
        fn state(&self) -> watch::Receiver<ConnectionState> {
            self.state.subscribe()
        }

        fn generation(&self) -> u64 {
            1
        }

        fn last_error(&self) -> Option<Error> {
            self.last_error.lock().unwrap().clone()
        }
    }

    /// Log lines written while a [`capture_logs`] guard lives (this thread's tasks).
    #[derive(Clone, Default)]
    struct LogBuf(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for LogBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl LogBuf {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
        }
    }

    /// What [`capture_logs`] keeps alive.
    struct LogCapture {
        _default: tracing::subscriber::DefaultGuard,
        _other: tracing::Dispatch,
    }

    /// Captures what the app logs at the default level (`info`) on this thread.
    ///
    /// With a single dispatcher registered, `tracing` caches a callsite's interest from
    /// whichever thread hits it first: another test's thread (no subscriber) would make it
    /// "never" for this one too. A second, silent dispatcher makes every callsite ask the
    /// current thread's subscriber instead.
    fn capture_logs() -> (LogBuf, LogCapture) {
        let buf = LogBuf::default();
        let writer = buf.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .finish();
        let capture = LogCapture {
            _default: tracing::subscriber::set_default(subscriber),
            _other: tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()),
        };
        (buf, capture)
    }

    /// Lets the monitor tasks see the latest state.
    async fn settle() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    struct Fixture {
        manager: SessionManager,
        broker: PromptBroker,
        vault: VaultHandle,
        dir: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        fixture_with(Arc::new(MemoryStore::new()))
    }

    fn fixture_with(store: Arc<dyn hoprs_vault::SecretStore>) -> Fixture {
        fixture_config(
            store,
            SessionConfig {
                workers: 2,
                ..SessionConfig::default()
            },
        )
    }

    fn fixture_config(store: Arc<dyn hoprs_vault::SecretStore>, config: SessionConfig) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path(), store).unwrap();
        let vault = VaultHandle::new(vault);
        let broker = PromptBroker::new(vault.clone());
        let engine = TransferEngine::start(
            QueueStore::open_in_memory().unwrap(),
            EngineConfig::default(),
        )
        .unwrap();
        let config = SessionConfig {
            known_hosts: Some(dir.path().join("known_hosts")),
            ..config
        };
        let manager = SessionManager::new(engine, vault.clone(), broker.clone(), config);
        Fixture {
            manager,
            broker,
            vault,
            dir,
        }
    }

    async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
        for _ in 0..200 {
            if check() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("timed out waiting for {what}");
    }

    #[tokio::test]
    async fn close_aborts_watcher() {
        let f = fixture();
        let (state, _) = watch::channel(ConnectionState::Connected);
        let conn = Arc::new(FakeConn::new(state));
        let remote = MemFs::new();
        remote.add_dir("/srv");
        let site = SiteId::new();
        let id = f
            .manager
            .attach(site.clone(), conn.clone(), Arc::new(remote))
            .await
            .unwrap();
        let listed = f
            .manager
            .remote(id)
            .unwrap()
            .list(&RemotePath::root())
            .await
            .unwrap();
        assert_eq!(listed[0].kind, EntryKind::Dir);
        // The session and its watcher hold the connection.
        assert!(
            Arc::strong_count(&conn) >= 3,
            "{}",
            Arc::strong_count(&conn)
        );
        assert!(f.manager.state(id).is_some());

        f.manager.close(id).await;
        eventually("the watcher to drop the connection", || {
            Arc::strong_count(&conn) == 1
        })
        .await;
        assert!(f.manager.remote(id).is_err());
        assert!(f.manager.state(id).is_none());
        // Closing twice is harmless.
        f.manager.close(id).await;
    }

    /// Only SSH sessions have terminals; a closed session has none.
    #[tokio::test]
    async fn open_shell_needs_an_ssh_session() {
        let f = fixture();
        let (state, _) = watch::channel(ConnectionState::Connected);
        let id = f
            .manager
            .attach(
                SiteId::new(),
                Arc::new(FakeConn::new(state)),
                Arc::new(MemFs::new()),
            )
            .await
            .unwrap();
        let e = f.manager.open_shell(id, 80, 24).await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Protocol);
        assert_eq!(e.message, "Terminals need an SSH site");
        f.manager.close(id).await;
        assert_eq!(
            f.manager.open_shell(id, 80, 24).await.unwrap_err(),
            closed()
        );
    }

    /// The window is closing: every session goes, its watch with it.
    #[tokio::test]
    async fn close_all_closes_every_session() {
        let f = fixture();
        let mut ids = Vec::new();
        let mut conns = Vec::new();
        for _ in 0..2 {
            let (state, _) = watch::channel(ConnectionState::Connected);
            let conn = Arc::new(FakeConn::new(state));
            let id = f
                .manager
                .attach(SiteId::new(), conn.clone(), Arc::new(MemFs::new()))
                .await
                .unwrap();
            ids.push(id);
            conns.push(conn);
        }
        assert!(ids.iter().all(|id| f.manager.state(*id).is_some()));
        // Not an SSH session: no login method to tell.
        assert_eq!(f.manager.auth(ids[0]), None);
        f.manager.close_all().await;
        assert!(ids.iter().all(|id| f.manager.state(*id).is_none()));
        assert!(f.manager.remote(ids[1]).is_err());
        for conn in conns {
            eventually("the watcher to drop the connection", || {
                Arc::strong_count(&conn) == 1
            })
            .await;
        }
        assert_eq!(f.manager.auth(SessionId(999)), None);
    }

    /// Every connection state change is logged at `info` with the site, protocol,
    /// `host:port`, the new state, the reason and the idle time; an unanswered keepalive
    /// also at `warn`. Nothing else about the session (no secret) is in the lines.
    #[tokio::test]
    async fn connection_transitions_are_logged_at_info() {
        let (logs, _guard) = capture_logs();
        let f = fixture();
        let (state, _) = watch::channel(ConnectionState::Connected);
        let conn = Arc::new(FakeConn::new(state));
        let mut site = Site::new("Shop files", "ftp.shop.example", "deploy", Protocol::Ftp);
        site.port = 2121;
        let remote = MemFs::new();
        remote.add_dir("/srv");
        let id = f
            .manager
            .attach_site(&site, conn.clone(), Arc::new(remote))
            .await
            .unwrap();
        f.manager
            .remote(id)
            .unwrap()
            .list(&RemotePath::root())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(1100)).await;

        // Idle for a second, the server stops answering: the keepalive gives up.
        conn.lose(
            Error::new(ErrorKind::Network, "Connection lost")
                .with_detail(hoprs_core::KEEPALIVE_UNANSWERED),
        );
        for _ in 0..200 {
            if logs.text().contains("reconnecting 1/5") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            logs.text().contains("reconnecting 1/5"),
            "the drop in the log: {}",
            logs.text()
        );
        conn.state.send_replace(ConnectionState::Connected);
        for _ in 0..200 {
            if logs.text().matches("state=\"connected\"").count() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            logs.text().matches("state=\"connected\"").count(),
            2,
            "{}",
            logs.text()
        );

        let text = logs.text();
        let dropped = text
            .lines()
            .find(|l| l.contains("reconnecting 1/5"))
            .unwrap();
        for expected in [
            "INFO",
            "connection state changed",
            "site=\"Shop files\"",
            "protocol=ftp",
            "endpoint=ftp.shop.example:2121",
            "state=\"reconnecting 1/5\"",
            "reason=\"network: Connection lost (no reply to the keepalive)\"",
        ] {
            assert!(dropped.contains(expected), "{expected:?} in {dropped}");
        }
        // About a second since the listing (a slow machine may round up).
        let since: u64 = dropped
            .split("since_last_request_secs=")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("no since_last_request_secs in {dropped}"));
        assert!((1..=3).contains(&since), "{dropped}");
        let warned = text
            .lines()
            .find(|l| l.contains("WARN"))
            .expect("a warning for the keepalive");
        assert!(warned.contains("keepalive not answered"), "{warned}");
        assert!(warned.contains("site=\"Shop files\""), "{warned}");
        let back = text
            .lines()
            .rfind(|l| l.contains("state=\"connected\""))
            .unwrap();
        assert!(back.contains("INFO"), "{back}");
        assert!(back.contains("endpoint=ftp.shop.example:2121"), "{back}");
        assert!(!text.contains("deploy"), "no user name: {text}");
    }

    /// The transport gives up while nobody uses the site: the manager starts one more round
    /// by itself, quietly. Once only, until the session is connected again; never after a
    /// refusal (authentication, host key, certificate).
    #[tokio::test]
    async fn idle_drop_reconnects_once_quietly() {
        let f = fixture();
        let (state, _) = watch::channel(ConnectionState::Connected);
        let conn = Arc::new(FakeConn::new(state));
        let id = f
            .manager
            .attach(SiteId::new(), conn.clone(), Arc::new(MemFs::new()))
            .await
            .unwrap();
        let lost = Error::new(ErrorKind::Network, "Connection lost");
        let give_up = |error: Error| {
            let conn = conn.clone();
            async move {
                conn.lose(Error::new(ErrorKind::Network, "Connection lost"));
                settle().await;
                conn.state
                    .send_replace(ConnectionState::Disconnected(error));
                settle().await;
            }
        };
        let back = || {
            let conn = conn.clone();
            async move {
                conn.state.send_replace(ConnectionState::Connected);
                settle().await;
            }
        };

        give_up(lost.clone()).await;
        assert_eq!(f.manager.fake_reconnects(id), 1, "one quiet round");
        give_up(lost.clone()).await;
        assert_eq!(
            f.manager.fake_reconnects(id),
            1,
            "then it waits for the user"
        );
        back().await;
        give_up(lost.clone()).await;
        assert_eq!(
            f.manager.fake_reconnects(id),
            2,
            "connected again: one more"
        );

        for refused in [
            Error::new(ErrorKind::Auth, "Authentication failed"),
            hoprs_ssh::testing::changed_host_key_error("test", 1),
            Error::new(ErrorKind::Tls, "Certificate not trusted"),
        ] {
            back().await;
            give_up(refused.clone()).await;
            assert_eq!(f.manager.fake_reconnects(id), 2, "{refused:?}");
        }
        f.manager.close(id).await;
    }

    /// A saved FTP site the fixture's vault knows, for the user's connect path.
    async fn saved_site(f: &Fixture) -> Site {
        let mut site = Site::new("agent", "127.0.0.1", "u", Protocol::Ftp);
        site.port = 1;
        f.vault.upsert_site(site.clone()).await.unwrap();
        site
    }

    fn connected_link() -> Arc<FakeConn> {
        let (state, _) = watch::channel(ConnectionState::Connected);
        Arc::new(FakeConn::new(state))
    }

    /// A session an agent opened is the agent's until the user connects the same site:
    /// then it is the tab's (same session), and no longer listed as the agent's.
    #[tokio::test]
    async fn agent_session_is_marked_and_a_tab_takes_it_over() {
        let f = fixture();
        let site = saved_site(&f).await;
        let id = f
            .manager
            .attach_for_agent(site.id.clone(), connected_link(), Arc::new(MemFs::new()))
            .await
            .unwrap();
        assert_eq!(f.manager.agent_sites(), std::slice::from_ref(&site.id));
        assert_eq!(f.manager.session_of(&site.id), Some(id));

        let mut events = f.broker.take_events().unwrap();
        let ask = |prompter: SessionPrompter| {
            tokio::spawn(async move {
                hoprs_ssh::Prompter::password(&prompter, "u", "h").await;
            })
        };
        let asked = ask(f.manager.prompter(id).unwrap());
        let Some(BrokerEvent::Request(request)) = events.recv().await else {
            panic!("a dialog");
        };
        assert!(request.by_agent, "the agent's session says so");
        f.broker.answer(request.id, PromptAnswer::Cancel);
        asked.await.unwrap();

        let taken = f.manager.connect(site.id.clone()).await.unwrap();
        assert_eq!(taken.session, id, "the same session");
        assert!(f.manager.agent_sites().is_empty());
        // Its dialogs are the user's now.
        let asked = ask(f.manager.prompter(id).unwrap());
        let Some(BrokerEvent::Request(request)) = events.recv().await else {
            panic!("a dialog");
        };
        assert!(!request.by_agent);
        f.broker.answer(request.id, PromptAnswer::Cancel);
        asked.await.unwrap();
        // An agent's connect afterwards reuses it as it is: still the tab's.
        let again = f.manager.connect_for_agent(site.id.clone()).await.unwrap();
        assert_eq!(again.session, id);
        assert!(f.manager.agent_sites().is_empty());
    }

    /// A port nothing listens on: bound, then released (a refused connection, at once, on
    /// every system).
    fn closed_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    /// The production path: an agent's connect of a site with no session starts one that
    /// is the agent's, and its dialogs say so.
    #[tokio::test]
    async fn connect_for_agent_starts_an_agent_session() {
        let f = fixture();
        let mut site = Site::new("agent", "127.0.0.1", "u", Protocol::Ftp);
        site.port = closed_port();
        f.vault.upsert_site(site.clone()).await.unwrap();
        let mut events = f.broker.take_events().unwrap();
        let broker = f.broker.clone();
        let flags = Arc::new(Mutex::new(Vec::new()));
        let answer = tokio::spawn({
            let flags = flags.clone();
            async move {
                while let Some(event) = events.recv().await {
                    if let BrokerEvent::Request(r) = event {
                        flags.lock().unwrap().push(r.by_agent);
                        broker.answer(r.id, PromptAnswer::Cancel);
                    }
                }
            }
        });

        let err = f
            .manager
            .connect_for_agent(site.id.clone())
            .await
            .unwrap_err();
        let id = err.session.expect("the session is kept");
        assert_eq!(f.manager.agent_sites(), std::slice::from_ref(&site.id));
        // Its own connection's dialog (the password) is flagged...
        let seen = flags.lock().unwrap().clone();
        assert!(!seen.is_empty() && seen.iter().all(|b| *b), "{seen:?}");
        // ...and so is any later one of the session.
        flags.lock().unwrap().clear();
        let prompter = f.manager.prompter(id).unwrap();
        assert!(
            hoprs_ssh::Prompter::password(&prompter, "u", "h")
                .await
                .is_none()
        );
        assert_eq!(*flags.lock().unwrap(), [true]);
        answer.abort();
    }

    /// An agent never retries a tab's `Disconnected` session: it gets the error and the
    /// tab is left as it is. An agent-only session is retried for the agent.
    #[tokio::test]
    async fn connect_for_agent_retries_only_its_own_session() {
        for agent_only in [false, true] {
            let f = fixture();
            let site = saved_site(&f).await;
            let conn = connected_link();
            let id = if agent_only {
                f.manager
                    .attach_for_agent(site.id.clone(), conn.clone(), Arc::new(MemFs::new()))
                    .await
            } else {
                f.manager
                    .attach(site.id.clone(), conn.clone(), Arc::new(MemFs::new()))
                    .await
            }
            .unwrap();
            conn.state
                .send_replace(ConnectionState::Disconnected(Error::new(
                    ErrorKind::Network,
                    "gone",
                )));
            let err = f
                .manager
                .connect_for_agent(site.id.clone())
                .await
                .unwrap_err();
            assert_eq!(err.session, Some(id));
            assert_eq!(err.error.kind, ErrorKind::Network);
            assert_eq!(
                f.manager.fake_reconnects(id),
                usize::from(agent_only),
                "agent_only {agent_only}"
            );
        }
    }

    /// The agent side never closes a session a tab uses.
    #[tokio::test]
    async fn close_agent_session_spares_a_tab_session() {
        let f = fixture();
        let user_site = SiteId::new();
        let user = f
            .manager
            .attach(user_site.clone(), connected_link(), Arc::new(MemFs::new()))
            .await
            .unwrap();
        assert!(!f.manager.close_agent_session(&user_site).await);
        assert!(f.manager.state(user).is_some(), "still there");
        assert!(f.manager.remote(user).is_ok());

        let agent_site = SiteId::new();
        let agent = f
            .manager
            .attach_for_agent(agent_site.clone(), connected_link(), Arc::new(MemFs::new()))
            .await
            .unwrap();
        assert!(f.manager.close_agent_session(&agent_site).await);
        assert!(f.manager.state(agent).is_none(), "gone");
        assert!(f.manager.agent_sites().is_empty());
        // Nothing left to close.
        assert!(!f.manager.close_agent_session(&agent_site).await);
        assert!(!f.manager.close_agent_session(&SiteId::new()).await);
    }

    /// `connected_session` names a session only while it is connected and its remote
    /// filesystem is built.
    #[tokio::test]
    async fn connected_session_needs_a_live_connection() {
        let f = fixture();
        let site = SiteId::new();
        assert_eq!(f.manager.connected_session(&site), None);
        let conn = connected_link();
        let id = f
            .manager
            .attach_for_agent(site.clone(), conn.clone(), Arc::new(MemFs::new()))
            .await
            .unwrap();
        assert_eq!(f.manager.connected_session(&site), Some(id));
        conn.state
            .send_replace(ConnectionState::Disconnected(Error::new(
                ErrorKind::Network,
                "gone",
            )));
        assert_eq!(f.manager.connected_session(&site), None);
        assert_eq!(f.manager.session_of(&site), Some(id), "any state");
        conn.state
            .send_replace(ConnectionState::Reconnecting { attempt: 1 });
        assert_eq!(f.manager.connected_session(&site), None);
        conn.state.send_replace(ConnectionState::Connected);
        assert_eq!(f.manager.connected_session(&site), Some(id));
    }

    /// Commands run over SSH only; a closed session is the "session closed" error.
    #[tokio::test]
    async fn exec_needs_ssh() {
        let f = fixture();
        let site = SiteId::new();
        let id = f
            .manager
            .attach_for_agent(site, connected_link(), Arc::new(MemFs::new()))
            .await
            .unwrap();
        let limits = hoprs_ssh::ExecLimits {
            stream_bytes: 1024,
            timeout: Duration::from_secs(5),
        };
        let e = f
            .manager
            .exec(id, "uptime".into(), limits)
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Protocol);
        assert_eq!(e.message, "Commands need an SSH site");
        f.manager.close(id).await;
        assert_eq!(
            f.manager
                .exec(id, "uptime".into(), limits)
                .await
                .unwrap_err(),
            closed()
        );
    }

    #[tokio::test]
    async fn session_ids_are_monotonic() {
        let f = fixture();
        let mut ids = Vec::new();
        for _ in 0..3 {
            let (state, _) = watch::channel(ConnectionState::Connected);
            let conn = Arc::new(FakeConn::new(state));
            let id = f
                .manager
                .attach(SiteId::new(), conn, Arc::new(MemFs::new()))
                .await
                .unwrap();
            ids.push(id);
        }
        f.manager.close(ids[1]).await;
        let (state, _) = watch::channel(ConnectionState::Connected);
        let conn = Arc::new(FakeConn::new(state));
        let last = f
            .manager
            .attach(SiteId::new(), conn, Arc::new(MemFs::new()))
            .await
            .unwrap();
        ids.push(last);
        assert!(ids.windows(2).all(|w| w[0] < w[1]), "{ids:?}");
    }

    #[tokio::test]
    async fn connect_unknown_site_is_not_found() {
        let f = fixture();
        let err = f.manager.connect(SiteId::new()).await.unwrap_err();
        assert_eq!(err.session, None);
        assert_eq!(err.error.kind, ErrorKind::NotFound);

        let missing = SiteId::new();
        let results = f.manager.connect_many(vec![missing.clone()]).await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, missing);
        assert_eq!(
            results[0].1.as_ref().unwrap_err().error.kind,
            ErrorKind::NotFound
        );
    }

    /// A failed first connection keeps the session: its id comes with the error, its
    /// state shows the failure, and `close` drops it.
    #[tokio::test]
    async fn failed_first_connect_keeps_the_session() {
        let f = fixture();
        // Nothing listens on port 1.
        let mut site = Site::new("down", "127.0.0.1", "u", Protocol::Ftp);
        site.port = 1;
        f.vault.upsert_site(site.clone()).await.unwrap();
        let mut events = f.broker.take_events().unwrap();
        let answer = tokio::spawn({
            let broker = f.broker.clone();
            async move {
                while let Some(event) = events.recv().await {
                    if let BrokerEvent::Request(r) = event {
                        broker.answer(
                            r.id,
                            PromptAnswer::Secret {
                                secret: SecretString::from("pw"),
                                save: false,
                            },
                        );
                    }
                }
            }
        });

        let err = f.manager.connect(site.id.clone()).await.unwrap_err();
        let id = err.session.expect("the session is kept");
        assert_eq!(err.error.kind, ErrorKind::Network, "{:?}", err.error);
        assert!(matches!(
            *f.manager.state(id).unwrap().borrow(),
            ConnectionState::Disconnected(_)
        ));
        assert!(f.manager.remote(id).is_err());
        // Connecting the site again retries the same session.
        let again = f.manager.connect(site.id.clone()).await.unwrap_err();
        assert_eq!(again.session, Some(id));
        assert!(f.manager.reconnect(id).await.is_err());
        f.manager.close(id).await;
        assert!(f.manager.state(id).is_none());
        answer.abort();
    }

    /// Answers every dialog with the password `pw` (and refuses the rest).
    fn answer_passwords(broker: &PromptBroker) -> (JoinHandle<()>, Arc<Mutex<Vec<PromptKind>>>) {
        let mut events = broker.take_events().unwrap();
        let broker = broker.clone();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let task = tokio::spawn({
            let seen = seen.clone();
            async move {
                while let Some(event) = events.recv().await {
                    if let BrokerEvent::Request(r) = event {
                        seen.lock().unwrap().push(r.kind);
                        broker.answer(
                            r.id,
                            PromptAnswer::Secret {
                                secret: SecretString::from("pw"),
                                save: false,
                            },
                        );
                    }
                }
            }
        });
        (task, seen)
    }

    /// A locked keyring is reported with the connect result, error path included, and
    /// the stored password falls back to the prompt. A site with no stored secret is not
    /// marked.
    #[tokio::test]
    async fn keyring_unavailable_reaches_the_caller() {
        let store = Arc::new(super::super::vault::tests::LockedStore::default());
        let f = fixture_with(store.clone());
        let mut locked = Site::new("locked", "127.0.0.1", "u", Protocol::Ftp);
        locked.port = 1;
        let mut plain = Site::new("plain", "127.0.0.1", "u", Protocol::Ftp);
        plain.port = 1;
        f.vault.upsert_site(locked.clone()).await.unwrap();
        f.vault.upsert_site(plain.clone()).await.unwrap();
        let id = locked.id.clone();
        f.vault
            .call(move |v| v.set_secret(&id, SecretKind::Password, &SecretString::from("x")))
            .await
            .unwrap();
        let (answer, seen) = answer_passwords(&f.broker);

        let results = f
            .manager
            .connect_many(vec![locked.id.clone(), plain.id.clone()])
            .await;
        let locked_err = results[0].1.as_ref().unwrap_err();
        assert!(locked_err.keyring_unavailable, "{locked_err:?}");
        assert!(locked_err.session.is_some());
        let plain_err = results[1].1.as_ref().unwrap_err();
        assert!(!plain_err.keyring_unavailable, "{plain_err:?}");
        // Both asked for the password instead; the keyring was tried once.
        let passwords = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|k| matches!(k, PromptKind::Password { .. }))
            .count();
        assert_eq!(passwords, 2);
        assert_eq!(store.gets.load(std::sync::atomic::Ordering::SeqCst), 1);

        // Connecting the kept session again reports the flag it was started with.
        let again = f.manager.connect(locked.id.clone()).await.unwrap_err();
        assert_eq!(again.session, locked_err.session);
        assert!(again.keyring_unavailable);
        answer.abort();
    }

    /// Open workspace: every site's result is sent on its own, a missing site included,
    /// and a locked keyring is asked once for the whole batch.
    #[tokio::test]
    async fn connect_batch_sends_each_result_with_one_secret_batch() {
        let store = Arc::new(super::super::vault::tests::LockedStore::default());
        let f = fixture_with(store.clone());
        let mut ids = Vec::new();
        for name in ["a", "b", "c"] {
            let mut site = Site::new(name, "127.0.0.1", "u", Protocol::Ftp);
            site.port = 1;
            f.vault.upsert_site(site.clone()).await.unwrap();
            let id = site.id.clone();
            f.vault
                .call(move |v| v.set_secret(&id, SecretKind::Password, &SecretString::from("x")))
                .await
                .unwrap();
            ids.push(site.id);
        }
        let missing = SiteId::new();
        ids.insert(1, missing.clone());
        let (answer, _) = answer_passwords(&f.broker);

        let (tx, mut rx) = mpsc::unbounded_channel();
        f.manager.connect_batch(ids.clone(), tx).await;
        let mut got = Vec::new();
        while let Some(result) = rx.recv().await {
            got.push(result);
        }
        assert_eq!(got.len(), 4);
        let mut sites: Vec<_> = got.iter().map(|(id, _)| id.clone()).collect();
        sites.sort_by(|a, b| a.0.cmp(&b.0));
        let mut expected = ids.clone();
        expected.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(sites, expected, "one result per site");
        for (id, result) in &got {
            let err = result.as_ref().unwrap_err();
            if *id == missing {
                assert_eq!(err.error.kind, ErrorKind::NotFound);
            } else {
                assert!(err.keyring_unavailable, "{err:?}");
            }
        }
        assert_eq!(
            store.gets.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "one keyring call for the whole batch"
        );
        answer.abort();
    }

    /// `close` pauses the site in the engine (nothing new is dispatched to the dead
    /// connection) and the next connect of the site resumes it.
    #[tokio::test]
    async fn close_pauses_the_site_and_reconnecting_resumes_it() {
        use hoprs_transfer::{Direction, TransferSpec, TransferState};

        let f = fixture();
        let site = SiteId::new();
        let remote = MemFs::new();
        remote.add_file("/f.txt", b"data", std::time::SystemTime::UNIX_EPOCH);
        let local = tempfile::tempdir().unwrap();
        let local_path = RemotePath::new(&format!(
            "{}/f.txt",
            local.path().to_string_lossy().replace('\\', "/")
        ));
        let Ok(local_path) = local_path else {
            return; // A drive-letter path: LocalFs("/") cannot address it (Windows).
        };
        let (state, _) = watch::channel(ConnectionState::Connected);
        let conn: Arc<dyn Connection> = Arc::new(FakeConn::new(state));
        let id = f
            .manager
            .attach(site.clone(), conn.clone(), Arc::new(remote.clone()))
            .await
            .unwrap();
        f.manager.close(id).await;

        let engine = f.manager.inner.engine.clone();
        let item = engine
            .enqueue(TransferSpec {
                site: site.clone(),
                direction: Direction::Download,
                local: local_path,
                remote: RemotePath::new("/f.txt").unwrap(),
            })
            .await
            .unwrap();
        let state_of = |items: Vec<hoprs_transfer::TransferItem>| {
            items.into_iter().find(|i| i.id == item).unwrap().state
        };
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(state_of(engine.snapshot().await), TransferState::Queued);

        f.manager
            .attach(site, conn, Arc::new(remote))
            .await
            .unwrap();
        for _ in 0..200 {
            if state_of(engine.snapshot().await) == TransferState::Done {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(state_of(engine.snapshot().await), TransferState::Done);
        assert_eq!(std::fs::read(local.path().join("f.txt")).unwrap(), b"data");
    }

    /// "Save in the system keyring": the typed password reaches the keyring only once the
    /// session it was typed for connects *with it* (auth reported it accepted); a session
    /// that got in another way, a failed connection or a close drops it.
    #[tokio::test]
    async fn typed_secret_saved_only_after_connect() {
        use hoprs_ssh::Prompter as _;
        use hoprs_vault::SecretStore as _;
        use secrecy::ExposeSecret;

        let store = Arc::new(MemoryStore::new());
        let f = fixture_with(store.clone());
        let mut events = f.broker.take_events().unwrap();
        let stored = |site: &Site| {
            store
                .get(&site.id, SecretKind::Password)
                .unwrap()
                .map(|s| s.expose_secret().to_owned())
        };

        // Each case: a session still connecting, whose password prompt is answered with
        // "save"; then the connection succeeds, fails, or the session is closed.
        for (n, outcome) in ["connects", "another method", "fails", "closes"]
            .into_iter()
            .enumerate()
        {
            let site = Site::new(outcome, "h", "u", Protocol::Ssh);
            f.vault.upsert_site(site.clone()).await.unwrap();
            let (state, _) = watch::channel(ConnectionState::Connecting);
            let conn = Arc::new(FakeConn::new(state));
            let attach = tokio::spawn({
                let manager = f.manager.clone();
                let site = site.id.clone();
                let conn = conn.clone();
                async move { manager.attach(site, conn, Arc::new(MemFs::new())).await }
            });
            let id = SessionId(n as u64 + 1);
            eventually("the session", || f.manager.prompter(id).is_some()).await;
            let prompter = f.manager.prompter(id).unwrap();
            let ask = tokio::spawn({
                let prompter = prompter.clone();
                async move { prompter.password("u", "h").await }
            });
            let Some(BrokerEvent::Request(r)) = events.recv().await else {
                panic!("a password prompt");
            };
            f.broker.answer(
                r.id,
                PromptAnswer::Secret {
                    secret: SecretString::from(format!("pw-{outcome}")),
                    save: true,
                },
            );
            ask.await.unwrap().unwrap();
            assert_eq!(stored(&site), None, "{outcome}: not before the outcome");

            match outcome {
                "connects" => {
                    // Auth reports the typed password right, then the state follows.
                    prompter.secret_accepted(hoprs_ssh::AcceptedSecret::Password);
                    conn.state.send_replace(ConnectionState::Connected);
                    attach.await.unwrap().unwrap();
                    assert_eq!(stored(&site), Some("pw-connects".into()));
                    assert!(
                        f.vault
                            .site(site.id.clone())
                            .await
                            .unwrap()
                            .unwrap()
                            .has_password
                    );
                }
                "another method" => {
                    // The server refused the password; keyboard-interactive (or a key)
                    // got in: the session connects, the typed password is not saved.
                    conn.state.send_replace(ConnectionState::Connected);
                    attach.await.unwrap().unwrap();
                    assert_eq!(stored(&site), None);
                    assert!(
                        !f.vault
                            .site(site.id.clone())
                            .await
                            .unwrap()
                            .unwrap()
                            .has_password
                    );
                }
                "fails" => {
                    let error = Error::new(ErrorKind::Auth, "Denied");
                    conn.state
                        .send_replace(ConnectionState::Disconnected(error));
                    assert!(attach.await.unwrap().is_err());
                    assert_eq!(stored(&site), None);
                    assert!(prompter.take_pending_secrets().is_empty(), "dropped");
                }
                _ => {
                    f.manager.close(id).await;
                    assert!(prompter.take_pending_secrets().is_empty(), "dropped");
                    // The close ends the wait: nothing is saved even if it connects now.
                    conn.state.send_replace(ConnectionState::Connected);
                    assert!(attach.await.unwrap().is_err());
                    assert_eq!(stored(&site), None);
                }
            }
        }
    }

    /// Answers every dialog: trusts host keys and certificates, refuses the rest. Returns
    /// the task and the kinds of the dialogs it saw.
    fn answer_prompts(broker: &PromptBroker) -> (JoinHandle<()>, Arc<Mutex<Vec<PromptKind>>>) {
        let mut events = broker.take_events().unwrap();
        let broker = broker.clone();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let task = tokio::spawn({
            let seen = seen.clone();
            async move {
                while let Some(event) = events.recv().await {
                    if let BrokerEvent::Request(r) = event {
                        let answer = match r.kind {
                            PromptKind::HostKey { .. } | PromptKind::Certificate { .. } => {
                                PromptAnswer::Decision(true)
                            }
                            _ => PromptAnswer::Cancel,
                        };
                        seen.lock().unwrap().push(r.kind);
                        broker.answer(r.id, answer);
                    }
                }
            }
        });
        (task, seen)
    }

    #[tokio::test]
    #[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
    async fn connect_ssh_site_end_to_end() {
        let f = fixture();
        let mut site = Site::new("docker ssh", "127.0.0.1", "alice", Protocol::Ssh);
        site.port = hoprs_ssh::testing::SSH_PORT;
        site.ssh.use_agent = false;
        site.ssh.key_file = Some(hoprs_ssh::testing::fixture("id_ed25519"));
        f.vault.upsert_site(site.clone()).await.unwrap();
        let (prompts, seen) = answer_prompts(&f.broker);

        let connected = f.manager.connect(site.id.clone()).await.unwrap();
        assert!(!connected.keyring_unavailable);
        let id = connected.session;
        let entries = f
            .manager
            .remote(id)
            .unwrap()
            .list(&RemotePath::root())
            .await
            .unwrap();
        assert!(
            entries
                .iter()
                .any(|e| e.name == "home" && e.kind == EntryKind::Dir),
            "{entries:?}"
        );
        // The host key was asked once and recorded in the test's known_hosts.
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert!(matches!(seen[0], PromptKind::HostKey { .. }), "{seen:?}");
        assert!(f.dir.path().join("known_hosts").exists());
        // Connecting again returns the same session.
        let again = f.manager.connect(site.id.clone()).await.unwrap();
        assert_eq!(again.session, id);
        // The status bar's login method.
        assert_eq!(
            f.manager.auth(id),
            Some(hoprs_ssh::AuthMethod::KeyFile {
                key_type: "ed25519".into()
            })
        );

        let state = f.manager.state(id).unwrap();
        f.manager.close(id).await;
        assert!(matches!(*state.borrow(), ConnectionState::Disconnected(_)));
        prompts.abort();
    }

    /// Reads a shell's output until it contains `needle`.
    async fn read_until(output: &mut BoxStream<'static, ShellOutput>, needle: &str) {
        let mut text = String::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !text.contains(needle) {
            let next = tokio::time::timeout_at(deadline, output.next())
                .await
                .unwrap_or_else(|_| panic!("timed out; output so far: {text:?}"));
            match next {
                Some(ShellOutput::Data(d)) => text.push_str(&String::from_utf8_lossy(&d)),
                other => panic!("unexpected {other:?}; output so far: {text:?}"),
            }
        }
    }

    /// Skips output until `Closed`, which ends the stream.
    async fn wait_closed(output: &mut BoxStream<'static, ShellOutput>) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let next = tokio::time::timeout_at(deadline, output.next())
                .await
                .expect("timed out waiting for Closed");
            match next {
                Some(ShellOutput::Closed) => break,
                Some(_) => {}
                None => panic!("output ended without Closed"),
            }
        }
        assert!(output.next().await.is_none());
    }

    /// Terminals and files on one SSH session (Review Focus 5): closing a shell leaves
    /// the files and the other shells alone; closing the session closes its shells.
    #[tokio::test]
    #[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
    async fn ssh_shells_close_with_their_session() {
        let f = fixture();
        let mut site = Site::new("docker ssh", "127.0.0.1", "alice", Protocol::Ssh);
        site.port = hoprs_ssh::testing::SSH_PORT;
        site.ssh.use_agent = false;
        site.ssh.key_file = Some(hoprs_ssh::testing::fixture("id_ed25519"));
        f.vault.upsert_site(site.clone()).await.unwrap();
        let (prompts, _) = answer_prompts(&f.broker);
        let id = f.manager.connect(site.id.clone()).await.unwrap().session;
        let shells = f.manager.shells();

        let one = f.manager.open_shell(id, 120, 40).await.unwrap();
        let two = f.manager.open_shell(id, 80, 24).await.unwrap();
        let mut out_one = shells.output(one).unwrap();
        let mut out_two = shells.output(two).unwrap();
        assert!(shells.output(one).is_none(), "handed out once");
        shells
            .input(one, b"printf 'size=<%s>\\n' \"$(stty size)\"\n".to_vec())
            .await
            .unwrap();
        read_until(&mut out_one, "size=<40 120>").await;
        shells.resize(one, 100, 30).await.unwrap();

        // Closing one shell leaves the other and the files working.
        shells.close(one);
        wait_closed(&mut out_one).await;
        assert!(shells.input(one, b"x".to_vec()).await.is_err());
        shells
            .input(two, b"echo two-$((1+1))\n".to_vec())
            .await
            .unwrap();
        read_until(&mut out_two, "two-2").await;
        let remote = f.manager.remote(id).unwrap();
        assert!(remote.list(&RemotePath::root()).await.is_ok());

        // Closing the session closes its remaining shell.
        f.manager.close(id).await;
        wait_closed(&mut out_two).await;
        let e = shells.input(two, b"x".to_vec()).await.unwrap_err();
        assert_eq!(e.message, "Shell closed");
        prompts.abort();
    }

    /// Unpauses a compose service when dropped. Never panics.
    struct Unpause(&'static str);

    impl Unpause {
        /// Unpauses now, off the runtime's thread, and disarms the guard.
        async fn now(self) {
            let service = self.0;
            std::mem::forget(self);
            tokio::task::spawn_blocking(move || {
                hoprs_ssh::testing::docker(&["unpause", service]);
            })
            .await
            .unwrap();
        }
    }

    impl Drop for Unpause {
        fn drop(&mut self) {
            let service = self.0;
            if std::panic::catch_unwind(|| hoprs_ssh::testing::docker(&["unpause", service]))
                .is_err()
            {
                eprintln!("could not unpause {service}");
            }
        }
    }

    /// Review Focus 3: the server goes silent (`docker pause`: connections are accepted,
    /// nothing answers), as when a NAT drops an idle connection. The keepalive notices, the
    /// transport tries its round and gives up; once the server answers again the session
    /// comes back by itself, with no click, and the next listing works.
    ///
    /// The server comes back during the monitor's one quiet round, which the test waits
    /// for in the connection log. Not on the state channel: `Disconnected` lasts only until
    /// the monitor starts that round, and a watch receiver can miss it; it then saw only the
    /// quiet round's own `Disconnected`, unpaused after the last round and waited for
    /// nothing (CI runs 43510478053, 43582756753). On one thread, so the log captures every
    /// task.
    #[tokio::test]
    #[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
    async fn silent_drop_is_detected_and_recovered() {
        let (logs, _guard) = capture_logs();
        let f = fixture_config(
            Arc::new(MemoryStore::new()),
            SessionConfig {
                workers: 1,
                keepalive: Duration::from_secs(2),
                connect_timeout: Some(Duration::from_secs(3)),
                reconnect: Some(ReconnectPolicy {
                    max_attempts: 2,
                    base_delay: Duration::from_millis(500),
                    max_delay: Duration::from_secs(1),
                }),
                ..SessionConfig::default()
            },
        );
        let mut site = Site::new("idle ftp", "127.0.0.1", "frank", Protocol::Ftp);
        site.port = hoprs_remote::testing::FTP_IDLE_PORT;
        f.vault.upsert_site(site.clone()).await.unwrap();
        let id_for_secret = site.id.clone();
        f.vault
            .call(move |v| {
                v.set_secret(
                    &id_for_secret,
                    SecretKind::Password,
                    &SecretString::from("frank-pass"),
                )
            })
            .await
            .unwrap();
        let id = f.manager.connect(site.id.clone()).await.unwrap().session;
        let remote = f.manager.remote(id).unwrap();
        let home = RemotePath::new("/home/frank").unwrap();
        remote.list(&home).await.unwrap();
        let mut state = f.manager.state(id).unwrap();

        let unpause = Unpause("ftp-idle");
        tokio::task::spawn_blocking(|| hoprs_ssh::testing::docker(&["pause", "ftp-idle"]))
            .await
            .unwrap();
        // The keepalive notices, the transport's round fails and gives up: the monitor
        // starts its one quiet round (only after a network failure). Its first attempt is
        // now pending, `connect_timeout` long, and a second one follows.
        let quiet_round = "reconnection given up; trying one more round";
        tokio::time::timeout(Duration::from_secs(30), async {
            while !logs.text().contains(quiet_round) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "the drop is detected and the transport gives up: {}",
                logs.text()
            )
        });
        let text = logs.text();
        let gave_up = text
            .lines()
            .find(|l| l.contains("state=\"disconnected\""))
            .unwrap_or_else(|| panic!("the give-up in the log: {text}"));
        assert!(gave_up.contains("reason=\"network: "), "{gave_up}");
        unpause.now().await;

        tokio::time::timeout(
            Duration::from_secs(30),
            state.wait_for(|s| *s == ConnectionState::Connected),
        )
        .await
        .unwrap_or_else(|_| panic!("back without a click: {}", logs.text()))
        .unwrap();
        let entries = remote.list(&home).await.expect("the next listing works");
        assert!(!entries.is_empty());
        f.manager.close(id).await;
    }

    #[tokio::test]
    #[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
    async fn connect_ftps_site_end_to_end() {
        let f = fixture();
        let mut site = Site::new("docker ftps", "127.0.0.1", "frank", Protocol::FtpsImplicit);
        site.port = hoprs_remote::testing::FTPS_IMPLICIT_PORT;
        f.vault.upsert_site(site.clone()).await.unwrap();
        let id_for_secret = site.id.clone();
        f.vault
            .call(move |v| {
                v.set_secret(
                    &id_for_secret,
                    SecretKind::Password,
                    &SecretString::from("frank-pass"),
                )
            })
            .await
            .unwrap();
        let (prompts, seen) = answer_prompts(&f.broker);

        let connected = f.manager.connect(site.id.clone()).await.unwrap();
        assert!(!connected.keyring_unavailable);
        let id = connected.session;
        let entries = f
            .manager
            .remote(id)
            .unwrap()
            .list(&RemotePath::root())
            .await
            .unwrap();
        assert!(!entries.is_empty());
        // One certificate dialog, no password dialog (the password came from the vault).
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert!(
            matches!(seen[0], PromptKind::Certificate { .. }),
            "{seen:?}"
        );
        // The accepted certificate was pinned in the vault through the broker.
        let saved = f.vault.site(site.id.clone()).await.unwrap().unwrap();
        assert_eq!(
            saved.ftp.pinned_fingerprint,
            Some(hoprs_remote::testing::server_fingerprint())
        );

        // FTP has no login method of its own: the status bar names the protocol.
        assert_eq!(f.manager.auth(id), None);
        f.manager.close(id).await;
        prompts.abort();
    }

    /// Plan 6 Task 6: connecting with stored secrets (a password and a key passphrase
    /// read from the keyring; SSH, FTP and FTPS) through the real session manager and
    /// its site mapping, then a reconnect: neither secret reaches the log, at trace, on
    /// any thread. The connects fail fast (nothing listens on port 1), before any
    /// dialog: typed answers are covered by the broker's
    /// `prompt_answers_never_reach_the_log`.
    #[test]
    fn connect_secrets_never_reach_the_log() {
        const STORED: &str = "stored-Sess-Pw40";
        const PASSPHRASE: &str = "stored-Sess-Kp17";
        let _lock = crate::subscriber_test_lock();
        let (buffer, capture) = crate::capture_log(&crate::log_filter(Some("trace")));
        capture.runtime().block_on(async {
            let f = fixture();
            let mut sites = Vec::new();
            for (name, protocol) in [
                ("ssh", Protocol::Ssh),
                ("ftp", Protocol::Ftp),
                ("ftps", Protocol::FtpsExplicit),
            ] {
                let mut site = Site::new(name, "127.0.0.1", "deploy", protocol);
                site.port = 1;
                site.ssh.use_agent = false;
                f.vault.upsert_site(site.clone()).await.unwrap();
                let id = site.id.clone();
                f.vault
                    .call(move |v| {
                        v.set_secret(&id, SecretKind::Password, &SecretString::from(STORED))?;
                        v.set_secret(
                            &id,
                            SecretKind::KeyPassphrase,
                            &SecretString::from(PASSPHRASE),
                        )
                    })
                    .await
                    .unwrap();
                sites.push(site.id.clone());
            }
            // Any dialog is answered with `pw` (none is expected).
            let (answer, _) = answer_passwords(&f.broker);

            let results = f.manager.connect_many(sites).await;
            let mut sessions = Vec::new();
            for (site, result) in results {
                let err = result.expect_err("nothing listens on port 1");
                sessions.push(err.session.unwrap_or_else(|| panic!("{site:?}: {err:?}")));
            }
            assert!(f.manager.reconnect(sessions[0]).await.is_err());
            for id in sessions {
                f.manager.close(id).await;
            }
            answer.abort();
            crate::log_canaries().await;
        });
        drop(capture);
        crate::assert_no_secrets(&buffer, &[STORED, PASSPHRASE]);
        // Not vacuous: the reconnect was logged.
        assert!(
            buffer.text().contains("reconnect finished"),
            "{}",
            buffer.text()
        );
    }

    /// Plan 6 Task 6: an upload that fails on the server (its error names the file)
    /// never puts the file's content in the log, at trace, on any thread (the local
    /// file is read on the blocking pool).
    #[test]
    fn failed_upload_never_logs_the_content() {
        use hoprs_transfer::{Direction, TransferSpec, TransferState};

        const CONTENT: &str = "content-Upload-Zp36";
        let _lock = crate::subscriber_test_lock();
        let (buffer, capture) = crate::capture_log(&crate::log_filter(Some("trace")));
        let local = tempfile::tempdir().unwrap();
        std::fs::write(local.path().join("notes.txt"), CONTENT).unwrap();
        let Ok(local_path) = RemotePath::new(&format!(
            "{}/notes.txt",
            local.path().to_string_lossy().replace('\\', "/")
        )) else {
            return; // A drive-letter path: LocalFs("/") cannot address it (Windows).
        };
        let error = capture.runtime().block_on(async {
            let f = fixture();
            let site = SiteId::new();
            let remote = MemFs::new();
            remote.add_dir("/ro");
            remote.deny_writes_under("/ro");
            let (state, _) = watch::channel(ConnectionState::Connected);
            let conn: Arc<dyn Connection> = Arc::new(FakeConn::new(state));
            f.manager
                .attach(site.clone(), conn, Arc::new(remote))
                .await
                .unwrap();
            let engine = f.manager.inner.engine.clone();
            let item = engine
                .enqueue(TransferSpec {
                    site,
                    direction: Direction::Upload,
                    local: local_path,
                    remote: RemotePath::new("/ro/notes.txt").unwrap(),
                })
                .await
                .unwrap();
            let mut error = None;
            for _ in 0..300 {
                let state = engine
                    .snapshot()
                    .await
                    .into_iter()
                    .find(|i| i.id == item)
                    .unwrap()
                    .state;
                if let TransferState::Failed(e) = state {
                    error = Some(e);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            crate::log_canaries().await;
            error.expect("the upload fails")
        });
        drop(capture);
        assert!(error.to_string().contains("notes.txt"), "{error}");
        assert!(!error.to_string().contains(CONTENT), "{error}");
        crate::assert_no_secrets(&buffer, &[CONTENT]);
    }
}
