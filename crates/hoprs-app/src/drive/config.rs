//! The build-time OAuth client and the endpoints Google Drive is reached at.
//!
//! The client id and secret are never in the repository: release builds read them from
//! the environment at build time (`option_env!`); debug builds also read them at run
//! time, where the run-time value wins. Without both, the Drive destination is hidden.
//! (A desktop app's client secret is not confidential in Google's model.)

use std::fmt;

/// Build-time (and, in debug builds, run-time) OAuth client id.
pub const CLIENT_ID_ENV: &str = "HOPRS_GOOGLE_CLIENT_ID";
/// Build-time (and, in debug builds, run-time) OAuth client secret.
pub const CLIENT_SECRET_ENV: &str = "HOPRS_GOOGLE_CLIENT_SECRET";
/// Debug builds only: one `https://` base under which the five endpoints live.
pub const BASE_URL_ENV: &str = "HOPRS_DRIVE_URL";
/// The only scope hoprs asks for: files this app created.
pub const SCOPE: &str = "https://www.googleapis.com/auth/drive.file";
/// The folder the exports go to.
// TODO(task 9): the upload looks the folder up; remove this allow then.
#[allow(dead_code)]
pub const FOLDER: &str = "hoprs";

/// The OAuth client of this build.
#[derive(Clone, PartialEq, Eq)]
// TODO(task 9 or 10): the connect flow reads these fields; remove this allow then.
#[allow(dead_code)]
pub struct OAuthClient {
    pub id: String,
    pub secret: String,
}

/// Shows neither value.
impl fmt::Debug for OAuthClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthClient").finish_non_exhaustive()
    }
}

/// The client of this build, or `None` when the build has none (Drive is then hidden).
// TODO(task 9 or 10): the connect flow calls this; remove this allow then.
#[allow(dead_code)]
pub fn client() -> Option<OAuthClient> {
    choose(
        (
            option_env!("HOPRS_GOOGLE_CLIENT_ID"),
            option_env!("HOPRS_GOOGLE_CLIENT_SECRET"),
        ),
        (
            std::env::var(CLIENT_ID_ENV).ok(),
            std::env::var(CLIENT_SECRET_ENV).ok(),
        ),
        cfg!(debug_assertions),
    )
}

/// A complete pair: both values present and not empty (an unset repository secret
/// arrives as an empty string).
fn pair(id: Option<&str>, secret: Option<&str>) -> Option<OAuthClient> {
    match (id, secret) {
        (Some(id), Some(secret)) if !id.is_empty() && !secret.is_empty() => Some(OAuthClient {
            id: id.to_owned(),
            secret: secret.to_owned(),
        }),
        _ => None,
    }
}

/// The built pair, or in a debug build the run-time pair when it is complete.
fn choose(
    built: (Option<&str>, Option<&str>),
    run_time: (Option<String>, Option<String>),
    debug: bool,
) -> Option<OAuthClient> {
    if debug && let Some(client) = pair(run_time.0.as_deref(), run_time.1.as_deref()) {
        return Some(client);
    }
    pair(built.0, built.1)
}

/// Where each Google call goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    pub auth: String,
    pub token: String,
    pub revoke: String,
    pub api: String,
    pub upload: String,
}

/// Google's own endpoints.
pub fn google() -> Endpoints {
    Endpoints {
        auth: "https://accounts.google.com/o/oauth2/v2/auth".to_owned(),
        token: "https://oauth2.googleapis.com/token".to_owned(),
        revoke: "https://oauth2.googleapis.com/revoke".to_owned(),
        api: "https://www.googleapis.com/drive/v3".to_owned(),
        upload: "https://www.googleapis.com/upload/drive/v3".to_owned(),
    }
}

/// The five endpoints under `base`; `None` unless it starts with `https://`.
pub fn under(base: &str) -> Option<Endpoints> {
    if !base.starts_with("https://") {
        return None;
    }
    let base = base.trim_end_matches('/');
    Some(Endpoints {
        auth: format!("{base}/auth"),
        token: format!("{base}/token"),
        revoke: format!("{base}/revoke"),
        api: format!("{base}/drive/v3"),
        upload: format!("{base}/upload/drive/v3"),
    })
}

/// The endpoints in use: Google's, or in a debug build those under `HOPRS_DRIVE_URL`.
// TODO(task 9 or 10): the OAuth and upload calls use this; remove this allow then.
#[allow(dead_code)]
pub fn current() -> Endpoints {
    if cfg!(debug_assertions)
        && let Ok(base) = std::env::var(BASE_URL_ENV)
        && let Some(endpoints) = under(&base)
    {
        return endpoints;
    }
    google()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_needs_both_values() {
        let some = |id: &str, s: &str| choose((Some(id), Some(s)), (None, None), false);
        let c = some("id", "s").unwrap();
        assert_eq!((c.id.as_str(), c.secret.as_str()), ("id", "s"));
        assert!(some("", "s").is_none());
        assert!(some("id", "").is_none());
        assert!(choose((Some("id"), None), (None, None), false).is_none());
        assert!(choose((None, Some("s")), (None, None), false).is_none());
        assert!(choose((None, None), (None, None), false).is_none());
        let shown = format!("{:?}", some("id-value", "secret-value"));
        assert!(!shown.contains("id-value") && !shown.contains("secret-value"));
    }

    #[test]
    fn run_time_values_only_in_debug() {
        let run = || (Some("rid".to_owned()), Some("rs".to_owned()));
        let release = choose((Some("id"), Some("s")), run(), false).unwrap();
        assert_eq!(release.id, "id");
        let debug = choose((Some("id"), Some("s")), run(), true).unwrap();
        assert_eq!((debug.id.as_str(), debug.secret.as_str()), ("rid", "rs"));
        assert!(choose((None, None), run(), false).is_none());
        assert!(choose((None, None), run(), true).is_some());
        // An incomplete run-time pair does not shadow a complete built pair.
        let partial = choose(
            (Some("id"), Some("s")),
            (Some("rid".to_owned()), None),
            true,
        );
        assert_eq!(partial.unwrap().id, "id");
    }

    #[test]
    fn endpoints_under_a_base() {
        let e = under("https://localhost:8443").unwrap();
        assert_eq!(e.auth, "https://localhost:8443/auth");
        assert_eq!(e.token, "https://localhost:8443/token");
        assert_eq!(e.revoke, "https://localhost:8443/revoke");
        assert_eq!(e.api, "https://localhost:8443/drive/v3");
        assert_eq!(e.upload, "https://localhost:8443/upload/drive/v3");
        assert_eq!(under("https://localhost:8443/").unwrap(), e);
        assert!(under("http://localhost").is_none());
        assert!(under("localhost").is_none());
        assert_eq!(google().token, "https://oauth2.googleapis.com/token");
        assert_eq!(
            google().upload,
            "https://www.googleapis.com/upload/drive/v3"
        );
    }

    #[test]
    fn only_the_drive_file_scope() {
        assert!(SCOPE.ends_with("/auth/drive.file"));
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("drive");
        let forbidden = [
            "auth/drive\"",
            "drive.readonly",
            "drive.metadata",
            "drive.appdata",
        ];
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let code = text.split("#[cfg(test)]").next().unwrap();
            for word in forbidden {
                assert!(!code.contains(word), "{} has {word}", path.display());
            }
        }
    }
}
