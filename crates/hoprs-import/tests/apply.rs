use std::path::{Path, PathBuf};
use std::sync::Arc;

use hoprs_core::{Error, ErrorKind, FtpMode, Protocol, Result, SiteId};
use hoprs_import::{ImportOptions, apply, delete_source, parse_sitemanager};
use hoprs_vault::{MemoryStore, SecretKind, SecretStore, Vault};
use secrecy::{ExposeSecret, SecretString};
use tempfile::TempDir;

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn parsed(name: &str) -> hoprs_import::ParsedSiteManager {
    parse_sitemanager(&std::fs::read_to_string(fixture_path(name)).unwrap()).unwrap()
}

fn open() -> (TempDir, Arc<MemoryStore>, Vault) {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(MemoryStore::new());
    let vault = Vault::open(dir.path(), store.clone()).unwrap();
    (dir, store, vault)
}

const FLAT: ImportOptions = ImportOptions {
    folders_as_workspaces: false,
};
const WS: ImportOptions = ImportOptions {
    folders_as_workspaces: true,
};

#[test]
fn imports_supported_sites_with_secrets() {
    let (dir, store, mut vault) = open();
    let report = apply(&mut vault, parsed("sitemanager-3.66.xml"), &FLAT).unwrap();
    // 10 sites, one with an unsupported protocol.
    assert_eq!(report.imported.len(), 9);
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(vault.sites().len(), 9);
    // Every password reached the store.
    assert!(report.imported.iter().all(|s| !s.secret_not_saved));

    let legacy = vault
        .sites()
        .iter()
        .find(|s| s.name == "Legacy FTP")
        .unwrap();
    assert_eq!(legacy.protocol, Protocol::FtpsExplicit);
    assert!(legacy.has_password);
    let pw = store
        .get(&legacy.id, SecretKind::Password)
        .unwrap()
        .unwrap();
    assert_eq!(pw.expose_secret(), "hunter2-fictional");

    let shell = vault.sites().iter().find(|s| s.name == "Shell").unwrap();
    assert_eq!(shell.port, 2222);
    assert_eq!(
        shell.ssh.key_file.as_deref(),
        Some(Path::new("/home/alice/.ssh/id_ed25519"))
    );
    assert!(!shell.has_password);
    let implicit = vault
        .sites()
        .iter()
        .find(|s| s.name == "Implicit TLS")
        .unwrap();
    assert_eq!(implicit.ftp.mode, FtpMode::Active);
    let explicit = vault
        .sites()
        .iter()
        .find(|s| s.name == "Explicit TLS")
        .unwrap();
    assert_eq!(explicit.port, 21);

    // The password is nowhere in the report or on disk.
    let report_text = format!("{report:?}");
    assert!(!report_text.contains("hunter2"));
    for file in std::fs::read_dir(dir.path()).unwrap() {
        let text = std::fs::read_to_string(file.unwrap().path()).unwrap_or_default();
        assert!(!text.contains("hunter2"), "{text}");
    }
    // Report order follows the XML.
    assert_eq!(report.imported[0].name, "Legacy FTP");
    assert_eq!(report.skipped[0].name, "Object storage");
}

#[test]
fn folders_as_workspaces_creates_workspaces() {
    let (_dir, _store, mut vault) = open();
    let report = apply(&mut vault, parsed("sitemanager-3.40-folders.xml"), &WS).unwrap();
    assert_eq!(
        report.workspaces_created,
        vec![
            "Work",
            "Work/Clientes de Produção",
            "Work/Clientes de Produção/Deep"
        ]
    );
    assert_eq!(vault.workspaces().len(), 3);
    let names = |ws: &str| -> Vec<String> {
        let w = vault.workspaces().iter().find(|w| w.name == ws).unwrap();
        w.sites
            .iter()
            .map(|s| vault.site(&s.site).unwrap().name.clone())
            .collect()
    };
    assert_eq!(names("Work"), vec!["Alpha"]);
    assert_eq!(names("Work/Clientes de Produção"), vec!["Beta"]);
    assert_eq!(names("Work/Clientes de Produção/Deep"), vec!["Gamma"]);
    // Names are not prefixed in this mode.
    assert!(vault.sites().iter().any(|s| s.name == "Top level"));
}

