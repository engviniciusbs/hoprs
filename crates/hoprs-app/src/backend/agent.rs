//! The agent service's host over the real backend ([`RealHost`]), and the guard the
//! transfer engine runs on every agent item's local path ([`guard`]).
//!
//! **The protected roots** ([`crate::agent::paths`]): the fixed ones (hoprs's own
//! directories, `~/.ssh`, the `known_hosts` in use, computed once when the backend opens)
//! and every saved site's `ssh.key_file`, read from the vault at each check, whatever the
//! site's agent access (a site an agent cannot reach still has its key protected). No
//! root reaches a log line, a `Debug` output or an error: a refusal names only the item's
//! own local path, the one the agent gave.
//!
//! **What the guard covers.** The engine calls it for every item of origin Agent before
//! any filesystem call of that item (first runs, retries, restored items; files and
//! folders alike), and never for the user's items. It refuses a path that is a protected
//! root, is inside one, or resolves into one (links followed, on Unix also by file
//! identity). A link found while listing an uploaded folder never reaches it: the engine
//! skips such a link straight from the listing, so nothing behind it is read. What it
//! does **not** cover: the time between its check and the engine's open. A directory on
//! the path swapped for a link to a protected place in that window is followed by the
//! open. Closing that window needs the engine to open relative to a checked directory
//! handle; it is a follow-up, not done here.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use hoprs_core::{Error, ErrorKind, RemotePath, Result, Site, SiteId};
use hoprs_remote::RemoteFs;
use hoprs_ssh::{ExecLimits, ExecOutput};
use hoprs_transfer::{
    ConflictPolicy, EnqueueOptions, Event, LocalGuard, Origin, TransferEngine, TransferId,
    TransferItem, TransferSpec,
};
use tokio::sync::{broadcast, watch};

use super::broker::PromptBroker;
use super::sessions::{SessionId, SessionManager};
use super::vault::VaultHandle;
use crate::agent::host::{AgentHost, ConnectFailure};
use crate::agent::paths::{Prepared, Protected, local_os_path};

/// How long the guard reuses the roots it resolved, while they stay the same list: a
/// folder of many files is checked item after item, and resolving every root costs about
/// a hundred system calls. Within this window a root replaced by a link counts by its
/// path only (see [`Prepared`]).
const GUARD_REUSE: Duration = Duration::from_secs(2);

/// The agent service's host over the backend's own handles (see the module docs).
pub struct RealHost {
    vault: VaultHandle,
    engine: TransferEngine,
    sessions: SessionManager,
    broker: PromptBroker,
    agents: watch::Receiver<bool>,
    /// The protected roots before the saved sites' key files.
    fixed: Arc<Vec<PathBuf>>,
}

impl RealHost {
    pub(super) fn new(
        vault: VaultHandle,
        engine: TransferEngine,
        sessions: SessionManager,
        broker: PromptBroker,
        agents: watch::Receiver<bool>,
        fixed: Vec<PathBuf>,
    ) -> RealHost {
        RealHost {
            vault,
            engine,
            sessions,
            broker,
            agents,
            fixed: Arc::new(fixed),
        }
    }
}

/// The fixed roots plus every saved site's key file, read from the vault now. If the
/// vault cannot be read, everything is protected (the filesystem's root): a check never
/// passes for want of the key files.
async fn protected_now(vault: &VaultHandle, fixed: &[PathBuf]) -> Protected {
    let keys = vault
        .call(|v| {
            Ok(v.sites()
                .iter()
                .filter_map(|site| site.ssh.key_file.clone())
                .collect::<Vec<PathBuf>>())
        })
        .await;
    match keys {
        Ok(keys) => Protected::new(fixed.iter().cloned().chain(keys)),
        Err(error) => {
            tracing::warn!(
                kind = error.kind.as_str(),
                "cannot read the saved sites; every local path is protected"
            );
            Protected::new([std::path::absolute("/").unwrap_or_else(|_| PathBuf::from("/"))])
        }
    }
}

/// The refusal of a protected local path: names only the path the item has.
fn refused(path: &Path) -> Error {
    let path = path.display().to_string();
    Error::coded(
        ErrorKind::Permission,
        "agent-protected-path",
        format!("Protected path: {path}"),
    )
    .arg("path", path)
}

