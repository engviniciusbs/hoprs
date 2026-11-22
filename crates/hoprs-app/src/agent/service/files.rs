//! The tools on a site's files: reading (`list_directory`, `stat`, `read_file`) and
//! changing (`write_file`, `make_directory`, `rename`, `set_permissions`, `delete`).
//!
//! Every function here runs after the call's access check, on a site the call may read
//! (or, for the changing tools, change), with its arguments' values checked. A failed
//! remote operation answers `<what failed>: <the error's English message>`; nothing here
//! logs (the service writes the call's one line).
//!
//! **Nothing is replaced by default.** `write_file` refuses an existing path unless the
//! call says `overwrite`, and always refuses a folder or a link at the path; `rename`
//! never replaces what is at its target; `delete` removes a folder that is not empty only
//! with `recursive` (without it a folder is only ever `remove_dir`ed, which fails on a
//! folder that is not empty), and the walk removes a link as a link, never what it points
//! to, as far as the server reports it: some FTP servers list a link to a folder as a
//! folder, and the walk then goes into it (see `backend::ops`). `write_file` and
//! `set_permissions` refuse a link at the path, again as the server reports it.

use std::time::SystemTime;

use chrono::{DateTime, SecondsFormat, Utc};
use hoprs_core::{EntryKind, Error, ErrorKind, Protocol, RemoteEntry, RemotePath, Site};
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;

use super::{Done, Failure, Service, count};
use crate::backend::edit::{self, UploadMode};
use crate::backend::ops::{self, DeleteTarget};

impl Service {
    /// The folder's entries sorted by name, the page `offset .. offset + limit` of them.
    pub(super) async fn list_directory(
        &self,
        site: &Site,
        path: &RemotePath,
        offset: u64,
        limit: Option<u64>,
    ) -> Result<Vec<String>, Failure> {
        let remote = self.remote(site).await?;
        let mut entries = remote
            .list(path)
            .await
            .map_err(|e| failed(&format!("Could not list {path}"), &e))?;
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let total = entries.len();
        let limit = limit
            .map_or(self.limits.list_default, |l| {
                usize::try_from(l).unwrap_or(usize::MAX)
            })
            .clamp(1, self.limits.list_max);
        let start = usize::try_from(offset).unwrap_or(usize::MAX).min(total);
        let end = start.saturating_add(limit).min(total);
        let page: Vec<Value> = entries[start..end].iter().map(entry).collect();
        let result = json!({
            "path": path.as_str(),
            "entries": page,
            "offset": offset,
            "total": total,
            "truncated": end < total,
        });
        Ok(vec![result.to_string()])
    }

    /// One path's entry, with its `path`.
    pub(super) async fn stat(
        &self,
        site: &Site,
        path: &RemotePath,
    ) -> Result<Vec<String>, Failure> {
        let remote = self.remote(site).await?;
        let found = remote
            .stat(path)
            .await
            .map_err(|e| failed(&format!("Could not describe {path}"), &e))?;
        let mut result = entry(&found);
        if let Value::Object(map) = &mut result {
            map.insert("path".to_owned(), Value::from(path.as_str()));
        }
        Ok(vec![result.to_string()])
    }

