//! The agent service: one tool call from the channel in, one [`Reply`] out.
//!
//! **Every call is checked again, in this order, before anything else happens:** the
//! tool is known; the master switch is on ([`TURNED_OFF`]); the arguments match the
//! tool's schema (an unknown, missing or mistyped argument is refused, and the error
//! names it); the site, read from the vault now, is open to agents at the level the tool
//! needs ([`Need`]). A site that does not exist and a site at "No access" give the same
//! answer ([`SITE_UNAVAILABLE`]) and the same log line, and nothing else happens for
//! either: no connection, no filesystem call, no event, no time recorded. A level the
//! user changes therefore applies on the next call; a call already past its check
//! finishes.
//!
//! **Connections.** A site whose session is not up is connected through the host, for
//! at most [`Limits::prompt_wait`]: the user may have to answer a dialog, the agent never
//! does. Calls that need the same site at the same time wait on one attempt. A failed or timed-out attempt closes the session if only an agent used it
//! (which withdraws its dialogs), and the agent gets a fixed sentence per kind of
//! failure, never the error's own text (an authentication error can name a key file).
//!
//! **Idle sessions.** [`Service::reap`] closes an agent-only session whose site is no
//! longer open to agents, or whose last call ended more than [`Limits::idle_close`] ago,
//! unless the site has a call running (calls already running finish) or an unfinished
//! transfer.
//!
//! **The log.** One INFO line per call under the target `agent`, written when the call
//! ends: the action, the path, the site's name and the outcome. Never file content, an
//! error's detail, or anything about a site the call could not use. Control characters
//! become spaces (so do line separators and bidirectional controls), so one call is one
//! line that reads in order. A call dropped after its access check (the agent cancelled
//! it) writes its line then, ending in `(cancelled)`: whatever it already did is on
//! record.
//!
//! All the work goes through [`AgentHost`], and no lock is held across an await.

mod files;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};
use hoprs_core::{AgentAccess, Error, ErrorKind, PathError, Protocol, RemotePath, Site, SiteId};
use hoprs_remote::RemoteFs;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::host::AgentHost;
use super::proto::Reply;
use super::tools::{self, Need, ToolSpec};
use super::{
    AgentEvent, HOST_KEY_CHANGED, Limits, SITE_UNAVAILABLE, TURNED_OFF, WAITING_FOR_USER,
    read_only, ssh_only,
};
use crate::backend::SessionId;

/// The answer when the vault cannot be read: it names no site.
const SITES_UNREADABLE: &str =
    "hoprs could not read its saved sites. Ask the user to check hoprs, then call the tool again.";

/// Answers the agents' tool calls (see the module docs).
pub struct Service {
    host: Arc<dyn AgentHost>,
    limits: Limits,
    events: mpsc::UnboundedSender<AgentEvent>,
    /// The calls running on each site and when the last one ended (for the reaper).
    activity: Mutex<Activity>,
    /// The connection attempts under way, shared by the calls that wait on them.
    connecting: Mutex<HashMap<SiteId, Pending>>,
    /// Numbers the attempts, so a waiter knows its own.
    next_attempt: AtomicU64,
    /// The last [`AgentEvent::Sites`] sent.
    sites_sent: Mutex<Option<Vec<SiteId>>>,
}

/// One connection attempt, awaited by every call that needs its site meanwhile.
type Attempt = Shared<BoxFuture<'static, AttemptEnd>>;

/// How an attempt ended, the same for all its waiters.
#[derive(Clone)]
enum AttemptEnd {
    Up(SessionId),
    Failed(Error),
    /// Its time ran out; `asked` if a dialog was open then.
    TimedOut {
        asked: bool,
    },
}

/// An attempt and the calls waiting on it.
struct Pending {
    id: u64,
    attempt: Attempt,
    waiters: usize,
}

/// One call waiting on an attempt. When the last waiter goes away before the attempt
/// ended (its calls were cancelled), the attempt goes with it: the map's handle is
/// dropped here and the waiter's own right after, so the connect future is dropped,
/// which releases the host's connect lock and the site's secrets (nothing of it is in
/// the user's way); and an agent-only session it left is closed, withdrawing its
/// dialog: nobody asks any more.
struct Waiter<'a> {
    service: &'a Service,
    site: SiteId,
    id: u64,
    attempt: Attempt,
    ended: bool,
}

