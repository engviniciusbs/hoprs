//! [`FakeHost`]: an in-memory [`AgentHost`] over one [`MemFs`] per site, for the agent
//! service's tests (test builds only).
//!
//! Sessions: [`connect`](AgentHost::connect) gives a site a connected session that only an
//! agent uses (as the real host does for a new session) and reuses one the site already
//! has; [`connect_user`](FakeHost::connect_user) stands for a tab's session, which
//! [`close_agent_session`](AgentHost::close_agent_session) leaves alone. A connect can be
//! made to fail ([`fail_connect`](FakeHost::fail_connect)) or never end
//! ([`hang_connect`](FakeHost::hang_connect)); both leave an agent-only session that is
//! not connected, like a real attempt that failed or waits on a dialog.
//!
//! Holds: [`hold`](FakeHost::hold) stops one kind of filesystem call of a site, and
//! [`hold_exec`](FakeHost::hold_exec) every command, [`hold_connect`](FakeHost::hold_connect)
//! a site's connects, until the [`Hold`] is released or dropped; the
//! hold tells when a call reached it and whether a held call was dropped (its caller went
//! away). [`calls`](FakeHost::calls) counts the calls per site and kind, and
//! [`streamed`](FakeHost::streamed) the bytes read from a site's files.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::task::{Context, Poll};
use std::time::SystemTime;

use async_trait::async_trait;
use hoprs_core::{Error, ErrorKind, Protocol, RemoteEntry, RemotePath, Result, Site, SiteId};
use hoprs_remote::{BoxAsyncRead, BoxAsyncWrite, MemFs, RemoteFs};
use hoprs_ssh::{ExecEnd, ExecLimits, ExecOutput};
use hoprs_transfer::{ConflictPolicy, Event, TransferId, TransferItem, TransferSpec};
use tokio::io::{AsyncRead, ReadBuf};
use tokio::sync::{broadcast, watch};

use super::host::{AgentHost, ConnectFailure};
use super::paths::Protected;
use crate::backend::SessionId;

/// A kind of filesystem call [`FakeHost::hold`] can stop and [`FakeHost::calls`] counts.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum FsOp {
    List,
    Stat,
    /// `open_read` (and `size`).
    Read,
    /// `open_write`.
    Write,
    Mkdir,
    Rename,
    /// `remove_file` and `remove_dir`.
    Remove,
    /// `chmod` and `set_modified`.
    Chmod,
}

/// Holds calls until [`release`](Self::release), or until the last clone of the handle
/// is dropped (a test that forgets to release never hangs). Cheap to clone; clones are
/// one hold.
#[derive(Clone)]
pub struct Hold(Arc<HoldState>);

/// The fake keeps only a `Weak` of this: dropping every [`Hold`] drops it, which closes
/// `released` and lets the held calls through.
struct HoldState {
    /// Calls that reached the hold.
    entered: watch::Sender<usize>,
    released: watch::Sender<bool>,
    /// Held calls whose future was dropped before the release.
    dropped: Arc<AtomicUsize>,
}

impl Hold {
    fn new() -> Hold {
        Hold(Arc::new(HoldState {
            entered: watch::Sender::new(0),
            released: watch::Sender::new(false),
            dropped: Arc::default(),
        }))
    }

    fn weak(&self) -> Weak<HoldState> {
        Arc::downgrade(&self.0)
    }

    /// Waits until a call reached the hold.
    pub async fn entered(&self) {
        let mut entered = self.0.entered.subscribe();
        let _ = entered.wait_for(|n| *n > 0).await;
    }

    /// Lets every held call, and every later one, through.
    pub fn release(&self) {
        self.0.released.send_replace(true);
    }

    /// Whether a held call was dropped (its caller stopped waiting) before the release.
    pub fn dropped(&self) -> bool {
        self.0.dropped.load(Ordering::SeqCst) > 0
    }
}