#[test]
fn folders_flattened_into_names() {
    let (_dir, _store, mut vault) = open();
    let report = apply(&mut vault, parsed("sitemanager-3.40-folders.xml"), &FLAT).unwrap();
    assert!(report.workspaces_created.is_empty());
    assert!(vault.workspaces().is_empty());
    let names: Vec<_> = report.imported.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "Work/Alpha",
            "Work/Clientes de Produção/Beta",
            "Work/Clientes de Produção/Deep/Gamma",
            "Top level"
        ]
    );
}

#[test]
fn skipped_sites_have_reasons() {
    let (_dir, _store, mut vault) = open();
    let report = apply(&mut vault, parsed("sitemanager-3.66.xml"), &FLAT).unwrap();
    assert_eq!(
        report.skipped,
        vec![hoprs_import::SkippedSite {
            name: "Object storage".into(),
            reason: "unsupported protocol 7".into()
        }]
    );
    assert!(!vault.sites().iter().any(|s| s.name == "Object storage"));
}

#[test]
fn encrypted_passwords_import_without_secret_and_note() {
    let (_dir, store, mut vault) = open();
    let report = apply(&mut vault, parsed("sitemanager-crypt.xml"), &FLAT).unwrap();
    let locked = report.imported.iter().find(|s| s.name == "Locked").unwrap();
    assert_eq!(
        locked.notes,
        vec!["password is protected by a FileZilla master password; enter it in hoprs"]
    );
    let site = vault.site(&locked.id).unwrap();
    assert!(!site.has_password);
    assert!(
        store
            .get(&locked.id, SecretKind::Password)
            .unwrap()
            .is_none()
    );
}

#[test]
fn reimport_is_idempotent() {
    let (_dir, _store, mut vault) = open();
    let first = apply(&mut vault, parsed("sitemanager-3.40-folders.xml"), &WS).unwrap();
    let again = apply(&mut vault, parsed("sitemanager-3.40-folders.xml"), &WS).unwrap();
    assert!(again.imported.is_empty());
    assert!(again.workspaces_created.is_empty());
    assert_eq!(again.skipped.len(), first.imported.len());
    assert!(again.skipped.iter().all(|s| s.reason == "already in hoprs"));
    assert_eq!(vault.sites().len(), first.imported.len());
    assert_eq!(vault.workspaces().len(), 3);

    let (_dir, _store, mut vault) = open();
    let first = apply(&mut vault, parsed("sitemanager-3.66.xml"), &FLAT).unwrap();
    let again = apply(&mut vault, parsed("sitemanager-3.66.xml"), &FLAT).unwrap();
    assert!(again.imported.is_empty());
    assert_eq!(
        again.skipped.len(),
        first.imported.len() + first.skipped.len()
    );
    assert_eq!(vault.sites().len(), first.imported.len());
}

struct FailingStore;

impl SecretStore for FailingStore {
    fn get(&self, _: &SiteId, _: SecretKind) -> Result<Option<SecretString>> {
        Ok(None)
    }
    fn set(&self, _: &SiteId, _: SecretKind, _: &SecretString) -> Result<()> {
        Err(Error::new(ErrorKind::Io, "System keyring unavailable"))
    }
    fn delete(&self, _: &SiteId, _: SecretKind) -> Result<()> {
        Ok(())
    }
}

#[test]
fn keyring_failure_still_imports_site() {
    let dir = TempDir::new().unwrap();
    let mut vault = Vault::open(dir.path(), Arc::new(FailingStore)).unwrap();
    let report = apply(&mut vault, parsed("sitemanager-3.66.xml"), &FLAT).unwrap();
    assert_eq!(report.imported.len(), 9);
    let legacy = report
        .imported
        .iter()
        .find(|s| s.name == "Legacy FTP")
        .unwrap();
    assert!(
        legacy
            .notes
            .contains(&"password not saved: System keyring unavailable".to_string()),
        "{:?}",
        legacy.notes
    );
    assert!(!format!("{report:?}").contains("hunter2"));
    assert!(!vault.site(&legacy.id).unwrap().has_password);
    // The flag callers act on (the source is the only copy of that password); sites
    // without a password in the file are not flagged.
    assert!(legacy.secret_not_saved);
    assert!(report.imported.iter().any(
        |s| !s.secret_not_saved && !s.notes.iter().any(|n| n.starts_with("password not saved"))
    ));
}

