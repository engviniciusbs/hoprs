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
//! **Commands.** `run_command` runs one command at a time per site: a call waits for the
//! site's turn (counted as running meanwhile, so the reaper leaves the session alone), and
//! its time limit starts when its command does. The command line is logged (it is the
//! record the user asked for, made one line like every other), its output never is.
//!
//! **Transfers.** `download` and `upload` go through the transfer queue the user sees,
//! never under a policy that asks, after their local path passed the protected check;
//! the call waits about a minute for the end (see `transfers`). A call cancelled meanwhile
//! is logged; its transfer keeps running.
//!
//! All the work goes through [`AgentHost`], and no std lock is held across an await (the
//! command turn is a tokio lock, held for the command's run).

mod files;
mod transfers;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};
use hoprs_core::{AgentAccess, Error, ErrorKind, PathError, Protocol, RemotePath, Site, SiteId};
use hoprs_remote::RemoteFs;
use hoprs_ssh::{ExecEnd, ExecLimits};
use hoprs_transfer::Direction;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::host::AgentHost;
use super::paths::engine_path;
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
    /// Each site's command turn: one `run_command` at a time per site.
    commands: Mutex<HashMap<SiteId, Arc<tokio::sync::Mutex<()>>>>,
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

/// A site's command turn, held by one `run_command` call from before it waits until it
/// ends or is dropped. The last holder removes the site's entry.
struct CommandTurn<'a> {
    service: &'a Service,
    site: SiteId,
    turn: Option<Arc<tokio::sync::Mutex<()>>>,
}

impl<'a> CommandTurn<'a> {
    fn take(service: &'a Service, site: &SiteId) -> CommandTurn<'a> {
        let turn = service.commands().entry(site.clone()).or_default().clone();
        CommandTurn {
            service,
            site: site.clone(),
            turn: Some(turn),
        }
    }

    /// Waits for the site's turn.
    async fn lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.turn
            .as_ref()
            .expect("taken until dropped")
            .lock()
            .await
    }
}

impl Drop for CommandTurn<'_> {
    fn drop(&mut self) {
        drop(self.turn.take());
        let mut commands = self.service.commands();
        // Clones are made under this lock only: a count of one is the map's own.
        if commands
            .get(&self.site)
            .is_some_and(|turn| Arc::strong_count(turn) == 1)
        {
            commands.remove(&self.site);
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

/// `write_file`. `Debug` is not derived: `content` is the file's content.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteArgs {
    site: String,
    path: String,
    content: String,
    #[serde(default)]
    overwrite: bool,
}

/// `rename`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameArgs {
    site: String,
    from: String,
    to: String,
}

/// `set_permissions`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChmodArgs {
    site: String,
    path: String,
    mode: String,
}

/// `delete`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteArgs {
    site: String,
    path: String,
    #[serde(default)]
    recursive: bool,
}

/// `run_command`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunArgs {
    site: String,
    command: String,
    #[serde(default)]
    timeout_seconds: Option<u64>,
}

/// `download` and `upload`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TransferArgs {
    site: String,
    remote_path: String,
    local_path: String,
    #[serde(default)]
    overwrite: bool,
}

/// `transfer_status`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusArgs {
    transfer: u64,
}

/// A call's arguments, checked against its tool's schema.
enum Args {
    ListSites,
    List(ListArgs),
    Stat(PathArgs),
    Read(ReadArgs),
    Write(WriteArgs),
    Mkdir(PathArgs),
    Rename(RenameArgs),
    Chmod(ChmodArgs),
    Delete(DeleteArgs),
    Run(RunArgs),
    Download(TransferArgs),
    Upload(TransferArgs),
    Status(StatusArgs),
}

impl Args {
    fn parse(spec: &ToolSpec, args: Value) -> Result<Args, String> {
        let map = checked(spec, args)?;
        Ok(match spec.name {
            "list_sites" => Args::ListSites,
            "list_directory" => Args::List(typed(spec, map)?),
            "stat" => Args::Stat(typed(spec, map)?),
            "read_file" => Args::Read(typed(spec, map)?),
            "write_file" => Args::Write(typed(spec, map)?),
            "make_directory" => Args::Mkdir(typed(spec, map)?),
            "rename" => Args::Rename(typed(spec, map)?),
            "set_permissions" => Args::Chmod(typed(spec, map)?),
            "delete" => Args::Delete(typed(spec, map)?),
            "run_command" => Args::Run(typed(spec, map)?),
            "download" => Args::Download(typed(spec, map)?),
            "upload" => Args::Upload(typed(spec, map)?),
            "transfer_status" => Args::Status(typed(spec, map)?),
            other => return Err(format!("Unknown tool: {other}")),
        })
    }