/// Waits at `hold` (if the test still has it) until it is released or its handles are
/// dropped; a call dropped before that is counted.
async fn pass(hold: &Weak<HoldState>) {
    struct Waiting {
        dropped: Arc<AtomicUsize>,
        done: bool,
    }
    impl Drop for Waiting {
        fn drop(&mut self) {
            if !self.done {
                self.dropped.fetch_add(1, Ordering::SeqCst);
            }
        }
    }
    let Some(hold) = hold.upgrade() else {
        return;
    };
    let mut released = hold.released.subscribe();
    hold.entered.send_modify(|n| *n += 1);
    let mut waiting = Waiting {
        dropped: hold.dropped.clone(),
        done: false,
    };
    // Only the test's handles keep the hold: once they are all dropped, `released`
    // closes and the wait ends.
    drop(hold);
    let _ = released.wait_for(|r| *r).await;
    waiting.done = true;
}

/// A site's session in the fake.
#[derive(Clone, Copy)]
struct FakeSession {
    id: SessionId,
    connected: bool,
    agent_only: bool,
}

/// How the next connects of a site go.
#[derive(Clone)]
enum ConnectPlan {
    Fail(Error),
    Hang,
}

struct State {
    enabled: bool,
    sites: Vec<Site>,
    files: HashMap<SiteId, MemFs>,
    sessions: HashMap<SiteId, FakeSession>,
    next_session: u64,
    connects: HashMap<SiteId, usize>,
    plans: HashMap<SiteId, ConnectPlan>,
    prompt_open: HashMap<SiteId, bool>,
    closed: Vec<SiteId>,
    exec_output: Result<ExecOutput>,
    execs: Vec<(SiteId, String, ExecLimits)>,
    exec_hold: Option<Weak<HoldState>>,
    holds: HashMap<(SiteId, FsOp), Weak<HoldState>>,
    connect_holds: HashMap<SiteId, Weak<HoldState>>,
    /// Files whose `stat` says size 0 whatever they hold (see [`FakeHost::stat_zero_size`]).
    zero_size: HashSet<(SiteId, RemotePath)>,
    /// Why a tab's session that is not connected failed (see
    /// [`FakeHost::disconnect_user`]).
    tab_errors: HashMap<SiteId, Error>,
    calls: HashMap<(SiteId, FsOp), usize>,
    streamed: HashMap<SiteId, u64>,
    enqueued: Vec<(TransferSpec, ConflictPolicy)>,
    next_transfer: i64,
    transfers: Vec<TransferItem>,
    protected: Vec<PathBuf>,
}

/// The in-memory [`AgentHost`] (see the module docs).
pub struct FakeHost {
    shared: Arc<Shared>,
    // TODO(Task 11): remove
    #[allow(dead_code)]
    events: broadcast::Sender<Event>,
}

/// The fake's state, shared with the [`HeldFs`] it hands out.
struct Shared {
    state: Mutex<State>,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Counts a call and waits at its hold, if any.
    async fn enter(&self, site: &SiteId, op: FsOp) {
        let hold = {
            let mut state = self.state();
            *state.calls.entry((site.clone(), op)).or_default() += 1;
            state.holds.get(&(site.clone(), op)).cloned()
        };
        if let Some(hold) = hold {
            pass(&hold).await;
        }
    }

    fn add_streamed(&self, site: &SiteId, bytes: u64) {
        *self.state().streamed.entry(site.clone()).or_default() += bytes;
    }
}

fn session_closed() -> Error {
    Error::coded(ErrorKind::NotFound, "session-closed", "Session closed")
}

fn not_connected() -> Error {
    Error::coded(ErrorKind::Network, "net-not-connected", "Not connected")
}

