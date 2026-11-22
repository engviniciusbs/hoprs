//! The transfer tools: `download` and `upload`, which go through the transfer queue the
//! user sees, and `transfer_status`.
//!
//! **The local side is checked before anything is enqueued.** The local path is
//! `engine_path` of the argument (checked with the call's other arguments), then judged
//! against the protected paths on the blocking pool: as a file, and for a folder transfer
//! as a folder (`tree`), so a folder that holds a protected path is refused as a whole.
//! A refusal names only the path the agent gave (never a key file's own path), as
//! protected, and nothing about which protected place it is or whose key. A path that does not exist yet is judged through its
//! nearest existing ancestor, a link by where it leads. What appears during the walk of a
//! folder is the engine's guard's: it checks every agent item before touching it, and
//! the engine never follows a link it finds while listing a folder.
//!
//! **The order**, each refusal before any side effect: for a `download`, the local path
//! as a file, then the connection, the server's `stat` of the remote path (a link is
//! refused: the engine would skip it), and for a folder the local path as a folder; for
//! an `upload`, the local source (`symlink_metadata`), the protected check, then the
//! source's kind (it must exist and be a regular file or a folder, not a link: a device,
//! a pipe or a socket would read without end or block; on Windows a device name such as
//! `CON` is refused without asking the system about it), then the connection (an item for
//! a site that is not connected would stay queued). Only then the enqueue. A missing
//! source is reported after the protected check, so the answer never tells whether a
//! protected file exists.
//!
//! **Never a question.** Every agent transfer is enqueued with `Skip` (an existing file is
//! skipped and reported) or, with `overwrite`, `Overwrite`; never the user's default,
//! which may be `Ask`. The engine refuses anything else for an agent, and every item of a
//! folder inherits the policy.
//!
//! **The wait.** The call waits on the engine's events (subscribed before the enqueue)
//! until the root ends or waits on the user, at most `limits.transfer_wait`, then answers
//! [`summary`]. The transfer belongs to the queue: when the call is cancelled or times out
//! it keeps running, and `transfer_status` answers the same summary later.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use hoprs_core::{AgentAccess, EntryKind, Error, ErrorKind, RemotePath, Site};
use hoprs_transfer::{
    ConflictPolicy, Direction, Event, EventKind, Origin, TransferId, TransferItem, TransferSpec,
    TransferState,
};
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;

use super::files::failed;
use super::{Done, Failure, Refusal, Reply, SITES_UNREADABLE, Service, ToolSpec, refused};
use crate::agent::paths::{Protected, local_os_path};
use crate::agent::{NO_SUCH_TRANSFER, protected};

/// A `download` or `upload` with its arguments checked.
pub(super) struct TransferJob {
    pub(super) direction: Direction,
    pub(super) remote: RemotePath,
    /// `engine_path` of the argument.
    pub(super) local: RemotePath,
    /// The local path as the agent wrote it: what a refusal and the log name.
    pub(super) given: String,
    pub(super) overwrite: bool,
}