    /// The `site` argument, for the tools that take one.
    fn site(&self) -> Option<&str> {
        match self {
            Args::ListSites | Args::Status(_) => None,
            Args::List(a) => Some(&a.site),
            Args::Stat(a) | Args::Mkdir(a) => Some(&a.site),
            Args::Read(a) => Some(&a.site),
            Args::Write(a) => Some(&a.site),
            Args::Rename(a) => Some(&a.site),
            Args::Chmod(a) => Some(&a.site),
            Args::Delete(a) => Some(&a.site),
            Args::Run(a) => Some(&a.site),
            Args::Download(a) | Args::Upload(a) => Some(&a.site),
        }
    }
}

/// A call on a site with its arguments' values checked: remote paths valid, the content
/// and the mode within their rules, the time limit clamped.
enum Job {
    List {
        path: RemotePath,
        offset: u64,
        limit: Option<u64>,
    },
    Stat {
        path: RemotePath,
    },
    Read {
        path: RemotePath,
        offset: u64,
        length: Option<u64>,
    },
    Write {
        path: RemotePath,
        content: String,
        overwrite: bool,
    },
    Mkdir {
        path: RemotePath,
    },
    Rename {
        from: RemotePath,
        to: RemotePath,
    },
    Chmod {
        path: RemotePath,
        mode: u32,
    },
    Delete {
        path: RemotePath,
        recursive: bool,
    },
    Run {
        command: String,
        timeout: Duration,
    },
    Transfer(transfers::TransferJob),
}

impl Job {
    /// The job of `args` on `site`, or the refusal of a value that breaks its rule.
    fn new(site: &Site, args: Args, limits: &Limits) -> Result<Job, Refusal> {
        let bad = |text: String| Refusal {
            text,
            reason: "bad arguments",
        };
        let path = |name: &str, raw: &str| remote_path(site, name, raw).map_err(bad);
        Ok(match args {
            Args::List(a) => Job::List {
                path: path("path", &a.path)?,
                offset: a.offset,
                limit: a.limit,
            },
            Args::Stat(a) => Job::Stat {
                path: path("path", &a.path)?,
            },
            Args::Read(a) => Job::Read {
                path: path("path", &a.path)?,
                offset: a.offset,
                length: a.length,
            },
            Args::Write(a) => {
                let path = path("path", &a.path)?;
                if a.content.len() > limits.file_max {
                    return Err(Refusal {
                        text: format!(
                            "The content is {} bytes: write_file takes at most {} bytes per call. Nothing was written. For a larger file, use upload.",
                            a.content.len(),
                            limits.file_max
                        ),
                        reason: "content over the limit",
                    });
                }
                Job::Write {
                    path,
                    content: a.content,
                    overwrite: a.overwrite,
                }
            }
            Args::Mkdir(a) => Job::Mkdir {
                path: path("path", &a.path)?,
            },
            Args::Rename(a) => {
                let (from, to) = (path("from", &a.from)?, path("to", &a.to)?);
                for (name, p) in [("from", &from), ("to", &to)] {
                    if *p == RemotePath::root() {
                        return Err(bad(format!(
                            "Argument `{name}` is the root folder /, which rename cannot move or replace."
                        )));
                    }
                }
                Job::Rename { from, to }
            }
            Args::Chmod(a) => Job::Chmod {
                path: path("path", &a.path)?,
                mode: octal_mode(&a.mode).ok_or_else(|| {
                    bad("Argument `mode` must be 3 or 4 octal digits (0 to 7), such as \"644\" or \"0755\".".to_owned())
                })?,
            },
            Args::Delete(a) => {
                let path = path("path", &a.path)?;
                if path == RemotePath::root() {
                    return Err(bad(
                        "delete refuses the root folder /: name the files or folders under it instead."
                            .to_owned(),
                    ));
                }
                Job::Delete {
                    path,
                    recursive: a.recursive,
                }
            }
            Args::Run(a) => {
                // The schema's minimum is 1; a value over the maximum is the maximum.
                let max = limits.exec_max.as_secs().max(1);
                let timeout = a
                    .timeout_seconds
                    .map_or(limits.exec_default, |s| Duration::from_secs(s.clamp(1, max)));
                Job::Run {
                    command: a.command,
                    timeout,
                }
            }
            Args::Download(a) => Job::Transfer(transfer_job(site, Direction::Download, a)?),
            Args::Upload(a) => Job::Transfer(transfer_job(site, Direction::Upload, a)?),
            // Tools without a site never come here.
            Args::ListSites | Args::Status(_) => return Err(unavailable()),
        })
    }

