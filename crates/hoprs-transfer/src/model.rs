use std::time::SystemTime;

use hoprs_core::{Error, RemotePath, SiteId};

/// Identifier of a transfer item in the engine queue.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct TransferId(pub i64);

/// Direction of a transfer relative to the local machine.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Upload,
    Download,
}

/// What to do when the destination already exists.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ConflictPolicy {
    Ask,
    Overwrite,
    Skip,
    OverwriteIfNewer,
    Resume,
}

/// Who asked for a transfer: the person using hoprs, or an agent through
/// the MCP server. Every item of a tree has its root's origin.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Origin {
    #[default]
    User,
    Agent,
}

/// User request to transfer one path. The engine discovers whether it is a file or a folder via `stat` on the source.
#[derive(Clone, Debug, PartialEq)]
pub struct TransferSpec {
    pub site: SiteId,
    pub direction: Direction,
    pub local: RemotePath,
    pub remote: RemotePath,
}

/// Lifecycle state of a transfer item.
#[derive(Clone, Debug, PartialEq)]
pub enum TransferState {
    Queued,
    Active,
    Paused,
    AwaitingDecision(ConflictInfo),
    Done,
    Skipped(String),
    Failed(Error),
    Cancelled,
}

impl TransferState {
    /// Returns true when the item will not change state again.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TransferState::Done
                | TransferState::Skipped(_)
                | TransferState::Failed(_)
                | TransferState::Cancelled
        )
    }
}

/// Source and destination metadata presented to the user when a conflict needs a decision.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ConflictInfo {
    pub src_size: u64,
    pub dst_size: u64,
    pub src_modified: Option<SystemTime>,
    pub dst_modified: Option<SystemTime>,
}

/// One row of the transfer queue.
#[derive(Clone, Debug, PartialEq)]
pub struct TransferItem {
    pub id: TransferId,
    pub parent: Option<TransferId>,
    pub spec: TransferSpec,
    pub is_dir: Option<bool>,
    pub size: Option<u64>,
    pub bytes_done: u64,
    pub state: TransferState,
    pub origin: Origin,
    /// The conflict policy the item's tree was enqueued with (see
    /// `TransferEngine::enqueue_with`): what the item runs under when it has
    /// no continuation of its own, instead of the global policy.
    pub tree_policy: Option<ConflictPolicy>,
}

#[cfg(test)]
mod tests {
    use hoprs_core::{Error, ErrorKind};

    use super::TransferState;

    #[test]
    fn terminal_states_are_done_skipped_failed_and_cancelled() {
        assert!(TransferState::Done.is_terminal());
        assert!(TransferState::Skipped("exists".to_string()).is_terminal());
        assert!(TransferState::Failed(Error::new(ErrorKind::Io, "boom")).is_terminal());
        assert!(TransferState::Cancelled.is_terminal());

        assert!(!TransferState::Queued.is_terminal());
        assert!(!TransferState::Active.is_terminal());
        assert!(!TransferState::Paused.is_terminal());
    }
}
