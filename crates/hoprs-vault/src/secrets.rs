//! Site secrets (password, key passphrase) in a credential store.
//!
//! Each secret lives under the service `hoprs/<site-id>` with the account
//! `password` or `key-passphrase`. Values only travel as [`SecretString`].

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use hoprs_core::{Error, ErrorKind, Result, SiteId};
use keyring_core::CredentialStore;
use secrecy::{ExposeSecret, SecretString};

/// Which secret of a site.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SecretKind {
    Password,
    KeyPassphrase,
}

impl SecretKind {
    /// Account name of this secret in the credential store.
    pub fn account(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::KeyPassphrase => "key-passphrase",
        }
    }
}

/// A secret that belongs to the app, not to a site.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AppSecret {
    /// The Google Drive connection (its refresh token).
    GoogleDrive,
}

impl AppSecret {
    /// Credential-store service name of this secret.
    pub fn service(self) -> &'static str {
        match self {
            Self::GoogleDrive => "hoprs/google-drive",
        }
    }

    /// Account name of this secret in the credential store.
    pub fn account(self) -> &'static str {
        match self {
            Self::GoogleDrive => "refresh-token",
        }
    }
}

/// Credential-store service name of a site's secrets.
pub(crate) fn service(site: &SiteId) -> String {
    format!("hoprs/{site}")
}

/// Where site secrets are kept.
pub trait SecretStore: Send + Sync {
    /// The secret, or `None` if there is none.
    fn get(&self, site: &SiteId, kind: SecretKind) -> Result<Option<SecretString>>;
    /// Stores `value`, replacing any previous one.
    fn set(&self, site: &SiteId, kind: SecretKind, value: &SecretString) -> Result<()>;
    /// Deletes the secret. Deleting a missing secret is `Ok`.
    fn delete(&self, site: &SiteId, kind: SecretKind) -> Result<()>;
}

/// Where the app's own secrets are kept (the [`SecretStore`] is keyed by site).
pub trait AppSecretStore: Send + Sync {
    /// The secret, or `None` if there is none.
    fn get_app(&self, which: AppSecret) -> Result<Option<SecretString>>;
    /// Stores `value`, replacing any previous one.
    fn set_app(&self, which: AppSecret, value: &SecretString) -> Result<()>;
    /// Deletes the secret. Deleting a missing secret is `Ok`.
    fn delete_app(&self, which: AppSecret) -> Result<()>;
}

/// Secrets in the system keyring (Secret Service on Linux, Keychain on
/// macOS, Credential Manager on Windows), through the `keyring` crate.
pub struct KeyringStore {
    store: Arc<CredentialStore>,
}

impl KeyringStore {
    /// Connects to the platform keyring.
    ///
    /// Fails with `Io` "System keyring unavailable" when there is no usable
    /// backend (e.g. no Secret Service or no D-Bus session).
    ///
    /// On Linux and the other Secret Service platforms this opens a new
    /// D-Bus connection on every call, so calling it again after a failure
    /// (e.g. once the user has unlocked or started their keyring) retries
    /// for real. It does not touch keyring-core's process-wide default store.
    ///
    /// On macOS and Windows it goes through `keyring` v4, which initializes
    /// the platform store once per process (a `LazyLock`) and caches the
    /// outcome: a failure there is final until the app restarts.
    ///
    /// Blocking: connecting does D-Bus round trips (and so does every call
    /// on the store); call it from `spawn_blocking`, not on the async
    /// runtime or the UI thread.
    pub fn new() -> Result<Self> {
        Self::platform_store().map(|store| Self { store })
    }

