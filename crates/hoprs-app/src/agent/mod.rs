//! The MCP server for coding agents: what an agent may do on the user's saved sites, and
//! the limits on it. Nothing here imports iced: the window only shows what this module
//! reports.

pub mod host;
pub mod paths;
pub mod proto;
pub mod service;
#[cfg(test)]
pub mod testing;
pub mod tools;

use std::time::Duration;

use hoprs_core::{Error, SiteId};

use crate::backend::SessionId;

/// What the agent side tells the window. None of it opens a tab or takes focus.
#[derive(Clone, Debug, PartialEq)]
pub enum AgentEvent {
    /// How many relays (`hoprs mcp` processes) are connected now.
    // TODO(Task 12): remove
    #[allow(dead_code)]
    Relays(usize),
    /// The sites whose session only an agent uses, sorted.
    Sites(Vec<SiteId>),
    /// A session an agent asked for is up.
    Connected(SessionId),
    /// The channel could not start (`Some`), or started after a failure (`None`).
    // TODO(Task 12): remove
    #[allow(dead_code)]
    Channel(Option<Error>),
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
// TODO(Task 12): remove
#[allow(dead_code)]
pub const REAP_EVERY: Duration = Duration::from_secs(30);
/// The most failed items a transfer summary names.
pub const FAILED_LISTED: usize = 20;
/// Calls in flight per relay; the next ones wait.
// TODO(Task 12): remove
#[allow(dead_code)]
pub const MAX_IN_FLIGHT: usize = 8;
/// Relays connected at once; one more is refused at the hello.
// TODO(Task 12): remove
#[allow(dead_code)]
pub const MAX_RELAYS: usize = 16;
/// A relay's hello must arrive within this.
// TODO(Task 12): remove
#[allow(dead_code)]
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