impl Drop for Waiter<'_> {
    fn drop(&mut self) {
        let abandoned = {
            let mut connecting = self.service.connecting();
            match connecting.get_mut(&self.site) {
                Some(pending) if pending.id == self.id => {
                    pending.waiters = pending.waiters.saturating_sub(1);
                    if pending.waiters == 0 && !self.ended {
                        connecting.remove(&self.site)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        };
        if abandoned.is_none() {
            return;
        }
        drop(abandoned);
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let host = self.service.host.clone();
            let site = self.site.clone();
            runtime.spawn(async move {
                host.close_agent_session(&site).await;
            });
        }
    }
}

#[derive(Default)]
struct Activity {
    /// Calls past their access check and not ended, per site.
    running: HashMap<SiteId, usize>,
    /// When the site's last call ended.
    ended: HashMap<SiteId, Instant>,
}

/// A call admitted on a site, from its check until it ends or is dropped. While one
/// lives the reaper leaves the site alone; when it ends the site's idle time starts. If
/// it is dropped while `pending` holds its log line (the call never ended), the line is
/// written with `(cancelled)`.
struct Running<'a> {
    service: &'a Service,
    site: SiteId,
    pending: Option<String>,
}

impl<'a> Running<'a> {
    fn start(service: &'a Service, site: &SiteId) -> Running<'a> {
        *service.activity().running.entry(site.clone()).or_default() += 1;
        Running {
            service,
            site: site.clone(),
            pending: None,
        }
    }
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        {
            let mut activity = self.service.activity();
            if let Some(n) = activity.running.get_mut(&self.site) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    activity.running.remove(&self.site);
                }
            }
            activity.ended.insert(self.site.clone(), Instant::now());
        }
        if let Some(line) = self.pending.take() {
            log(&format!("{line} (cancelled)"));
        }
    }
}

/// A call that failed after its access check: the agent's text, and the outcome the log
/// line gives (never an error's detail, never content).
struct Failure {
    text: String,
    reason: String,
}

impl Failure {
    fn new(text: impl Into<String>, reason: impl Into<String>) -> Failure {
        Failure {
            text: text.into(),
            reason: reason.into(),
        }
    }
}

/// A call refused before it did anything: the agent's text, and the log line's reason
/// (fixed, so it says nothing the agent sent).
struct Refusal {
    text: String,
    reason: &'static str,
}

/// `list_directory`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    site: String,
    path: String,
    #[serde(default)]
    offset: u64,
    #[serde(default)]
    limit: Option<u64>,
}

/// `stat`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathArgs {
    site: String,
    path: String,
}

/// `read_file`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArgs {
    site: String,
    path: String,
    #[serde(default)]
    offset: u64,
    #[serde(default)]
    length: Option<u64>,
}

/// A call's arguments, checked against its tool's schema.
enum Args {
    ListSites,
    List(ListArgs),
    Stat(PathArgs),
    Read(ReadArgs),
    /// A tool this version does not run yet (writing, commands, transfers): its access
    /// is checked all the same.
    Later {
        site: Option<String>,
    },
}

impl Args {
    fn parse(spec: &ToolSpec, args: Value) -> Result<Args, String> {
        let map = checked(spec, args)?;
        Ok(match spec.name {
            "list_sites" => Args::ListSites,
            "list_directory" => Args::List(typed(spec, map)?),
            "stat" => Args::Stat(typed(spec, map)?),
            "read_file" => Args::Read(typed(spec, map)?),
            _ => Args::Later {
                site: map.get("site").and_then(Value::as_str).map(str::to_owned),
            },
        })
    }

    /// The `site` argument, for the tools that take one.
    fn site(&self) -> Option<&str> {
        match self {
            Args::ListSites => None,
            Args::List(a) => Some(&a.site),
            Args::Stat(a) => Some(&a.site),
            Args::Read(a) => Some(&a.site),
            Args::Later { site } => site.as_deref(),
        }
    }

    /// The `path` argument, for the tools that take one.
    fn path(&self) -> Option<&str> {
        match self {
            Args::List(a) => Some(&a.path),
            Args::Stat(a) => Some(&a.path),
            Args::Read(a) => Some(&a.path),
            Args::ListSites | Args::Later { .. } => None,
        }
    }
}

impl Service {
    // TODO(Task 12): remove
    #[allow(dead_code)]
    pub fn new(
        host: Arc<dyn AgentHost>,
        limits: Limits,
        events: mpsc::UnboundedSender<AgentEvent>,
    ) -> Arc<Service> {
        Arc::new(Service {
            host,
            limits,
            events,
            activity: Mutex::default(),
            connecting: Mutex::default(),
            next_attempt: AtomicU64::new(0),
            sites_sent: Mutex::default(),
        })
    }