    #[cfg(all(
        unix,
        not(any(target_os = "macos", target_os = "ios", target_os = "android"))
    ))]
    fn platform_store() -> Result<Arc<CredentialStore>> {
        match zbus_secret_service_keyring_store::Store::new() {
            Ok(store) => Ok(store),
            Err(err) => Err(unavailable(&err)),
        }
    }

    #[cfg(not(all(
        unix,
        not(any(target_os = "macos", target_os = "ios", target_os = "android"))
    )))]
    fn platform_store() -> Result<Arc<CredentialStore>> {
        if let Err(err) = keyring::Entry::store_status() {
            return Err(unavailable(err));
        }
        keyring_core::get_default_store()
            .ok_or_else(|| unavailable(&keyring_core::Error::NoDefaultStore))
    }

    /// Uses `store` instead of the platform keyring (e.g. keyring-core's
    /// mock store in tests).
    pub fn with_store(store: Arc<CredentialStore>) -> Self {
        Self { store }
    }

    fn entry(&self, site: &SiteId, kind: SecretKind) -> Result<keyring_core::Entry> {
        self.store
            .build(&service(site), kind.account(), None)
            .map_err(|e| map_error(&e))
    }

    fn app_entry(&self, which: AppSecret) -> Result<keyring_core::Entry> {
        self.store
            .build(which.service(), which.account(), None)
            .map_err(|e| map_error(&e))
    }
}

impl fmt::Debug for KeyringStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyringStore")
            .field("vendor", &self.store.vendor())
            .finish()
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, site: &SiteId, kind: SecretKind) -> Result<Option<SecretString>> {
        match self.entry(site, kind)?.get_password() {
            Ok(value) => Ok(Some(SecretString::from(value))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(err) => Err(map_error(&err)),
        }
    }

    fn set(&self, site: &SiteId, kind: SecretKind, value: &SecretString) -> Result<()> {
        self.entry(site, kind)?
            .set_password(value.expose_secret())
            .map_err(|e| map_error(&e))
    }

    fn delete(&self, site: &SiteId, kind: SecretKind) -> Result<()> {
        match self.entry(site, kind)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(err) => Err(map_error(&err)),
        }
    }
}

impl AppSecretStore for KeyringStore {
    fn get_app(&self, which: AppSecret) -> Result<Option<SecretString>> {
        match self.app_entry(which)?.get_password() {
            Ok(value) => Ok(Some(SecretString::from(value))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(err) => Err(map_error(&err)),
        }
    }

    fn set_app(&self, which: AppSecret, value: &SecretString) -> Result<()> {
        self.app_entry(which)?
            .set_password(value.expose_secret())
            .map_err(|e| map_error(&e))
    }

    fn delete_app(&self, which: AppSecret) -> Result<()> {
        match self.app_entry(which)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(err) => Err(map_error(&err)),
        }
    }
}

/// Message of the `Io` error that means the keyring backend cannot be reached.
const UNAVAILABLE_MESSAGE: &str = "System keyring unavailable";

/// True for the error a [`SecretStore`] returns when the keyring backend is
/// unreachable (no session bus, locked and not unlocked, ...). Batch callers
/// stop issuing secret operations after the first one, so a locked keyring
/// raises one unlock prompt, not one per secret.
pub fn is_keyring_unavailable(err: &Error) -> bool {
    err.kind == ErrorKind::Io && err.message == UNAVAILABLE_MESSAGE
}

/// The keyring backend cannot be reached at all.
fn unavailable(err: &keyring_core::Error) -> Error {
    Error::coded(
        ErrorKind::Io,
        "vault-keyring-unavailable",
        "System keyring unavailable",
    )
    .with_detail(err.to_string())
}

/// Maps a keyring error to `Io`. The detail never carries secret bytes:
/// variants that hold them (or entries that may) get a fixed description.
fn map_error(err: &keyring_core::Error) -> Error {
    use keyring_core::Error as K;
    let detail = match err {
        K::PlatformFailure(_) | K::NoStorageAccess(_) | K::NoDefaultStore => {
            return unavailable(err);
        }
        K::BadEncoding(_) => "stored secret is not valid UTF-8".to_owned(),
        K::BadDataFormat(..) => "stored secret is malformed".to_owned(),
        K::Ambiguous(entries) => format!("{} matching keyring entries", entries.len()),
        other => other.to_string(),
    };
    Error::coded(ErrorKind::Io, "vault-keyring-error", "System keyring error").with_detail(detail)
}

