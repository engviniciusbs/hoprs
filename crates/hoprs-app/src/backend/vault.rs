//! The [`Vault`] behind `spawn_blocking`: profile files, the keyring and Argon2 may all
//! block, and the UI thread never does.

use std::sync::{Arc, Mutex, PoisonError};

use hoprs_core::{Error, ErrorKind, Protocol, Result, Site, SiteId, Workspace, WorkspaceId};
use hoprs_vault::{
    AppSecret, AppSecretStore, KeyringStore, SecretKind, SecretStore, UnreadableEntry, Vault,
    is_keyring_unavailable,
};
use secrecy::SecretString;

use super::mapping::SiteSecrets;

/// Everything the sidebar shows, read in one vault call (a consistent view).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProfileSnapshot {
    pub sites: Vec<Site>,
    pub workspaces: Vec<Workspace>,
    /// Entries this version cannot read (written by a newer hoprs?), kept on disk.
    pub unreadable: Vec<UnreadableEntry>,
}

/// Shared handle to the vault. Every call runs on tokio's blocking pool.
#[derive(Clone)]
pub struct VaultHandle {
    vault: Arc<Mutex<Vault>>,
}

impl VaultHandle {
    pub fn new(vault: Vault) -> Self {
        Self {
            vault: Arc::new(Mutex::new(vault)),
        }
    }