impl Service {
    /// Checks the local side, connects, enqueues and waits (see the module docs).
    pub(super) async fn transfer(
        &self,
        spec: &ToolSpec,
        site: &Site,
        job: &TransferJob,
    ) -> Result<Done, Failure> {
        let os = local_os_path(&job.local);
        match job.direction {
            Direction::Download => {
                check(self.host.protected().await, &os, false, &job.given).await?;
                let remote = self.remote(spec, site).await?;
                let found = remote
                    .stat(&job.remote)
                    .await
                    .map_err(|e| failed(&format!("Could not read {}", job.remote), &e))?;
                match found.kind {
                    EntryKind::Symlink => {
                        return Err(Failure::new(
                            format!(
                                "{} is a symbolic link, and the transfer queue does not follow a link: download the file or folder it points to by its own path. Nothing was downloaded.",
                                job.remote
                            ),
                            "a symbolic link",
                        ));
                    }
                    EntryKind::Dir => {
                        check(self.host.protected().await, &os, true, &job.given).await?;
                    }
                    EntryKind::File => {}
                }
            }
            Direction::Upload => {
                let given = &job.given;
                let odd = || {
                    Failure::new(
                        format!(
                            "{given} is not a regular file or a folder (a device, a pipe or a socket): upload cannot read it. Nothing was uploaded."
                        ),
                        "not a regular file or a folder",
                    )
                };
                // A device name is refused without asking the system about it.
                #[cfg(windows)]
                if windows_device(&os) {
                    check(self.host.protected().await, &os, false, given).await?;
                    return Err(odd());
                }
                let meta = tokio::fs::symlink_metadata(&os).await;
                let link = meta.as_ref().is_ok_and(|m| m.file_type().is_symlink());
                // A link is judged by where it leads; a folder as a folder.
                let tree = if link {
                    tokio::fs::metadata(&os).await.is_ok_and(|m| m.is_dir())
                } else {
                    meta.as_ref().is_ok_and(|m| m.is_dir())
                };
                check(self.host.protected().await, &os, tree, given).await?;
                let meta = match meta {
                    Ok(meta) => meta,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        return Err(Failure::new(
                            format!(
                                "{given} does not exist on this computer. Nothing was uploaded."
                            ),
                            "no such local path",
                        ));
                    }
                    Err(e) => {
                        return Err(Failure::new(
                            format!(
                                "Could not read {given} on this computer: {e}. Nothing was uploaded."
                            ),
                            format!("the local path could not be read: {e}"),
                        ));
                    }
                };
                if link {
                    return Err(Failure::new(
                        format!(
                            "{given} is a symbolic link, and upload does not follow a link: upload the file or folder it points to by its own path. Nothing was uploaded."
                        ),
                        "a symbolic link",
                    ));
                }
                if !(meta.is_file() || meta.is_dir()) {
                    return Err(odd());
                }
                // An item for a site that is not connected would stay queued.
                self.remote(spec, site).await?;
            }
        }
        let spec = TransferSpec {
            site: site.id.clone(),
            direction: job.direction,
            local: job.local.clone(),
            remote: job.remote.clone(),
        };
        let policy = if job.overwrite {
            ConflictPolicy::Overwrite
        } else {
            ConflictPolicy::Skip
        };
        // Subscribed before the enqueue: no event of the transfer is missed.
        let mut events = self.host.transfer_events();
        let id = self
            .host
            .enqueue(spec, policy)
            .await
            .map_err(|e| failed("Could not queue the transfer", &e))?;
        let mut seen = None;
        let wait = async {
            loop {
                match events.recv().await {
                    Ok(Event {
                        kind: EventKind::StateChanged { id: changed, state },
                        ..
                    }) if changed == id => {
                        if settled(&state) {
                            seen = Some(state);
                            return;
                        }
                    }
                    Ok(_) => {}
                    // Events were lost: the queue says where the root is.
                    Err(RecvError::Lagged(_)) => {
                        let now = self.host.transfers().await;
                        if let Some(root) = now.into_iter().find(|item| item.id == id)
                            && settled(&root.state)
                        {
                            seen = Some(root.state);
                            return;
                        }
                    }
                    Err(RecvError::Closed) => return,
                }
            }
        };
        // Past the wait the transfer keeps running: the answer says so.
        let _ = tokio::time::timeout(self.limits.transfer_wait, wait).await;
        let items = self.host.transfers().await;
        let mut value = summary(id, &items, self.limits.failed_listed);
        // Cleared from the queue right after it ended: its end is the one seen.
        if !items.iter().any(|item| item.id == id)
            && let (Some(state), Value::Object(map)) = (&seen, &mut value)
        {
            map.insert("state".to_owned(), Value::from(state_name(state)));
            map.remove("note");
        }
        let state = value["state"].as_str().unwrap_or("running").to_owned();
        Ok(Done {
            blocks: vec![value.to_string()],
            note: Some(format!("transfer {}: {state}", id.0)),
        })
    }

    /// `transfer_status`: the summary of an agent's transfer on a site the caller may
    /// still read; anything else is one refusal.
    pub(super) async fn transfer_status(&self, transfer: u64) -> (Reply, String) {
        let none = || {
            refused(
                "transfer_status",
                Refusal {
                    text: NO_SUCH_TRANSFER.to_owned(),
                    reason: "no such agent transfer",
                },
            )
        };
        let Ok(id) = i64::try_from(transfer).map(TransferId) else {
            return none();
        };
        let items = self.host.transfers().await;
        let Some(root) = items
            .iter()
            .find(|item| item.id == id && item.parent.is_none() && item.origin == Origin::Agent)
        else {
            return none();
        };
        let Ok(sites) = self.host.sites().await else {
            return refused(
                "transfer_status",
                Refusal {
                    text: SITES_UNREADABLE.to_owned(),
                    reason: "the saved sites could not be read",
                },
            );
        };
        let Some(site) = sites
            .iter()
            .find(|s| s.id == root.spec.site && s.agent_access != AgentAccess::None)
        else {
            return none();
        };
        let value = summary(id, &items, self.limits.failed_listed);
        let line = format!(
            "checked transfer {} on \"{}\" ({})",
            id.0,
            site.name,
            value["state"].as_str().unwrap_or("running")
        );
        (Reply::Ok(vec![value.to_string()]), line)
    }
}

