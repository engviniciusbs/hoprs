//! Channel budget: how many channels one SSH connection may have open at once.
//!
//! OpenSSH refuses more than `MaxSessions` (10 by default) open session channels per
//! connection; past it, a terminal would fail with an obscure open failure, or the transfers
//! would starve it. So [`SshSession`](crate::SshSession) counts its channels against
//! [`SshConfig::max_channels`](crate::SshConfig::max_channels) (default
//! [`DEFAULT_MAX_CHANNELS`] = 10).
//!
//! **What counts.** Every channel opened through
//! [`SshSession::open_channel`](crate::SshSession::open_channel) holds one slot for as long
//! as its [`ChannelSlot`] lives: the SFTP metadata channel, each SFTP stream channel and each
//! PTY ([`SshSession::open_shell`](crate::SshSession::open_shell)) and each exec channel
//! ([`SshSession::exec`](crate::SshSession::exec), which counts like a PTY: it never takes
//! the last slot). A shell and an exec give their slot
//! back as soon as it ends (closed by us, by the server, or by a lost connection), without
//! waiting for the server's reply to our close.
//!
//! **Reservation for terminals (ruling R2).** SFTP opens at most
//! [`sftp_stream_channels`]`(max_channels, configured)` =
//! `min(configured, max_channels - 1 metadata - 3 reserved)` stream channels (at least 1),
//! so with the default 10 the transfers use at most 1 + 6 slots and [`PTY_RESERVE`] = 3
//! stay free: two for terminals, and the last one, which a PTY never takes (below). Two
//! terminals therefore always fit next to a busy SFTP. Below 5 the reservation shrinks:
//! the one stream channel SFTP needs comes first.
//!
//! **PTYs never take the last slot.** A PTY opens only while fewer than
//! `max_channels - 1` channels are open: the SFTP metadata channel can always be opened
//! (again: after a reconnect the terminals may reopen before SFTP does), however many
//! terminals there are.
//!
//! **Over the limit.** Opening a channel past `max_channels` fails at once, without asking
//! the server, with `ErrorKind::Protocol`, "Too many channels on this connection (limit N)".
//! A channel open the server refuses (its own `MaxSessions` is lower than ours) gives the
//! same error, with the server's answer as the detail. The refused channel holds no slot.
//!
//! **Reconnects.** The count belongs to one connection (one generation of the session).
//! A new connection starts from zero; slots of the dead connection are not counted
//! anymore, and dropping them later does not free slots of the new one.

use std::sync::{Arc, Mutex, PoisonError};

use hoprs_core::{Error, ErrorKind, Result};

/// OpenSSH's default `MaxSessions`.
pub const DEFAULT_MAX_CHANNELS: usize = 10;

/// Slots the SFTP stream channels leave free: two terminals, plus the slot PTYs never take
/// (kept for the metadata channel).
pub const PTY_RESERVE: usize = 3;

/// The SFTP metadata channel.
const METADATA_CHANNELS: usize = 1;

/// The error for a channel past `limit` (ours or the server's).
pub(crate) fn too_many_channels(limit: usize) -> Error {
    Error::coded(
        ErrorKind::Protocol,
        "ssh-too-many-channels",
        format!("Too many channels on this connection (limit {limit})"),
    )
    .arg("limit", limit.to_string())
}

/// Whether `error` is the "Too many channels" error of a channel open.
pub fn is_channel_limit(error: &Error) -> bool {
    error.code == Some("ssh-too-many-channels")
}

/// How many SFTP stream channels fit under `max_channels` beside the metadata channel and
/// the terminal reserve: `min(configured, max_channels - 1 - 3)`, at least 1.
pub fn sftp_stream_channels(max_channels: usize, configured: usize) -> usize {
    configured
        .min(max_channels.saturating_sub(METADATA_CHANNELS + PTY_RESERVE))
        .max(1)
}