/// Secrets kept in memory only: for tests and dry-run imports.
#[derive(Default)]
pub struct MemoryStore {
    entries: Mutex<HashMap<(SiteId, SecretKind), SecretString>>,
    app: Mutex<HashMap<AppSecret, SecretString>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<(SiteId, SecretKind), SecretString>> {
        // A panic while holding the lock cannot leave the map half-updated.
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_app(&self) -> std::sync::MutexGuard<'_, HashMap<AppSecret, SecretString>> {
        self.app.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl AppSecretStore for MemoryStore {
    fn get_app(&self, which: AppSecret) -> Result<Option<SecretString>> {
        Ok(self.lock_app().get(&which).cloned())
    }

    fn set_app(&self, which: AppSecret, value: &SecretString) -> Result<()> {
        self.lock_app().insert(which, value.clone());
        Ok(())
    }

    fn delete_app(&self, which: AppSecret) -> Result<()> {
        self.lock_app().remove(&which);
        Ok(())
    }
}

/// Prints the number of entries, never their values.
impl fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryStore")
            .field("entries", &(self.lock().len() + self.lock_app().len()))
            .finish()
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, site: &SiteId, kind: SecretKind) -> Result<Option<SecretString>> {
        Ok(self.lock().get(&(site.clone(), kind)).cloned())
    }

    fn set(&self, site: &SiteId, kind: SecretKind, value: &SecretString) -> Result<()> {
        self.lock().insert((site.clone(), kind), value.clone());
        Ok(())
    }

    fn delete(&self, site: &SiteId, kind: SecretKind) -> Result<()> {
        self.lock().remove(&(site.clone(), kind));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use keyring_core::mock;

    use super::*;

    fn secret(s: &str) -> SecretString {
        SecretString::from(s)
    }

    fn exposed(s: Option<SecretString>) -> Option<String> {
        s.map(|s| s.expose_secret().to_owned())
    }

    #[test]
    fn accounts_and_service() {
        assert_eq!(SecretKind::Password.account(), "password");
        assert_eq!(SecretKind::KeyPassphrase.account(), "key-passphrase");
        assert_eq!(service(&SiteId("abc".into())), "hoprs/abc");
    }

    #[test]
    fn memory_store_roundtrip() {
        let store = MemoryStore::new();
        let site = SiteId::new();
        assert_eq!(
            exposed(store.get(&site, SecretKind::Password).unwrap()),
            None
        );
        store.delete(&site, SecretKind::Password).unwrap();
        store
            .set(&site, SecretKind::Password, &secret("hunter2"))
            .unwrap();
        store
            .set(&site, SecretKind::KeyPassphrase, &secret("phrase"))
            .unwrap();
        assert_eq!(
            exposed(store.get(&site, SecretKind::Password).unwrap()).as_deref(),
            Some("hunter2")
        );
        store.delete(&site, SecretKind::Password).unwrap();
        assert_eq!(
            exposed(store.get(&site, SecretKind::Password).unwrap()),
            None
        );
        assert_eq!(
            exposed(store.get(&site, SecretKind::KeyPassphrase).unwrap()).as_deref(),
            Some("phrase")
        );
        let debug = format!("{store:?}");
        assert!(!debug.contains("phrase"), "{debug}");
    }

    #[test]
    fn keyring_store_uses_service_and_account() {
        let mock: Arc<CredentialStore> = mock::Store::new().unwrap();
        let store = KeyringStore::with_store(mock.clone());
        let site = SiteId::new();
        assert_eq!(
            exposed(store.get(&site, SecretKind::Password).unwrap()),
            None
        );
        store.delete(&site, SecretKind::Password).unwrap();

        store
            .set(&site, SecretKind::Password, &secret("hunter2"))
            .unwrap();
        let entry = mock
            .build(&format!("hoprs/{site}"), "password", None)
            .unwrap();
        assert_eq!(entry.get_password().unwrap(), "hunter2");
        assert_eq!(
            exposed(store.get(&site, SecretKind::Password).unwrap()).as_deref(),
            Some("hunter2")
        );
        assert_eq!(
            exposed(store.get(&site, SecretKind::KeyPassphrase).unwrap()),
            None
        );

        store.delete(&site, SecretKind::Password).unwrap();
        assert!(matches!(
            entry.get_password(),
            Err(keyring_core::Error::NoEntry)
        ));
        assert_eq!(
            exposed(store.get(&site, SecretKind::Password).unwrap()),
            None
        );
        assert!(!format!("{store:?}").contains("hunter2"));
    }

    #[test]
    fn keyring_backend_failure_is_unavailable() {
        let mock: Arc<CredentialStore> = mock::Store::new().unwrap();
        let store = KeyringStore::with_store(mock.clone());
        let site = SiteId::new();
        let entry = mock
            .build(&format!("hoprs/{site}"), "password", None)
            .unwrap();
        let cred: &mock::Cred = entry.as_any().downcast_ref().unwrap();

        let failures = [
            keyring_core::Error::PlatformFailure("no D-Bus session".into()),
            keyring_core::Error::NoStorageAccess("collection locked".into()),
            keyring_core::Error::NoDefaultStore,
        ];
        for failure in failures {
            let shown = failure.to_string();
            cred.set_error(failure);
            let err = store
                .set(&site, SecretKind::Password, &secret("hunter2"))
                .unwrap_err();
            assert_eq!(err.kind, ErrorKind::Io);
            assert_eq!(err.message, "System keyring unavailable");
            assert_eq!(err.detail.as_deref(), Some(shown.as_str()));
        }
        assert_eq!(
            exposed(store.get(&site, SecretKind::Password).unwrap()),
            None
        );

        cred.set_error(keyring_core::Error::PlatformFailure("down".into()));
        let err = store.get(&site, SecretKind::Password).unwrap_err();
        assert_eq!(err.message, "System keyring unavailable");
        cred.set_error(keyring_core::Error::PlatformFailure("down".into()));
        let err = store.delete(&site, SecretKind::Password).unwrap_err();
        assert_eq!(err.message, "System keyring unavailable");

        // Other store errors are Io too, with a detail that holds no secret.
        cred.set_error(keyring_core::Error::BadEncoding(b"hunter2\xff".to_vec()));
        let err = store.get(&site, SecretKind::Password).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Io);
        assert!(!format!("{err:?}").contains("hunter2"), "{err:?}");
    }

