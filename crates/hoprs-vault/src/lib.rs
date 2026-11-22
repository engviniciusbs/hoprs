//! Local storage of saved sites and workspaces.
//!
//! Profiles live in two TOML files (`sites.toml`, `workspaces.toml`) under
//! [`default_dir`]. They never contain secrets: passwords and key
//! passphrases go to a [`SecretStore`] (the system keyring in the app), and
//! the site only records that one exists (`has_password`,
//! `has_key_passphrase`).

mod atomic;
mod export;
mod profiles;
mod secrets;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hoprs_core::{Error, ErrorKind, Result, Site, SiteId, Workspace, WorkspaceId};
use secrecy::SecretString;

pub use atomic::write_atomic;
pub use export::{BackupImportReport, ExportParams, ExportReport, ExportScope, ImportMode};
pub use profiles::UnreadableEntry;
pub use secrets::{KeyringStore, MemoryStore, SecretKind, SecretStore, is_keyring_unavailable};

use profiles::Profiles;

/// Default profile directory: `<config dir>/hoprs` (e.g. `~/.config/hoprs`).
pub fn default_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("hoprs"))
}

/// Saved sites and workspaces, plus their secrets. Every profile mutation
/// is written to disk before it becomes visible; a failed write leaves the
/// vault unchanged.
pub struct Vault {
    dir: PathBuf,
    profiles: Profiles,
    secrets: Arc<dyn SecretStore>,
}

/// Lists ids and names only.
impl fmt::Debug for Vault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sites: Vec<_> = self
            .profiles
            .sites
            .iter()
            .map(|s| (&s.id.0, &s.name))
            .collect();
        let workspaces: Vec<_> = self
            .profiles
            .workspaces
            .iter()
            .map(|w| (&w.id.0, &w.name))
            .collect();
        f.debug_struct("Vault")
            .field("dir", &self.dir)
            .field("sites", &sites)
            .field("workspaces", &workspaces)
            .field("unreadable", &self.profiles.unreadable().len())
            .finish_non_exhaustive()
    }
}

fn unknown_site(id: &SiteId) -> Error {
    Error::coded(ErrorKind::NotFound, "vault-unknown-site", "Unknown site")
        .with_detail(id.0.clone())
}

fn unknown_workspace(id: &WorkspaceId) -> Error {
    Error::coded(
        ErrorKind::NotFound,
        "vault-unknown-workspace",
        "Unknown workspace",
    )
    .with_detail(id.0.clone())
}

impl Vault {
    /// Opens the profiles in `dir`, keeping secrets in `secrets`. A missing
    /// directory or file is an empty vault; nothing is created until the
    /// first save.
    pub fn open(dir: &Path, secrets: Arc<dyn SecretStore>) -> Result<Vault> {
        Ok(Vault {
            dir: dir.to_path_buf(),
            profiles: profiles::load(dir)?,
            secrets,
        })
    }

    pub fn sites(&self) -> &[Site] {
        &self.profiles.sites
    }

    pub fn site(&self, id: &SiteId) -> Option<&Site> {
        self.profiles.sites.iter().find(|s| &s.id == id)
    }

    /// Inserts `site`, or replaces the site with the same id, and saves.
    ///
    /// `has_password` and `has_key_passphrase` are owned by the secret APIs
    /// ([`Vault::set_secret`], [`Vault::delete_secret`]): the stored flags of
    /// an existing site are kept, and a new site starts with both unset,
    /// whatever `site` says.
    pub fn upsert_site(&mut self, mut site: Site) -> Result<()> {
        if self.profiles.is_unreadable_site_id(&site.id.0) {
            return Err(Error::coded(
                ErrorKind::Protocol,
                "profile-site-id-unreadable",
                "Site id is taken by an unreadable entry",
            )
            .with_detail(site.id.0));
        }
        let mut next = self.profiles.clone();
        match next.sites.iter_mut().find(|s| s.id == site.id) {
            Some(slot) => {
                site.has_password = slot.has_password;
                site.has_key_passphrase = slot.has_key_passphrase;
                *slot = site;
            }
            None => {
                site.has_password = false;
                site.has_key_passphrase = false;
                next.sites.push(site);
            }
        }
        profiles::save_sites(&self.dir, &next)?;
        self.profiles = next;
        Ok(())
    }

    pub fn workspaces(&self) -> &[Workspace] {
        &self.profiles.workspaces
    }

    /// Inserts `workspace`, or replaces the one with the same id, and saves.
    /// Every site it references must exist (readable or not).
    pub fn upsert_workspace(&mut self, workspace: Workspace) -> Result<()> {
        if let Some(missing) = workspace.sites.iter().find(|ws| {
            self.site(&ws.site).is_none() && !self.profiles.is_unreadable_site_id(&ws.site.0)
        }) {
            return Err(Error::coded(
                ErrorKind::Protocol,
                "vault-workspace-unknown-site",
                "Unknown site in workspace",
            )
            .with_detail(missing.site.0.clone()));
        }
        if self.profiles.is_unreadable_workspace_id(&workspace.id.0) {
            return Err(Error::coded(
                ErrorKind::Protocol,
                "profile-workspace-id-unreadable",
                "Workspace id is taken by an unreadable entry",
            )
            .with_detail(workspace.id.0));
        }
        let mut next = self.profiles.clone();
        match next.workspaces.iter_mut().find(|w| w.id == workspace.id) {
            Some(slot) => *slot = workspace,
            None => next.workspaces.push(workspace),
        }
        profiles::save_workspaces(&self.dir, &next)?;
        self.profiles = next;
        Ok(())
    }

