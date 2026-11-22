//! The MCP server for coding agents: what an agent may do on the user's saved sites, and
//! the limits on it. Nothing here imports iced: the window only shows what this module
//! reports.

pub mod paths;

use std::time::Duration;

/// Entries a `list_directory` call answers without a `limit`.
// TODO(Task 9): remove
#[allow(dead_code)]
pub const LIST_DEFAULT: usize = 1000;
/// The most entries one `list_directory` call answers (`limit` is capped here).
// TODO(Task 9): remove
#[allow(dead_code)]
pub const LIST_MAX: usize = 5000;
/// The most bytes one `read_file` returns or one `write_file` takes (1 MiB).
// TODO(Task 9): remove
#[allow(dead_code)]
pub const FILE_MAX: usize = 1024 * 1024;
/// The most bytes of each stream (stdout, stderr) `run_command` keeps (256 KiB).
// TODO(Task 9): remove
#[allow(dead_code)]
pub const EXEC_STREAM_MAX: usize = 256 * 1024;
/// A command's time limit when the call names none.
// TODO(Task 9): remove
#[allow(dead_code)]
pub const EXEC_DEFAULT: Duration = Duration::from_secs(60);
/// The longest time limit a call may ask for.
// TODO(Task 9): remove
#[allow(dead_code)]
pub const EXEC_MAX: Duration = Duration::from_secs(600);
/// How long `download` / `upload` wait before answering `"running"`.
// TODO(Task 9): remove
#[allow(dead_code)]
pub const TRANSFER_WAIT: Duration = Duration::from_secs(60);
/// How long a call waits for a connection, security prompts included.
// TODO(Task 9): remove
#[allow(dead_code)]
pub const PROMPT_WAIT: Duration = Duration::from_secs(120);
/// An agent-only session closes after this long without a call.
// TODO(Task 9): remove
#[allow(dead_code)]
pub const IDLE_CLOSE: Duration = Duration::from_secs(10 * 60);
/// How often the idle sessions are looked for.
// TODO(Task 12): remove
#[allow(dead_code)]
pub const REAP_EVERY: Duration = Duration::from_secs(30);
/// The most failed items a transfer summary names.
// TODO(Task 9): remove
#[allow(dead_code)]
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
// TODO(Task 9): remove
#[allow(dead_code)]
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
}