/// The count of open channels of the current connection.
#[derive(Debug)]
pub(crate) struct ChannelBudget {
    limit: usize,
    count: Mutex<Count>,
}

#[derive(Debug, Default)]
struct Count {
    /// The connection the count belongs to; 0 before the first one.
    generation: u64,
    open: usize,
}

impl ChannelBudget {
    /// A budget of `limit` channels (at least 1).
    pub(crate) fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit: limit.max(1),
            count: Mutex::new(Count::default()),
        })
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit
    }

    fn count(&self) -> std::sync::MutexGuard<'_, Count> {
        self.count.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Channels counted for the current connection.
    pub(crate) fn open(&self) -> usize {
        self.count().open
    }

    /// A new connection (`generation`) is up: it starts with no channel.
    pub(crate) fn reset(&self, generation: u64) {
        *self.count() = Count {
            generation,
            open: 0,
        };
    }

    /// Takes a slot for a channel of connection `generation`. Fails with "Too many
    /// channels" when all are taken, and with `Network`, "Not connected" when `generation`
    /// is no longer the current connection.
    pub(crate) fn acquire(self: &Arc<Self>, generation: u64) -> Result<ChannelSlot> {
        self.acquire_below(generation, self.limit)
    }

    /// Takes a slot for a PTY: like [`acquire`](Self::acquire), but only while fewer than
    /// `limit - 1` channels are open, so the last slot always stays free for the SFTP
    /// metadata channel (which a reconnect or a lost channel may have to open again).
    pub(crate) fn acquire_pty(self: &Arc<Self>, generation: u64) -> Result<ChannelSlot> {
        self.acquire_below(generation, self.limit - METADATA_CHANNELS)
    }

    /// Takes a slot if fewer than `cap` are open; the error names the whole limit.
    fn acquire_below(self: &Arc<Self>, generation: u64, cap: usize) -> Result<ChannelSlot> {
        let mut count = self.count();
        if count.generation != generation {
            return Err(Error::coded(
                ErrorKind::Network,
                "net-not-connected",
                "Not connected",
            ));
        }
        if count.open >= cap {
            return Err(too_many_channels(self.limit));
        }
        count.open += 1;
        Ok(ChannelSlot {
            budget: self.clone(),
            generation,
        })
    }
}

/// One channel's place in the budget; dropping it frees the place. See the module docs.
#[derive(Debug)]
pub struct ChannelSlot {
    budget: Arc<ChannelBudget>,
    generation: u64,
}