    /// Answers one call. Every problem is a `Reply::Err` with a text for the agent; the
    /// call's log line is written when it ends.
    // TODO(Task 12): remove
    #[allow(dead_code)]
    pub async fn call(&self, tool: &str, args: Value) -> Reply {
        let (reply, line) = self.run(tool, args).await;
        log(&line);
        reply
    }

    /// Closes the agent-only sessions whose site is gone or no longer open to agents, or
    /// whose last call ended `limits.idle_close` ago (or that never had one), except
    /// while the site has a call running or an unfinished transfer; then tells which
    /// sites still hold one (when that changed).
    // TODO(Task 12): remove
    #[allow(dead_code)]
    pub async fn reap(&self) {
        // A vault that cannot be read opens no site.
        let sites = self.host.sites().await.unwrap_or_default();
        let transfers = self.host.transfers().await;
        let now = Instant::now();
        for id in self.host.agent_sites() {
            if transfers
                .iter()
                .any(|t| t.spec.site == id && !t.state.is_terminal())
            {
                continue;
            }
            let idle = {
                let activity = self.activity();
                if activity.running.contains_key(&id) {
                    continue;
                }
                activity
                    .ended
                    .get(&id)
                    .is_none_or(|at| now.duration_since(*at) >= self.limits.idle_close)
            };
            let open = sites
                .iter()
                .any(|s| s.id == id && s.agent_access != AgentAccess::None);
            if !open || idle {
                self.host.close_agent_session(&id).await;
            }
        }
        self.activity()
            .ended
            .retain(|_, at| now.duration_since(*at) < self.limits.idle_close);
        // An attempt no call waits on is never kept (its waiters retire it; this is
        // only a second guard).
        self.connecting().retain(|_, pending| pending.waiters > 0);
        self.send_sites();
    }

    /// The call's reply and its log line (see the module docs for the order).
    async fn run(&self, tool: &str, args: Value) -> (Reply, String) {
        match tools::find(tool) {
            Some(spec) => self.run_spec(spec, args).await,
            // The line says nothing the agent chose.
            None => (
                Reply::Err(format!("Unknown tool: {tool}")),
                "refused an unknown tool".to_owned(),
            ),
        }
    }

    /// A call of the tool `spec`.
    async fn run_spec(&self, spec: &ToolSpec, args: Value) -> (Reply, String) {
        if !self.host.enabled() {
            return refused(
                spec.name,
                Refusal {
                    text: TURNED_OFF.to_owned(),
                    reason: "agent access is off",
                },
            );
        }
        let args = match Args::parse(spec, args) {
            Ok(args) => args,
            Err(text) => {
                return refused(
                    spec.name,
                    Refusal {
                        text,
                        reason: "bad arguments",
                    },
                );
            }
        };
        // The table says whether the call needs a site, not the arguments: a tool that
        // needs one and comes without it is refused like a site that is not open.
        let site = if spec.need == Need::Nothing {
            None
        } else {
            let Some(id) = args.site() else {
                return refused(spec.name, unavailable());
            };
            match self.admit(spec, id).await {
                Ok(site) => Some(site),
                Err(refusal) => return refused(spec.name, refusal),
            }
        };
        match (site, args) {
            (None, Args::ListSites) => self.list_sites().await,
            (Some(site), args) => self.on_site(spec, &site, args).await,
            (None, _) => (
                Reply::Err(not_yet(spec)),
                format!("refused {}: not available in this version", spec.name),
            ),
        }
    }

    /// The site `id` if it is open to agents at the level `spec` needs, read from the
    /// vault now. No site and "No access" are one refusal.
    async fn admit(&self, spec: &ToolSpec, id: &str) -> Result<Site, Refusal> {
        let sites = self.host.sites().await.map_err(|_| Refusal {
            text: SITES_UNREADABLE.to_owned(),
            reason: "the saved sites could not be read",
        })?;
        let Some(site) = sites
            .into_iter()
            .find(|s| s.id.0 == id && s.agent_access != AgentAccess::None)
        else {
            return Err(unavailable());
        };
        let needs_all = matches!(spec.need, Need::All | Need::AllSsh);
        if needs_all && !site.agent_access.allows(AgentAccess::All) {
            return Err(Refusal {
                text: read_only(&site.name),
                reason: "the site allows reading only",
            });
        }
        if spec.need == Need::AllSsh && site.protocol != Protocol::Ssh {
            return Err(Refusal {
                text: ssh_only(&site.name),
                reason: "not an SSH site",
            });
        }
        Ok(site)
    }