/// `path` checked against `protected` on the blocking pool (`tree` for a folder
/// transfer); a hit refuses the call naming only `given`.
async fn check(roots: Protected, path: &Path, tree: bool, given: &str) -> Result<(), Failure> {
    let path: PathBuf = path.to_path_buf();
    let checked = tokio::task::spawn_blocking(move || roots.check(&path, tree)).await;
    match checked {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => Err(Failure::new(protected(given), "protected local path")),
        // The check did not run: nothing goes through unchecked.
        Err(_) => Err(Failure::new(
            "hoprs could not check the local path. Nothing was transferred; call the tool again.",
            "the local path could not be checked",
        )),
    }
}

/// Whether the wait ends at `state`: the root ended, or it waits on the user.
fn settled(state: &TransferState) -> bool {
    state.is_terminal()
        || matches!(
            state,
            TransferState::Paused | TransferState::AwaitingDecision(_)
        )
}

/// The name a summary gives `state`.
fn state_name(state: &TransferState) -> &'static str {
    match state {
        TransferState::Queued | TransferState::Active => "running",
        TransferState::Paused | TransferState::AwaitingDecision(_) => "paused",
        TransferState::Done => "done",
        TransferState::Skipped(_) => "skipped",
        TransferState::Failed(_) => "failed",
        TransferState::Cancelled => "cancelled",
    }
}