#[derive(Default)]
struct CountingFailingStore {
    sets: std::sync::atomic::AtomicUsize,
}

impl SecretStore for CountingFailingStore {
    fn get(&self, _: &SiteId, _: SecretKind) -> Result<Option<SecretString>> {
        Ok(None)
    }
    fn set(&self, _: &SiteId, _: SecretKind, _: &SecretString) -> Result<()> {
        self.sets.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(Error::new(ErrorKind::Io, "System keyring unavailable"))
    }
    fn delete(&self, _: &SiteId, _: SecretKind) -> Result<()> {
        Ok(())
    }
}

#[test]
fn unavailable_keyring_is_tried_once_per_import() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(CountingFailingStore::default());
    let mut vault = Vault::open(dir.path(), store.clone()).unwrap();
    let xml = format!(
        "{}{}{}",
        server("A", "a.test", true),
        server("B", "b.test", true),
        server("C", "c.test", true)
    );
    let report = apply(&mut vault, inline_xml(&xml), &FLAT).unwrap();
    assert_eq!(report.imported.len(), 3);
    assert_eq!(store.sets.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(
        report.imported[0]
            .notes
            .contains(&"password not saved: System keyring unavailable".to_string())
    );
    assert!(report.imported.iter().all(|s| s.secret_not_saved));
    for site in &report.imported[1..] {
        assert!(
            site.notes
                .contains(&"password not saved: keyring unavailable".to_string()),
            "{:?}",
            site.notes
        );
    }
}

#[test]
fn delete_source_only_deletes_filezilla_xml() {
    let dir = TempDir::new().unwrap();
    let xml = dir.path().join("sitemanager.xml");
    std::fs::copy(fixture_path("sitemanager-3.66.xml"), &xml).unwrap();
    delete_source(&xml).unwrap();
    assert!(!xml.exists());

    let other = dir.path().join("notes.txt");
    std::fs::write(&other, "<?xml version=\"1.0\"?><other/>").unwrap();
    assert_eq!(delete_source(&other).unwrap_err().kind, ErrorKind::Protocol);
    let plain = dir.path().join("plain.txt");
    std::fs::write(&plain, "hello <FileZilla3").unwrap();
    assert_eq!(delete_source(&plain).unwrap_err().kind, ErrorKind::Protocol);
    assert!(other.exists() && plain.exists());

    assert_eq!(
        delete_source(&dir.path().join("missing.xml"))
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        delete_source(dir.path()).unwrap_err().kind,
        ErrorKind::Protocol
    );
}

#[cfg(unix)]
#[test]
fn delete_source_refuses_symlink() {
    let dir = TempDir::new().unwrap();
    let real = dir.path().join("real.xml");
    std::fs::copy(fixture_path("sitemanager-3.66.xml"), &real).unwrap();
    let link = dir.path().join("link.xml");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert_eq!(delete_source(&link).unwrap_err().kind, ErrorKind::Protocol);
    assert!(real.exists() && link.exists());
}

fn inline_xml(servers: &str) -> hoprs_import::ParsedSiteManager {
    parse_sitemanager(&format!(
        "<?xml version=\"1.0\"?><FileZilla3><Servers>{servers}</Servers></FileZilla3>"
    ))
    .unwrap()
}

fn server(name: &str, host: &str, pass: bool) -> String {
    let pass = if pass {
        "<Pass encoding=\"base64\">aHVudGVyMg==</Pass>"
    } else {
        ""
    };
    format!(
        "<Server><Host>{host}</Host><Protocol>1</Protocol><User>u</User>{pass}<Logontype>1</Logontype>{name}</Server>"
    )
}

#[test]
fn duplicate_within_the_same_xml_is_skipped() {
    let (_dir, _store, mut vault) = open();
    let xml = format!(
        "{}{}",
        server("Same", "h.test", false),
        server("Same", "h.test", false)
    );
    let report = apply(&mut vault, inline_xml(&xml), &FLAT).unwrap();
    assert_eq!(report.imported.len(), 1);
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].reason, "already in hoprs");
    assert_eq!(vault.sites().len(), 1);
}

