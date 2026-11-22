//! `sites.toml` and `workspaces.toml`: loading, validation and saving.
//!
//! Saving re-serializes the whole file from the parsed data: keys, values
//! and key order survive (including unknown fields and unreadable entries),
//! but comments and hand formatting are lost on the first save.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::Path;

use hoprs_core::{Error, ErrorKind, Result, Site, Workspace};
use serde::Serialize;
use serde::de::DeserializeOwned;
use toml::{Table, Value};

use crate::atomic::write_atomic;

pub(crate) const SITES_FILE: &str = "sites.toml";
pub(crate) const WORKSPACES_FILE: &str = "workspaces.toml";
const VERSION: i64 = 1;

/// A `[[site]]` or `[[workspace]]` entry that this version of hoprs cannot
/// understand (for example a protocol added by a newer version). It is kept
/// unchanged on disk and left out of the vault's lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnreadableEntry {
    /// `"sites.toml"` or `"workspaces.toml"`.
    pub file: &'static str,
    /// Position of the entry in the file's array when it was loaded.
    pub index: usize,
    /// The entry's `name`, if it has a readable one.
    pub name: Option<String>,
    /// Why the entry could not be read.
    pub reason: String,
}

/// An unreadable entry's original table, plus where to put it back.
#[derive(Clone, Debug)]
struct RawEntry {
    /// Id of the readable entry that preceded it, `None` if it came first.
    after: Option<String>,
    /// Raw `id`, if readable; still reserved against duplicates.
    id: Option<String>,
    table: Table,
}

/// What a file holds besides its readable entries.
#[derive(Clone, Debug, Default)]
struct FileRest {
    /// Unknown top-level keys.
    top: Table,
    unreadable: Vec<RawEntry>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Profiles {
    pub sites: Vec<Site>,
    pub workspaces: Vec<Workspace>,
    /// Unknown keys of each readable site, by site id.
    site_extras: HashMap<String, Table>,
    /// Unknown keys of each readable workspace, by workspace id.
    workspace_extras: HashMap<String, Table>,
    sites_rest: FileRest,
    workspaces_rest: FileRest,
    unreadable: Vec<UnreadableEntry>,
}

impl Profiles {
    pub fn unreadable(&self) -> &[UnreadableEntry] {
        &self.unreadable
    }

    /// Whether `id` is taken by an unreadable `[[site]]` entry.
    pub fn is_unreadable_site_id(&self, id: &str) -> bool {
        self.sites_rest.has_unreadable_id(id)
    }

    /// Whether `id` is taken by an unreadable `[[workspace]]` entry.
    pub fn is_unreadable_workspace_id(&self, id: &str) -> bool {
        self.workspaces_rest.has_unreadable_id(id)
    }

    /// The unreadable `[[site]]` entry whose raw `id` is `id`. The n-th raw
    /// entry of `sites_rest` pairs with the n-th [`UnreadableEntry`] of
    /// `sites.toml`: `load` pushes both together.
    pub fn unreadable_site(&self, id: &str) -> Option<&UnreadableEntry> {
        let n = self
            .sites_rest
            .unreadable
            .iter()
            .position(|e| e.id.as_deref() == Some(id))?;
        self.unreadable
            .iter()
            .filter(|e| e.file == SITES_FILE)
            .nth(n)
    }

    /// Profiles holding exactly `sites` and `workspaces`: no unknown keys,
    /// no unreadable entries.
    pub fn from_items(sites: Vec<Site>, workspaces: Vec<Workspace>) -> Self {
        Self {
            sites,
            workspaces,
            ..Self::default()
        }
    }

    /// Id and name of every unreadable `[[site]]` entry that has an id.
    pub fn unreadable_sites(&self) -> Vec<(String, Option<String>)> {
        self.sites_rest
            .unreadable
            .iter()
            .filter_map(|e| {
                let id = e.id.clone()?;
                let name = e
                    .table
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                Some((id, name))
            })
            .collect()
    }

    pub fn forget_site_extras(&mut self, id: &str) {
        self.site_extras.remove(id);
    }