/// The roots the guard resolved last (see [`GUARD_REUSE`]).
struct Resolved {
    protected: Protected,
    at: Instant,
    prepared: Arc<Prepared>,
}

/// `path` checked against `protected`, reusing `cache` while it holds the same roots and
/// is recent. Touches the disk: on the blocking pool.
fn check_reusing(
    cache: &Mutex<Option<Resolved>>,
    protected: Protected,
    path: &Path,
) -> std::result::Result<(), PathBuf> {
    let prepared = {
        let mut slot = cache.lock().unwrap_or_else(PoisonError::into_inner);
        match slot.as_ref() {
            Some(last) if last.protected == protected && last.at.elapsed() < GUARD_REUSE => {
                last.prepared.clone()
            }
            _ => {
                let prepared = Arc::new(protected.prepare());
                *slot = Some(Resolved {
                    protected,
                    at: Instant::now(),
                    prepared: prepared.clone(),
                });
                prepared
            }
        }
    };
    prepared.check(path, false)
}

/// The engine's guard of agent items: the item's local path (`local_os_path`) checked
/// against the fixed roots and the saved sites' key files (read now), on the blocking
/// pool. A protected path fails the item with `agent-protected-path`. Each item is
/// checked on its own (`tree` is false): the engine calls the guard for every item of a
/// folder transfer too.
pub fn guard(vault: VaultHandle, fixed: Vec<PathBuf>) -> LocalGuard {
    let fixed = Arc::new(fixed);
    let cache: Arc<Mutex<Option<Resolved>>> = Arc::default();
    LocalGuard::new(move |local: RemotePath| {
        let vault = vault.clone();
        let fixed = fixed.clone();
        let cache = cache.clone();
        Box::pin(async move {
            let protected = protected_now(&vault, &fixed).await;
            let path = local_os_path(&local);
            let checked = tokio::task::spawn_blocking(move || {
                let outcome = check_reusing(&cache, protected, &path);
                (path, outcome)
            })
            .await
            .map_err(super::background_failed)?;
            match checked {
                (_, Ok(())) => Ok(()),
                (path, Err(_)) => Err(refused(&path)),
            }
        })
    })
}

#[async_trait]
impl AgentHost for RealHost {
    fn enabled(&self) -> bool {
        *self.agents.borrow()
    }

    async fn sites(&self) -> Result<Vec<Site>> {
        self.vault.call(|v| Ok(v.sites().to_vec())).await
    }

    fn connected(&self, site: &SiteId) -> Option<SessionId> {
        self.sessions.connected_session(site)
    }

    async fn connect(&self, site: &SiteId) -> std::result::Result<SessionId, ConnectFailure> {
        self.sessions
            .connect_for_agent(site.clone())
            .await
            .map(|connected| connected.session)
            .map_err(|failed| ConnectFailure {
                error: failed.error,
            })
    }

    fn prompt_open(&self, site: &SiteId) -> bool {
        self.sessions
            .session_of(site)
            .is_some_and(|session| self.broker.has_open(session))
    }

    async fn close_agent_session(&self, site: &SiteId) -> bool {
        self.sessions.close_agent_session(site).await
    }

    fn agent_sites(&self) -> Vec<SiteId> {
        self.sessions.agent_sites()
    }

    fn remote(&self, session: SessionId) -> Result<Arc<dyn RemoteFs>> {
        self.sessions.remote(session)
    }

    async fn exec(
        &self,
        session: SessionId,
        command: String,
        limits: ExecLimits,
    ) -> Result<ExecOutput> {
        self.sessions.exec(session, command, limits).await
    }

    async fn enqueue(&self, spec: TransferSpec, policy: ConflictPolicy) -> Result<TransferId> {
        let options = EnqueueOptions {
            policy: Some(policy),
            origin: Origin::Agent,
        };
        self.engine.enqueue_with(spec, options).await
    }

    async fn transfers(&self) -> Vec<TransferItem> {
        self.engine.snapshot().await
    }

    fn transfer_events(&self) -> broadcast::Receiver<Event> {
        self.engine.subscribe()
    }

