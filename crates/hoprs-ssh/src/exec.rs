//! One command on an exec channel.
//!
//! [`SshSession::exec`] runs a single command on a new channel of the session's connection:
//! no PTY, no stdin, stdout and stderr kept apart, each capped, with a time limit. The
//! limits are what makes it safe to hand to an agent: output past the cap is read and
//! dropped as it arrives (never buffered, and the command is never blocked on a full pipe),
//! the time limit ends the wait whatever the remote process does (the close is sent in the
//! background and never waited for), and dropping the future closes the channel and gives
//! its slot of the channel budget back. The channel counts like a PTY ([`crate::channels`]).
//!
//! The command goes to the server as given; this module does not interpret, quote or log
//! it, and never logs output.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use hoprs_core::{Error, ErrorKind, Result};
use russh::{ChannelMsg, Sig};
use tokio::time::Instant;

use crate::channels::ChannelSlot;
use crate::session::{SshSession, map_russh_error};
use crate::shell::{ShellReader, ShellWriter, close_detached, expect_reply};

/// The `ExtendedData` type code of stderr (RFC 4254 section 5.2).
const STDERR: u32 = 1;

/// What bounds one command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecLimits {
    /// Most bytes kept of each of stdout and stderr; the rest is read and discarded.
    pub stream_bytes: usize,
    /// How long the whole call may take (channel open to end), after which the channel is
    /// closed and the result is what was read.
    pub timeout: Duration,
}

/// How a command ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecEnd {
    /// The command's exit status.
    Exit(u32),
    /// Killed by this signal (its name without `SIG`, as the server sent it).
    Signal(String),
    /// The time limit ran out; the channel was closed.
    TimedOut,
    /// The channel closed without a status: the connection was lost, or the server closed it.
    Lost,
}

/// What a command produced.
#[derive(Clone, PartialEq, Eq)]
pub struct ExecOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Output past `stream_bytes` was dropped.
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub end: ExecEnd,
}

/// Lengths and the end only: the output itself may hold anything.
impl fmt::Debug for ExecOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecOutput")
            .field("stdout_len", &self.stdout.len())
            .field("stderr_len", &self.stderr.len())
            .field("stdout_truncated", &self.stdout_truncated)
            .field("stderr_truncated", &self.stderr_truncated)
            .field("end", &self.end)
            .finish()
    }
}

/// Appends `data` to `kept` up to `cap` bytes in all; returns whether anything was dropped.
fn keep(kept: &mut Vec<u8>, data: &[u8], cap: usize) -> bool {
    let room = cap.saturating_sub(kept.len());
    kept.extend_from_slice(&data[..data.len().min(room)]);
    data.len() > room
}

fn signal_name(sig: &Sig) -> String {
    match sig {
        Sig::Custom(name) => name.clone(),
        other => format!("{other:?}"),
    }
}

/// Reads the channel until it closes, or until `deadline` (then the end is `TimedOut`).
/// Nothing but the capped streams is kept, so the memory held is at most `2 * stream_bytes`
/// however much the command prints.
async fn collect_until(
    reader: &mut dyn ShellReader,
    stream_bytes: usize,
    deadline: Option<Instant>,
) -> ExecOutput {
    let mut out = ExecOutput {
        stdout: Vec::new(),
        stderr: Vec::new(),
        stdout_truncated: false,
        stderr_truncated: false,
        end: ExecEnd::Lost,
    };
    let mut status: Option<ExecEnd> = None;
    loop {
        let msg = match deadline {
            // A reader that is always ready never lets the timer fire: check the clock too.
            Some(at) if Instant::now() >= at => {
                out.end = ExecEnd::TimedOut;
                return out;
            }
            Some(at) => match tokio::time::timeout_at(at, reader.wait()).await {
                Ok(msg) => msg,
                Err(_) => {
                    out.end = ExecEnd::TimedOut;
                    return out;
                }
            },
            None => reader.wait().await,
        };
        match msg {
            Some(ChannelMsg::Data { data }) => {
                out.stdout_truncated |= keep(&mut out.stdout, &data, stream_bytes);
            }
            Some(ChannelMsg::ExtendedData { data, ext }) if ext == STDERR => {
                out.stderr_truncated |= keep(&mut out.stderr, &data, stream_bytes);
            }
            Some(ChannelMsg::ExitStatus { exit_status }) => {
                status.get_or_insert(ExecEnd::Exit(exit_status));
            }
            Some(ChannelMsg::ExitSignal {
                signal_name: sig, ..
            }) => {
                status.get_or_insert(ExecEnd::Signal(signal_name(&sig)));
            }
            Some(ChannelMsg::Close) | None => {
                out.end = status.unwrap_or(ExecEnd::Lost);
                return out;
            }
            Some(_) => {}
        }
    }
}