    pub fn forget_workspace_extras(&mut self, id: &str) {
        self.workspace_extras.remove(id);
    }
}

impl FileRest {
    fn has_unreadable_id(&self, id: &str) -> bool {
        self.unreadable.iter().any(|e| e.id.as_deref() == Some(id))
    }
}

/// How one profile file maps to its entry type.
struct Kind<T> {
    file: &'static str,
    /// Key of the array of tables: `site` or `workspace`.
    key: &'static str,
    id_of: fn(&T) -> &str,
}

const SITES: Kind<Site> = Kind {
    file: SITES_FILE,
    key: "site",
    id_of: |s| &s.id.0,
};

const WORKSPACES: Kind<Workspace> = Kind {
    file: WORKSPACES_FILE,
    key: "workspace",
    id_of: |w| &w.id.0,
};

/// One file's content, as loaded.
struct Loaded<T> {
    items: Vec<T>,
    extras: HashMap<String, Table>,
    rest: FileRest,
    unreadable: Vec<UnreadableEntry>,
}

impl<T> Default for Loaded<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            extras: HashMap::new(),
            rest: FileRest::default(),
            unreadable: Vec::new(),
        }
    }
}

pub(crate) fn load(dir: &Path) -> Result<Profiles> {
    let sites = load_file(dir, &SITES)?;
    let workspaces = load_file(dir, &WORKSPACES)?;
    let mut unreadable = sites.unreadable;
    unreadable.extend(workspaces.unreadable);
    Ok(Profiles {
        sites: sites.items,
        workspaces: workspaces.items,
        site_extras: sites.extras,
        workspace_extras: workspaces.extras,
        sites_rest: sites.rest,
        workspaces_rest: workspaces.rest,
        unreadable,
    })
}

pub(crate) fn save_sites(dir: &Path, profiles: &Profiles) -> Result<()> {
    save_file(
        dir,
        &SITES,
        &profiles.sites,
        &profiles.site_extras,
        &profiles.sites_rest,
    )
}

pub(crate) fn save_workspaces(dir: &Path, profiles: &Profiles) -> Result<()> {
    save_file(
        dir,
        &WORKSPACES,
        &profiles.workspaces,
        &profiles.workspace_extras,
        &profiles.workspaces_rest,
    )
}

fn cannot_read(file: &str, detail: impl Into<String>) -> Error {
    Error::coded(
        ErrorKind::Protocol,
        "profile-cannot-read",
        format!("Cannot read {file}"),
    )
    .arg("file", file)
    .with_detail(detail)
}

fn load_file<T: DeserializeOwned + Serialize>(dir: &Path, kind: &Kind<T>) -> Result<Loaded<T>> {
    let file = kind.file;
    let text = match fs::read_to_string(dir.join(file)) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Loaded::default()),
        Err(err) if err.kind() == io::ErrorKind::InvalidData => {
            return Err(cannot_read(file, err.to_string()));
        }
        Err(err) => {
            let mut e = Error::from(err);
            e.detail = Some(std::mem::replace(
                &mut e.message,
                format!("Cannot read {file}"),
            ));
            return Err(e);
        }
    };
    let mut doc: Table = text
        .parse()
        .map_err(|e: toml::de::Error| cannot_read(file, e.to_string()))?;

    match doc.remove("version") {
        Some(Value::Integer(v)) if v == VERSION => {}
        Some(Value::Integer(v)) if v > VERSION => {
            return Err(Error::coded(
                ErrorKind::Protocol,
                "profile-newer-version",
                "Profile was written by a newer hoprs",
            )
            .with_detail(format!("{file} has version {v}")));
        }
        Some(Value::Integer(v)) => {
            return Err(cannot_read(file, format!("unsupported version {v}")));
        }
        Some(_) => return Err(cannot_read(file, "`version` is not an integer")),
        // Hand-written or empty file: hoprs always writes `version`.
        None => {}
    }

    let raw_entries = match doc.remove(kind.key) {
        None => Vec::new(),
        Some(Value::Array(values)) => values
            .into_iter()
            .map(|v| match v {
                Value::Table(t) => Ok(t),
                _ => Err(()),
            })
            .collect::<std::result::Result<Vec<_>, ()>>()
            .map_err(|()| {
                cannot_read(file, format!("`{}` must be an array of tables", kind.key))
            })?,
        Some(_) => {
            return Err(cannot_read(
                file,
                format!("`{}` must be an array of tables", kind.key),
            ));
        }
    };

    let mut loaded = Loaded {
        rest: FileRest {
            top: doc,
            unreadable: Vec::new(),
        },
        ..Loaded::default()
    };
    let mut seen = HashSet::new();
    let mut previous: Option<String> = None;
    for (index, raw) in raw_entries.into_iter().enumerate() {
        match raw.clone().try_into::<T>() {
            Ok(item) => {
                let id = (kind.id_of)(&item).to_owned();
                let typed = Table::try_from(&item).map_err(|e| cannot_read(file, e.to_string()))?;
                let extras = unknown_keys(&raw, &typed);
                if !extras.is_empty() {
                    loaded.extras.insert(id.clone(), extras);
                }
                check_unique(&mut seen, file, &id)?;
                previous = Some(id);
                loaded.items.push(item);
            }
            Err(err) => {
                let id = raw.get("id").and_then(Value::as_str).map(str::to_owned);
                if let Some(id) = &id {
                    check_unique(&mut seen, file, id)?;
                }
                loaded.unreadable.push(UnreadableEntry {
                    file,
                    index,
                    name: raw.get("name").and_then(Value::as_str).map(str::to_owned),
                    reason: err.message().to_owned(),
                });
                loaded.rest.unreadable.push(RawEntry {
                    after: previous.clone(),
                    id,
                    table: raw,
                });
            }
        }
    }
    Ok(loaded)
}