    /// The text of `offset .. offset + length` (ruling 10): `stat` first, then at most
    /// the range is read (a large file is never loaded whole). Bytes of a character cut
    /// by either end of the range are left out, and the header says what was returned.
    ///
    /// A regular file whose size is 0 may not be empty (`/proc` and `/sys` files say 0
    /// over SFTP): it is read up to the cap, and the end is where the bytes stop. A
    /// symbolic link at the end of the path is refused: `RemoteFs` has no way to find
    /// its target, and SFTP's `open_read` refuses anything but a regular file. A link in
    /// the middle of the path is the server's to follow.
    pub(super) async fn read_file(
        &self,
        site: &Site,
        path: &RemotePath,
        offset: u64,
        length: Option<u64>,
    ) -> Result<Vec<String>, Failure> {
        let remote = self.remote(site).await?;
        let found = remote
            .stat(path)
            .await
            .map_err(|e| failed(&format!("Could not read {path}"), &e))?;
        let max = self.limits.file_max as u64;
        // The size, when the `stat` can be trusted with it.
        let known = match found.kind {
            EntryKind::Dir => {
                return Err(Failure::new(
                    format!(
                        "{path} is a folder, not a file. Use list_directory to see what is in it."
                    ),
                    "a folder",
                ));
            }
            EntryKind::Symlink => {
                return Err(Failure::new(
                    format!(
                        "{path} is a symbolic link, and read_file does not follow a link at the end of a path: stat on it, or listing its folder, shows that it is a link. Read the file it points to by that file's own path. A link in the middle of a path is followed."
                    ),
                    "a symbolic link",
                ));
            }
            EntryKind::File if found.size == 0 => None,
            EntryKind::File => Some(found.size),
        };
        // How many bytes to ask for.
        let want = match known {
            Some(size) if offset >= size => 0,
            Some(size) => {
                let available = size - offset;
                let want = length.map_or(available, |l| l.min(available));
                if want > max {
                    return Err(too_long(path, &format!("is {size} bytes"), offset, max));
                }
                want
            }
            // One byte over the cap tells a range that is too long.
            None => length.unwrap_or(u64::MAX).min(max.saturating_add(1)),
        };
        let mut bytes = Vec::new();
        if want > 0 {
            let reader = remote
                .open_read(path, offset)
                .await
                .map_err(|e| failed(&format!("Could not read {path}"), &e))?;
            let mut reader = reader.take(want);
            reader
                .read_to_end(&mut bytes)
                .await
                .map_err(|e| failed(&format!("Could not read {path}"), &Error::from(e)))?;
        }
        if known.is_none() && bytes.len() as u64 > max {
            return Err(too_long(
                path,
                &format!("has more than {max} bytes from offset {offset}"),
                offset,
                max,
            ));
        }
        let got = bytes.len() as u64;
        let end = offset.saturating_add(got);
        // The end of the file: the known size reached, or fewer bytes than asked for.
        let eof = match known {
            Some(size) => end >= size,
            None => got < length.unwrap_or(u64::MAX),
        };
        let (skipped, text) = utf8_range(&bytes, offset > 0, !eof).ok_or_else(|| {
            Failure::new(
                format!(
                    "{path} is not UTF-8 text, and read_file returns text only. Use download to copy it to this computer."
                ),
                "not UTF-8 text",
            )
        })?;
        let start = offset.saturating_add(skipped as u64);
        let returned = text.len() as u64;
        // Without a size from `stat`, the end is known only where bytes stopped: past
        // it (an offset and nothing read) the size is not known.
        let size = known.or((eof && (got > 0 || offset == 0)).then_some(end));
        let header = json!({
            "path": path.as_str(),
            "size": size,
            "offset": start,
            "length": returned,
            "next_offset": start.saturating_add(returned),
            // A cut at the end happens only before the end of the file.
            "eof": eof,
        });
        Ok(vec![header.to_string(), text.to_owned()])
    }
}

impl Service {
    /// Writes `content` as the whole file (ruling 10): `stat` first, so an existing path
    /// without `overwrite`, a folder and a link are refused before anything is written;
    /// then like a remote edit, by a temporary file renamed over it on SFTP and in place
    /// on FTP.
    pub(super) async fn write_file(
        &self,
        site: &Site,
        path: &RemotePath,
        content: &str,
        overwrite: bool,
    ) -> Result<Done, Failure> {
        let remote = self.remote(site).await?;
        let what = format!("Could not write {path}");
        match remote.stat(path).await {
            Ok(found) => match found.kind {
                EntryKind::Dir => {
                    return Err(Failure::new(
                        format!(
                            "{path} is a folder: write_file writes files only, and never replaces a folder. Nothing was written."
                        ),
                        "a folder",
                    ));
                }
                EntryKind::Symlink => {
                    return Err(Failure::new(
                        format!(
                            "{path} is a symbolic link: write_file neither writes through a link nor replaces one. Write the file it points to by that file's own path. Nothing was written."
                        ),
                        "a symbolic link",
                    ));
                }
                EntryKind::File if !overwrite => {
                    return Err(Failure::new(
                        format!(
                            "{path} already exists, and nothing was written. To replace it, call write_file again with overwrite set to true."
                        ),
                        "already exists",
                    ));
                }
                EntryKind::File => {}
            },
            Err(e) if e.kind == ErrorKind::NotFound => {}
            Err(e) => return Err(failed(&what, &e)),
        }
        let mode = match site.protocol {
            Protocol::Ssh => UploadMode::TempRename,
            Protocol::Ftp | Protocol::FtpsExplicit | Protocol::FtpsImplicit => UploadMode::InPlace,
        };
        if let Err(cut) = edit::put(remote.as_ref(), path, mode, content.as_bytes()).await {
            let mut failure = failed(&what, &cut.error);
            if cut.may_be_incomplete {
                failure.text.push_str(&format!(
                    ". It was being written in place, so {path} on the server may now be incomplete: write it again, or check it with read_file."
                ));
            }
            return Err(failure);
        }
        let result = json!({"ok": true, "path": path.as_str(), "bytes": content.len()});
        Ok(vec![result.to_string()].into())
    }

