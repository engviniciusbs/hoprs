//! Control commands on items (pause, resume, cancel, retry, resolve). A
//! command acts on an item and, for a directory, on its descendants. An
//! item holding a worker slot cannot change state until its worker reports
//! back: the command records an `Intent` and cancels the item's token, and
//! the worker's result is mapped through that intent.

use std::collections::HashMap;

use hoprs_core::{Error, ErrorKind, Result};

use super::actor::{Actor, Intent};
use super::dirs::is_duplicate_name;
use super::{Resolution, ResolveAction};
use crate::model::{ConflictPolicy, TransferId, TransferState};

/// A control command on one item.
#[derive(Copy, Clone, Debug)]
pub(super) enum Control {
    Pause,
    Resume,
    Cancel,
    Retry,
    Resolve(Resolution),
}

const USER_SKIP: &str = "skipped by user";

/// Failed or cancelled, except a duplicate-name child, which stays failed.
fn is_retryable(state: &TransferState) -> bool {
    matches!(state, TransferState::Failed(_) | TransferState::Cancelled)
        && !is_duplicate_name(state)
}

impl Actor {
    pub(super) fn control(
        &mut self,
        id: TransferId,
        state: &TransferState,
        op: Control,
    ) -> Result<()> {
        match op {
            Control::Pause | Control::Cancel if state.is_terminal() => Err(Error::coded(
                ErrorKind::Conflict,
                "transfer-already-finished",
                "Transfer already finished",
            )),
            Control::Pause if *state == TransferState::Paused => Ok(()),
            Control::Pause => {
                self.stop_subtree(id, Intent::Pause);
                Ok(())
            }
            Control::Cancel => {
                self.stop_subtree(id, Intent::Cancel);
                Ok(())
            }
            Control::Resume if *state != TransferState::Paused => Err(Error::coded(
                ErrorKind::Conflict,
                "transfer-not-paused",
                "Transfer is not paused",
            )),
            Control::Resume => {
                self.restart_subtree(id, |s| *s == TransferState::Paused, Intent::Pause);
                Ok(())
            }
            Control::Retry if is_duplicate_name(state) => Err(Error::coded(
                ErrorKind::Conflict,
                "transfer-duplicate-name-no-retry",
                "A duplicate name in a directory listing cannot be retried",
            )),
            Control::Retry if !is_retryable(state) => Err(Error::coded(
                ErrorKind::Conflict,
                "transfer-not-failed-or-cancelled",
                "Transfer is not failed or cancelled",
            )),
            Control::Retry => {
                self.restart_subtree(id, is_retryable, Intent::Cancel);
                Ok(())
            }
            Control::Resolve(_) if !matches!(state, TransferState::AwaitingDecision(_)) => {
                Err(Error::coded(
                    ErrorKind::Conflict,
                    "transfer-not-awaiting-decision",
                    "Transfer is not awaiting a decision",
                ))
            }
            Control::Resolve(resolution) => {
                self.resolve(id, resolution);
                Ok(())
            }
        }
    }

    /// Child ids of every directory that has been listed into child items.
    pub(super) fn children_index(&self) -> HashMap<TransferId, Vec<TransferId>> {
        let mut children: HashMap<TransferId, Vec<TransferId>> = HashMap::new();
        for item in self.items.values() {
            if let Some(parent) = item.parent {
                children.entry(parent).or_default().push(item.id);
            }
        }
        children
    }

    /// `id` and its descendants, children before their parent.
    pub(super) fn subtree(
        children: &HashMap<TransferId, Vec<TransferId>>,
        id: TransferId,
    ) -> Vec<TransferId> {
        fn walk(
            id: TransferId,
            children: &HashMap<TransferId, Vec<TransferId>>,
            out: &mut Vec<TransferId>,
        ) {
            for &child in children.get(&id).into_iter().flatten() {
                walk(child, children, out);
            }
            out.push(id);
        }
        let mut out = Vec::new();
        walk(id, children, &mut out);
        out
    }

