//! Directory bookkeeping. Directories expand lazily: a worker lists a
//! directory when it is dispatched, and its entries join the queue as child
//! items. A directory stays `Active` (without holding a worker slot) until
//! all of its children are terminal, counting them in a `DirProgress`.

use std::collections::HashSet;

use hoprs_core::{EntryKind, Error, ErrorKind, RemoteEntry};

use super::actor::{Actor, Intent, SYMLINK_SKIP};
use crate::model::{TransferId, TransferItem, TransferSpec, TransferState};
use crate::store::NewChild;

/// The error of a child whose name repeats an earlier entry of the same
/// listing. Such a child is terminal for good: retrying it (directly or
/// with its folder) would make two items write the same path. It is
/// recognised by this message, which the queue persists with the error, so
/// the rule survives a restart.
pub(super) const DUPLICATE_NAME: &str = "Duplicate name in directory listing";

/// Whether `state` is the failure of a duplicate-name child (see
/// `DUPLICATE_NAME`).
pub(super) fn is_duplicate_name(state: &TransferState) -> bool {
    matches!(
        state,
        TransferState::Failed(e) if e.kind == ErrorKind::Io && e.message == DUPLICATE_NAME
    )
}

/// Children of an expanded directory that are not terminal yet, and how
/// many of its finished children ended `Failed`.
pub(super) struct DirProgress {
    pub(super) pending: usize,
    pub(super) failed: usize,
}

impl Actor {
    /// Makes the expanded directory `id` `Active` again, counting its
    /// current `children`. Finishes it right away if none is pending.
    pub(super) fn reactivate_dir(&mut self, id: TransferId, children: &[TransferId]) {
        let (mut pending, mut failed) = (0, 0);
        for child in children.iter().filter_map(|c| self.items.get(c)) {
            match child.state {
                TransferState::Failed(_) => failed += 1,
                ref s if !s.is_terminal() => pending += 1,
                _ => {}
            }
        }
        // Through `Active`, so that leaving and re-entering a terminal state
        // is reported to the parent directory.
        if !self.transition(id, TransferState::Active) {
            return;
        }
        if pending == 0 {
            self.finish_dir(id, failed);
        } else {
            self.dirs.insert(id, DirProgress { pending, failed });
        }
    }

    /// Accounts for a child of `dir` that left a terminal state (retried):
    /// a counting `dir` waits for it again; a finished `dir` is reopened,
    /// which recurses to its own parent. A paused `dir` recounts its
    /// children when it is resumed.
    pub(super) fn child_reopened(&mut self, dir: TransferId, was_failed: bool) {
        if let Some(progress) = self.dirs.get_mut(&dir) {
            progress.pending += 1;
            if was_failed {
                progress.failed = progress.failed.saturating_sub(1);
            }
            return;
        }
        if self.items.get(&dir).is_some_and(|d| d.state.is_terminal()) {
            let children: Vec<TransferId> = self
                .items
                .values()
                .filter(|i| i.parent == Some(dir))
                .map(|i| i.id)
                .collect();
            self.reactivate_dir(dir, &children);
        }
    }

    /// Accounts for a child of `dir` that reached a terminal state. When it
    /// was the last one, `dir` finishes too (which recurses to its parent).
    /// A directory that is not counting (paused or finished) is left alone.
    pub(super) fn child_finished(&mut self, dir: TransferId, failed: bool) {
        let Some(progress) = self.dirs.get_mut(&dir) else {
            return;
        };
        progress.pending = progress.pending.saturating_sub(1);
        if failed {
            progress.failed += 1;
        }
        if progress.pending > 0 {
            return;
        }
        let failed = progress.failed;
        self.dirs.remove(&dir);
        self.finish_dir(dir, failed);
    }

    /// Finishes a directory: `Done`, or `Failed` if `failed` children failed.
    /// Children that were skipped or cancelled do not count as failures.
    pub(super) fn finish_dir(&mut self, dir: TransferId, failed: usize) {
        let state = if failed == 0 {
            TransferState::Done
        } else {
            TransferState::Failed(
                Error::coded(
                    ErrorKind::Io,
                    "transfer-items-failed",
                    format!("{failed} items failed"),
                )
                .arg("failed", failed.to_string()),
            )
        };
        self.transition(dir, state);
    }