impl FakeHost {
    /// No sites, the switch on, every command exits 0 with no output.
    pub fn new() -> Arc<FakeHost> {
        Arc::new(FakeHost {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    enabled: true,
                    sites: Vec::new(),
                    files: HashMap::new(),
                    sessions: HashMap::new(),
                    next_session: 1,
                    connects: HashMap::new(),
                    plans: HashMap::new(),
                    prompt_open: HashMap::new(),
                    closed: Vec::new(),
                    exec_output: Ok(ExecOutput {
                        stdout: Vec::new(),
                        stderr: Vec::new(),
                        stdout_truncated: false,
                        stderr_truncated: false,
                        end: ExecEnd::Exit(0),
                    }),
                    execs: Vec::new(),
                    exec_hold: None,
                    holds: HashMap::new(),
                    connect_holds: HashMap::new(),
                    zero_size: HashSet::new(),
                    tab_errors: HashMap::new(),
                    calls: HashMap::new(),
                    streamed: HashMap::new(),
                    enqueued: Vec::new(),
                    next_transfer: 1,
                    transfers: Vec::new(),
                    protected: Vec::new(),
                }),
            }),
            events: broadcast::channel(64).0,
        })
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.shared.state()
    }

    /// Saves `site` (after the others) and gives it an empty filesystem, returned (clones
    /// share it).
    pub fn add_site(&self, site: Site) -> MemFs {
        let fs = MemFs::new();
        let mut state = self.state();
        state.files.insert(site.id.clone(), fs.clone());
        state.sites.retain(|other| other.id != site.id);
        state.sites.push(site);
        fs
    }

    /// The master switch.
    pub fn set_enabled(&self, on: bool) {
        self.state().enabled = on;
    }

    /// Replaces the saved sites (a level changed, a site deleted). Filesystems stay.
    pub fn set_sites(&self, sites: Vec<Site>) {
        let mut state = self.state();
        for site in &sites {
            state.files.entry(site.id.clone()).or_default();
        }
        state.sites = sites;
    }

    /// How many times `connect` was called for the site.
    pub fn connects(&self, site: &SiteId) -> usize {
        self.state().connects.get(site).copied().unwrap_or(0)
    }

    /// The site's next connects fail with `error`.
    pub fn fail_connect(&self, site: &SiteId, error: Error) {
        self.state()
            .plans
            .insert(site.clone(), ConnectPlan::Fail(error));
    }

    /// The site's next connects never return.
    pub fn hang_connect(&self, site: &SiteId) {
        self.state().plans.insert(site.clone(), ConnectPlan::Hang);
    }

    /// Whether the site's session waits on a dialog.
    pub fn set_prompt_open(&self, site: &SiteId, open: bool) {
        self.state().prompt_open.insert(site.clone(), open);
    }

    /// The sites whose agent-only session `close_agent_session` closed, in order.
    pub fn closed(&self) -> Vec<SiteId> {
        self.state().closed.clone()
    }

    /// Gives the site a connected session only an agent uses (or marks its session so),
    /// without a `connect` call.
    pub fn mark_agent_only(&self, site: &SiteId) {
        let mut state = self.state();
        let id = SessionId(state.next_session);
        let session = state.sessions.entry(site.clone()).or_insert(FakeSession {
            id,
            connected: true,
            agent_only: true,
        });
        session.agent_only = true;
        if session.id == id {
            state.next_session += 1;
        }
    }

    /// Gives the site a connected session a tab uses (the user's); returns it.
    pub fn connect_user(&self, site: &SiteId) -> SessionId {
        let mut state = self.state();
        let id = SessionId(state.next_session);
        let session = state.sessions.entry(site.clone()).or_insert(FakeSession {
            id,
            connected: true,
            agent_only: false,
        });
        session.agent_only = false;
        session.connected = true;
        let id_now = session.id;
        if id_now == id {
            state.next_session += 1;
        }
        state.tab_errors.remove(site);
        id_now
    }

    /// Gives the site a session a tab uses that is not connected (its connection ended
    /// with `error`); returns it. An agent's `connect` answers `error` and leaves it
    /// as it is, like the real host (the user retries a tab's session, not the agent).
    pub fn disconnect_user(&self, site: &SiteId, error: Error) -> SessionId {
        let id = self.connect_user(site);
        let mut state = self.state();
        if let Some(session) = state.sessions.get_mut(site) {
            session.connected = false;
        }
        state.tab_errors.insert(site.clone(), error);
        id
    }

    /// What the next commands return.
    // TODO(Task 10): remove
    #[allow(dead_code)]
    pub fn set_exec(&self, output: Result<ExecOutput>) {
        self.state().exec_output = output;
    }

    /// The commands run so far: site, command, limits.
    pub fn execs(&self) -> Vec<(SiteId, String, ExecLimits)> {
        self.state().execs.clone()
    }

    /// Holds every command (after it is recorded in [`execs`](Self::execs)) until the
    /// hold is released.
    pub fn hold_exec(&self) -> Hold {
        let hold = Hold::new();
        self.state().exec_hold = Some(hold.weak());
        hold
    }

    /// Holds the site's connects (after they are counted in [`connects`](Self::connects))
    /// until the hold is released.
    pub fn hold_connect(&self, site: &SiteId) -> Hold {
        let hold = Hold::new();
        self.state().connect_holds.insert(site.clone(), hold.weak());
        hold
    }

    /// The file's `stat` says size 0 from now on, whatever it holds, as SFTP says of
    /// `/proc` and `/sys` files.
    pub fn stat_zero_size(&self, site: &SiteId, path: &str) {
        let path = RemotePath::new(path).expect("an absolute path");
        self.state().zero_size.insert((site.clone(), path));
    }

    /// Holds the site's filesystem calls of kind `op` until the hold is released.
    pub fn hold(&self, site: &SiteId, op: FsOp) -> Hold {
        let hold = Hold::new();
        self.state().holds.insert((site.clone(), op), hold.weak());
        hold
    }

    /// How many filesystem calls of kind `op` the site's remote received.
    pub fn calls(&self, site: &SiteId, op: FsOp) -> usize {
        self.state()
            .calls
            .get(&(site.clone(), op))
            .copied()
            .unwrap_or(0)
    }

    /// Bytes read so far from the site's files (through `open_read` streams).
    pub fn streamed(&self, site: &SiteId) -> u64 {
        self.state().streamed.get(site).copied().unwrap_or(0)
    }

    /// The transfers enqueued so far, with their policies.
    // TODO(Task 11): remove
    #[allow(dead_code)]
    pub fn enqueued(&self) -> Vec<(TransferSpec, ConflictPolicy)> {
        self.state().enqueued.clone()
    }

    /// What `transfers` returns.
    pub fn set_transfers(&self, items: Vec<TransferItem>) {
        self.state().transfers = items;
    }

    /// Sends an engine event to the receivers of `transfer_events`.
    // TODO(Task 11): remove
    #[allow(dead_code)]
    pub fn send_event(&self, event: Event) {
        let _ = self.events.send(event);
    }

    /// The protected roots `protected` answers.
    // TODO(Task 11): remove
    #[allow(dead_code)]
    pub fn set_protected(&self, roots: Vec<PathBuf>) {
        self.state().protected = roots;
    }

    fn site_of(&self, session: SessionId) -> Option<(SiteId, FakeSession)> {
        self.state()
            .sessions
            .iter()
            .find(|(_, s)| s.id == session)
            .map(|(site, s)| (site.clone(), *s))
    }
}

