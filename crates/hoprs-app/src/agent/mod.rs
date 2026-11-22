//! The MCP server for coding agents: what an agent may do on the user's saved sites, and
//! the limits on it. Nothing here imports iced: the window only shows what this module
//! reports.

pub mod channel;
pub mod host;
pub mod paths;
pub mod proto;
pub mod server;
pub mod service;
#[cfg(test)]
pub mod testing;
pub mod tools;

use std::collections::VecDeque;
use std::time::Duration;

use hoprs_core::{Error, SiteId};
use tokio::sync::mpsc;

use crate::backend::SessionId;

/// What the agent side tells the window. None of it opens a tab or takes focus.
#[derive(Clone, Debug, PartialEq)]
pub enum AgentEvent {
    /// How many relays (`hoprs mcp` processes) are connected now.
    Relays(usize),
    /// The sites whose session only an agent uses, sorted.
    Sites(Vec<SiteId>),
    /// A session an agent asked for is up.
    Connected(SessionId),
    /// The channel could not start (`Some`), or started after a failure (`None`).
    Channel(Option<Error>),
}

/// The most agent events waiting for the UI (see [`latest_events`]).
pub const EVENTS_BUFFER: usize = 16;
/// The most [`AgentEvent::Connected`] kept while the UI does not read.
const CONNECTED_KEPT: usize = 16;

/// What [`latest_events`] holds back while the UI's channel is full: the latest of each
/// state, and the last few sessions that came up.
#[derive(Default)]
struct Held {
    channel: Option<Option<Error>>,
    relays: Option<usize>,
    sites: Option<Vec<SiteId>>,
    connected: VecDeque<SessionId>,
}

impl Held {
    fn add(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Channel(state) => self.channel = Some(state),
            AgentEvent::Relays(n) => self.relays = Some(n),
            AgentEvent::Sites(sites) => self.sites = Some(sites),
            AgentEvent::Connected(session) => {
                if self.connected.len() == CONNECTED_KEPT {
                    self.connected.pop_front();
                }
                self.connected.push_back(session);
            }
        }
    }

    fn next(&mut self) -> Option<AgentEvent> {
        if let Some(state) = self.channel.take() {
            return Some(AgentEvent::Channel(state));
        }
        if let Some(n) = self.relays.take() {
            return Some(AgentEvent::Relays(n));
        }
        if let Some(sites) = self.sites.take() {
            return Some(AgentEvent::Sites(sites));
        }
        self.connected.pop_front().map(AgentEvent::Connected)
    }

    fn is_empty(&self) -> bool {
        self.channel.is_none()
            && self.relays.is_none()
            && self.sites.is_none()
            && self.connected.is_empty()
    }
}

/// The agent side's events for the UI, in constant memory whether or not anyone reads:
/// at most [`EVENTS_BUFFER`] wait in the returned channel; past that, each state (the
/// channel's, the relay count, the agent sites) keeps only its latest value and only
/// the last few `Connected` are kept, and they follow as soon as there is room. So a
/// reader that comes late still ends on the current state. Spawns its task (inside the
/// tokio runtime); it ends when `from` closes and all is handed on, or the receiver is
/// dropped.
pub fn latest_events(mut from: mpsc::UnboundedReceiver<AgentEvent>) -> mpsc::Receiver<AgentEvent> {
    let (to, events) = mpsc::channel(EVENTS_BUFFER);
    tokio::spawn(async move {
        let mut held = Held::default();
        loop {
            if held.is_empty() {
                match from.recv().await {
                    Some(event) => held.add(event),
                    None => return,
                }
                continue;
            }
            tokio::select! {
                event = from.recv() => match event {
                    Some(event) => held.add(event),
                    None => {
                        while let Some(event) = held.next() {
                            if to.send(event).await.is_err() {
                                return;
                            }
                        }
                        return;
                    }
                },
                room = to.reserve() => match (room, held.next()) {
                    (Ok(room), Some(event)) => room.send(event),
                    (Ok(_), None) => {}
                    (Err(_), _) => return,
                },
            }
        }
    });
    events
}