    /// Pauses or cancels `id` and its non-terminal descendants. Directories
    /// stop counting their children first, so the cascade never finalizes
    /// them; a cancelled directory becomes `Cancelled` directly.
    pub(super) fn stop_subtree(&mut self, id: TransferId, intent: Intent) {
        let ids = Self::subtree(&self.children_index(), id);
        for &i in &ids {
            self.dirs.remove(&i);
        }
        for i in ids {
            self.stop_one(i, intent);
        }
    }

    pub(super) fn stop_one(&mut self, id: TransferId, intent: Intent) {
        let Some(item) = self.items.get(&id) else {
            return;
        };
        if item.state.is_terminal()
            || (intent == Intent::Pause && item.state == TransferState::Paused)
        {
            return;
        }
        if let Some(running) = self.running.get_mut(&id) {
            // A pending cancel is never downgraded to a pause.
            if running.intent != Some(Intent::Cancel) {
                running.intent = Some(intent);
            }
            // A pause lets a prepare finish (a folder being listed then
            // gets its children, paused); a cancel stops it at once.
            if intent != Intent::Pause || running.copying {
                running.cancel.cancel();
            }
            return;
        }
        self.overrides.remove(&id);
        let state = match intent {
            Intent::Cancel => TransferState::Cancelled,
            Intent::Pause | Intent::Requeue => TransferState::Paused,
        };
        self.transition(id, state);
    }

    /// Restarts every item of the subtree of `id` whose state matches
    /// `restartable`, children before their parent: an expanded directory
    /// becomes `Active` again (counting its current children, never listing
    /// it again), anything else becomes `Queued`. An item still stopping with
    /// the `stopping` intent (its worker has not reported back yet) is
    /// requeued when it does.
    pub(super) fn restart_subtree(
        &mut self,
        id: TransferId,
        restartable: fn(&TransferState) -> bool,
        stopping: Intent,
    ) {
        let children = self.children_index();
        for i in Self::subtree(&children, id) {
            if let Some(running) = self.running.get_mut(&i) {
                if running.intent == Some(stopping) {
                    running.intent = Some(Intent::Requeue);
                }
                continue;
            }
            let Some(item) = self.items.get(&i) else {
                continue;
            };
            if !restartable(&item.state) {
                continue;
            }
            // An interrupted run continues under its stored continuation;
            // anything else runs under its tree policy (applied at dispatch),
            // else the global policy, again.
            match self.store.next_policy(i).ok().flatten() {
                Some(policy) => self.overrides.insert(i, policy),
                None => self.overrides.remove(&i),
            };
            if let Some(kids) = children.get(&i) {
                self.reactivate_dir(i, kids);
            } else {
                self.transition(i, TransferState::Queued);
            }
        }
    }

    /// Applies `resolution` to `id`, or with `apply_to_all` to every item
    /// awaiting a decision, which also becomes the global policy.
    pub(super) fn resolve(&mut self, id: TransferId, resolution: Resolution) {
        if !resolution.apply_to_all {
            self.resolve_one(id, resolution.action);
            return;
        }
        self.policy = match resolution.action {
            ResolveAction::Overwrite => ConflictPolicy::Overwrite,
            ResolveAction::Resume => ConflictPolicy::Resume,
            ResolveAction::Skip => ConflictPolicy::Skip,
        };
        let pending: Vec<TransferId> = self
            .items
            .values()
            .filter(|i| matches!(i.state, TransferState::AwaitingDecision(_)))
            .map(|i| i.id)
            .collect();
        for i in pending {
            self.resolve_one(i, resolution.action);
        }
    }

    pub(super) fn resolve_one(&mut self, id: TransferId, action: ResolveAction) {
        let policy = match action {
            ResolveAction::Overwrite => ConflictPolicy::Overwrite,
            ResolveAction::Resume => ConflictPolicy::Resume,
            ResolveAction::Skip => {
                self.overrides.remove(&id);
                self.transition(id, TransferState::Skipped(USER_SKIP.to_string()));
                return;
            }
        };
        self.overrides.insert(id, policy);
        self.transition(id, TransferState::Queued);
    }
}
