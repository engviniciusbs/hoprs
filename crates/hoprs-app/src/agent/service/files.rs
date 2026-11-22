//! The reading tools on a site's files: `list_directory`, `stat` and `read_file`.
//!
//! Every function here runs after the call's access check, on a site the call may read.
//! A failed remote operation answers `<what failed>: <the error's English message>`;
//! nothing here logs (the service writes the call's one line).

use std::time::SystemTime;

use chrono::{DateTime, SecondsFormat, Utc};
use hoprs_core::{EntryKind, Error, RemoteEntry, RemotePath, Site};
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

use super::{Failure, Service};

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
        // A link's own size says nothing of its target's: read up to the cap and see.
        let known = match found.kind {
            EntryKind::Dir => {
                return Err(Failure::new(
                    format!(
                        "{path} is a folder, not a file. Use list_directory to see what is in it."
                    ),
                    "a folder",
                ));
            }
            EntryKind::File => Some(found.size),
            EntryKind::Symlink => None,
        };
        // How many bytes to ask for, and whether that is the end of the file.
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
            None => length.unwrap_or(u64::MAX).min(max + 1),
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
        let end = offset + got;
        // The end of the file: past the known size, or fewer bytes than asked for.
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
        let start = offset + skipped as u64;
        let returned = text.len() as u64;
        let size = known.or(eof.then_some(end));
        let header = json!({
            "path": path.as_str(),
            "size": size,
            "offset": start,
            "length": returned,
            "next_offset": start + returned,
            // A cut at the end happens only before the end of the file.
            "eof": eof,
        });
        Ok(vec![header.to_string(), text.to_owned()])
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