    /// Creates the folder: its parent must exist, and a folder already there is fine
    /// (every backend's `mkdir` answers so).
    pub(super) async fn make_directory(
        &self,
        site: &Site,
        path: &RemotePath,
    ) -> Result<Done, Failure> {
        let remote = self.remote(site).await?;
        if let Err(e) = remote.mkdir(path).await {
            let mut failure = failed(&format!("Could not create the folder {path}"), &e);
            if e.kind == ErrorKind::NotFound {
                failure
                    .text
                    .push_str(". Its parent folder must exist: create the folders above it first.");
            }
            return Err(failure);
        }
        Ok(vec![json!({"ok": true, "path": path.as_str()}).to_string()].into())
    }

    /// Renames or moves, never over something that is at `to`.
    pub(super) async fn rename(
        &self,
        site: &Site,
        from: &RemotePath,
        to: &RemotePath,
    ) -> Result<Done, Failure> {
        let remote = self.remote(site).await?;
        match ops::rename_no_replace(remote.as_ref(), from, to).await {
            Ok(()) => {
                let result = json!({"ok": true, "from": from.as_str(), "to": to.as_str()});
                Ok(vec![result.to_string()].into())
            }
            Err(e) if e.kind == ErrorKind::Conflict => Err(Failure::new(
                format!(
                    "Could not rename {from} to {to}: something is already at {to}, and rename never replaces it. Nothing changed. Delete it first, or choose another name."
                ),
                "the target already exists",
            )),
            Err(e) => Err(failed(&format!("Could not rename {from} to {to}"), &e)),
        }
    }

    /// Sets the permission bits. Over FTP it is `SITE CHMOD`, which many servers lack.
    pub(super) async fn set_permissions(
        &self,
        site: &Site,
        path: &RemotePath,
        mode: u32,
    ) -> Result<Done, Failure> {
        let remote = self.remote(site).await?;
        let what = format!("Could not change the permissions of {path}");
        // Over SFTP a chmod follows a link and changes its target: refused, as the server
        // reports the path.
        let found = remote.stat(path).await.map_err(|e| failed(&what, &e))?;
        if found.kind == EntryKind::Symlink {
            return Err(Failure::new(
                format!(
                    "{path} is a symbolic link: set_permissions does not change permissions through a link. Change the file or folder it points to by its own path. Nothing changed."
                ),
                "a symbolic link",
            ));
        }
        match remote.chmod(path, mode).await {
            Ok(()) => {
                let result =
                    json!({"ok": true, "path": path.as_str(), "mode": format!("{mode:04o}")});
                Ok(vec![result.to_string()].into())
            }
            // The FTP backend reports an answer to SITE CHMOD it does not expect (500, 502,
            // 504: a server without it, or one that rejects this mode) as an unexpected
            // reply.
            Err(e) if site.protocol != Protocol::Ssh && e.kind == ErrorKind::Protocol => {
                Err(Failure::new(
                    format!(
                        "{what}: the FTP server of \"{}\" did not accept the change of permissions to {mode:04o} (SITE CHMOD). Some FTP servers do not support it at all.",
                        site.name
                    ),
                    "the server did not accept SITE CHMOD",
                ))
            }
            Err(e) => Err(failed(&what, &e)),
        }
    }

    /// Deletes a file or a link, an empty folder, or with `recursive` a folder and all in
    /// it; answers how many items went. A failure half way says how many went before it.
    pub(super) async fn delete(
        &self,
        site: &Site,
        path: &RemotePath,
        recursive: bool,
    ) -> Result<Done, Failure> {
        let remote = self.remote(site).await?;
        let what = format!("Could not delete {path}");
        let found = remote.stat(path).await.map_err(|e| failed(&what, &e))?;
        if found.kind == EntryKind::Dir && !recursive {
            let inside = remote.list(path).await.map_err(|e| failed(&what, &e))?;
            let inside = inside
                .iter()
                .filter(|e| !matches!(e.name.as_str(), "" | "." | ".."))
                .count();
            if inside > 0 {
                return Err(Failure::new(
                    format!(
                        "{path} is a folder that is not empty ({}). Nothing was deleted. To delete it with everything in it, call delete again with recursive set to true.",
                        count(inside as u64, "item")
                    ),
                    "a folder that is not empty",
                ));
            }
        }
        if found.kind == EntryKind::Dir && !recursive {
            // Never the walk: a folder filled since the listing makes this fail, and
            // nothing in it is removed.
            remote.remove_dir(path).await.map_err(|e| {
                let mut failure = failed(&what, &e);
                failure.text.push_str(". Nothing was deleted.");
                failure
            })?;
            let result = json!({"ok": true, "path": path.as_str(), "deleted": 1});
            return Ok(Done {
                blocks: vec![result.to_string()],
                note: Some(count(1, "item")),
            });
        }
        let target = DeleteTarget {
            path: path.clone(),
            kind: found.kind,
        };
        let (progress, mut counts) = mpsc::unbounded_channel();
        let result = ops::delete_all(remote.as_ref(), vec![target], &progress).await;
        drop(progress);
        match result {
            Ok(deleted) => {
                let result = json!({"ok": true, "path": path.as_str(), "deleted": deleted});
                Ok(Done {
                    blocks: vec![result.to_string()],
                    note: Some(count(deleted, "item")),
                })
            }
            Err(e) => {
                // The last count sent is how far the walk got.
                let mut deleted = 0;
                while let Ok(n) = counts.try_recv() {
                    deleted = n;
                }
                let mut failure = failed(&what, &e);
                failure.text.push_str(&if deleted == 0 {
                    ". Nothing was deleted.".to_owned()
                } else {
                    format!(
                        ". {} deleted before the failure; the rest of {path} is still there.",
                        if deleted == 1 {
                            "1 item was".to_owned()
                        } else {
                            format!("{deleted} items were")
                        }
                    )
                });
                failure.reason = format!("{}; {} deleted", failure.reason, count(deleted, "item"));
                Err(failure)
            }
        }
    }
}

