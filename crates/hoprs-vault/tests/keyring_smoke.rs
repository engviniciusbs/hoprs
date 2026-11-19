//! Smoke test of the platform keyring the vault uses on Windows (Credential
//! Manager) and macOS (Keychain): store, read back, delete. Linux CI runners
//! have no Secret Service, so on Linux this file compiles to nothing.

#![cfg(any(windows, target_os = "macos"))]

use std::time::{SystemTime, UNIX_EPOCH};

const SERVICE: &str = "hoprs-ci-smoke";

/// An account name no other run (or a leftover from a crashed one) shares.
fn random_account() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after 1970")
        .as_nanos();
    format!("smoke-{}-{nanos}", std::process::id())
}

/// Removes the entry when dropped, so a failing assertion cannot leave it in the
/// developer's real keyring. "Already gone" is fine; other errors must not panic here
/// (this also runs while unwinding), so they are only reported.
struct EntryGuard<'a>(&'a keyring::Entry);

impl Drop for EntryGuard<'_> {
    fn drop(&mut self) {
        match self.0.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(error) => eprintln!("could not remove the smoke-test keyring entry: {error}"),
        }
    }
}

#[test]
fn store_read_delete_round_trip() {
    let account = random_account();
    let entry = keyring::Entry::new(SERVICE, &account).expect("build entry");

    entry.set_password("s3cret value").expect("store");
    let _cleanup = EntryGuard(&entry);
    assert_eq!(entry.get_password().expect("read back"), "s3cret value");

    entry.delete_credential().expect("delete");
    match entry.get_password() {
        Err(keyring::Error::NoEntry) => {}
        other => panic!("expected NoEntry after delete, got {other:?}"),
    }
}