    async fn protected(&self) -> Protected {
        protected_now(&self.vault, &self.fixed).await
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use hoprs_core::{ConnectionState, Protocol};
    use hoprs_remote::{Connection, MemFs};
    use hoprs_transfer::{Direction, TransferState};
    use hoprs_vault::{ExportParams, MemoryStore};

    use super::super::{Backend, BackendApi, BackendConfig, SessionConfig};
    use super::*;
    use crate::agent::paths::engine_path;

    /// A connection that is up and stays up.
    struct Up(watch::Sender<ConnectionState>);

    impl Connection for Up {
        fn state(&self) -> watch::Receiver<ConnectionState> {
            self.0.subscribe()
        }

        fn generation(&self) -> u64 {
            1
        }
    }

    fn up() -> Arc<dyn Connection> {
        Arc::new(Up(watch::channel(ConnectionState::Connected).0))
    }

    /// A tempdir the engine's local side can name: on Windows the temporary folder may
    /// be on another drive than the one `LocalFs::new("/")` reaches.
    fn local_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        if engine_path(dir.path()).is_ok() {
            return dir;
        }
        tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()
    }

    fn open(dir: &Path) -> Backend {
        let config = BackendConfig {
            profile_dir: dir.join("profiles"),
            queue_path: Some(dir.join("data").join("queue.db")),
            sessions: SessionConfig {
                known_hosts: Some(dir.join("known_hosts")),
                ..SessionConfig::default()
            },
            backup_params: ExportParams {
                m_cost: 8,
                t_cost: 1,
                p_cost: 1,
            },
            edit_dir: dir.join("edit"),
            recovered_dir: dir.join("recovered"),
            agent_dir: Some(dir.join("agent")),
        };
        Backend::open(config, Arc::new(MemoryStore::new())).unwrap()
    }

    fn engine_local(path: &Path) -> RemotePath {
        engine_path(path).unwrap_or_else(|e| panic!("{path:?}: {e}"))
    }