fn check_unique(seen: &mut HashSet<String>, file: &str, id: &str) -> Result<()> {
    if seen.insert(id.to_owned()) {
        Ok(())
    } else {
        Err(Error::coded(
            ErrorKind::Protocol,
            "profile-duplicate-id",
            format!("Duplicate id in {file}"),
        )
        .arg("file", file)
        .with_detail(id))
    }
}

fn save_file<T: Serialize>(
    dir: &Path,
    kind: &Kind<T>,
    items: &[T],
    extras: &HashMap<String, Table>,
    rest: &FileRest,
) -> Result<()> {
    let file = kind.file;
    let cannot_save = |detail: String| {
        Error::coded(
            ErrorKind::Protocol,
            "profile-cannot-save",
            format!("Cannot save {file}"),
        )
        .arg("file", file)
        .with_detail(detail)
    };
    let raw_after = |anchor: Option<&str>| -> Vec<Value> {
        rest.unreadable
            .iter()
            .filter(|e| e.after.as_deref() == anchor)
            .map(|e| Value::Table(e.table.clone()))
            .collect()
    };

    let mut entries = raw_after(None);
    for item in items {
        let id = (kind.id_of)(item);
        let mut table =
            Table::try_from(item).map_err(|e| cannot_save(format!("{} {id}: {e}", kind.key)))?;
        if let Some(extra) = extras.get(id) {
            merge_unknown(&mut table, extra);
        }
        entries.push(Value::Table(table));
        entries.extend(raw_after(Some(id)));
    }
    // Unreadable entries whose anchor no longer exists go last.
    let anchors: HashSet<&str> = items.iter().map(|i| (kind.id_of)(i)).collect();
    entries.extend(
        rest.unreadable
            .iter()
            .filter(|e| e.after.as_deref().is_some_and(|a| !anchors.contains(a)))
            .map(|e| Value::Table(e.table.clone())),
    );

    let mut doc = Table::new();
    doc.insert("version".into(), Value::Integer(VERSION));
    for (k, v) in &rest.top {
        doc.insert(k.clone(), v.clone());
    }
    if !entries.is_empty() {
        doc.insert(kind.key.into(), Value::Array(entries));
    }
    let text = toml::to_string(&doc).map_err(|e| cannot_save(e.to_string()))?;
    write_atomic(&dir.join(file), text.as_bytes())
}

/// Keys of `raw` that the typed entry does not produce, recursing into
/// sub-tables present on both sides and, element by element, into arrays of
/// tables of the same length on both sides (e.g. `[[workspace.sites]]`).
///
/// Arrays whose lengths differ are not recursed into: elements can no longer
/// be paired up by position, so their unknown keys are not preserved. This
/// cannot happen on load (the typed value comes from the same table) and is
/// handled the same way on save, see [`merge_unknown`].
fn unknown_keys(raw: &Table, typed: &Table) -> Table {
    let mut out = Table::new();
    for (key, value) in raw {
        let extra = match (value, typed.get(key)) {
            (_, None) => Some(value.clone()),
            (Value::Table(r), Some(Value::Table(t))) => {
                Some(Value::Table(unknown_keys(r, t))).filter(|v| !is_empty_extra(v))
            }
            (Value::Array(r), Some(Value::Array(t))) if r.len() == t.len() => {
                let nested: Option<Vec<Value>> = r
                    .iter()
                    .zip(t)
                    .map(|pair| match pair {
                        (Value::Table(r), Value::Table(t)) => {
                            Some(Value::Table(unknown_keys(r, t)))
                        }
                        _ => None,
                    })
                    .collect();
                nested.map(Value::Array).filter(|v| !is_empty_extra(v))
            }
            _ => None,
        };
        if let Some(extra) = extra {
            out.insert(key.clone(), extra);
        }
    }
    out
}