    /// Inserts one child per entry of the listed directory `id`. Only the
    /// first entry of a name is copied: a later one with the same name
    /// becomes a `Failed` child (`DUPLICATE_NAME`, never retried), so that
    /// two items never write the same path. The directory stays `Active`
    /// until its children finish, or finishes right away if none is pending. If it was paused or cancelled
    /// while listing, its children are created (and it ends) paused or
    /// cancelled. The children inherit the directory's tree policy and
    /// origin.
    pub(super) fn expand(
        &mut self,
        id: TransferId,
        entries: Vec<RemoteEntry>,
        created: bool,
        intent: Option<Intent>,
    ) {
        let initial = match intent {
            Some(Intent::Pause) => TransferState::Paused,
            Some(Intent::Cancel) => TransferState::Cancelled,
            Some(Intent::Requeue) | None => TransferState::Queued,
        };
        if let Err(e) = self.store.set_meta(id, true, None) {
            self.transition(id, TransferState::Failed(e));
            return;
        }
        let Some(dir) = self.items.get_mut(&id) else {
            return;
        };
        dir.is_dir = Some(true);
        dir.size = None;
        let parent = dir.spec.clone();
        // Every child is of its folder's tree: same policy, same origin.
        let (tree_policy, origin) = (dir.tree_policy, dir.origin);
        let mut seen = HashSet::new();
        let children: Vec<NewChild> = entries
            .iter()
            .map(|e| NewChild {
                spec: TransferSpec {
                    site: parent.site.clone(),
                    direction: parent.direction,
                    local: parent.local.join(&e.name),
                    remote: parent.remote.join(&e.name),
                },
                is_dir: e.kind == EntryKind::Dir,
                size: (e.kind == EntryKind::File).then_some(e.size),
                state: if !seen.insert(e.name.as_str()) {
                    TransferState::Failed(
                        Error::coded(
                            ErrorKind::Io,
                            "transfer-duplicate-name",
                            "Duplicate name in directory listing",
                        )
                        .with_detail(format!("{:?} in {}", e.name, parent.local)),
                    )
                } else if e.kind == EntryKind::Symlink {
                    TransferState::Skipped(SYMLINK_SKIP.to_string())
                } else {
                    initial.clone()
                },
                tree_policy,
                origin,
            })
            .collect();
        let ids = match self.store.insert_children(id, &children) {
            Ok(ids) => ids,
            Err(e) => {
                self.transition(id, TransferState::Failed(e));
                return;
            }
        };
        let pending = children.iter().filter(|c| !c.state.is_terminal()).count();
        let failed = children
            .iter()
            .filter(|c| matches!(c.state, TransferState::Failed(_)))
            .count();
        if pending > 0 && initial == TransferState::Queued {
            self.dirs.insert(id, DirProgress { pending, failed });
        }
        // The folder is fresh only while no other tree writes there. When one does, the
        // fresh folders of others that overlap this one lose their marks too (any that
        // overlap a child overlap this folder).
        let root = self.root_of(id);
        let parent_dst = super::actor::destination(&parent).clone();
        let shared = self.destination_shared(root, &parent_dst);
        if shared {
            self.drop_overlapping_fresh(root, &parent_dst);
        }
        let fresh = created && !shared;
        let mut fresh_files = Vec::new();
        for (child_id, child) in ids.into_iter().zip(children) {
            if fresh && !child.is_dir && child.state == TransferState::Queued {
                self.fresh_dst.insert(child_id);
                fresh_files.push(child_id);
            }
            self.add_item(TransferItem {
                id: child_id,
                parent: Some(id),
                spec: child.spec,
                is_dir: Some(child.is_dir),
                size: child.size,
                bytes_done: 0,
                state: child.state,
                origin: child.origin,
                tree_policy: child.tree_policy,
            });
        }
        if !fresh_files.is_empty() {
            self.fresh_dirs.insert(
                id,
                super::actor::FreshDir {
                    root,
                    dst: parent_dst,
                    files: fresh_files,
                },
            );
        }
        if initial == TransferState::Cancelled {
            self.transition(id, TransferState::Cancelled);
        } else if pending == 0 {
            self.finish_dir(id, failed);
        } else if initial == TransferState::Paused {
            self.transition(id, TransferState::Paused);
        }
    }
}