impl ChannelSlot {
    /// The session generation (connection) the channel was opened on.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

impl Drop for ChannelSlot {
    fn drop(&mut self) {
        let mut count = self.budget.count();
        // A slot of a dead connection was forgotten by `reset` already.
        if count.generation == self.generation {
            count.open = count.open.saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use hoprs_core::ErrorKind;

    use super::*;

    #[test]
    fn the_limit_error_is_coded_and_recognized() {
        let e = too_many_channels(3);
        assert_eq!(e.code, Some("ssh-too-many-channels"));
        assert_eq!(e.args, vec![("limit", "3".to_owned())]);
        assert!(is_channel_limit(&e));
    }

    #[test]
    fn channel_budget_reserves_room_for_a_pty() {
        // Metadata + streams leave three slots (ruling R2): two PTYs and the spare.
        assert_eq!(sftp_stream_channels(10, 8), 6);
        assert_eq!(sftp_stream_channels(10, 4), 4);
        assert_eq!(sftp_stream_channels(5, 8), 1);
        assert_eq!(sftp_stream_channels(4, 8), 1);
        assert_eq!(sftp_stream_channels(1, 8), 1);

        // stream_channels = 8 on the default budget: two terminals still fit.
        let budget = ChannelBudget::new(10);
        budget.reset(1);
        let streams = sftp_stream_channels(budget.limit(), 8);
        let mut held: Vec<ChannelSlot> = (0..1 + streams)
            .map(|_| budget.acquire(1).expect("metadata and stream channels"))
            .collect();
        assert_eq!(budget.open(), 7);
        held.push(budget.acquire_pty(1).expect("first PTY"));
        held.push(budget.acquire_pty(1).expect("second PTY"));
        assert_eq!(budget.open(), 9);

        // A third PTY would take the metadata's spare slot: refused.
        let e = budget.acquire_pty(1).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Protocol);
        assert_eq!(e.message, "Too many channels on this connection (limit 10)");
        assert!(is_channel_limit(&e));
        assert_eq!(budget.open(), 9, "a refused slot is not counted");
        // The spare still takes a plain channel; past it, nothing does.
        held.push(budget.acquire(1).expect("the spare slot"));
        assert!(is_channel_limit(&budget.acquire(1).unwrap_err()));

        held.pop();
        held.pop();
        assert_eq!(budget.open(), 8, "a closed channel frees its slot");
        let _pty = budget.acquire_pty(1).expect("the freed slot");
    }

    #[test]
    fn ptys_always_leave_the_metadata_slot() {
        // A PTY only opens while open < max_channels - 1: SFTP can always (re)open its
        // metadata channel, however many terminals are open.
        let budget = ChannelBudget::new(4);
        budget.reset(1);
        let _ptys: Vec<ChannelSlot> = (0..3)
            .map(|_| budget.acquire_pty(1).expect("PTY under the cap"))
            .collect();
        assert_eq!(budget.open(), 3);
        let e = budget.acquire_pty(1).unwrap_err();
        assert!(is_channel_limit(&e), "{e:?}");
        assert_eq!(e.message, "Too many channels on this connection (limit 4)");
        assert_eq!(budget.open(), 3, "a refused PTY holds no slot");
        // The last slot still takes the metadata channel (a plain acquire).
        let metadata = budget.acquire(1).expect("the metadata slot is left");
        assert_eq!(budget.open(), 4);
        drop(metadata);

        // With SFTP holding metadata + streams, PTYs stop one short of the limit too.
        let budget = ChannelBudget::new(10);
        budget.reset(1);
        let sftp: Vec<ChannelSlot> = (0..1 + sftp_stream_channels(10, 8))
            .map(|_| budget.acquire(1).unwrap())
            .collect();
        let ptys = [
            budget.acquire_pty(1).unwrap(),
            budget.acquire_pty(1).unwrap(),
        ];
        assert!(
            budget.acquire_pty(1).is_err(),
            "the last slot is not a PTY's"
        );
        drop(ptys);
        assert!(budget.acquire_pty(1).is_ok());
        // An old generation is "Not connected", as for any channel.
        assert_eq!(budget.acquire_pty(0).unwrap_err().kind, ErrorKind::Network);
        // A budget of 1 has no room for a PTY at all.
        let tiny = ChannelBudget::new(1);
        tiny.reset(1);
        assert!(tiny.acquire_pty(1).is_err());
        drop(sftp);
    }

    #[test]
    fn channel_budget_resets_on_reconnect() {
        let budget = ChannelBudget::new(2);
        budget.reset(1);
        let old = [budget.acquire(1).unwrap(), budget.acquire(1).unwrap()];
        assert!(budget.acquire(1).is_err());
        assert_eq!(old[0].generation(), 1);

        // A new connection starts empty: the dead connection's channels no longer count.
        budget.reset(2);
        assert_eq!(budget.open(), 0);
        let fresh = budget.acquire(2).unwrap();
        // Dropping a channel of the dead connection does not free a slot of the new one.
        drop(old);
        assert_eq!(budget.open(), 1);
        // A slot asked for the old generation after the reset is refused.
        assert_eq!(budget.acquire(1).unwrap_err().kind, ErrorKind::Network);
        drop(fresh);
        assert_eq!(budget.open(), 0);
    }
}