    /// Runs `f` on the vault in `spawn_blocking`.
    pub async fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Vault) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let vault = self.vault.clone();
        tokio::task::spawn_blocking(move || {
            // A panic inside a call cannot leave the vault half-updated: every mutation is
            // written to disk before it is committed in memory.
            let mut vault = vault.lock().unwrap_or_else(PoisonError::into_inner);
            f(&mut vault)
        })
        .await
        .map_err(|e| {
            Error::coded(ErrorKind::Io, "app-vault-task-failed", "Vault task failed")
                .with_detail(e.to_string())
        })?
    }

    // Test convenience: the app reads through `snapshot`.
    #[cfg(test)]
    pub async fn sites(&self) -> Result<Vec<Site>> {
        self.call(|v| Ok(v.sites().to_vec())).await
    }

    #[cfg(test)]
    pub async fn site(&self, id: SiteId) -> Result<Option<Site>> {
        self.call(move |v| Ok(v.site(&id).cloned())).await
    }

    #[cfg(test)]
    pub async fn workspaces(&self) -> Result<Vec<Workspace>> {
        self.call(|v| Ok(v.workspaces().to_vec())).await
    }

    #[cfg(test)]
    pub async fn upsert_site(&self, site: Site) -> Result<()> {
        self.call(move |v| v.upsert_site(site)).await
    }

    pub async fn snapshot(&self) -> Result<ProfileSnapshot> {
        self.call(|v| {
            Ok(ProfileSnapshot {
                sites: v.sites().to_vec(),
                workspaces: v.workspaces().to_vec(),
                unreadable: v.unreadable_entries().to_vec(),
            })
        })
        .await
    }

    /// Saves a site edited in the UI without overwriting what the backend owns.
    /// `has_password`/`has_key_passphrase` are kept by [`Vault::upsert_site`] itself. The
    /// FTPS pin is the vault's (the prompt broker may have written it while the form was
    /// open), see [`merged_pin`]: the UI's copy of it is never used.
    pub async fn save_site(&self, mut site: Site) -> Result<()> {
        self.call(move |v| {
            site.ftp.pinned_fingerprint = merged_pin(v.site(&site.id), &site);
            v.upsert_site(site)
        })
        .await
    }

    /// Stores (`Some`) or deletes (`None`) the site's password in the keyring; the vault
    /// sets or clears `has_password` with it.
    pub async fn set_password(&self, site: SiteId, password: Option<SecretString>) -> Result<()> {
        self.call(move |v| match password {
            Some(password) => v.set_secret(&site, SecretKind::Password, &password),
            None => v.delete_secret(&site, SecretKind::Password),
        })
        .await
    }

    /// Stores one secret of the site in the keyring; the vault sets its `has_*` flag.
    pub async fn set_secret(
        &self,
        site: SiteId,
        kind: SecretKind,
        secret: SecretString,
    ) -> Result<()> {
        self.call(move |v| v.set_secret(&site, kind, &secret)).await
    }

    pub async fn remove_site(&self, id: SiteId) -> Result<()> {
        self.call(move |v| v.remove_site(&id)).await
    }

    pub async fn upsert_workspace(&self, workspace: Workspace) -> Result<()> {
        self.call(move |v| v.upsert_workspace(workspace)).await
    }

    /// See [`BackendApi::save_workspace_tabs`](super::BackendApi::save_workspace_tabs).
    pub async fn save_workspace_tabs(
        &self,
        id: WorkspaceId,
        tabs: super::WorkspaceTabs,
    ) -> Result<()> {
        self.call(move |v| {
            let mut workspace = v
                .workspaces()
                .iter()
                .find(|w| w.id == id)
                .cloned()
                .ok_or_else(|| {
                    Error::coded(
                        ErrorKind::NotFound,
                        "vault-unknown-workspace",
                        "Unknown workspace",
                    )
                    .with_detail(id.0.clone())
                })?;
            let mut changed = false;
            for (site, saved) in tabs {
                if let Some(entry) = workspace.sites.iter_mut().find(|e| e.site == site)
                    && entry.tabs != saved
                {
                    entry.tabs = saved;
                    changed = true;
                }
            }
            if changed {
                v.upsert_workspace(workspace)?;
            }
            Ok(())
        })
        .await
    }

    pub async fn remove_workspace(&self, id: WorkspaceId) -> Result<()> {
        self.call(move |v| v.remove_workspace(&id)).await
    }

    /// Pins `fingerprint` whatever the site's endpoint (the app pins through
    /// [`set_ftps_pin_at`](Self::set_ftps_pin_at)).
    #[cfg(test)]
    pub async fn set_ftps_pin(&self, site: SiteId, fingerprint: String) -> Result<()> {
        self.call(move |v| v.set_ftps_pin(&site, &fingerprint))
            .await
    }

    /// Pins `fingerprint` on the site if it still connects to `endpoint` (`host:port`), in
    /// one vault call: a certificate accepted on the old endpoint of a site edited
    /// meanwhile belongs to another server. Returns whether it pinned.
    pub async fn set_ftps_pin_at(
        &self,
        site: SiteId,
        endpoint: String,
        fingerprint: String,
    ) -> Result<bool> {
        self.call(move |v| {
            let same = v
                .site(&site)
                .is_some_and(|s| format!("{}:{}", s.host, s.port) == endpoint);
            if !same {
                return Ok(false);
            }
            v.set_ftps_pin(&site, &fingerprint)?;
            Ok(true)
        })
        .await
    }

    /// Clears the site's pin if it is still `stale` (compared ignoring case): a pin the
    /// user accepted meanwhile is kept. Returns whether it cleared one.
    pub async fn clear_stale_ftps_pin(&self, site: SiteId, stale: String) -> Result<bool> {
        self.call(move |v| {
            let current = v
                .site(&site)
                .and_then(|s| s.ftp.pinned_fingerprint.as_deref());
            if !current.is_some_and(|pin| pin.eq_ignore_ascii_case(&stale)) {
                return Ok(false);
            }
            v.clear_ftps_pin(&site)?;
            Ok(true)
        })
        .await
    }

    /// Each site with its secrets, read with one [`SecretBatch`] (one blocking task). A
    /// site missing from the vault is `NotFound`.
    pub async fn sites_with_secrets(
        &self,
        ids: Vec<SiteId>,
    ) -> Result<Vec<(SiteId, Result<(Site, SiteSecrets)>)>> {
        self.call(move |v| {
            let mut batch = SecretBatch::new(v);
            Ok(ids
                .into_iter()
                .map(|id| {
                    let found = match v.site(&id) {
                        Some(site) => Ok((site.clone(), batch.read(site))),
                        None => Err(Error::coded(
                            ErrorKind::NotFound,
                            "vault-unknown-site",
                            "Unknown site",
                        )
                        .with_detail(id.0.clone())),
                    };
                    (id, found)
                })
                .collect())
        })
        .await
    }
}