#[test]
fn existing_workspace_name_is_skipped() {
    let (_dir, _store, mut vault) = open();
    vault
        .upsert_workspace(hoprs_core::Workspace {
            id: hoprs_core::WorkspaceId::new(),
            name: "Work".into(),
            sites: vec![],
        })
        .unwrap();
    let report = apply(&mut vault, parsed("sitemanager-3.40-folders.xml"), &WS).unwrap();
    assert_eq!(
        report.workspaces_created,
        vec![
            "Work/Clientes de Produção",
            "Work/Clientes de Produção/Deep"
        ]
    );
    assert_eq!(vault.workspaces().len(), 3);
    assert_eq!(
        report.skipped_workspaces,
        vec!["Work: a workspace with this name already exists"]
    );
    let work = vault
        .workspaces()
        .iter()
        .find(|w| w.name == "Work")
        .unwrap();
    assert!(work.sites.is_empty());
}

#[test]
fn delete_source_accepts_bom_and_refuses_late_marker() {
    let dir = TempDir::new().unwrap();
    let bom = dir.path().join("bom.xml");
    let mut bytes = b"\xEF\xBB\xBF".to_vec();
    bytes.extend(std::fs::read(fixture_path("sitemanager-3.66.xml")).unwrap());
    std::fs::write(&bom, bytes).unwrap();
    delete_source(&bom).unwrap();
    assert!(!bom.exists());

    let late = dir.path().join("late.xml");
    let body = format!("<?xml version=\"1.0\"?>{}<FileZilla3/>", " ".repeat(5000));
    std::fs::write(&late, body).unwrap();
    assert_eq!(delete_source(&late).unwrap_err().kind, ErrorKind::Protocol);
    assert!(late.exists());
}

#[cfg(unix)]
mod unix_failures {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Makes the vault dir read-only on the first secret it is asked to
    /// store (while armed), so later writes fail.
    struct LockingStore {
        inner: MemoryStore,
        dir: PathBuf,
        armed: AtomicBool,
    }

    impl SecretStore for LockingStore {
        fn get(&self, s: &SiteId, k: SecretKind) -> Result<Option<SecretString>> {
            self.inner.get(s, k)
        }
        fn set(&self, s: &SiteId, k: SecretKind, v: &SecretString) -> Result<()> {
            let r = self.inner.set(s, k, v);
            if self.armed.swap(false, Ordering::SeqCst) {
                set_mode(&self.dir, 0o500);
            }
            r
        }
        fn delete(&self, s: &SiteId, k: SecretKind) -> Result<()> {
            self.inner.delete(s, k)
        }
    }