    #[test]
    fn app_secret_names() {
        assert_eq!(AppSecret::GoogleDrive.service(), "hoprs/google-drive");
        assert_eq!(AppSecret::GoogleDrive.account(), "refresh-token");
    }

    #[test]
    fn memory_store_app_roundtrip() {
        let store = MemoryStore::new();
        let which = AppSecret::GoogleDrive;
        assert_eq!(exposed(store.get_app(which).unwrap()), None);
        store.delete_app(which).unwrap();
        let site = SiteId("google-drive".into());
        store
            .set(&site, SecretKind::Password, &secret("site-pw"))
            .unwrap();
        store.set_app(which, &secret("token")).unwrap();
        assert_eq!(
            exposed(store.get_app(which).unwrap()).as_deref(),
            Some("token")
        );
        assert_eq!(
            exposed(store.get(&site, SecretKind::Password).unwrap()).as_deref(),
            Some("site-pw")
        );
        store.delete_app(which).unwrap();
        assert_eq!(exposed(store.get_app(which).unwrap()), None);
        assert_eq!(
            exposed(store.get(&site, SecretKind::Password).unwrap()).as_deref(),
            Some("site-pw")
        );
        assert!(!format!("{store:?}").contains("token"));
    }

    #[test]
    fn keyring_store_app_entry_uses_service_and_account() {
        let mock: Arc<CredentialStore> = mock::Store::new().unwrap();
        let store = KeyringStore::with_store(mock.clone());
        let which = AppSecret::GoogleDrive;
        assert_eq!(exposed(store.get_app(which).unwrap()), None);
        store.delete_app(which).unwrap();

        store.set_app(which, &secret("token-value")).unwrap();
        let entry = mock
            .build("hoprs/google-drive", "refresh-token", None)
            .unwrap();
        assert_eq!(entry.get_password().unwrap(), "token-value");
        assert_eq!(
            exposed(store.get_app(which).unwrap()).as_deref(),
            Some("token-value")
        );
        assert!(!format!("{store:?}").contains("token-value"));

        let cred: &mock::Cred = entry.as_any().downcast_ref().unwrap();
        cred.set_error(keyring_core::Error::PlatformFailure("down".into()));
        let err = store.get_app(which).unwrap_err();
        assert!(is_keyring_unavailable(&err), "{err:?}");
        assert!(!format!("{err:?}").contains("token-value"));

        store.delete_app(which).unwrap();
        assert!(matches!(
            entry.get_password(),
            Err(keyring_core::Error::NoEntry)
        ));
    }
}