/// The FTPS pin a saved site keeps: the stored one, only while the edit leaves the same
/// endpoint (host and port) and the protocol is still FTPS. A pin certifies one server: a
/// new host or port, or a protocol without certificates, drops it.
pub fn merged_pin(stored: Option<&Site>, edited: &Site) -> Option<String> {
    let stored = stored?;
    let ftps = matches!(
        edited.protocol,
        Protocol::FtpsExplicit | Protocol::FtpsImplicit
    );
    let same_endpoint = stored.host == edited.host && stored.port == edited.port;
    if ftps && same_endpoint {
        stored.ftp.pinned_fingerprint.clone()
    } else {
        None
    }
}

/// Reads the secrets of several sites (Open workspace) with the import's protection: after
/// the first "keyring unavailable" error it stops asking the keyring, so a locked keyring
/// raises at most one unlock prompt, and marks every remaining site `keyring_unavailable`
/// (their secrets then come from the normal prompts).
pub struct SecretBatch<'a> {
    vault: &'a Vault,
    keyring_down: bool,
}

impl<'a> SecretBatch<'a> {
    pub fn new(vault: &'a Vault) -> Self {
        Self {
            vault,
            keyring_down: false,
        }
    }

    /// The secrets `site` has (`has_password`, `has_key_passphrase`). Any other keyring
    /// error is logged and that secret is left out (it will be asked for).
    pub fn read(&mut self, site: &Site) -> SiteSecrets {
        let mut secrets = SiteSecrets::default();
        let wanted = [
            (site.has_password, SecretKind::Password),
            (site.has_key_passphrase, SecretKind::KeyPassphrase),
        ];
        for (_, kind) in wanted.into_iter().filter(|(has, _)| *has) {
            if self.keyring_down {
                secrets.keyring_unavailable = true;
                continue;
            }
            match self.vault.secret(&site.id, kind) {
                Ok(value) => match kind {
                    SecretKind::Password => secrets.password = value,
                    SecretKind::KeyPassphrase => secrets.key_passphrase = value,
                },
                Err(err) if is_keyring_unavailable(&err) => {
                    self.keyring_down = true;
                    secrets.keyring_unavailable = true;
                }
                Err(err) => {
                    // Keyring errors never carry secret values.
                    tracing::warn!(site = %site.id, ?kind, error = %err, "cannot read a site secret; it will be asked for");
                }
            }
        }
        secrets
    }
}

/// The system keyring, connected on first use and again after a failure (the user may
/// unlock or start it later). Connecting opens a D-Bus connection, so it happens on the
/// blocking pool like every other vault call, never at startup on the UI thread.
#[derive(Default)]
pub struct LazyKeyringStore {
    store: Mutex<Option<Arc<KeyringStore>>>,
}

impl LazyKeyringStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn get_store(&self) -> Result<Arc<KeyringStore>> {
        let mut slot = self.store.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(store) = slot.as_ref() {
            return Ok(store.clone());
        }
        let store = Arc::new(KeyringStore::new()?);
        *slot = Some(store.clone());
        Ok(store)
    }
}

impl SecretStore for LazyKeyringStore {
    fn get(&self, site: &SiteId, kind: SecretKind) -> Result<Option<SecretString>> {
        self.get_store()?.get(site, kind)
    }

    fn set(&self, site: &SiteId, kind: SecretKind, value: &SecretString) -> Result<()> {
        self.get_store()?.set(site, kind, value)
    }

    fn delete(&self, site: &SiteId, kind: SecretKind) -> Result<()> {
        self.get_store()?.delete(site, kind)
    }
}

impl AppSecretStore for LazyKeyringStore {
    fn get_app(&self, which: AppSecret) -> Result<Option<SecretString>> {
        self.get_store()?.get_app(which)
    }

    fn set_app(&self, which: AppSecret, value: &SecretString) -> Result<()> {
        self.get_store()?.set_app(which, value)
    }