    pub fn remove_workspace(&mut self, id: &WorkspaceId) -> Result<()> {
        let Some(pos) = self.profiles.workspaces.iter().position(|w| &w.id == id) else {
            return Err(unknown_workspace(id));
        };
        let mut next = self.profiles.clone();
        next.workspaces.remove(pos);
        next.forget_workspace_extras(&id.0);
        profiles::save_workspaces(&self.dir, &next)?;
        self.profiles = next;
        Ok(())
    }

    /// Entries kept on disk but not understood by this version of hoprs.
    pub fn unreadable_entries(&self) -> &[UnreadableEntry] {
        self.profiles.unreadable()
    }

    fn site_index(&self, id: &SiteId) -> Result<usize> {
        self.profiles
            .sites
            .iter()
            .position(|s| &s.id == id)
            .ok_or_else(|| unknown_site(id))
    }

    /// Applies `change` to a copy of the site, saves `sites.toml`, then
    /// commits.
    fn edit_site(&mut self, id: &SiteId, change: impl FnOnce(&mut Site)) -> Result<()> {
        let pos = self.site_index(id)?;
        let mut next = self.profiles.clone();
        change(&mut next.sites[pos]);
        profiles::save_sites(&self.dir, &next)?;
        self.profiles = next;
        Ok(())
    }

    /// Removes the site, its references in every workspace and its secrets.
    ///
    /// `workspaces.toml` is saved first, then `sites.toml`, each committed
    /// as soon as it is written: a failure in between leaves the site in
    /// place without dangling references. Secrets are deleted last; if the
    /// keyring fails there, the site is already removed and the error is
    /// returned as `Io`.
    pub fn remove_site(&mut self, id: &SiteId) -> Result<()> {
        let pos = self.site_index(id)?;
        let name = self.profiles.sites[pos].name.clone();

        let mut next = self.profiles.clone();
        let mut referenced = false;
        for ws in &mut next.workspaces {
            let before = ws.sites.len();
            ws.sites.retain(|s| &s.site != id);
            referenced |= ws.sites.len() != before;
        }
        if referenced {
            profiles::save_workspaces(&self.dir, &next)?;
            self.profiles = next.clone();
        }

        next.sites.remove(pos);
        next.forget_site_extras(&id.0);
        profiles::save_sites(&self.dir, &next)?;
        self.profiles = next;

        // Flags may be stale, so both secrets are always deleted.
        let mut failure = None;
        for kind in [SecretKind::Password, SecretKind::KeyPassphrase] {
            if let Err(err) = self.secrets.delete(id, kind) {
                failure.get_or_insert(err);
            }
        }
        match failure {
            None => Ok(()),
            Some(err) => {
                let cause = describe(&err);
                Err(Error::coded(
                    ErrorKind::Io,
                    "vault-site-secrets-not-deleted",
                    "Site removed, but its secrets could not be deleted from the keyring",
                )
                .with_detail(format!("site {name:?} ({id}): {cause}")))
            }
        }
    }

    /// Stores a secret of the site and records that it has one.
    ///
    /// If the store fails, nothing changes. If the store succeeds but the
    /// profile cannot be saved, the secret is deleted again (best effort)
    /// and the save error is returned; if that delete fails too, its reason
    /// is appended to the error's detail.
    pub fn set_secret(
        &mut self,
        site: &SiteId,
        kind: SecretKind,
        value: &SecretString,
    ) -> Result<()> {
        let pos = self.site_index(site)?;
        self.secrets.set(site, kind, value)?;
        if flag(&self.profiles.sites[pos], kind) {
            return Ok(());
        }
        let result = self.edit_site(site, |s| *flag_mut(s, kind) = true);
        if let Err(mut err) = result {
            if let Err(rollback) = self.secrets.delete(site, kind) {
                append_detail(
                    &mut err,
                    &format!("rollback failed: {}", describe(&rollback)),
                );
            }
            return Err(err);
        }
        Ok(())
    }

    /// The site's secret, if it has one. A site whose flag is unset answers
    /// `None` without asking the store.
    pub fn secret(&self, site: &SiteId, kind: SecretKind) -> Result<Option<SecretString>> {
        let pos = self.site_index(site)?;
        if !flag(&self.profiles.sites[pos], kind) {
            return Ok(None);
        }
        self.secrets.get(site, kind)
    }

    /// Deletes a secret of the site and clears its flag. The store is
    /// always asked, even if the flag is already unset.
    pub fn delete_secret(&mut self, site: &SiteId, kind: SecretKind) -> Result<()> {
        let pos = self.site_index(site)?;
        self.secrets.delete(site, kind)?;
        if !flag(&self.profiles.sites[pos], kind) {
            return Ok(());
        }
        self.edit_site(site, |s| *flag_mut(s, kind) = false)
    }

    /// Pins the FTPS certificate fingerprint of the site.
    pub fn set_ftps_pin(&mut self, site: &SiteId, fingerprint: &str) -> Result<()> {
        self.edit_site(site, |s| {
            s.ftp.pinned_fingerprint = Some(fingerprint.to_owned());
        })
    }

    /// Forgets the pinned FTPS certificate of the site.
    pub fn clear_ftps_pin(&mut self, site: &SiteId) -> Result<()> {
        self.edit_site(site, |s| s.ftp.pinned_fingerprint = None)
    }
}

/// `message: detail`, or just the message. Errors never carry secret values.
fn describe(err: &Error) -> String {
    match &err.detail {
        Some(detail) => format!("{}: {detail}", err.message),
        None => err.message.clone(),
    }
}

/// Appends `extra` to the error's detail.
fn append_detail(err: &mut Error, extra: &str) {
    err.detail = Some(match err.detail.take() {
        Some(detail) => format!("{detail}; {extra}"),
        None => extra.to_owned(),
    });
}