/// Entries a `list_directory` call answers without a `limit`.
pub const LIST_DEFAULT: usize = 1000;
/// The most entries one `list_directory` call answers (`limit` is capped here).
pub const LIST_MAX: usize = 5000;
/// The most bytes one `read_file` returns or one `write_file` takes (1 MiB).
pub const FILE_MAX: usize = 1024 * 1024;
/// The most bytes of each stream (stdout, stderr) `run_command` keeps (256 KiB).
pub const EXEC_STREAM_MAX: usize = 256 * 1024;
/// A command's time limit when the call names none.
pub const EXEC_DEFAULT: Duration = Duration::from_secs(60);
/// The longest time limit a call may ask for.
pub const EXEC_MAX: Duration = Duration::from_secs(600);
/// How long `download` / `upload` wait before answering `"running"`.
pub const TRANSFER_WAIT: Duration = Duration::from_secs(60);
/// How long a call waits for a connection, security prompts included.
pub const PROMPT_WAIT: Duration = Duration::from_secs(120);
/// An agent-only session closes after this long without a call.
pub const IDLE_CLOSE: Duration = Duration::from_secs(10 * 60);
/// How often the idle sessions are looked for.
pub const REAP_EVERY: Duration = Duration::from_secs(30);
/// The most failed items a transfer summary names.
pub const FAILED_LISTED: usize = 20;
/// Calls in flight per relay; the next ones wait.
pub const MAX_IN_FLIGHT: usize = 8;
/// Relays connected at once; one more is refused at the hello.
pub const MAX_RELAYS: usize = 16;
/// A relay's hello must arrive within this.
pub const HELLO_WAIT: Duration = Duration::from_secs(5);

/// The limits the agent service applies, as a value so tests can shorten them.
/// [`Default`] is the constants above.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub list_default: usize,
    pub list_max: usize,
    pub file_max: usize,
    pub exec_stream: usize,
    pub exec_default: Duration,
    pub exec_max: Duration,
    pub transfer_wait: Duration,
    pub prompt_wait: Duration,
    pub idle_close: Duration,
    pub failed_listed: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            list_default: LIST_DEFAULT,
            list_max: LIST_MAX,
            file_max: FILE_MAX,
            exec_stream: EXEC_STREAM_MAX,
            exec_default: EXEC_DEFAULT,
            exec_max: EXEC_MAX,
            transfer_wait: TRANSFER_WAIT,
            prompt_wait: PROMPT_WAIT,
            idle_close: IDLE_CLOSE,
            failed_listed: FAILED_LISTED,
        }
    }
}

// ---- The texts an agent receives (English only, not in the Fluent catalogs) ----

/// The relay could not reach the app.
// TODO(Task 13): remove
#[allow(dead_code)]
pub const UNREACHABLE: &str = "hoprs is not reachable: it is not running, or agent access is off. Open hoprs and turn on Settings › Agents (MCP), then call the tool again.";
/// The app closed the channel because the user turned agent access off.
pub const TURNED_OFF: &str = "Agent access was turned off in hoprs. Turn on Settings › Agents (MCP) in the hoprs window, then call the tool again.";
/// The running window is another version than the relay.
// TODO(Task 13): remove
#[allow(dead_code)]
pub fn version_mismatch(app: &str, relay: &str) -> String {
    format!(
        "hoprs was updated while its window stayed open (window {app}, this program {relay}). Restart hoprs, then call the tool again."
    )
}
/// An unknown site id, and a site with no access: one text for both.
pub const SITE_UNAVAILABLE: &str =
    "No site with this id is open to agents. Call list_sites to see the sites you can use.";