#[async_trait]
impl AgentHost for FakeHost {
    fn enabled(&self) -> bool {
        self.state().enabled
    }

    async fn sites(&self) -> Result<Vec<Site>> {
        Ok(self.state().sites.clone())
    }

    fn connected(&self, site: &SiteId) -> Option<SessionId> {
        self.state()
            .sessions
            .get(site)
            .filter(|s| s.connected)
            .map(|s| s.id)
    }

    async fn connect(&self, site: &SiteId) -> std::result::Result<SessionId, ConnectFailure> {
        let hold = {
            let mut state = self.state();
            *state.connects.entry(site.clone()).or_default() += 1;
            state.connect_holds.get(site).cloned()
        };
        if let Some(hold) = hold {
            pass(&hold).await;
        }
        let plan = {
            let mut state = self.state();
            if !state.sites.iter().any(|s| s.id == *site) {
                let error = Error::new(ErrorKind::NotFound, "No such site");
                return Err(ConnectFailure { error });
            }
            // A session the site has: a connected one is reused, whatever the plan; a
            // tab's that is not connected answers its error and is not retried.
            if let Some(session) = state.sessions.get(site).copied() {
                if session.connected {
                    return Ok(session.id);
                }
                if !session.agent_only {
                    let error = state
                        .tab_errors
                        .get(site)
                        .cloned()
                        .unwrap_or_else(not_connected);
                    return Err(ConnectFailure { error });
                }
            }
            let id = SessionId(state.next_session);
            let plan = state.plans.get(site).cloned();
            let session = state.sessions.entry(site.clone()).or_insert(FakeSession {
                id,
                connected: false,
                agent_only: true,
            });
            if plan.is_none() {
                session.connected = true;
            }
            let session = *session;
            if session.id == id {
                state.next_session += 1;
            }
            match plan {
                None => return Ok(session.id),
                Some(plan) => plan,
            }
        };
        match plan {
            ConnectPlan::Fail(error) => Err(ConnectFailure { error }),
            ConnectPlan::Hang => std::future::pending().await,
        }
    }