/// Whether an extras value carries no unknown key at all.
fn is_empty_extra(value: &Value) -> bool {
    match value {
        Value::Table(t) => t.values().all(is_empty_extra),
        Value::Array(a) => a.iter().all(is_empty_extra),
        _ => false,
    }
}

/// Puts `extras` back into `typed`; typed values win on conflict. Extras for
/// an array whose length changed since load are dropped (see
/// [`unknown_keys`]).
fn merge_unknown(typed: &mut Table, extras: &Table) {
    for (key, value) in extras {
        match (typed.get_mut(key), value) {
            (None, _) => {
                typed.insert(key.clone(), value.clone());
            }
            (Some(Value::Table(t)), Value::Table(e)) => merge_unknown(t, e),
            (Some(Value::Array(t)), Value::Array(e)) if t.len() == e.len() => {
                for pair in t.iter_mut().zip(e) {
                    if let (Value::Table(t), Value::Table(e)) = pair {
                        merge_unknown(t, e);
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreadable_site_finds_the_entry_by_id() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join(SITES_FILE),
            "version = 1\n\n\
             [[site]]\nid = \"s1\"\nname = \"One\"\nprotocol = \"ssh\"\nhost = \"h\"\nport = 22\nuser = \"u\"\n\n\
             [[site]]\nid = \"odd1\"\nname = \"Odd One\"\nprotocol = \"s3\"\n\n\
             [[site]]\nid = \"odd2\"\nname = \"Odd Two\"\nprotocol = \"gopher\"\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join(WORKSPACES_FILE),
            "version = 1\n\n[[workspace]]\nid = \"w1\"\nname = \"Odd Workspace\"\nsites = 7\n",
        )
        .unwrap();
        let profiles = load(tmp.path()).unwrap();
        assert_eq!(profiles.sites.len(), 1);
        assert_eq!(profiles.unreadable().len(), 3);

        let one = profiles.unreadable_site("odd1").unwrap();
        assert_eq!(
            (one.file, one.name.as_deref()),
            (SITES_FILE, Some("Odd One"))
        );
        let two = profiles.unreadable_site("odd2").unwrap();
        assert_eq!(
            (two.file, two.name.as_deref()),
            (SITES_FILE, Some("Odd Two"))
        );
        assert!(profiles.unreadable_site("s1").is_none());
        assert!(profiles.unreadable_site("w1").is_none());
    }

    #[test]
    fn agent_access_round_trips_in_sites_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let mut all = Site::new("Open", "h", "u", hoprs_core::Protocol::Ssh);
        all.agent_access = hoprs_core::AgentAccess::All;
        let none = Site::new("Closed", "h", "u", hoprs_core::Protocol::Ssh);
        let profiles = Profiles {
            sites: vec![all.clone(), none.clone()],
            ..Profiles::default()
        };
        save_sites(tmp.path(), &profiles).unwrap();
        let text = fs::read_to_string(tmp.path().join(SITES_FILE)).unwrap();
        assert_eq!(text.matches("agent_access").count(), 1, "{text}");
        assert_eq!(
            text.lines()
                .filter(|l| *l == "agent_access = \"all\"")
                .count(),
            1,
            "{text}"
        );
        let loaded = load(tmp.path()).unwrap();
        assert_eq!(loaded.sites[0].agent_access, hoprs_core::AgentAccess::All);
        assert_eq!(loaded.sites[1].agent_access, hoprs_core::AgentAccess::None);
    }

    #[test]
    fn unknown_agent_access_is_no_access_not_unreadable() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join(SITES_FILE),
            "version = 1\n\n\
             [[site]]\nid = \"s1\"\nname = \"One\"\nprotocol = \"ssh\"\nhost = \"h\"\nport = 22\nuser = \"u\"\nagent_access = \"admin\"\n",
        )
        .unwrap();
        let profiles = load(tmp.path()).unwrap();
        assert!(profiles.unreadable().is_empty());
        assert_eq!(profiles.sites.len(), 1);
        assert_eq!(
            profiles.sites[0].agent_access,
            hoprs_core::AgentAccess::None
        );
    }
}