/// The site's level is reading only.
pub fn read_only(site: &str) -> String {
    format!(
        "The site \"{site}\" allows reading only. The user can allow more in hoprs: edit the site, Agents (MCP)."
    )
}
/// `run_command` on a site that is not SSH.
pub fn ssh_only(site: &str) -> String {
    format!(
        "The site \"{site}\" is not an SSH site, and only SSH sites can run commands. Use the file tools on it instead."
    )
}
/// A protected local path.
pub fn protected(path: &str) -> String {
    format!(
        "{path} is protected: agents cannot transfer to or from hoprs's own files, ~/.ssh or the key files of saved sites."
    )
}
/// `transfer_status` of anything but an agent's transfer on a site the caller may read:
/// one text for an unknown id, a user's transfer, a cleared one and a closed site.
pub const NO_SUCH_TRANSFER: &str = "No such agent transfer. Pass the transfer number that download or upload answered; a transfer cleared from the hoprs queue, or on a site no longer open to agents, is not known.";
/// A connection needs an answer from the user.
pub const WAITING_FOR_USER: &str = "hoprs is waiting for the user: a question about this connection (host key, certificate, password or passphrase) must be answered in the hoprs window. Ask the user, then call the tool again.";
/// The server's host key changed.
pub const HOST_KEY_CHANGED: &str = "The server's host key changed. The user has to resolve it in the hoprs window; an agent cannot accept a new key.";
/// The server's `instructions`.
// TODO(Task 13): remove
#[allow(dead_code)]
pub const INSTRUCTIONS: &str = "hoprs is a desktop client for SSH, SFTP, FTP and FTPS. These tools work on the sites the user saved in hoprs and opened to agents; the user sees every action in hoprs's log. Call list_sites first: it returns the sites you can use, with the id every other tool wants. Each site has one of two levels: read (list folders, read files, download) or all (also write, rename, delete, change permissions, upload, and run commands on SSH sites). A tool that needs more than the site allows is refused with a message saying so. Remote paths are absolute. There is no interactive terminal. If hoprs needs the user to answer a question about a connection, the tool says so: ask the user, then call it again.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_limits_are_the_rulings_values() {
        let limits = Limits::default();
        assert_eq!(limits.list_default, 1000);
        assert_eq!(limits.list_max, 5000);
        assert_eq!(limits.file_max, 1_048_576);
        assert_eq!(limits.exec_stream, 262_144);
        assert_eq!(limits.exec_default, Duration::from_secs(60));
        assert_eq!(limits.exec_max, Duration::from_secs(600));
        assert_eq!(limits.transfer_wait, Duration::from_secs(60));
        assert_eq!(limits.prompt_wait, Duration::from_secs(120));
        assert_eq!(limits.idle_close, Duration::from_secs(600));
        assert_eq!(limits.failed_listed, 20);
        assert_eq!(REAP_EVERY, Duration::from_secs(30));
        assert_eq!(
            (MAX_IN_FLIGHT, MAX_RELAYS, HELLO_WAIT),
            (8, 16, Duration::from_secs(5))
        );
    }
    #[tokio::test]
    async fn events_keep_the_latest_state_in_constant_memory() {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut events = latest_events(rx);
        // Nobody reads while a run's worth of events comes.
        for n in 0..1000 {
            tx.send(AgentEvent::Relays(n % 17)).unwrap();
            tx.send(AgentEvent::Sites(vec![SiteId(format!("s{n}"))]))
                .unwrap();
            tx.send(AgentEvent::Connected(SessionId(n as u64))).unwrap();
        }
        tx.send(AgentEvent::Relays(3)).unwrap();
        drop(tx);
        // Let the hand-on catch up with nobody reading: the channel is full, the rest
        // held.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut got = Vec::new();
        while let Some(event) = events.recv().await {
            got.push(event);
        }
        assert!(
            got.len() <= EVENTS_BUFFER + 3 + CONNECTED_KEPT,
            "{} events kept",
            got.len()
        );
        let last_relays = got.iter().rev().find_map(|e| match e {
            AgentEvent::Relays(n) => Some(*n),
            _ => None,
        });
        assert_eq!(last_relays, Some(3), "the latest count wins");
        let last_sites = got.iter().rev().find_map(|e| match e {
            AgentEvent::Sites(s) => Some(s.clone()),
            _ => None,
        });
        assert_eq!(last_sites, Some(vec![SiteId("s999".into())]));
        assert_eq!(
            got.last(),
            Some(&AgentEvent::Connected(SessionId(999))),
            "the last sessions that came up are kept"
        );
    }

    #[tokio::test]
    async fn events_pass_in_order_while_read() {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut events = latest_events(rx);
        for n in [1, 2, 1, 0] {
            tx.send(AgentEvent::Relays(n)).unwrap();
            assert_eq!(events.recv().await, Some(AgentEvent::Relays(n)));
        }
    }

    #[test]
    fn texts_tell_what_to_do() {
        assert!(UNREACHABLE.contains("Settings › Agents (MCP)"));
        assert!(TURNED_OFF.contains("Settings › Agents (MCP)"));
        assert!(version_mismatch("1.6.0", "1.6.1").contains("Restart hoprs"));
        assert!(version_mismatch("1.6.0", "1.6.1").contains("window 1.6.0"));
        assert!(SITE_UNAVAILABLE.contains("list_sites"));
        assert!(read_only("Acme").contains("\"Acme\" allows reading only"));
        assert!(ssh_only("Acme").contains("\"Acme\""));
        assert!(ssh_only("Acme").contains("SSH"));
        assert!(protected("/home/u/.ssh/id").starts_with("/home/u/.ssh/id is protected"));
        assert!(WAITING_FOR_USER.contains("hoprs window"));
        assert!(HOST_KEY_CHANGED.contains("host key changed"));
        assert!(INSTRUCTIONS.contains("list_sites"));
        assert!(INSTRUCTIONS.contains("log"));
    }
}