    /// A call on a site that passed its check: the tool, then the log line.
    async fn on_site(&self, spec: &ToolSpec, site: &Site, args: Args) -> (Reply, String) {
        let path = match args.path().map(|raw| remote_path(site, "path", raw)) {
            None => None,
            Some(Ok(path)) => Some(path),
            Some(Err(text)) => {
                return refused(
                    spec.name,
                    Refusal {
                        text,
                        reason: "bad arguments",
                    },
                );
            }
        };
        let mut running = Running::start(self, &site.id);
        let action = match (&args, &path) {
            (Args::List(_), Some(path)) => format!("listed {path}"),
            (Args::Stat(_), Some(path)) => format!("described {path}"),
            (Args::Read(_), Some(path)) => format!("read {path}"),
            _ => spec.name.to_owned(),
        };
        let done = format!("{action} on \"{}\"", site.name);
        running.pending = Some(done.clone());
        let result = match (args, path) {
            (Args::List(a), Some(path)) => {
                self.list_directory(site, &path, a.offset, a.limit).await
            }
            (Args::Stat(_), Some(path)) => self.stat(site, &path).await,
            (Args::Read(a), Some(path)) => self.read_file(site, &path, a.offset, a.length).await,
            _ => Err(Failure::new(not_yet(spec), "not available in this version")),
        };
        // It ended: the caller writes its line.
        running.pending = None;
        drop(running);
        match result {
            Ok(blocks) => (Reply::Ok(blocks), done),
            Err(failure) => (
                Reply::Err(failure.text),
                format!("{done} (failed: {})", failure.reason),
            ),
        }
    }

    /// The sites open to agents, in the vault's order, with the fields of ruling 7 only
    /// (never a key file, a secret's flag, a pinned fingerprint or a local folder).
    async fn list_sites(&self) -> (Reply, String) {
        let Ok(sites) = self.host.sites().await else {
            return (
                Reply::Err(SITES_UNREADABLE.to_owned()),
                "listed the sites (failed: the saved sites could not be read)".to_owned(),
            );
        };
        let open: Vec<Value> = sites
            .iter()
            .filter_map(|site| {
                let level = match site.agent_access {
                    AgentAccess::None => return None,
                    AgentAccess::Read => "read",
                    AgentAccess::All => "all",
                };
                Some(json!({
                    "id": site.id.0,
                    "name": site.name,
                    "protocol": protocol_name(site.protocol),
                    "host": site.host,
                    "port": site.port,
                    "user": site.user,
                    "level": level,
                    "connected": self.host.connected(&site.id).is_some(),
                    "start_folder": site.remote_dir,
                }))
            })
            .collect();
        (
            Reply::Ok(vec![json!({ "sites": open }).to_string()]),
            "listed the sites".to_owned(),
        )
    }

    /// The remote filesystem of the site's session, connecting it first if need be.
    async fn remote(&self, site: &Site) -> Result<Arc<dyn RemoteFs>, Failure> {
        let session = self.session(site).await?;
        self.host.remote(session).map_err(|_| {
            Failure::new(
                format!(
                    "The connection to \"{}\" closed. Call the tool again.",
                    site.name
                ),
                "connection closed",
            )
        })
    }

    /// The site's session: the one that is up, or the end of the site's connection
    /// attempt (joined if one is under way), which lasts at most `limits.prompt_wait`
    /// from its start, the time the user may take to answer a dialog.
    async fn session(&self, site: &Site) -> Result<SessionId, Failure> {
        if let Some(session) = self.host.connected(&site.id) {
            return Ok(session);
        }
        let mut waiter = self.join_attempt(&site.id);
        let end = waiter.attempt.clone().await;
        waiter.ended = true;
        // The first waiter back retires the attempt: a later call starts anew.
        let first = {
            let mut connecting = self.connecting();
            let ours = connecting
                .get(&site.id)
                .is_some_and(|pending| pending.id == waiter.id);
            if ours {
                connecting.remove(&site.id);
            }
            ours
        };
        drop(waiter);
        if let (AttemptEnd::Up(session), true) = (&end, first) {
            self.send(AgentEvent::Connected(*session));
        }
        self.send_sites();
        match end {
            AttemptEnd::Up(session) => Ok(session),
            AttemptEnd::Failed(error) => Err(connect_failure(site, &error)),
            AttemptEnd::TimedOut { asked: true } => {
                Err(Failure::new(WAITING_FOR_USER, "waiting for the user"))
            }
            AttemptEnd::TimedOut { asked: false } => Err(Failure::new(
                format!(
                    "Connecting to \"{}\" ({}:{}) took more than {}. Call the tool again later; if it keeps happening, ask the user to check the site in hoprs.",
                    site.name,
                    site.host,
                    site.port,
                    span(self.limits.prompt_wait)
                ),
                "the connection timed out",
            )),
        }
    }