    /// Only a session can wait on a dialog: `false` without one, whatever was set.
    fn prompt_open(&self, site: &SiteId) -> bool {
        let state = self.state();
        state.sessions.contains_key(site) && state.prompt_open.get(site).copied().unwrap_or(false)
    }

    async fn close_agent_session(&self, site: &SiteId) -> bool {
        let mut state = self.state();
        if !state.sessions.get(site).is_some_and(|s| s.agent_only) {
            return false;
        }
        state.sessions.remove(site);
        state.closed.push(site.clone());
        true
    }

    fn agent_sites(&self) -> Vec<SiteId> {
        let mut sites: Vec<SiteId> = self
            .state()
            .sessions
            .iter()
            .filter(|(_, s)| s.agent_only)
            .map(|(site, _)| site.clone())
            .collect();
        sites.sort_by(|a, b| a.0.cmp(&b.0));
        sites
    }

    fn remote(&self, session: SessionId) -> Result<Arc<dyn RemoteFs>> {
        let (site, s) = self.site_of(session).ok_or_else(session_closed)?;
        if !s.connected {
            return Err(not_connected());
        }
        let inner = self
            .state()
            .files
            .get(&site)
            .cloned()
            .ok_or_else(not_connected)?;
        Ok(Arc::new(HeldFs {
            shared: self.shared.clone(),
            site,
            inner,
        }))
    }

    async fn exec(
        &self,
        session: SessionId,
        command: String,
        limits: ExecLimits,
    ) -> Result<ExecOutput> {
        let (site, _) = self.site_of(session).ok_or_else(session_closed)?;
        let (hold, output) = {
            let mut state = self.state();
            let ssh = state
                .sites
                .iter()
                .any(|s| s.id == site && s.protocol == Protocol::Ssh);
            if !ssh {
                return Err(Error::new(ErrorKind::Protocol, "Commands need an SSH site"));
            }
            state.execs.push((site, command, limits));
            (state.exec_hold.clone(), state.exec_output.clone())
        };
        if let Some(hold) = hold {
            pass(&hold).await;
        }
        output
    }

    async fn enqueue(&self, spec: TransferSpec, policy: ConflictPolicy) -> Result<TransferId> {
        let mut state = self.state();
        state.enqueued.push((spec, policy));
        let id = TransferId(state.next_transfer);
        state.next_transfer += 1;
        Ok(id)
    }

    async fn transfers(&self) -> Vec<TransferItem> {
        self.state().transfers.clone()
    }