    fn delete_app(&self, which: AppSecret) -> Result<()> {
        self.get_store()?.delete_app(which)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use hoprs_core::Protocol;
    use hoprs_vault::MemoryStore;
    use secrecy::ExposeSecret;

    use super::*;

    /// Stores secrets in memory, but every `get` fails as an unreachable keyring would.
    #[derive(Default)]
    pub(crate) struct LockedStore {
        inner: MemoryStore,
        pub(crate) gets: AtomicU32,
    }

    impl SecretStore for LockedStore {
        fn get(&self, _: &SiteId, _: SecretKind) -> Result<Option<SecretString>> {
            self.gets.fetch_add(1, Ordering::SeqCst);
            Err(Error::new(ErrorKind::Io, "System keyring unavailable"))
        }

        fn set(&self, site: &SiteId, kind: SecretKind, value: &SecretString) -> Result<()> {
            self.inner.set(site, kind, value)
        }

        fn delete(&self, site: &SiteId, kind: SecretKind) -> Result<()> {
            self.inner.delete(site, kind)
        }
    }

    /// A vault in a temp dir with `n` SSH sites, each with a stored password.
    pub(crate) fn vault_with_sites(
        dir: &std::path::Path,
        store: Arc<dyn SecretStore>,
        n: usize,
    ) -> (Vault, Vec<Site>) {
        let mut vault = Vault::open(dir, store).unwrap();
        let mut sites = Vec::new();
        for i in 0..n {
            let site = Site::new(format!("s{i}"), "h", "u", Protocol::Ssh);
            vault.upsert_site(site.clone()).unwrap();
            vault
                .set_secret(
                    &site.id,
                    SecretKind::Password,
                    &SecretString::from(format!("pw{i}")),
                )
                .unwrap();
            sites.push(vault.site(&site.id).unwrap().clone());
        }
        (vault, sites)
    }

    #[test]
    fn secret_batch_stops_after_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(LockedStore::default());
        let (vault, sites) = vault_with_sites(dir.path(), store.clone(), 3);

        let mut batch = SecretBatch::new(&vault);
        let read: Vec<SiteSecrets> = sites.iter().map(|s| batch.read(s)).collect();

        assert_eq!(
            store.gets.load(Ordering::SeqCst),
            1,
            "one keyring call only"
        );
        for secrets in &read {
            assert!(secrets.keyring_unavailable, "{secrets:?}");
            assert!(secrets.password.is_none());
        }
    }

    #[test]
    fn secret_batch_reads_only_flagged_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let (mut vault, sites) = vault_with_sites(dir.path(), Arc::new(MemoryStore::new()), 1);
        let mut plain = Site::new("no secrets", "h", "u", Protocol::Ftp);
        vault.upsert_site(plain.clone()).unwrap();
        plain = vault.site(&plain.id).unwrap().clone();