    /// Joins the site's attempt under way, or starts one. The attempt holds the
    /// connect, its time limit and, when it does not come up, the close of an
    /// agent-only session (which withdraws its dialog): every waiter gets the same end.
    fn join_attempt(&self, site: &SiteId) -> Waiter<'_> {
        let mut connecting = self.connecting();
        let pending = connecting.entry(site.clone()).or_insert_with(|| {
            let host = self.host.clone();
            let id = site.clone();
            let wait = self.limits.prompt_wait;
            let attempt = async move {
                match tokio::time::timeout(wait, host.connect(&id)).await {
                    Ok(Ok(session)) => AttemptEnd::Up(session),
                    Ok(Err(failure)) => {
                        host.close_agent_session(&id).await;
                        AttemptEnd::Failed(failure.error)
                    }
                    Err(_) => {
                        // Read before the close, which withdraws the dialog.
                        let asked = host.prompt_open(&id);
                        host.close_agent_session(&id).await;
                        AttemptEnd::TimedOut { asked }
                    }
                }
            };
            Pending {
                id: self.next_attempt.fetch_add(1, Ordering::Relaxed),
                attempt: attempt.boxed().shared(),
                waiters: 0,
            }
        });
        pending.waiters += 1;
        Waiter {
            service: self,
            site: site.clone(),
            id: pending.id,
            attempt: pending.attempt.clone(),
            ended: false,
        }
    }

    /// [`AgentEvent::Sites`], when the list changed since the last one sent.
    fn send_sites(&self) {
        let now = self.host.agent_sites();
        let mut sent = lock(&self.sites_sent);
        if sent.as_ref() != Some(&now) {
            *sent = Some(now.clone());
            self.send(AgentEvent::Sites(now));
        }
    }

    fn activity(&self) -> MutexGuard<'_, Activity> {
        lock(&self.activity)
    }

    fn connecting(&self) -> MutexGuard<'_, HashMap<SiteId, Pending>> {
        lock(&self.connecting)
    }

    fn send(&self, event: AgentEvent) {
        // No receiver (the window is gone): nothing to tell.
        let _ = self.events.send(event);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Writes a call's one line.
fn log(line: &str) {
    tracing::info!(target: "agent", "{}", printable(line));
}

/// No such site, or one at "No access": one refusal.
fn unavailable() -> Refusal {
    Refusal {
        text: SITE_UNAVAILABLE.to_owned(),
        reason: "site not available",
    }
}

/// A refused call's reply and log line.
fn refused(tool: &str, refusal: Refusal) -> (Reply, String) {
    (
        Reply::Err(refusal.text),
        format!("refused {tool}: {}", refusal.reason),
    )
}

fn not_yet(spec: &ToolSpec) -> String {
    format!("{} is not available in this version of hoprs.", spec.name)
}

/// The arguments as an object that fits the tool's schema: no unknown argument, every
/// required one, each of its type, an integer not under its minimum. An integer over
/// its maximum is accepted here and taken as the maximum by the tool.
fn checked(spec: &ToolSpec, args: Value) -> Result<Map<String, Value>, String> {
    let map = match args {
        Value::Null => Map::new(),
        Value::Object(map) => map,
        _ => {
            return Err(format!(
                "The arguments of {} must be a JSON object.",
                spec.name
            ));
        }
    };
    let schema = (spec.schema)();
    let none = Map::new();
    let properties = schema["properties"].as_object().unwrap_or(&none);
    for name in map.keys() {
        if !properties.contains_key(name) {
            let known: Vec<&str> = properties.keys().map(String::as_str).collect();
            let known = if known.is_empty() {
                "none".to_owned()
            } else {
                known.join(", ")
            };
            let name: String = printable(name).chars().take(64).collect();
            return Err(format!(
                "Unknown argument `{name}` for {}. Its arguments are: {known}.",
                spec.name
            ));
        }
    }
    let required = schema["required"].as_array().into_iter().flatten();
    for name in required.filter_map(Value::as_str) {
        if !map.contains_key(name) {
            return Err(format!("Missing argument `{name}` for {}.", spec.name));
        }
    }
    for (name, value) in &map {
        let Some(property) = properties.get(name) else {
            continue;
        };
        let minimum = property["minimum"].as_u64().unwrap_or(0);
        let fits = match property["type"].as_str() {
            Some("string") => value.is_string(),
            Some("boolean") => value.is_boolean(),
            Some("integer") => value.as_u64().is_some_and(|n| n >= minimum),
            _ => true,
        };
        if !fits {
            let wanted = match property["type"].as_str() {
                Some("string") => "a string".to_owned(),
                Some("boolean") => "true or false".to_owned(),
                _ => format!("a whole number of at least {minimum}"),
            };
            return Err(format!("Argument `{name}` must be {wanted}."));
        }
    }
    Ok(map)
}

/// The checked arguments as the tool's own type.
fn typed<T: DeserializeOwned>(spec: &ToolSpec, map: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(map))
        .map_err(|_| format!("The arguments of {} do not fit its schema.", spec.name))
}