    /// What the call does, for its log line (never content).
    fn action(&self) -> String {
        match self {
            Job::List { path, .. } => format!("listed {}", capped(path.as_str())),
            Job::Stat { path } => format!("described {}", capped(path.as_str())),
            Job::Read { path, .. } => format!("read {}", capped(path.as_str())),
            Job::Write { path, content, .. } => format!(
                "wrote {} ({})",
                capped(path.as_str()),
                count(content.len() as u64, "byte")
            ),
            Job::Mkdir { path } => format!("created the folder {}", capped(path.as_str())),
            Job::Rename { from, to } => format!(
                "renamed {} to {}",
                capped(from.as_str()),
                capped(to.as_str())
            ),
            Job::Chmod { path, mode } => format!(
                "changed the permissions of {} to {mode:04o}",
                capped(path.as_str())
            ),
            Job::Delete { path, .. } => format!("deleted {}", capped(path.as_str())),
            Job::Run { command, .. } => format!("ran `{}`", capped(command)),
            Job::Transfer(job) => match job.direction {
                Direction::Download => format!(
                    "downloaded {} to {}",
                    capped(job.remote.as_str()),
                    capped(&job.given)
                ),
                Direction::Upload => format!(
                    "uploaded {} to {}",
                    capped(&job.given),
                    capped(job.remote.as_str())
                ),
            },
        }
    }
}

/// The most characters of a path or a command a log line keeps: the outcome at its end
/// must survive the log panel's cut of a long line.
const LOGGED_MAX: usize = 500;

/// `text` up to [`LOGGED_MAX`] characters, then `… (N more bytes)`.
fn capped(text: &str) -> String {
    match text.char_indices().nth(LOGGED_MAX) {
        None => text.to_owned(),
        Some((at, _)) => format!("{}… ({} more bytes)", &text[..at], text.len() - at),
    }
}

/// What a call that worked answers, and what its log line adds (in parentheses).
struct Done {
    blocks: Vec<String>,
    note: Option<String>,
}

impl From<Vec<String>> for Done {
    fn from(blocks: Vec<String>) -> Done {
        Done { blocks, note: None }
    }
}

/// `"1 item"`, `"3 items"`.
fn count(n: u64, what: &str) -> String {
    if n == 1 {
        format!("1 {what}")
    } else {
        format!("{n} {what}s")
    }
}

/// `"644"` or `"0755"` as mode bits: three or four ASCII octal digits, nothing else.
fn octal_mode(text: &str) -> Option<u32> {
    let digits = (3..=4).contains(&text.len()) && text.bytes().all(|b| (b'0'..=b'7').contains(&b));
    if digits {
        u32::from_str_radix(text, 8).ok()
    } else {
        None
    }
}