    /// Waits until every item of the tree under `root` has ended, then returns them.
    async fn finished(host: &RealHost, root: TransferId) -> Vec<TransferItem> {
        for _ in 0..500 {
            let items = host.transfers().await;
            let mut tree = vec![root];
            let mut grew = true;
            while grew {
                grew = false;
                for item in &items {
                    if item.parent.is_some_and(|p| tree.contains(&p)) && !tree.contains(&item.id) {
                        tree.push(item.id);
                        grew = true;
                    }
                }
            }
            let ours: Vec<TransferItem> = items
                .into_iter()
                .filter(|item| tree.contains(&item.id))
                .collect();
            if !ours.is_empty() && ours.iter().all(|item| item.state.is_terminal()) {
                return ours;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("transfer {root:?} never finished");
    }

    fn failed_code(item: &TransferItem) -> Option<&'static str> {
        match &item.state {
            TransferState::Failed(error) => error.code,
            _ => None,
        }
    }

    /// hoprs's own directories and every saved site's key file are protected; a key file
    /// changed in a site's form is protected from the next check on, the old one no more.
    /// The key of a site no agent may reach (the default level) counts too.
    #[tokio::test]
    async fn protected_names_hoprs_dirs_and_key_files() {
        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let keys = dir.path().join("keys");
        std::fs::create_dir_all(&keys).unwrap();
        std::fs::write(keys.join("a.pem"), b"key a").unwrap();
        std::fs::write(keys.join("b.pem"), b"key b").unwrap();
        let mut site = Site::new("Acme", "acme.example", "deploy", Protocol::Ssh);
        site.ssh.key_file = Some(keys.join("a.pem"));
        assert_eq!(site.agent_access, hoprs_core::AgentAccess::None);
        backend.save_site(site.clone()).await.unwrap();
        let host = backend.agent_host();

        let protected = host.protected().await;
        for path in [
            dir.path().join("profiles"),
            dir.path().join("profiles").join("sites.toml"),
            dir.path().join("data").join("queue.db"),
            dir.path().join("edit").join("1").join("x.php"),
            dir.path().join("recovered"),
            dir.path().join("agent").join("agent.sock"),
            dir.path().join("known_hosts"),
            keys.join("a.pem"),
        ] {
            assert!(protected.check(&path, false).is_err(), "{path:?}");
        }
        assert!(protected.check(&keys.join("b.pem"), false).is_ok());
        assert!(protected.check(&dir.path().join("work"), false).is_ok());
        let shown = format!("{protected:?}");
        assert!(
            !shown.contains("pem") && !shown.contains("profiles"),
            "{shown}"
        );

        site.ssh.key_file = Some(keys.join("b.pem"));
        backend.save_site(site).await.unwrap();
        let protected = host.protected().await;
        assert!(protected.check(&keys.join("b.pem"), false).is_err());
        assert!(protected.check(&keys.join("a.pem"), false).is_ok());
    }

    /// The engine's guard stops an agent's item whose local path is protected (inside a
    /// protected folder, through a link, as a download's destination), and never the
    /// user's own transfer of the same path. A link inside an uploaded folder is skipped
    /// from the listing: nothing behind it is read or uploaded.
    #[tokio::test]
    async fn agent_transfers_are_guarded() {
        let dir = local_dir();
        let backend = open(dir.path());
        let host = backend.agent_host();
        let site = SiteId::new();
        let remote = MemFs::new();
        backend
            .sessions
            .attach(site.clone(), up(), Arc::new(remote.clone()))
            .await
            .unwrap();
        let profiles = dir.path().join("profiles");
        std::fs::create_dir_all(&profiles).unwrap();
        let secret = profiles.join("secret.txt");
        std::fs::write(&secret, b"TOP-SECRET").unwrap();
        let upload = |local: &Path, remote: &str| TransferSpec {
            site: site.clone(),
            direction: Direction::Upload,
            local: engine_local(local),
            remote: RemotePath::new(remote).unwrap(),
        };

        // Inside a protected folder: refused, nothing uploaded.
        let id = host
            .enqueue(upload(&secret, "/secret.txt"), ConflictPolicy::Overwrite)
            .await
            .unwrap();
        let items = finished(&host, id).await;
        assert_eq!(items[0].origin, Origin::Agent);
        assert_eq!(failed_code(&items[0]), Some("agent-protected-path"));
        let TransferState::Failed(error) = &items[0].state else {
            unreachable!()
        };
        assert!(error.message.starts_with("Protected path: "), "{error:?}");
        assert!(remote.read_file("/secret.txt").is_none());

        // A download whose destination is protected: refused, nothing written there.
        remote.add_file("/new.txt", b"from the server", SystemTime::UNIX_EPOCH);
        let landing = profiles.join("new.txt");
        let id = host
            .enqueue(
                TransferSpec {
                    site: site.clone(),
                    direction: Direction::Download,
                    local: engine_local(&landing),
                    remote: RemotePath::new("/new.txt").unwrap(),
                },
                ConflictPolicy::Overwrite,
            )
            .await
            .unwrap();
        let items = finished(&host, id).await;
        assert_eq!(failed_code(&items[0]), Some("agent-protected-path"));
        assert!(!landing.exists());

        // The user's own transfer of the same path is not the guard's business.
        let id = backend
            .enqueue(upload(&secret, "/secret.txt"))
            .await
            .unwrap();
        let items = finished(&host, id).await;
        assert_eq!(items[0].origin, Origin::User);
        assert_eq!(items[0].state, TransferState::Done);
        assert_eq!(remote.read_file("/secret.txt").unwrap(), b"TOP-SECRET");

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            // An item whose own path is a link into a protected folder: refused.
            let work = dir.path().join("work");
            std::fs::create_dir_all(&work).unwrap();
            symlink(&secret, work.join("innocent.txt")).unwrap();
            let id = host
                .enqueue(
                    upload(&work.join("innocent.txt"), "/innocent.txt"),
                    ConflictPolicy::Overwrite,
                )
                .await
                .unwrap();
            let items = finished(&host, id).await;
            assert_eq!(failed_code(&items[0]), Some("agent-protected-path"));
            assert!(remote.read_file("/innocent.txt").is_none());
            // The refusal names the item's own path, never the protected place it leads to.
            let TransferState::Failed(error) = &items[0].state else {
                unreachable!()
            };
            let shown = format!("{error:?} {error}");
            assert!(shown.contains("innocent.txt"), "{shown}");
            assert!(!shown.contains("profiles"), "{shown}");

            // A folder holding such a link: the link is skipped, the rest goes up.
            let folder = dir.path().join("folder");
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("a.txt"), b"a").unwrap();
            std::fs::write(folder.join("b.txt"), b"b").unwrap();
            symlink(&secret, folder.join("link.txt")).unwrap();
            let id = host
                .enqueue(upload(&folder, "/folder"), ConflictPolicy::Overwrite)
                .await
                .unwrap();
            let items = finished(&host, id).await;
            let state_of = |name: &str| {
                items
                    .iter()
                    .find(|item| item.spec.remote.file_name() == Some(name))
                    .map(|item| item.state.clone())
                    .unwrap_or_else(|| panic!("no item for {name}: {items:?}"))
            };
            assert!(
                matches!(state_of("link.txt"), TransferState::Skipped(_)),
                "{items:?}"
            );
            assert_eq!(state_of("a.txt"), TransferState::Done);
            assert_eq!(state_of("b.txt"), TransferState::Done);
            assert!(items.iter().all(|item| item.origin == Origin::Agent));
            assert_eq!(remote.read_file("/folder/a.txt").unwrap(), b"a");
            assert!(remote.read_file("/folder/link.txt").is_none());
            let names: Vec<String> = remote
                .list(&RemotePath::new("/folder").unwrap())
                .await
                .unwrap()
                .into_iter()
                .map(|entry| entry.name)
                .collect();
            assert_eq!(names, ["a.txt", "b.txt"], "nothing else on the server");
        }
    }

    /// The host reads the master switch as it is now.
    #[tokio::test]
    async fn host_follows_the_switch() {
        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let host = backend.agent_host();
        assert!(!host.enabled());
        backend.set_agents_enabled(true).await;
        assert!(host.enabled());
        backend.set_agents_enabled(false).await;
        assert!(!host.enabled());
    }

    /// An agent's connect through the host: the session is the agent's, a prompt it waits
    /// on is seen, and closing it leaves a tab's session alone.
    #[tokio::test]
    async fn host_sessions_are_the_agents() {
        let dir = tempfile::tempdir().unwrap();
        let backend = open(dir.path());
        let host = backend.agent_host();
        let agent_site = SiteId::new();
        let agent = backend
            .sessions
            .attach_for_agent(agent_site.clone(), up(), Arc::new(MemFs::new()))
            .await
            .unwrap();
        let user_site = SiteId::new();
        backend
            .sessions
            .attach(user_site.clone(), up(), Arc::new(MemFs::new()))
            .await
            .unwrap();
        assert_eq!(host.agent_sites(), std::slice::from_ref(&agent_site));
        assert_eq!(host.connected(&agent_site), Some(agent));
        assert!(host.remote(agent).is_ok());
        assert!(!host.prompt_open(&agent_site));
        assert!(!host.prompt_open(&SiteId::new()));
        // A dialog of the agent's session is open: the host sees it, flagged.
        let mut dialogs = backend.prompt_events().unwrap();
        let prompter = backend.sessions.prompter(agent).unwrap();
        let asked =
            tokio::spawn(async move { hoprs_ssh::Prompter::password(&prompter, "u", "h").await });
        let Some(super::super::BrokerEvent::Request(request)) = dialogs.recv().await else {
            panic!("a dialog");
        };
        assert!(request.by_agent);
        assert!(host.prompt_open(&agent_site));
        assert!(!host.prompt_open(&user_site));
        backend
            .answer_prompt(request.id, super::super::PromptAnswer::Cancel)
            .await;
        assert!(asked.await.unwrap().is_none());
        assert!(!host.prompt_open(&agent_site));
        assert!(!host.close_agent_session(&user_site).await);
        assert!(host.connected(&user_site).is_some());
        assert!(host.close_agent_session(&agent_site).await);
        assert_eq!(host.connected(&agent_site), None);
        // A site that is not saved cannot be connected; the error is carried, not shown.
        let failure = host.connect(&SiteId::new()).await.unwrap_err();
        assert_eq!(failure.error.kind, ErrorKind::NotFound);
        assert!(host.agent_sites().is_empty());
    }
}