/// Ruling 9's answer for the transfer `root`, over it and its descendants (by `parent`):
/// `files_done` and `bytes_done` count the files done, `skipped` the items skipped
/// (existing files without `overwrite`, links found in a folder), `failed_count` the
/// items that failed and `failed` the first `failed_listed` of them, in queue order, as
/// `{path, reason}`: the path on the side read from (the server's for a download, this
/// computer's for an upload) and the error's English message. A folder's own "N items
/// failed" is not one more failure. The state is the root's (`"running"` while it is
/// queued or active, `"paused"` while it waits on the user); a root not in `items` reads
/// `"running"`. `"running"` and `"paused"` carry a `note` saying what to do next.
pub fn summary(root: TransferId, items: &[TransferItem], failed_listed: usize) -> Value {
    let mut children: HashMap<TransferId, Vec<&TransferItem>> = HashMap::new();
    for item in items {
        if let Some(parent) = item.parent {
            children.entry(parent).or_default().push(item);
        }
    }
    let top = items.iter().find(|item| item.id == root);
    let mut tree: Vec<&TransferItem> = Vec::new();
    let mut stack: Vec<&TransferItem> = top.into_iter().collect();
    while let Some(item) = stack.pop() {
        tree.push(item);
        // A queue with a cycle is not one the engine writes; never loop on one.
        if tree.len() > items.len() {
            break;
        }
        stack.extend(children.get(&item.id).into_iter().flatten().copied());
    }
    tree.sort_by_key(|item| item.id);
    let (mut files_done, mut bytes_done, mut skipped, mut failed_count) = (0u64, 0u64, 0u64, 0u64);
    let mut failed = Vec::new();
    for item in &tree {
        let folder = item.is_dir == Some(true);
        match &item.state {
            TransferState::Done if !folder => {
                files_done += 1;
                bytes_done += item.bytes_done;
            }
            TransferState::Skipped(_) => skipped += 1,
            TransferState::Failed(error)
                if !(folder && error.code == Some("transfer-items-failed")) =>
            {
                failed_count += 1;
                if failed.len() < failed_listed {
                    failed.push(json!({"path": source_path(item), "reason": reason(error)}));
                }
            }
            _ => {}
        }
    }
    let state = top.map_or("running", |item| state_name(&item.state));
    let mut value = json!({
        "transfer": root.0,
        "state": state,
        "files_done": files_done,
        "bytes_done": bytes_done,
        "skipped": skipped,
        "failed_count": failed_count,
        "failed": failed,
    });
    let note = match state {
        "running" => Some(format!(
            "The transfer is still running in the hoprs queue. Call transfer_status with transfer {} later to follow it.",
            root.0
        )),
        "paused" => Some(format!(
            "The transfer is paused in the hoprs queue: it waits on the user (the site's queue was paused, or its connection was lost). Ask the user to look at the queue in the hoprs window, then call transfer_status with transfer {}.",
            root.0
        )),
        _ => None,
    };
    if let (Some(note), Value::Object(map)) = (note, &mut value) {
        map.insert("note".to_owned(), Value::from(note));
    }
    value
}

/// The item's path on the side it is read from.
fn source_path(item: &TransferItem) -> String {
    match item.spec.direction {
        Direction::Download => item.spec.remote.as_str().to_owned(),
        Direction::Upload => local_os_path(&item.spec.local).display().to_string(),
    }
}

/// An item's failure as the agent reads it: the error's English message, except for a
/// connection's own failures, whose text can name a key file (ruling 12).
fn reason(error: &Error) -> String {
    match error.kind {
        ErrorKind::Auth => "hoprs could not log in to the site".to_owned(),
        ErrorKind::HostKey | ErrorKind::Tls => {
            "the connection was refused in the hoprs window".to_owned()
        }
        _ => error.message.clone(),
    }
}

/// A name Windows opens as a device whatever the folder (`CON`, `NUL.txt`, `COM1`): it
/// reads without end or blocks.
#[cfg(windows)]
fn windows_device(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix).is_some_and(|rest| {
            let mut chars = rest.chars();
            matches!(
                (chars.next(), chars.next()),
                (Some('0'..='9' | '¹' | '²' | '³'), None)
            )
        })
    };
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || numbered("COM")
        || numbered("LPT")
}

#[cfg(test)]
mod tests {
    use hoprs_core::SiteId;

    use super::*;

    fn item(id: i64, parent: Option<i64>, is_dir: bool, state: TransferState) -> TransferItem {
        TransferItem {
            id: TransferId(id),
            parent: parent.map(TransferId),
            spec: TransferSpec {
                site: SiteId("s".to_owned()),
                direction: Direction::Download,
                local: RemotePath::new(&format!("/home/u/out/{id}")).unwrap(),
                remote: RemotePath::new(&format!("/srv/{id}")).unwrap(),
            },
            is_dir: Some(is_dir),
            size: None,
            bytes_done: if is_dir { 0 } else { 10 },
            state,
            origin: Origin::Agent,
            tree_policy: Some(ConflictPolicy::Skip),
        }
    }

    fn denied() -> TransferState {
        TransferState::Failed(Error::new(ErrorKind::Permission, "Permission denied"))
    }