    fn transfer_events(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    async fn protected(&self) -> Protected {
        Protected::new(self.state().protected.clone())
    }
}

/// A site's [`MemFs`] behind the fake's holds and counters.
struct HeldFs {
    shared: Arc<Shared>,
    site: SiteId,
    inner: MemFs,
}

impl HeldFs {
    async fn enter(&self, op: FsOp) {
        self.shared.enter(&self.site, op).await;
    }
}

#[async_trait]
impl RemoteFs for HeldFs {
    async fn list(&self, path: &RemotePath) -> Result<Vec<RemoteEntry>> {
        self.enter(FsOp::List).await;
        self.inner.list(path).await
    }

    async fn stat(&self, path: &RemotePath) -> Result<RemoteEntry> {
        self.enter(FsOp::Stat).await;
        let mut entry = self.inner.stat(path).await?;
        let key = (self.site.clone(), path.clone());
        if self.shared.state().zero_size.contains(&key) {
            entry.size = 0;
        }
        Ok(entry)
    }

    async fn size(&self, path: &RemotePath) -> Result<u64> {
        self.enter(FsOp::Read).await;
        self.inner.size(path).await
    }

    async fn open_read(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncRead> {
        self.enter(FsOp::Read).await;
        let inner = self.inner.open_read(path, offset).await?;
        Ok(Box::new(CountingReader {
            inner,
            shared: self.shared.clone(),
            site: self.site.clone(),
        }))
    }

    async fn open_write(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncWrite> {
        self.enter(FsOp::Write).await;
        self.inner.open_write(path, offset).await
    }

    async fn mkdir(&self, path: &RemotePath) -> Result<()> {
        self.enter(FsOp::Mkdir).await;
        self.inner.mkdir(path).await
    }

    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()> {
        self.enter(FsOp::Rename).await;
        self.inner.rename(from, to).await
    }

    async fn remove_file(&self, path: &RemotePath) -> Result<()> {
        self.enter(FsOp::Remove).await;
        self.inner.remove_file(path).await
    }

    async fn remove_dir(&self, path: &RemotePath) -> Result<()> {
        self.enter(FsOp::Remove).await;
        self.inner.remove_dir(path).await
    }

    async fn chmod(&self, path: &RemotePath, mode: u32) -> Result<()> {
        self.enter(FsOp::Chmod).await;
        self.inner.chmod(path, mode).await
    }

    async fn set_modified(&self, path: &RemotePath, mtime: SystemTime) -> Result<()> {
        self.enter(FsOp::Chmod).await;
        self.inner.set_modified(path, mtime).await
    }
}

/// Counts the bytes a site's file stream hands out (see [`FakeHost::streamed`]).
struct CountingReader {
    inner: BoxAsyncRead,
    shared: Arc<Shared>,
    site: SiteId,
}

impl AsyncRead for CountingReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let polled = Pin::new(&mut self.inner).poll_read(cx, buf);
        let read = buf.filled().len() - before;
        if read > 0 {
            self.shared.add_streamed(&self.site, read as u64);
        }
        polled
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn site(name: &str, protocol: Protocol) -> Site {
        Site::new(name, "host.example", "u", protocol)
    }

    /// Connects reuse the site's session; a tab's session survives the agent's close.
    #[tokio::test]
    async fn sessions_behave_like_the_real_host() {
        let host = FakeHost::new();
        let ssh = site("ssh", Protocol::Ssh);
        host.add_site(ssh.clone());
        let id = host.connect(&ssh.id).await.unwrap();
        assert_eq!(host.connect(&ssh.id).await.unwrap(), id);
        assert_eq!(host.connects(&ssh.id), 2);
        assert_eq!(host.connected(&ssh.id), Some(id));
        assert_eq!(host.agent_sites(), std::slice::from_ref(&ssh.id));
        assert!(host.close_agent_session(&ssh.id).await);
        assert_eq!(host.closed(), std::slice::from_ref(&ssh.id));
        assert_eq!(host.connected(&ssh.id), None);

        let tab = host.connect_user(&ssh.id);
        assert_eq!(host.connect(&ssh.id).await.unwrap(), tab);
        assert!(host.agent_sites().is_empty());
        assert!(!host.close_agent_session(&ssh.id).await);
        assert_eq!(host.connected(&ssh.id), Some(tab));

        let ftp = site("ftp", Protocol::Ftp);
        host.add_site(ftp.clone());
        host.fail_connect(&ftp.id, Error::new(ErrorKind::Auth, "no"));
        let failure = host.connect(&ftp.id).await.unwrap_err();
        assert_eq!(failure.error.kind, ErrorKind::Auth);
        assert_eq!(host.agent_sites(), std::slice::from_ref(&ftp.id));
        assert_eq!(host.connected(&ftp.id), None);

        host.hang_connect(&ftp.id);
        let hung = tokio::time::timeout(Duration::from_millis(50), host.connect(&ftp.id)).await;
        assert!(hung.is_err(), "never returns");
        assert!(host.connect(&SiteId::new()).await.is_err());
    }

    /// A held listing waits for its release; a dropped one is seen; reads are counted.
    #[tokio::test]
    async fn holds_and_counters() {
        let host = FakeHost::new();
        let ssh = site("ssh", Protocol::Ssh);
        let fs = host.add_site(ssh.clone());
        fs.add_file("/a.txt", b"hello", SystemTime::UNIX_EPOCH);
        let session = host.connect(&ssh.id).await.unwrap();
        let remote = host.remote(session).unwrap();

        let hold = host.hold(&ssh.id, FsOp::List);
        let listing = tokio::spawn({
            let remote = remote.clone();
            async move { remote.list(&RemotePath::root()).await.unwrap().len() }
        });
        hold.entered().await;
        assert!(!listing.is_finished());
        hold.release();
        assert_eq!(listing.await.unwrap(), 1);
        assert!(!hold.dropped());

        let hold = host.hold(&ssh.id, FsOp::Stat);
        let stat = tokio::spawn({
            let remote = remote.clone();
            async move { remote.stat(&RemotePath::new("/a.txt").unwrap()).await }
        });
        hold.entered().await;
        stat.abort();
        let _ = stat.await;
        assert!(hold.dropped());

        assert_eq!(host.streamed(&ssh.id), 0);
        let mut reader = remote
            .open_read(&RemotePath::new("/a.txt").unwrap(), 0)
            .await
            .unwrap();
        let mut text = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut text)
            .await
            .unwrap();
        assert_eq!(text, b"hello");
        assert_eq!(host.calls(&ssh.id, FsOp::Read), 1);
        assert_eq!(host.streamed(&ssh.id), 5);
    }