/// `<what failed>: <the error's English message>`; the log keeps the message.
fn failed(what: &str, error: &Error) -> Failure {
    Failure::new(format!("{what}: {}", error.message), error.message.clone())
}

/// A range over the cap: names the size and the ways out.
fn too_long(path: &RemotePath, size: &str, offset: u64, max: u64) -> Failure {
    Failure::new(
        format!(
            "{path} {size}: more than read_file returns in one call ({max} bytes) from offset {offset}. Read it in pieces with offset and length, or use download."
        ),
        format!("range over {max} bytes"),
    )
}

/// The text in `bytes`, without the bytes of a character cut at the start (when
/// `cut_start`: the range does not begin the file) or at the end (when `cut_end`: the
/// range does not end it): how many leading bytes were left out, and the text. `None`
/// when what is left is not UTF-8.
fn utf8_range(bytes: &[u8], cut_start: bool, cut_end: bool) -> Option<(usize, &str)> {
    let skipped = if cut_start {
        // A character is at most four bytes: at most three continuation bytes lead.
        bytes
            .iter()
            .take(3)
            .take_while(|b| (**b & 0b1100_0000) == 0b1000_0000)
            .count()
    } else {
        0
    };
    let rest = &bytes[skipped..];
    match std::str::from_utf8(rest) {
        Ok(text) => Some((skipped, text)),
        // A character the range's end cut in two: what comes before it.
        Err(e) if cut_end && e.error_len().is_none() => {
            std::str::from_utf8(&rest[..e.valid_up_to()])
                .ok()
                .map(|text| (skipped, text))
        }
        Err(_) => None,
    }
}

/// An entry as the tools answer it (ruling 7).
fn entry(e: &RemoteEntry) -> Value {
    let kind = match e.kind {
        EntryKind::File => "file",
        EntryKind::Dir => "dir",
        EntryKind::Symlink => "symlink",
    };
    json!({
        "name": e.name,
        "kind": kind,
        "size": e.size,
        "modified": e.modified.map(rfc3339),
        "permissions": e.mode.map(|m| format!("{:04o}", m & 0o7777)),
    })
}

/// RFC 3339 in UTC, to the second.
fn rfc3339(time: SystemTime) -> String {
    DateTime::<Utc>::from(time).to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_range_cuts_only_at_the_ends_it_may() {
        let e = "aé".as_bytes(); // 61 C3 A9
        assert_eq!(utf8_range(e, false, false), Some((0, "aé")));
        assert_eq!(utf8_range(&e[..2], false, true), Some((0, "a")));
        // At the end of the file a half character is not text.
        assert_eq!(utf8_range(&e[..2], false, false), None);
        assert_eq!(utf8_range(&e[2..], true, false), Some((1, "")));
        // At the start of the file a continuation byte is not text.
        assert_eq!(utf8_range(&e[2..], false, false), None);
        assert_eq!(utf8_range(&[0xff, 0xfe], false, true), None);
        let four = "𝄞".as_bytes(); // F0 9D 84 9E
        assert_eq!(utf8_range(&four[1..], true, false), Some((3, "")));
        assert_eq!(utf8_range(&four[..3], false, true), Some((0, "")));
    }

    #[test]
    fn entries_use_rfc3339_and_four_octal_digits() {
        let e = RemoteEntry {
            name: "x".into(),
            path: RemotePath::new("/x").unwrap(),
            kind: EntryKind::Dir,
            size: 0,
            modified: None,
            mode: Some(0o40755),
        };
        assert_eq!(
            entry(&e),
            json!({"name": "x", "kind": "dir", "size": 0, "modified": null,
                "permissions": "0755"})
        );
        assert_eq!(rfc3339(SystemTime::UNIX_EPOCH), "1970-01-01T00:00:00Z");
    }
}