    /// A tree of 3 done files, 1 skipped, 25 failed, spread over a subfolder; another
    /// root's items do not count; at most 20 failures are named.
    #[test]
    fn summary_counts() {
        let folder_failed = TransferState::Failed(
            Error::coded(ErrorKind::Io, "transfer-items-failed", "25 items failed")
                .arg("failed", "25"),
        );
        let mut items = vec![
            item(1, None, true, folder_failed.clone()),
            item(2, Some(1), true, folder_failed),
            item(3, Some(1), false, TransferState::Done),
            item(4, Some(2), false, TransferState::Done),
            item(5, Some(2), false, TransferState::Done),
            item(
                6,
                Some(1),
                false,
                TransferState::Skipped("exists".to_owned()),
            ),
            // Another tree.
            item(100, None, false, TransferState::Done),
            item(101, Some(100), false, denied()),
        ];
        for n in 0..25 {
            items.push(item(
                10 + n,
                Some(if n % 2 == 0 { 1 } else { 2 }),
                false,
                denied(),
            ));
        }
        let value = summary(TransferId(1), &items, 20);
        assert_eq!(value["transfer"], 1);
        assert_eq!(value["state"], "failed");
        assert_eq!(value["files_done"], 3);
        assert_eq!(value["bytes_done"], 30);
        assert_eq!(value["skipped"], 1);
        assert_eq!(value["failed_count"], 25);
        let failed = value["failed"].as_array().unwrap();
        assert_eq!(failed.len(), 20);
        for entry in failed {
            assert!(
                entry["path"].as_str().unwrap().starts_with("/srv/"),
                "{entry}"
            );
            assert_eq!(entry["reason"], "Permission denied");
        }
        assert_eq!(failed[0]["path"], "/srv/10", "in queue order");
        assert!(value.get("note").is_none());
    }

    #[test]
    fn summary_states_and_notes() {
        for (state, name, note) in [
            (TransferState::Queued, "running", true),
            (TransferState::Active, "running", true),
            (TransferState::Paused, "paused", true),
            (TransferState::Done, "done", false),
            (
                TransferState::Skipped("exists".to_owned()),
                "skipped",
                false,
            ),
            (denied(), "failed", false),
            (TransferState::Cancelled, "cancelled", false),
        ] {
            let value = summary(TransferId(7), &[item(7, None, false, state)], 20);
            assert_eq!(value["state"], name);
            assert_eq!(value.get("note").is_some(), note, "{name}");
            if note {
                assert!(value["note"].as_str().unwrap().contains("transfer 7"));
            }
        }
        assert_eq!(summary(TransferId(9), &[], 20)["state"], "running");
    }

    /// A connection's failure never reaches the agent with its own text (it can name a
    /// key file); an upload's failure names the local path.
    #[test]
    fn failure_reasons_carry_no_key_path() {
        let auth = Error::new(ErrorKind::Auth, "No key accepted: /home/u/.ssh/id_deploy")
            .with_detail("tried /home/u/.ssh/id_deploy");
        let mut up = item(1, None, false, TransferState::Failed(auth));
        up.spec.direction = Direction::Upload;
        let value = summary(TransferId(1), &[up], 20);
        let text = value.to_string();
        assert!(!text.contains("id_deploy"), "{text}");
        assert_eq!(
            value["failed"][0]["reason"],
            "hoprs could not log in to the site"
        );
        let path = value["failed"][0]["path"].as_str().unwrap().to_owned();
        assert!(
            Path::new(&path).ends_with(Path::new("home").join("u").join("out").join("1")),
            "{path}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_device_names() {
        for name in [
            "CON", "con", "NUL.txt", "COM1", "lpt9.log", "AUX .x", "COM¹",
        ] {
            assert!(windows_device(&Path::new(r"C:\x").join(name)), "{name}");
        }
        for name in ["CONFIG", "COM10", "a.txt", "NULL"] {
            assert!(!windows_device(&Path::new(r"C:\x").join(name)), "{name}");
        }
    }
}