        let mut batch = SecretBatch::new(&vault);
        let first = batch.read(&sites[0]);
        assert_eq!(
            first.password.as_ref().map(|p| p.expose_secret()),
            Some("pw0")
        );
        assert!(first.key_passphrase.is_none());
        assert!(!first.keyring_unavailable);
        let none = batch.read(&plain);
        assert!(none.password.is_none() && !none.keyring_unavailable);
    }

    /// The site form's copy is older than the vault's: the pin the broker wrote meanwhile
    /// and the secret flags survive a save that keeps the endpoint; a new host or port,
    /// or leaving FTPS, drops the pin.
    #[tokio::test]
    async fn save_site_keeps_backend_owned_fields() {
        let dir = tempfile::tempdir().unwrap();
        let (vault, sites) = vault_with_sites(dir.path(), Arc::new(MemoryStore::new()), 1);
        let handle = VaultHandle::new(vault);
        let mut form_copy = sites[0].clone();
        form_copy.protocol = Protocol::FtpsExplicit;
        form_copy.port = 21;
        handle.save_site(form_copy.clone()).await.unwrap();
        handle
            .set_ftps_pin(form_copy.id.clone(), "SHA256:PIN".into())
            .await
            .unwrap();
        let pin = |handle: &VaultHandle, id: SiteId| {
            let handle = handle.clone();
            async move {
                handle
                    .site(id)
                    .await
                    .unwrap()
                    .unwrap()
                    .ftp
                    .pinned_fingerprint
            }
        };

        // Same endpoint: the stale form copy (no pin, no password flag) changes the user
        // only; the pin and the flag stay.
        form_copy.user = "someone".into();
        form_copy.has_password = false;
        form_copy.ftp.pinned_fingerprint = None;
        handle.save_site(form_copy.clone()).await.unwrap();
        let saved = handle.site(form_copy.id.clone()).await.unwrap().unwrap();
        assert_eq!(saved.user, "someone");
        assert_eq!(saved.ftp.pinned_fingerprint.as_deref(), Some("SHA256:PIN"));
        assert!(saved.has_password);
        // Explicit to implicit on the same host and port is still FTPS.
        form_copy.protocol = Protocol::FtpsImplicit;
        handle.save_site(form_copy.clone()).await.unwrap();
        assert_eq!(
            pin(&handle, form_copy.id.clone()).await.as_deref(),
            Some("SHA256:PIN")
        );
        // The UI can't set a pin either.
        form_copy.ftp.pinned_fingerprint = Some("SHA256:FORGED".into());
        handle.save_site(form_copy.clone()).await.unwrap();
        assert_eq!(
            pin(&handle, form_copy.id.clone()).await.as_deref(),
            Some("SHA256:PIN")
        );
        form_copy.ftp.pinned_fingerprint = None;

        // A new host drops the pin; so does a new port; so does leaving FTPS.
        for change in 0..3 {
            handle
                .set_ftps_pin(form_copy.id.clone(), "SHA256:PIN".into())
                .await
                .unwrap();
            let mut edited = form_copy.clone();
            match change {
                0 => edited.host = "new-host".into(),
                1 => edited.port = 2121,
                _ => edited.protocol = Protocol::Ftp,
            }
            handle.save_site(edited.clone()).await.unwrap();
            assert_eq!(
                pin(&handle, edited.id.clone()).await,
                None,
                "change {change}"
            );
            assert!(
                handle
                    .site(edited.id.clone())
                    .await
                    .unwrap()
                    .unwrap()
                    .has_password
            );
            handle.save_site(form_copy.clone()).await.unwrap();
        }
        let saved = handle.site(form_copy.id.clone()).await.unwrap().unwrap();

        // The password itself: set, then forgotten.
        let id = saved.id.clone();
        handle
            .set_password(id.clone(), Some(SecretString::from("new")))
            .await
            .unwrap();
        let stored = handle
            .call({
                let id = id.clone();
                move |v| v.secret(&id, SecretKind::Password)
            })
            .await
            .unwrap();
        assert_eq!(stored.as_ref().map(|p| p.expose_secret()), Some("new"));
        handle.set_password(id.clone(), None).await.unwrap();
        assert!(!handle.site(id).await.unwrap().unwrap().has_password);

        let snapshot = handle.snapshot().await.unwrap();
        assert_eq!(snapshot.sites.len(), 1);
        assert!(snapshot.unreadable.is_empty());
    }

    #[tokio::test]
    async fn handle_runs_vault_calls() {
        let dir = tempfile::tempdir().unwrap();
        let (vault, sites) = vault_with_sites(dir.path(), Arc::new(MemoryStore::new()), 2);
        let handle = VaultHandle::new(vault);
        assert_eq!(handle.sites().await.unwrap().len(), 2);

        let missing = SiteId::new();
        let read = handle
            .sites_with_secrets(vec![sites[1].id.clone(), missing.clone()])
            .await
            .unwrap();
        assert_eq!(read.len(), 2);
        let (site, secrets) = read[0].1.as_ref().unwrap();
        assert_eq!(site.id, sites[1].id);
        assert_eq!(
            secrets.password.as_ref().map(|p| p.expose_secret()),
            Some("pw1")
        );
        assert_eq!(read[1].0, missing);
        assert_eq!(read[1].1.as_ref().unwrap_err().kind, ErrorKind::NotFound);

        // The stale pin is cleared only while it is still the site's pin.
        let id = sites[0].id.clone();
        handle
            .set_ftps_pin(id.clone(), "SHA256:AA".into())
            .await
            .unwrap();
        assert!(
            !handle
                .clear_stale_ftps_pin(id.clone(), "SHA256:BB".into())
                .await
                .unwrap()
        );
        assert!(
            handle
                .clear_stale_ftps_pin(id.clone(), "sha256:aa".into())
                .await
                .unwrap()
        );
        let site = handle.site(id).await.unwrap().unwrap();
        assert!(site.ftp.pinned_fingerprint.is_none());
    }
}