impl Service {
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
            commands: Mutex::default(),
        })
    }

    /// Answers one call. Every problem is a `Reply::Err` with a text for the agent; the
    /// call's log line is written when it ends.
    pub async fn call(&self, tool: &str, args: Value) -> Reply {
        let (reply, line) = self.run(tool, args).await;
        log(&line);
        reply
    }

    /// Closes the agent-only sessions whose site is gone or no longer open to agents, or
    /// whose last call ended `limits.idle_close` ago (or that never had one), except
    /// while the site has a call running or an unfinished transfer; then tells which
    /// sites still hold one (when that changed). With the master switch off no site is
    /// open to agents: their sessions close at once, idle or not.
    pub async fn reap(&self) {
        // A vault that cannot be read opens no site; neither does the switch off.
        let sites = if self.host.enabled() {
            self.host.sites().await.unwrap_or_default()
        } else {
            Vec::new()
        };
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
            (None, Args::Status(a)) => self.transfer_status(a.transfer).await,
            (Some(site), args) => self.on_site(spec, &site, args).await,
            // A tool with a site whose table entry needs none: refused like a site that
            // is not open.
            (None, _) => refused(spec.name, unavailable()),
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
        let job = match Job::new(site, args, &self.limits) {
            Ok(job) => job,
            Err(refusal) => return refused(spec.name, refusal),
        };
        let mut running = Running::start(self, &site.id);
        let done = format!("{} on \"{}\"", job.action(), site.name);
        running.pending = Some(done.clone());
        let result = match job {
            Job::List {
                path,
                offset,
                limit,
            } => self
                .list_directory(site, &path, offset, limit)
                .await
                .map(Done::from),
            Job::Stat { path } => self.stat(site, &path).await.map(Done::from),
            Job::Read {
                path,
                offset,
                length,
            } => self
                .read_file(site, &path, offset, length)
                .await
                .map(Done::from),
            Job::Write {
                path,
                content,
                overwrite,
            } => self.write_file(site, &path, &content, overwrite).await,
            Job::Mkdir { path } => self.make_directory(site, &path).await,
            Job::Rename { from, to } => self.rename(site, &from, &to).await,
            Job::Chmod { path, mode } => self.set_permissions(site, &path, mode).await,
            Job::Delete { path, recursive } => self.delete(site, &path, recursive).await,
            Job::Run { command, timeout } => self.run_command(site, &command, timeout).await,
            Job::Transfer(job) => self.transfer(site, &job).await,
        };
        // It ended: the caller writes its line.
        running.pending = None;
        drop(running);
        match result {
            Ok(Done { blocks, note }) => {
                let line = match note {
                    Some(note) => format!("{done} ({note})"),
                    None => done,
                };
                (Reply::Ok(blocks), line)
            }
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

    /// Runs one command, after the site's command turn (one at a time per site): the
    /// time limit starts when the command does. The answer is ruling 7's header (with
    /// `lost`), then the non-empty streams, decoded lossily; the log adds how it ended.
    async fn run_command(
        &self,
        site: &Site,
        command: &str,
        timeout: Duration,
    ) -> Result<Done, Failure> {
        let turn = CommandTurn::take(self, &site.id);
        let _held = turn.lock().await;
        let session = self.session(site).await?;
        let limits = ExecLimits {
            stream_bytes: self.limits.exec_stream,
            timeout,
        };
        let out = self
            .host
            .exec(session, command.to_owned(), limits)
            .await
            .map_err(|e| {
                let mut text = format!(
                    "Could not run the command on \"{}\": {}.",
                    site.name, e.message
                );
                if e.kind == ErrorKind::Network {
                    text.push_str(" The connection may have been lost: call the tool again.");
                }
                Failure::new(text, e.message.clone())
            })?;
        let (exit_code, signal, note) = match &out.end {
            ExecEnd::Exit(code) => (Some(*code), None, format!("exit {code}")),
            ExecEnd::Signal(name) => (None, Some(name.clone()), format!("signal {name}")),
            ExecEnd::TimedOut => (None, None, "timed out".to_owned()),
            ExecEnd::Lost => (None, None, "connection lost".to_owned()),
        };
        let mut header = json!({
            "exit_code": exit_code,
            "signal": signal,
            "timed_out": out.end == ExecEnd::TimedOut,
            "lost": out.end == ExecEnd::Lost,
            "stdout_truncated": out.stdout_truncated,
            "stderr_truncated": out.stderr_truncated,
        });
        let explained = match out.end {
            ExecEnd::TimedOut => Some(format!(
                "The command ran past its time limit of {}: hoprs stopped waiting and sent it a KILL signal, but the process may still be running on the server.",
                span(timeout)
            )),
            ExecEnd::Lost => Some(
                "The connection was lost before the command ended: it may or may not have finished. Check its effects before running it again."
                    .to_owned(),
            ),
            _ => None,
        };
        if let (Some(text), Value::Object(map)) = (explained, &mut header) {
            map.insert("note".to_owned(), Value::from(text));
        }
        let mut blocks = vec![header.to_string()];
        for (name, bytes) in [("stdout", &out.stdout), ("stderr", &out.stderr)] {
            if !bytes.is_empty() {
                blocks.push(format!("{name}:\n{}", String::from_utf8_lossy(bytes)));
            }
        }
        Ok(Done {
            blocks,
            note: Some(note),
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
            self.send(AgentEvent::Connected(site.id.clone(), *session));
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

    fn commands(&self) -> MutexGuard<'_, HashMap<SiteId, Arc<tokio::sync::Mutex<()>>>> {
        lock(&self.commands)
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

/// A `download` or `upload` with its paths checked: the remote one as every remote path,
/// the local one by `engine_path` (its sentence is the refusal).
fn transfer_job(
    site: &Site,
    direction: Direction,
    a: TransferArgs,
) -> Result<transfers::TransferJob, Refusal> {
    let remote = remote_path(site, "remote_path", &a.remote_path).map_err(|text| Refusal {
        text,
        reason: "bad arguments",
    })?;
    let local = engine_path(std::path::Path::new(&a.local_path)).map_err(|text| Refusal {
        text: text.to_owned(),
        reason: "bad local path",
    })?;
    Ok(transfers::TransferJob {
        direction,
        remote,
        local,
        given: a.local_path,
        overwrite: a.overwrite,
    })
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

/// The remote path of argument `name`: absolute, with no `..` component (it would be
/// resolved here by its text, while the server resolves it through any link on the way,
/// so the log would name another path than the one touched).
fn remote_path(site: &Site, name: &str, raw: &str) -> Result<RemotePath, String> {
    if raw.is_empty() {
        return Err(format!(
            "Argument `{name}` is empty: give an absolute path on the server, such as /var/www."
        ));
    }
    if raw.split('/').any(|part| part == "..") {
        return Err(format!(
            "Argument `{name}` contains a `..` component, which the tools do not accept. Send the absolute path without it, such as /var/www rather than /var/www/app/.."
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