/// The remote path of argument `name`: absolute, not above `/`.
fn remote_path(site: &Site, name: &str, raw: &str) -> Result<RemotePath, String> {
    if raw.is_empty() {
        return Err(format!(
            "Argument `{name}` is empty: give an absolute path on the server, such as /var/www."
        ));
    }
    RemotePath::new(raw).map_err(|e| match e {
        PathError::NotAbsolute => {
            let start = match site.remote_dir.as_deref() {
                Some(dir) if !dir.is_empty() => format!("The site's start folder is {dir}."),
                _ => "The site names no start folder.".to_owned(),
            };
            format!(
                "Argument `{name}` must be an absolute path on the server (starting with /). {start}"
            )
        }
        PathError::EscapesRoot => format!("Argument `{name}` goes above the root folder /."),
    })
}

/// Ruling 12: one fixed sentence per kind of failure, never the error's own text.
fn connect_failure(site: &Site, error: &Error) -> Failure {
    let name = &site.name;
    match error.kind {
        ErrorKind::Auth => Failure::new(
            format!(
                "hoprs could not log in to \"{name}\". The user must fix the site's credentials in hoprs (edit the site), then call the tool again."
            ),
            "authentication failed",
        ),
        ErrorKind::HostKey if hoprs_ssh::host_key_change(error).is_some() => {
            Failure::new(HOST_KEY_CHANGED, "the host key changed")
        }
        ErrorKind::HostKey | ErrorKind::Tls => Failure::new(
            format!(
                "The connection to \"{name}\" was refused in the hoprs window: the server's host key or certificate was not accepted. The user has to resolve it there; an agent cannot."
            ),
            "refused in the hoprs window",
        ),
        ErrorKind::Network => Failure::new(
            format!(
                "Could not connect to {}:{}. Check that the server is up and reachable from this computer, then call the tool again.",
                site.host, site.port
            ),
            format!("could not connect to {}:{}", site.host, site.port),
        ),
        _ => Failure::new(
            format!(
                "Could not connect to \"{name}\". Ask the user to check the site in hoprs, then call the tool again."
            ),
            "could not connect",
        ),
    }
}

fn protocol_name(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Ssh => "ssh",
        Protocol::Ftp => "ftp",
        Protocol::FtpsExplicit => "ftps-explicit",
        Protocol::FtpsImplicit => "ftps-implicit",
    }
}

/// "2 minutes", "90 seconds".
fn span(time: Duration) -> String {
    let secs = time.as_secs();
    match secs {
        60 => "1 minute".to_owned(),
        s if s > 60 && s % 60 == 0 => format!("{} minutes", s / 60),
        1 => "1 second".to_owned(),
        s => format!("{s} seconds"),
    }
}

/// `text` with control characters (newlines included), line and paragraph separators
/// and bidirectional controls as spaces: one line, in the order it was written.
fn printable(text: &str) -> String {
    text.chars()
        .map(|c| {
            let breaks = matches!(
                c,
                '\u{2028}'
                    | '\u{2029}'
                    | '\u{200e}'
                    | '\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
            );
            if c.is_control() || breaks { ' ' } else { c }
        })
        .collect()
}

#[cfg(test)]
mod tests;