/// [`collect_until`] without a time limit.
#[cfg_attr(not(test), allow(dead_code))] // exec uses collect_until; this is the unbounded form
pub(crate) async fn collect(reader: &mut dyn ShellReader, stream_bytes: usize) -> ExecOutput {
    collect_until(reader, stream_bytes, None).await
}

/// Closes the channel and gives the budget slot back when dropped: when the call ends, and
/// when its future is dropped half way. The close is sent in the background.
struct CloseOnDrop {
    writer: Arc<dyn ShellWriter>,
    _slot: ChannelSlot,
}

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        close_detached(&self.writer);
    }
}

fn timed_out() -> ExecOutput {
    ExecOutput {
        stdout: Vec::new(),
        stderr: Vec::new(),
        stdout_truncated: false,
        stderr_truncated: false,
        end: ExecEnd::TimedOut,
    }
}

impl SshSession {
    /// Runs `command` on a new exec channel of this connection and returns what it printed
    /// and how it ended; see the module docs for the limits. There is no stdin (the channel
    /// is half-closed at once) and no PTY.
    ///
    /// The channel takes a slot of the channel budget like a PTY. While the session is not
    /// connected this fails at once with `Network`, "Not connected"; past the budget, or
    /// when the server refuses the channel, with `Protocol`, "Too many channels on this
    /// connection (limit N)"; when the server refuses the command, with `Protocol`,
    /// "Server refused the command". Other users of the connection are not affected.
    pub async fn exec(&self, command: &str, limits: ExecLimits) -> Result<ExecOutput> {
        let deadline = Instant::now() + limits.timeout;
        let opened = tokio::time::timeout_at(deadline, self.open_pty_channel())
            .await
            .map_err(|_| {
                Error::new(
                    ErrorKind::Network,
                    "Timed out opening a channel for the command",
                )
            })??;
        let (channel, slot) = opened.into_parts();
        let (mut reader, writer) = channel.split();
        let writer = Arc::new(writer);
        let _guard = CloseOnDrop {
            writer: writer.clone(),
            _slot: slot,
        };
        writer.exec(true, command).await.map_err(map_russh_error)?;
        let refused = Error::new(ErrorKind::Protocol, "Server refused the command");
        match tokio::time::timeout_at(deadline, expect_reply(&mut reader, "exec", refused)).await {
            Ok(reply) => reply?,
            Err(_) => return Ok(timed_out()),
        }
        // No stdin: the command sees end of input at once.
        writer.eof().await.map_err(map_russh_error)?;
        Ok(collect_until(&mut reader, limits.stream_bytes, Some(deadline)).await)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use async_trait::async_trait;

    use super::*;

    /// A reader that yields the scripted messages, then `None` (the channel is gone).
    struct Script(VecDeque<ChannelMsg>);

    fn script(msgs: Vec<ChannelMsg>) -> Script {
        Script(msgs.into())
    }

    #[async_trait]
    impl ShellReader for Script {
        async fn wait(&mut self) -> Option<ChannelMsg> {
            self.0.pop_front()
        }
    }

    fn data(bytes: &[u8]) -> ChannelMsg {
        ChannelMsg::Data {
            data: bytes.to_vec().into(),
        }
    }

    fn ext(bytes: &[u8], ext: u32) -> ChannelMsg {
        ChannelMsg::ExtendedData {
            data: bytes.to_vec().into(),
            ext,
        }
    }

    #[tokio::test]
    async fn collect_splits_the_streams_and_reads_the_status() {
        let mut r = script(vec![
            data(b"out"),
            ext(b"err", 1),
            ChannelMsg::ExitStatus { exit_status: 3 },
            ChannelMsg::Eof,
            ChannelMsg::Close,
        ]);
        let out = collect(&mut r, 1024).await;
        assert_eq!(out.stdout, b"out");
        assert_eq!(out.stderr, b"err");
        assert_eq!(out.end, ExecEnd::Exit(3));
        assert!(!out.stdout_truncated && !out.stderr_truncated);
    }

    #[tokio::test]
    async fn collect_caps_each_stream_and_keeps_draining() {
        let cap = 256 * 1024;
        // 300 KiB in 32 KiB pieces.
        let mut msgs: Vec<ChannelMsg> = (0..9).map(|_| data(&[b'x'; 32 * 1024])).collect();
        msgs.push(data(&[b'x'; 12 * 1024]));
        msgs.push(ChannelMsg::ExitStatus { exit_status: 0 });
        let mut r = script(msgs);
        let out = collect(&mut r, cap).await;
        assert_eq!(out.stdout.len(), 262_144);
        assert!(out.stdout_truncated);
        assert_eq!(out.end, ExecEnd::Exit(0));
        assert!(out.stderr.is_empty() && !out.stderr_truncated);
    }

    #[tokio::test]
    async fn collect_reports_a_signal_and_a_lost_channel() {
        let mut r = script(vec![
            ChannelMsg::ExitSignal {
                signal_name: Sig::KILL,
                core_dumped: false,
                error_message: String::new(),
                lang_tag: String::new(),
            },
            ChannelMsg::Close,
        ]);
        assert_eq!(
            collect(&mut r, 8).await.end,
            ExecEnd::Signal("KILL".to_owned())
        );
        let mut r = script(vec![data(b"partial"), ChannelMsg::Close]);
        let out = collect(&mut r, 8).await;
        assert_eq!(out.end, ExecEnd::Lost);
        assert_eq!(out.stdout, b"partial");
        // The reader vanishing (connection gone) is the same.
        let mut r = script(vec![]);
        assert_eq!(collect(&mut r, 8).await.end, ExecEnd::Lost);
    }

    #[tokio::test]
    async fn other_extended_streams_are_ignored() {
        let mut r = script(vec![
            ext(b"nope", 2),
            ext(b"err", 1),
            ChannelMsg::ExitStatus { exit_status: 0 },
            ChannelMsg::Close,
        ]);
        let out = collect(&mut r, 1024).await;
        assert_eq!(out.stderr, b"err");
        assert!(out.stdout.is_empty());
    }

    #[test]
    fn debug_shows_lengths_and_never_the_output() {
        let out = ExecOutput {
            stdout: b"hunter2".to_vec(),
            stderr: b"token".to_vec(),
            stdout_truncated: false,
            stderr_truncated: true,
            end: ExecEnd::Exit(0),
        };
        let shown = format!("{out:?}");
        assert!(
            !shown.contains("hunter2") && !shown.contains("token"),
            "{shown}"
        );
        assert!(shown.contains("stdout_len: 7"), "{shown}");
    }

    /// A command that prints for ever, never reporting an end.
    struct Flood;

    #[async_trait]
    impl ShellReader for Flood {
        async fn wait(&mut self) -> Option<ChannelMsg> {
            Some(data(&[b'y'; 32 * 1024]))
        }
    }

    /// A command that never prints and never ends.
    struct Silent;

    #[async_trait]
    impl ShellReader for Silent {
        async fn wait(&mut self) -> Option<ChannelMsg> {
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn the_deadline_ends_a_flood_and_a_silent_command() {
        let at = Instant::now() + Duration::from_millis(200);
        let out = collect_until(&mut Flood, 1000, Some(at)).await;
        assert_eq!(out.end, ExecEnd::TimedOut);
        assert_eq!(out.stdout.len(), 1000);
        assert!(out.stdout_truncated);

        let at = Instant::now() + Duration::from_millis(100);
        let out = collect_until(&mut Silent, 1000, Some(at)).await;
        assert_eq!(out.end, ExecEnd::TimedOut);
        assert!(out.stdout.is_empty());
    }
}