    fn set_mode(dir: &Path, mode: u32) {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    struct Restore(PathBuf);
    impl Drop for Restore {
        fn drop(&mut self) {
            set_mode(&self.0, 0o700);
        }
    }

    fn writable(dir: &Path) -> bool {
        let probe = dir.join("probe");
        let ok = std::fs::write(&probe, b"x").is_ok();
        let _ = std::fs::remove_file(probe);
        ok
    }

    #[test]
    fn mid_import_failure_is_reported_and_rerun_recovers_workspace() {
        let dir = TempDir::new().unwrap();
        let _restore = Restore(dir.path().to_path_buf());
        let store = Arc::new(LockingStore {
            inner: MemoryStore::new(),
            dir: dir.path().to_path_buf(),
            armed: AtomicBool::new(true),
        });
        let mut vault = Vault::open(dir.path(), store.clone()).unwrap();
        // First site has a password: storing it locks the directory.
        let xml = format!(
            "<Folder>F{}{}{}</Folder>",
            server("One", "1.test", true),
            server("Two", "2.test", false),
            server("Three", "3.test", false)
        );
        let report = apply(&mut vault, inline_xml(&xml), &WS).unwrap();
        if writable(dir.path()) {
            return; // running as root: permissions are not enforced
        }
        assert_eq!(report.imported.len(), 1, "{report:?}");
        assert_eq!(report.imported[0].name, "One");
        let failed: Vec<_> = report.failures.iter().map(String::as_str).collect();
        assert!(failed.iter().any(|f| f.starts_with("Two: ")), "{failed:?}");
        assert!(
            failed.iter().any(|f| f.starts_with("Three: ")),
            "{failed:?}"
        );
        assert!(failed.iter().any(|f| f.starts_with("F: ")), "{failed:?}");
        assert!(report.workspaces_created.is_empty());
        assert!(!format!("{report:?}").contains("hunter2"));

        // Fix the directory and re-run: One is a duplicate, the rest import,
        // and the workspace is created from existing and new ids.
        set_mode(dir.path(), 0o700);
        let again = apply(&mut vault, inline_xml(&xml), &WS).unwrap();
        assert!(again.failures.is_empty(), "{again:?}");
        assert_eq!(again.imported.len(), 2);
        assert_eq!(again.workspaces_created, vec!["F"]);
        assert_eq!(vault.workspaces()[0].sites.len(), 3);
    }

    #[test]
    fn rerun_after_workspace_only_failure_creates_workspace() {
        let dir = TempDir::new().unwrap();
        let _restore = Restore(dir.path().to_path_buf());
        let store = Arc::new(LockingStore {
            inner: MemoryStore::new(),
            dir: dir.path().to_path_buf(),
            armed: AtomicBool::new(true),
        });
        let mut vault = Vault::open(dir.path(), store.clone()).unwrap();
        // The last site's password locks the directory after both sites are
        // saved; only its flag write and the workspace write then fail.
        let xml = format!(
            "<Folder>F{}{}</Folder>",
            server("One", "1.test", false),
            server("Two", "2.test", true)
        );
        let report = apply(&mut vault, inline_xml(&xml), &WS).unwrap();
        if writable(dir.path()) {
            return;
        }
        assert_eq!(report.imported.len(), 2);
        assert!(report.failures.iter().any(|f| f.starts_with("F: ")));
        assert!(vault.workspaces().is_empty());

        set_mode(dir.path(), 0o700);
        let again = apply(&mut vault, inline_xml(&xml), &WS).unwrap();
        assert!(again.imported.is_empty());
        assert_eq!(again.skipped.len(), 2);
        assert_eq!(again.workspaces_created, vec!["F"]);
        assert_eq!(vault.workspaces()[0].sites.len(), 2);
    }
}

#[test]
fn empty_host_and_port_zero_are_skipped() {
    let (_dir, _store, mut vault) = open();
    let xml = "<Server><Host></Host><Protocol>1</Protocol><Logontype>1</Logontype><Name>NoHost</Name></Server>\
               <Server><Host>h.test</Host><Port>0</Port><Protocol>1</Protocol><Logontype>1</Logontype><Name>Port0</Name></Server>";
    let report = apply(&mut vault, inline_xml(xml), &FLAT).unwrap();
    assert!(report.imported.is_empty());
    let skipped: Vec<_> = report
        .skipped
        .iter()
        .map(|s| (s.name.as_str(), s.reason.as_str()))
        .collect();
    assert_eq!(
        skipped,
        vec![("NoHost", "missing host"), ("Port0", "invalid port")]
    );
    assert!(vault.sites().is_empty());
}

#[test]
fn imported_sites_have_no_agent_access() {
    let (_dir, _store, mut vault) = open();
    let report = apply(&mut vault, parsed("sitemanager-3.66.xml"), &FLAT).unwrap();
    assert!(!report.imported.is_empty());
    assert!(
        vault
            .sites()
            .iter()
            .all(|s| s.agent_access == hoprs_core::AgentAccess::None)
    );

    // A site the user opened up in between keeps its level on a second import.
    let mut shell = vault
        .sites()
        .iter()
        .find(|s| s.name == "Shell")
        .unwrap()
        .clone();
    shell.agent_access = hoprs_core::AgentAccess::All;
    vault.upsert_site(shell.clone()).unwrap();
    apply(&mut vault, parsed("sitemanager-3.66.xml"), &FLAT).unwrap();
    assert_eq!(
        vault.site(&shell.id).unwrap().agent_access,
        hoprs_core::AgentAccess::All
    );
    let others = vault.sites().iter().filter(|s| s.id != shell.id);
    assert!(others.into_iter().all(|s| s.agent_access.is_none()));
}