    /// Where the real host reuses a connected session, the fake does too: a failing plan
    /// applies only to a site without one.
    #[tokio::test]
    async fn fail_connect_spares_a_connected_session() {
        let host = FakeHost::new();
        let ssh = site("ssh", Protocol::Ssh);
        host.add_site(ssh.clone());
        let id = host.connect(&ssh.id).await.unwrap();
        host.fail_connect(&ssh.id, Error::new(ErrorKind::Auth, "no"));
        assert_eq!(host.connect(&ssh.id).await.unwrap(), id);
        host.hang_connect(&ssh.id);
        assert_eq!(host.connect(&ssh.id).await.unwrap(), id);
        assert!(host.close_agent_session(&ssh.id).await);
        // No session any more: the plan applies.
        let hung = tokio::time::timeout(Duration::from_millis(50), host.connect(&ssh.id)).await;
        assert!(hung.is_err());
    }

    /// A tab's session that is not connected: the agent gets its error, it is not
    /// retried, and the agent side does not close it.
    #[tokio::test]
    async fn a_disconnected_tab_session_answers_its_error() {
        let host = FakeHost::new();
        let ftp = site("ftp", Protocol::Ftp);
        host.add_site(ftp.clone());
        let tab = host.disconnect_user(&ftp.id, Error::new(ErrorKind::Network, "gone"));
        for _ in 0..2 {
            let failure = host.connect(&ftp.id).await.unwrap_err();
            assert_eq!(failure.error.kind, ErrorKind::Network);
            assert_eq!(failure.error.message, "gone");
        }
        assert_eq!(host.connected(&ftp.id), None, "not retried");
        assert!(host.agent_sites().is_empty());
        assert!(!host.close_agent_session(&ftp.id).await);
        // The user reconnects it: the agent reuses it.
        assert_eq!(host.connect_user(&ftp.id), tab);
        assert_eq!(host.connect(&ftp.id).await.unwrap(), tab);
    }