fn flag(site: &Site, kind: SecretKind) -> bool {
    match kind {
        SecretKind::Password => site.has_password,
        SecretKind::KeyPassphrase => site.has_key_passphrase,
    }
}

fn flag_mut(site: &mut Site, kind: SecretKind) -> &mut bool {
    match kind {
        SecretKind::Password => &mut site.has_password,
        SecretKind::KeyPassphrase => &mut site.has_key_passphrase,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use hoprs_core::{Protocol, SavedTab, TabKind, WorkspaceSite};

    use super::*;

    fn ssh_site(name: &str) -> Site {
        Site::new(name, "example.com", "deploy", Protocol::Ssh)
    }

    fn workspace(name: &str, sites: &[&Site]) -> Workspace {
        Workspace {
            id: WorkspaceId::new(),
            name: name.into(),
            sites: sites
                .iter()
                .map(|s| WorkspaceSite {
                    site: s.id.clone(),
                    tabs: vec![],
                })
                .collect(),
        }
    }

    fn open(dir: &Path) -> Result<Vault> {
        Vault::open(dir, Arc::new(MemoryStore::new()))
    }

    fn read_table(path: &Path) -> toml::Table {
        fs::read_to_string(path).unwrap().parse().unwrap()
    }

    #[test]
    fn default_dir_ends_with_hoprs() {
        if let Some(dir) = default_dir() {
            assert!(dir.ends_with("hoprs"), "{}", dir.display());
        }
    }

    #[test]
    fn empty_dir_loads_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = open(tmp.path()).unwrap();
        assert!(vault.sites().is_empty());
        assert!(vault.workspaces().is_empty());
        assert!(vault.unreadable_entries().is_empty());
        // Opening must not create anything.
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn missing_dir_loads_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = open(&tmp.path().join("nope")).unwrap();
        assert!(vault.sites().is_empty());
    }

    #[test]
    fn roundtrip_sites_and_workspaces() {
        let tmp = tempfile::tempdir().unwrap();
        let mut a = ssh_site("A");
        a.ssh.key_file = Some(PathBuf::from("keys").join("id_ed25519"));
        a.remote_dir = Some("/srv".into());
        let mut b = Site::new("B", "ftp.example.com", "web", Protocol::FtpsImplicit);
        b.ftp.pinned_fingerprint = Some("AB:CD".into());
        b.local_dir = Some(PathBuf::from("work"));
        let mut ws = workspace("Prod", &[&a, &b]);
        ws.sites[0].tabs = vec![
            SavedTab {
                kind: TabKind::Terminal,
                remote_dir: None,
                local_dir: None,
            },
            SavedTab {
                kind: TabKind::Files,
                remote_dir: Some("/var/www".into()),
                local_dir: Some(PathBuf::from("site")),
            },
        ];
        {
            let mut vault = open(tmp.path()).unwrap();
            vault.upsert_site(a.clone()).unwrap();
            vault.upsert_site(b.clone()).unwrap();
            vault.upsert_workspace(ws.clone()).unwrap();
            assert_eq!(vault.site(&b.id), Some(&b));
        }
        let vault = open(tmp.path()).unwrap();
        assert_eq!(vault.sites(), [a.clone(), b.clone()]);
        assert_eq!(vault.workspaces(), [ws]);
        assert_eq!(vault.site(&a.id), Some(&a));
        assert_eq!(vault.site(&SiteId::new()), None);

        let sites = fs::read_to_string(tmp.path().join("sites.toml")).unwrap();
        assert!(sites.starts_with("version = 1\n"), "{sites}");
        assert!(sites.contains("[[site]]"), "{sites}");
        assert!(!sites.lines().any(|l| l.starts_with("password")), "{sites}");
        let workspaces = fs::read_to_string(tmp.path().join("workspaces.toml")).unwrap();
        assert!(workspaces.starts_with("version = 1\n"), "{workspaces}");
        assert!(workspaces.contains("[[workspace]]"), "{workspaces}");
    }

    #[test]
    fn upsert_replaces_by_id() {
        let tmp = tempfile::tempdir().unwrap();
        let mut vault = open(tmp.path()).unwrap();
        let mut a = ssh_site("A");
        vault.upsert_site(a.clone()).unwrap();
        vault.upsert_site(ssh_site("B")).unwrap();
        a.name = "A2".into();
        vault.upsert_site(a.clone()).unwrap();
        assert_eq!(vault.sites().len(), 2);
        assert_eq!(vault.sites()[0], a);
        let reopened = open(tmp.path()).unwrap();
        assert_eq!(reopened.sites(), vault.sites());
    }

    #[test]
    fn remove_workspace_saves_and_unknown_is_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let mut vault = open(tmp.path()).unwrap();
        let s = ssh_site("A");
        vault.upsert_site(s.clone()).unwrap();
        let w1 = workspace("one", &[&s]);
        let w2 = workspace("two", &[&s]);
        vault.upsert_workspace(w1.clone()).unwrap();
        vault.upsert_workspace(w2.clone()).unwrap();
        vault.remove_workspace(&w1.id).unwrap();
        assert_eq!(vault.workspaces(), std::slice::from_ref(&w2));
        let err = vault.remove_workspace(&w1.id).unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        let reopened = open(tmp.path()).unwrap();
        assert_eq!(reopened.workspaces(), [w2]);
    }

    #[test]
    fn corrupt_sites_toml_is_protocol_and_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sites.toml");
        fs::write(&path, "[[site]\nname =").unwrap();
        let err = open(tmp.path()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
        assert_eq!(err.message, "Cannot read sites.toml");
        let detail = err.detail.unwrap();
        assert!(detail.contains("line"), "{detail}");
        assert_eq!(fs::read(&path).unwrap(), b"[[site]\nname =");
    }

    #[test]
    fn corrupt_workspaces_toml_is_protocol() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("workspaces.toml"), "version = = 1").unwrap();
        let err = open(tmp.path()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
        assert_eq!(err.message, "Cannot read workspaces.toml");
    }

    #[test]
    fn missing_version_or_empty_file_is_version_1() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("sites.toml"), "").unwrap();
        fs::write(
            tmp.path().join("workspaces.toml"),
            "[[workspace]]\nid = \"w\"\nname = \"Hand\"\n",
        )
        .unwrap();
        let mut vault = open(tmp.path()).unwrap();
        assert!(vault.sites().is_empty());
        assert_eq!(vault.workspaces()[0].name, "Hand");
        vault.upsert_site(ssh_site("A")).unwrap();
        let sites = fs::read_to_string(tmp.path().join("sites.toml")).unwrap();
        assert!(sites.starts_with("version = 1\n"), "{sites}");
    }

    #[test]
    fn non_integer_version_is_protocol() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("sites.toml"), "version = \"1\"\n").unwrap();
        let err = open(tmp.path()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
    }

    #[test]
    fn newer_version_is_refused() {
        for file in ["sites.toml", "workspaces.toml"] {
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join(file);
            fs::write(&path, "version = 2\n").unwrap();
            let err = open(tmp.path()).unwrap_err();
            assert_eq!(err.kind, ErrorKind::Protocol, "{file}");
            assert_eq!(err.message, "Profile was written by a newer hoprs");
            assert_eq!(fs::read(&path).unwrap(), b"version = 2\n");
        }
    }

    #[test]
    fn unknown_fields_survive_a_save() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sites.toml");
        fs::write(
            &path,
            r#"version = 1
future_top = true

[[site]]
id = "s1"
name = "Old"
host = "h"
port = 22
user = "u"
protocol = "ssh"
future_field = "x"

[site.ssh]
use_agent = false
future_nested = 7
"#,
        )
        .unwrap();
        let mut vault = open(tmp.path()).unwrap();
        assert!(!vault.sites()[0].ssh.use_agent);
        vault.upsert_site(ssh_site("New")).unwrap();
        // Editing the site itself keeps its unknown fields as well.
        let mut old = vault.sites()[0].clone();
        old.name = "Renamed".into();
        vault.upsert_site(old).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("future_field = \"x\""), "{text}");
        let doc = read_table(&path);
        assert_eq!(doc["future_top"].as_bool(), Some(true));
        let first = doc["site"].as_array().unwrap()[0].as_table().unwrap();
        assert_eq!(first["name"].as_str(), Some("Renamed"));
        assert_eq!(first["future_field"].as_str(), Some("x"));
        assert_eq!(first["ssh"]["future_nested"].as_integer(), Some(7));
        assert_eq!(first["ssh"]["use_agent"].as_bool(), Some(false));

        let reopened = open(tmp.path()).unwrap();
        assert_eq!(reopened.sites(), vault.sites());
    }

    #[test]
    fn unknown_fields_inside_arrays_of_tables_survive() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("sites.toml"),
            "version = 1\n\n[[site]]\nid = \"s\"\nname = \"n\"\nhost = \"h\"\nport = 22\nuser = \"u\"\nprotocol = \"ssh\"\n",
        )
        .unwrap();
        let path = tmp.path().join("workspaces.toml");
        fs::write(
            &path,
            r#"version = 1

[[workspace]]
id = "w"
name = "Prod"

[[workspace.sites]]
site = "s"
future_site_key = 1

[[workspace.sites.tabs]]
kind = "terminal"

[[workspace.sites.tabs]]
kind = "files"
remote_dir = "/srv"
future_tab_key = "split"
"#,
        )
        .unwrap();
        let mut vault = open(tmp.path()).unwrap();
        let mut ws = vault.workspaces()[0].clone();
        ws.name = "Renamed".into();
        vault.upsert_workspace(ws.clone()).unwrap();

        let doc = read_table(&path);
        let site = &doc["workspace"][0]["sites"][0];
        assert_eq!(site["future_site_key"].as_integer(), Some(1));
        assert_eq!(site["tabs"][1]["future_tab_key"].as_str(), Some("split"));
        assert_eq!(site["tabs"][1]["remote_dir"].as_str(), Some("/srv"));
        assert!(site["tabs"][0].get("future_tab_key").is_none());

        // Removing a tab shifts positions: the tabs' unknown keys are dropped
        // rather than attached to the wrong tab; the site's are kept.
        ws.sites[0].tabs.remove(0);
        vault.upsert_workspace(ws).unwrap();
        let doc = read_table(&path);
        let site = &doc["workspace"][0]["sites"][0];
        assert_eq!(site["future_site_key"].as_integer(), Some(1));
        assert!(site["tabs"][0].get("future_tab_key").is_none());
        assert_eq!(site["tabs"][0]["kind"].as_str(), Some("files"));
    }

    #[test]
    fn cleared_optional_field_is_not_resurrected() {
        let tmp = tempfile::tempdir().unwrap();
        let mut vault = open(tmp.path()).unwrap();
        let mut s = ssh_site("A");
        s.remote_dir = Some("/srv".into());
        s.ssh.key_file = Some(PathBuf::from("key"));
        vault.upsert_site(s.clone()).unwrap();
        let mut vault = open(tmp.path()).unwrap();
        s.remote_dir = None;
        s.ssh.key_file = None;
        vault.upsert_site(s.clone()).unwrap();
        let reopened = open(tmp.path()).unwrap();
        assert_eq!(reopened.sites(), [s]);
    }

    #[test]
    fn unknown_protocol_entry_is_preserved_and_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sites.toml");
        fs::write(
            &path,
            r#"version = 1

[[site]]
id = "s1"
name = "First"
host = "h"
port = 22
user = "u"
protocol = "ssh"

[[site]]
id = "s3-id"
name = "Bucket"
host = "s3.example.com"
port = 443
user = "u"
protocol = "s3"
region = "eu-west-1"

[[site]]
id = "s2"
name = "Second"
host = "h"
port = 21
user = "u"
protocol = "ftp"
"#,
        )
        .unwrap();
        let original_entry = read_table(&path)["site"].as_array().unwrap()[1].clone();

        let mut vault = open(tmp.path()).unwrap();
        let ids: Vec<_> = vault.sites().iter().map(|s| s.id.0.as_str()).collect();
        assert_eq!(ids, ["s1", "s2"]);
        let unreadable = vault.unreadable_entries();
        assert_eq!(unreadable.len(), 1);
        assert_eq!(unreadable[0].file, "sites.toml");
        assert_eq!(unreadable[0].index, 1);
        assert_eq!(unreadable[0].name.as_deref(), Some("Bucket"));
        assert!(
            unreadable[0].reason.contains("s3"),
            "{}",
            unreadable[0].reason
        );

        vault.upsert_site(ssh_site("Third")).unwrap();

        let entries = read_table(&path)["site"].as_array().unwrap().clone();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[1], original_entry, "kept verbatim, in place");

        let reopened = open(tmp.path()).unwrap();
        assert_eq!(reopened.sites().len(), 3);
        assert!(reopened.sites().iter().all(|s| s.id.0 != "s3-id"));
        assert_eq!(reopened.unreadable_entries(), vault.unreadable_entries());
    }

    #[test]
    fn unreadable_entry_ids_are_reserved() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("sites.toml"),
            "version = 1\n\n[[site]]\nid = \"dup\"\nname = \"x\"\nprotocol = \"s3\"\n\n\
             [[site]]\nid = \"dup\"\nname = \"y\"\nhost = \"h\"\nport = 22\nuser = \"u\"\nprotocol = \"ssh\"\n",
        )
        .unwrap();
        let err = open(tmp.path()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);

        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("sites.toml"),
            "version = 1\n\n[[site]]\nid = \"future\"\nname = \"x\"\nprotocol = \"s3\"\n",
        )
        .unwrap();
        let mut vault = open(tmp.path()).unwrap();
        let mut clash = ssh_site("clash");
        clash.id = SiteId("future".into());
        assert_eq!(
            vault.upsert_site(clash).unwrap_err().kind,
            ErrorKind::Protocol
        );
        // A workspace may still reference the unreadable site.
        let mut ws = workspace("w", &[]);
        ws.sites.push(WorkspaceSite {
            site: SiteId("future".into()),
            tabs: vec![],
        });
        vault.upsert_workspace(ws).unwrap();
    }

    #[test]
    fn unreadable_workspace_is_preserved() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("workspaces.toml");
        fs::write(
            &path,
            "version = 1\n\n[[workspace]]\nid = \"w1\"\nname = \"Odd\"\n\n\
             [[workspace.sites]]\nsite = \"s\"\ntabs = [{ kind = \"hologram\" }]\n",
        )
        .unwrap();
        let original = read_table(&path)["workspace"].clone();
        let mut vault = open(tmp.path()).unwrap();
        assert!(vault.workspaces().is_empty());
        assert_eq!(vault.unreadable_entries()[0].file, "workspaces.toml");
        assert_eq!(vault.unreadable_entries()[0].name.as_deref(), Some("Odd"));
        vault.upsert_workspace(workspace("New", &[])).unwrap();
        let entries = read_table(&path)["workspace"].as_array().unwrap().clone();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], original.as_array().unwrap()[0]);
    }

    #[test]
    fn duplicate_ids_are_protocol() {
        let tmp = tempfile::tempdir().unwrap();
        let entry = "[[site]]\nid = \"same\"\nname = \"n\"\nhost = \"h\"\nport = 22\nuser = \"u\"\nprotocol = \"ssh\"\n";
        fs::write(
            tmp.path().join("sites.toml"),
            format!("version = 1\n\n{entry}\n{entry}"),
        )
        .unwrap();
        let err = open(tmp.path()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);

        let tmp = tempfile::tempdir().unwrap();
        let entry = "[[workspace]]\nid = \"w\"\nname = \"n\"\n";
        fs::write(
            tmp.path().join("workspaces.toml"),
            format!("version = 1\n\n{entry}\n{entry}"),
        )
        .unwrap();
        let err = open(tmp.path()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
    }

    #[test]
    fn workspace_with_unknown_site_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let mut vault = open(tmp.path()).unwrap();
        let known = ssh_site("known");
        vault.upsert_site(known.clone()).unwrap();
        let ghost = ssh_site("ghost");
        let err = vault
            .upsert_workspace(workspace("w", &[&known, &ghost]))
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
        assert_eq!(err.message, "Unknown site in workspace");
        assert!(vault.workspaces().is_empty());
        assert!(!tmp.path().join("workspaces.toml").exists());
    }

    /// Restores a directory's permissions on drop so `TempDir` can clean up.
    #[cfg(unix)]
    struct RestoreMode(PathBuf);

    #[cfg(unix)]
    impl Drop for RestoreMode {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
        }
    }

    #[cfg(unix)]
    #[test]
    fn failed_save_leaves_memory_and_disk_unchanged() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("hoprs");
        let mut vault = open(&dir).unwrap();
        let a = ssh_site("A");
        vault.upsert_site(a.clone()).unwrap();
        let path = dir.join("sites.toml");
        let before = fs::read(&path).unwrap();

        let _guard = RestoreMode(dir.clone());
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();
        if fs::write(dir.join("probe"), b"").is_ok() {
            // Privileged user: permissions are not enforced, nothing to test.
            return;
        }

        let mut renamed = a.clone();
        renamed.name = "Renamed".into();
        assert!(vault.upsert_site(ssh_site("B")).is_err());
        assert!(vault.upsert_site(renamed).is_err());
        assert_eq!(vault.sites(), std::slice::from_ref(&a));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1, "no temporary left");

        drop(_guard);
        let reopened = open(&dir).unwrap();
        assert_eq!(reopened.sites(), [a]);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_path_fails_save_without_panic() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let tmp = tempfile::tempdir().unwrap();
        let mut vault = open(tmp.path()).unwrap();
        let a = ssh_site("A");
        vault.upsert_site(a.clone()).unwrap();
        let before = fs::read(tmp.path().join("sites.toml")).unwrap();

        let mut bad = ssh_site("Bad");
        bad.local_dir = Some(PathBuf::from(OsStr::from_bytes(b"dir\xff")));
        let err = vault.upsert_site(bad.clone()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
        let detail = err.detail.unwrap();
        assert!(detail.contains(&bad.id.0), "{detail}");
        assert_eq!(vault.sites(), [a]);
        assert_eq!(fs::read(tmp.path().join("sites.toml")).unwrap(), before);
    }

    #[cfg(unix)]
    #[test]
    fn file_modes_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("config").join("hoprs");
        let mut vault = open(&dir).unwrap();
        let s = ssh_site("A");
        vault.upsert_site(s.clone()).unwrap();
        vault.upsert_workspace(workspace("w", &[&s])).unwrap();

        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("sites.toml")), 0o600);
        assert_eq!(mode(&dir.join("workspaces.toml")), 0o600);
    }

    // ---- Secrets and the rest of the facade ----

    use secrecy::ExposeSecret;

    /// A secret store whose backend is never reachable.
    struct FailingStore;

    fn unavailable() -> Error {
        Error::new(ErrorKind::Io, "System keyring unavailable").with_detail("no D-Bus session")
    }

    impl SecretStore for FailingStore {
        fn get(&self, _: &SiteId, _: SecretKind) -> Result<Option<SecretString>> {
            Err(unavailable())
        }
        fn set(&self, _: &SiteId, _: SecretKind, _: &SecretString) -> Result<()> {
            Err(unavailable())
        }
        fn delete(&self, _: &SiteId, _: SecretKind) -> Result<()> {
            Err(unavailable())
        }
    }

    fn secret(s: &str) -> SecretString {
        SecretString::from(s)
    }

    fn exposed(s: Option<SecretString>) -> Option<String> {
        s.map(|s| s.expose_secret().to_owned())
    }

    #[test]
    fn secret_roundtrip_sets_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::new());
        let mut vault = Vault::open(tmp.path(), store.clone()).unwrap();
        let a = ssh_site("A");
        vault.upsert_site(a.clone()).unwrap();
        assert_eq!(
            exposed(vault.secret(&a.id, SecretKind::Password).unwrap()),
            None
        );

        vault
            .set_secret(&a.id, SecretKind::Password, &secret("hunter2"))
            .unwrap();
        let site = vault.site(&a.id).unwrap();
        assert!(site.has_password);
        assert!(!site.has_key_passphrase);
        assert_eq!(
            exposed(vault.secret(&a.id, SecretKind::Password).unwrap()).as_deref(),
            Some("hunter2")
        );
        assert_eq!(
            exposed(vault.secret(&a.id, SecretKind::KeyPassphrase).unwrap()),
            None
        );
        vault
            .set_secret(&a.id, SecretKind::KeyPassphrase, &secret("phrase"))
            .unwrap();
        // Replacing a secret keeps the flag.
        vault
            .set_secret(&a.id, SecretKind::Password, &secret("hunter3"))
            .unwrap();

        let reopened = Vault::open(tmp.path(), store).unwrap();
        let site = reopened.site(&a.id).unwrap();
        assert!(site.has_password && site.has_key_passphrase);
        assert_eq!(
            exposed(reopened.secret(&a.id, SecretKind::Password).unwrap()).as_deref(),
            Some("hunter3")
        );
        assert_eq!(
            exposed(reopened.secret(&a.id, SecretKind::KeyPassphrase).unwrap()).as_deref(),
            Some("phrase")
        );
    }

    #[test]
    fn delete_secret_clears_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::new());
        let mut vault = Vault::open(tmp.path(), store.clone()).unwrap();
        let a = ssh_site("A");
        vault.upsert_site(a.clone()).unwrap();
        vault
            .set_secret(&a.id, SecretKind::Password, &secret("hunter2"))
            .unwrap();
        vault
            .set_secret(&a.id, SecretKind::KeyPassphrase, &secret("phrase"))
            .unwrap();

        vault.delete_secret(&a.id, SecretKind::Password).unwrap();
        let site = vault.site(&a.id).unwrap();
        assert!(!site.has_password);
        assert!(site.has_key_passphrase);
        assert_eq!(
            exposed(store.get(&a.id, SecretKind::Password).unwrap()),
            None
        );
        assert_eq!(
            exposed(vault.secret(&a.id, SecretKind::Password).unwrap()),
            None
        );
        // Deleting again is fine.
        vault.delete_secret(&a.id, SecretKind::Password).unwrap();

        let reopened = Vault::open(tmp.path(), store).unwrap();
        assert!(!reopened.site(&a.id).unwrap().has_password);
        assert_eq!(
            exposed(reopened.secret(&a.id, SecretKind::KeyPassphrase).unwrap()).as_deref(),
            Some("phrase")
        );
    }

    #[test]
    fn unknown_site_is_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::new());
        let mut vault = Vault::open(tmp.path(), store.clone()).unwrap();
        let ghost = SiteId::new();
        let errors = [
            vault.secret(&ghost, SecretKind::Password).unwrap_err(),
            vault
                .set_secret(&ghost, SecretKind::Password, &secret("hunter2"))
                .unwrap_err(),
            vault
                .delete_secret(&ghost, SecretKind::Password)
                .unwrap_err(),
            vault.remove_site(&ghost).unwrap_err(),
            vault.set_ftps_pin(&ghost, "AB").unwrap_err(),
            vault.clear_ftps_pin(&ghost).unwrap_err(),
        ];
        for err in errors {
            assert_eq!(err.kind, ErrorKind::NotFound);
            assert_eq!(err.message, "Unknown site");
            assert_eq!(err.detail.as_deref(), Some(ghost.0.as_str()));
        }
        assert_eq!(
            exposed(store.get(&ghost, SecretKind::Password).unwrap()),
            None
        );
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn remove_site_deletes_secrets_and_workspace_refs() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::new());
        let mut vault = Vault::open(tmp.path(), store.clone()).unwrap();
        let a = ssh_site("A");
        let b = ssh_site("B");
        vault.upsert_site(a.clone()).unwrap();
        vault.upsert_site(b.clone()).unwrap();
        vault
            .set_secret(&a.id, SecretKind::Password, &secret("hunter2"))
            .unwrap();
        vault
            .set_secret(&a.id, SecretKind::KeyPassphrase, &secret("phrase"))
            .unwrap();
        vault
            .set_secret(&b.id, SecretKind::Password, &secret("other"))
            .unwrap();
        let both = workspace("both", &[&a, &b]);
        let only_a = workspace("only a", &[&a]);
        vault.upsert_workspace(both.clone()).unwrap();
        vault.upsert_workspace(only_a.clone()).unwrap();

        vault.remove_site(&a.id).unwrap();

        assert_eq!(vault.sites().len(), 1);
        assert_eq!(vault.sites()[0].id, b.id);
        let ids = |ws: &Workspace| ws.sites.iter().map(|s| s.site.clone()).collect::<Vec<_>>();
        assert_eq!(vault.workspaces().len(), 2);
        assert_eq!(ids(&vault.workspaces()[0]), std::slice::from_ref(&b.id));
        assert!(ids(&vault.workspaces()[1]).is_empty());
        assert_eq!(
            exposed(store.get(&a.id, SecretKind::Password).unwrap()),
            None
        );
        assert_eq!(
            exposed(store.get(&a.id, SecretKind::KeyPassphrase).unwrap()),
            None
        );
        assert_eq!(
            exposed(vault.secret(&b.id, SecretKind::Password).unwrap()).as_deref(),
            Some("other")
        );

        let reopened = Vault::open(tmp.path(), store).unwrap();
        assert_eq!(reopened.sites(), vault.sites());
        assert_eq!(reopened.workspaces(), vault.workspaces());
        for file in ["sites.toml", "workspaces.toml"] {
            let text = fs::read_to_string(tmp.path().join(file)).unwrap();
            assert!(!text.contains(&a.id.0), "{file}: {text}");
        }
        assert_eq!(
            vault.remove_site(&a.id).unwrap_err().kind,
            ErrorKind::NotFound
        );
    }

    #[test]
    fn remove_site_with_keyring_down_still_removes() {
        let tmp = tempfile::tempdir().unwrap();
        let mut vault = Vault::open(tmp.path(), Arc::new(FailingStore)).unwrap();
        let a = ssh_site("Doomed");
        vault.upsert_site(a.clone()).unwrap();
        vault.upsert_workspace(workspace("w", &[&a])).unwrap();

        let err = vault.remove_site(&a.id).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Io);
        let detail = err.detail.unwrap();
        assert!(detail.contains("Doomed"), "{detail}");
        assert!(vault.sites().is_empty());
        assert!(vault.workspaces()[0].sites.is_empty());

        let reopened = Vault::open(tmp.path(), Arc::new(FailingStore)).unwrap();
        assert!(reopened.sites().is_empty());
        assert!(reopened.workspaces()[0].sites.is_empty());
    }

    #[test]
    fn keyring_failure_leaves_flag_unset() {
        let tmp = tempfile::tempdir().unwrap();
        let mut vault = Vault::open(tmp.path(), Arc::new(FailingStore)).unwrap();
        let a = ssh_site("A");
        vault.upsert_site(a.clone()).unwrap();
        let before = fs::read(tmp.path().join("sites.toml")).unwrap();

        let err = vault
            .set_secret(&a.id, SecretKind::Password, &secret("hunter2"))
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Io);
        assert_eq!(err.message, "System keyring unavailable");
        assert!(!vault.site(&a.id).unwrap().has_password);
        assert_eq!(fs::read(tmp.path().join("sites.toml")).unwrap(), before);

        let reopened = Vault::open(tmp.path(), Arc::new(FailingStore)).unwrap();
        assert!(!reopened.site(&a.id).unwrap().has_password);
        // A site without a secret does not need the keyring to answer.
        assert_eq!(
            exposed(reopened.secret(&a.id, SecretKind::Password).unwrap()),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn profile_save_failure_rolls_back_secret() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("hoprs");
        let store = Arc::new(MemoryStore::new());
        let mut vault = Vault::open(&dir, store.clone()).unwrap();
        let a = ssh_site("A");
        vault.upsert_site(a.clone()).unwrap();

        let _guard = RestoreMode(dir.clone());
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();
        if fs::write(dir.join("probe"), b"").is_ok() {
            // Privileged user: permissions are not enforced, nothing to test.
            return;
        }

        assert!(
            vault
                .set_secret(&a.id, SecretKind::Password, &secret("hunter2"))
                .is_err()
        );
        assert_eq!(
            exposed(store.get(&a.id, SecretKind::Password).unwrap()),
            None
        );
        assert!(!vault.site(&a.id).unwrap().has_password);

        drop(_guard);
        let reopened = Vault::open(&dir, store).unwrap();
        assert!(!reopened.site(&a.id).unwrap().has_password);
    }

    #[test]
    fn set_and_clear_ftps_pin() {
        let tmp = tempfile::tempdir().unwrap();
        let mut vault = open(tmp.path()).unwrap();
        let s = Site::new("F", "ftp.example.com", "web", Protocol::FtpsExplicit);
        vault.upsert_site(s.clone()).unwrap();

        vault.set_ftps_pin(&s.id, "AB:CD:EF").unwrap();
        assert_eq!(
            vault.site(&s.id).unwrap().ftp.pinned_fingerprint.as_deref(),
            Some("AB:CD:EF")
        );
        let mut reopened = open(tmp.path()).unwrap();
        assert_eq!(
            reopened
                .site(&s.id)
                .unwrap()
                .ftp
                .pinned_fingerprint
                .as_deref(),
            Some("AB:CD:EF")
        );

        reopened.clear_ftps_pin(&s.id).unwrap();
        assert_eq!(reopened.site(&s.id).unwrap().ftp.pinned_fingerprint, None);
        let reopened = open(tmp.path()).unwrap();
        assert_eq!(reopened.site(&s.id).unwrap().ftp.pinned_fingerprint, None);
        let text = fs::read_to_string(tmp.path().join("sites.toml")).unwrap();
        assert!(!text.contains("pinned_fingerprint"), "{text}");
    }

    #[test]
    fn no_secret_in_toml_or_debug() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::new());
        let mut vault = Vault::open(tmp.path(), store.clone()).unwrap();
        let a = ssh_site("Visible name");
        vault.upsert_site(a.clone()).unwrap();
        vault.upsert_workspace(workspace("Prod", &[&a])).unwrap();
        vault
            .set_secret(&a.id, SecretKind::Password, &secret("hunter2"))
            .unwrap();
        vault
            .set_secret(&a.id, SecretKind::KeyPassphrase, &secret("swordfish"))
            .unwrap();

        for file in ["sites.toml", "workspaces.toml"] {
            let text = fs::read_to_string(tmp.path().join(file)).unwrap();
            assert!(!text.contains("hunter2"), "{file}: {text}");
            assert!(!text.contains("swordfish"), "{file}: {text}");
        }
        let debug = format!("{vault:?} {store:?}");
        assert!(!debug.contains("hunter2"), "{debug}");
        assert!(!debug.contains("swordfish"), "{debug}");
        assert!(debug.contains("Visible name"), "{debug}");
        assert!(debug.contains(&a.id.0), "{debug}");
    }

    #[test]
    fn upsert_site_keeps_secret_flags() {
        let tmp = tempfile::tempdir().unwrap();
        let mut vault = open(tmp.path()).unwrap();
        // A new site starts without flags, whatever the caller passed.
        let mut a = ssh_site("A");
        a.has_password = true;
        a.has_key_passphrase = true;
        vault.upsert_site(a.clone()).unwrap();
        let stored = vault.site(&a.id).unwrap();
        assert!(!stored.has_password && !stored.has_key_passphrase);

        vault
            .set_secret(&a.id, SecretKind::Password, &secret("hunter2"))
            .unwrap();
        let mut edited = vault.site(&a.id).unwrap().clone();
        edited.name = "A2".into();
        edited.has_password = false;
        edited.has_key_passphrase = true;
        vault.upsert_site(edited).unwrap();

        let stored = vault.site(&a.id).unwrap();
        assert_eq!(stored.name, "A2");
        assert!(stored.has_password);
        assert!(!stored.has_key_passphrase);
        assert_eq!(
            exposed(vault.secret(&a.id, SecretKind::Password).unwrap()).as_deref(),
            Some("hunter2")
        );
        let reopened = open(tmp.path()).unwrap();
        assert!(reopened.site(&a.id).unwrap().has_password);
    }

    /// Stores secrets, but can never delete them.
    #[cfg(unix)]
    struct NoDeleteStore(MemoryStore);

    #[cfg(unix)]
    impl SecretStore for NoDeleteStore {
        fn get(&self, site: &SiteId, kind: SecretKind) -> Result<Option<SecretString>> {
            self.0.get(site, kind)
        }
        fn set(&self, site: &SiteId, kind: SecretKind, value: &SecretString) -> Result<()> {
            self.0.set(site, kind, value)
        }
        fn delete(&self, _: &SiteId, _: SecretKind) -> Result<()> {
            Err(unavailable())
        }
    }

    #[cfg(unix)]
    #[test]
    fn failed_rollback_is_reported_in_detail() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("hoprs");
        let mut vault = Vault::open(&dir, Arc::new(NoDeleteStore(MemoryStore::new()))).unwrap();
        let a = ssh_site("A");
        vault.upsert_site(a.clone()).unwrap();

        let _guard = RestoreMode(dir.clone());
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();
        if fs::write(dir.join("probe"), b"").is_ok() {
            // Privileged user: permissions are not enforced, nothing to test.
            return;
        }

        let err = vault
            .set_secret(&a.id, SecretKind::Password, &secret("hunter2"))
            .unwrap_err();
        let detail = err.detail.clone().unwrap_or_default();
        assert!(detail.contains("rollback failed: "), "{detail}");
        assert!(detail.contains("System keyring unavailable"), "{detail}");
        assert!(!format!("{err:?}").contains("hunter2"), "{err:?}");
        assert!(!vault.site(&a.id).unwrap().has_password);
    }
}