    /// Without a session nothing can wait on a dialog.
    #[tokio::test]
    async fn prompt_open_needs_a_session() {
        let host = FakeHost::new();
        let ssh = site("ssh", Protocol::Ssh);
        host.add_site(ssh.clone());
        host.set_prompt_open(&ssh.id, true);
        assert!(!host.prompt_open(&ssh.id));
        host.hang_connect(&ssh.id);
        let hung = tokio::time::timeout(Duration::from_millis(50), host.connect(&ssh.id)).await;
        assert!(hung.is_err());
        assert!(host.prompt_open(&ssh.id), "the waiting session");
        assert!(host.close_agent_session(&ssh.id).await);
        assert!(!host.prompt_open(&ssh.id));
    }

    /// A test that drops its hold without releasing it lets the held calls go.
    #[tokio::test]
    async fn a_dropped_hold_releases_its_calls() {
        let host = FakeHost::new();
        let ssh = site("ssh", Protocol::Ssh);
        host.add_site(ssh.clone());
        let session = host.connect(&ssh.id).await.unwrap();
        let remote = host.remote(session).unwrap();
        let hold = host.hold(&ssh.id, FsOp::List);
        let listing = tokio::spawn({
            let remote = remote.clone();
            async move { remote.list(&RemotePath::root()).await }
        });
        hold.entered().await;
        drop(hold);
        tokio::time::timeout(Duration::from_secs(1), listing)
            .await
            .expect("the held listing ends")
            .unwrap()
            .unwrap();
        // And a later call does not wait.
        remote.list(&RemotePath::root()).await.unwrap();

        let limits = ExecLimits {
            stream_bytes: 10,
            timeout: Duration::from_secs(1),
        };
        let hold = host.hold_exec();
        let run = tokio::spawn({
            let host = host.clone();
            async move { host.exec(session, "uptime".into(), limits).await }
        });
        hold.entered().await;
        drop(hold);
        tokio::time::timeout(Duration::from_secs(1), run)
            .await
            .expect("the held command ends")
            .unwrap()
            .unwrap();
    }

    /// Commands: recorded with their limits, held while asked, refused on FTP.
    #[tokio::test]
    async fn exec_is_recorded_and_held() {
        let host = FakeHost::new();
        let ssh = site("ssh", Protocol::Ssh);
        host.add_site(ssh.clone());
        let session = host.connect(&ssh.id).await.unwrap();
        let limits = ExecLimits {
            stream_bytes: 10,
            timeout: Duration::from_secs(1),
        };
        let hold = host.hold_exec();
        let run = tokio::spawn({
            let host = host.clone();
            async move { host.exec(session, "uptime".into(), limits).await }
        });
        hold.entered().await;
        assert_eq!(
            host.execs(),
            [(ssh.id.clone(), "uptime".to_owned(), limits)]
        );
        hold.release();
        assert_eq!(run.await.unwrap().unwrap().end, ExecEnd::Exit(0));

        let ftp = site("ftp", Protocol::Ftp);
        host.add_site(ftp.clone());
        let session = host.connect(&ftp.id).await.unwrap();
        let e = host.exec(session, "ls".into(), limits).await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Protocol);
        assert_eq!(host.execs().len(), 1);
    }
}
