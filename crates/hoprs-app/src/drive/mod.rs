//! Google Drive as a second destination for the encrypted export file.
//!
//! [`Drive`] holds the connection: it signs in through the browser ([`Drive::connect`]),
//! keeps the refresh token in the system keyring (one entry, see ruling 10) and in memory,
//! uploads sealed files into the `hoprs` folder and disconnects. Every method blocks
//! (sockets, HTTPS, keyring): callers run them on the blocking pool.
//!
//! The keyring is only touched by [`Drive::status`] (once, while nothing is known),
//! [`Drive::connect`], [`Drive::upload`] (when nothing is known, or the grant is gone) and
//! [`Drive::disconnect`]; building a `Drive` touches nothing.

mod client;
mod config;
mod loopback;
mod oauth;

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use hoprs_core::{Error, ErrorKind};
use hoprs_vault::{AppSecret, AppSecretStore, is_keyring_unavailable};
use secrecy::{ExposeSecret, SecretString};

pub use client::Client;
use client::not_connected;
pub use loopback::Pages;

/// A file in the user's Google Drive, as the upload reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DriveFile {
    pub name: String,
    /// The file's page in Google Drive, only when it is an `https://` link on `google.com`.
    pub link: Option<String>,
}

/// The connected account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DriveAccount {
    /// The account's address, when Google told it.
    pub email: Option<String>,
    /// `false`: the keyring refused the entry, the connection lasts for this session only.
    pub stored: bool,
}

/// What [`Drive::connect`] reports: `Waiting` once the browser can be sent to `url`, then
/// exactly one `Done`, last.
#[derive(Clone, Debug)]
// TODO(task 11): the export flow reads the events; remove this allow then.
#[allow(dead_code)]
pub enum ConnectEvent {
    /// The authorisation URL (it holds the `state` and the PKCE challenge, which are not
    /// secret on their own; never log it).
    Waiting {
        url: String,
    },
    Done(Result<DriveAccount, Error>),
}

/// A sealed (encrypted) export file in memory, shared without copies.
#[derive(Clone)]
pub struct Sealed(Arc<Vec<u8>>);

impl Sealed {
    pub fn new(bytes: Vec<u8>) -> Sealed {
        Sealed(Arc::new(bytes))
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Only the length.
impl fmt::Debug for Sealed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sealed({} bytes)", self.0.len())
    }
}

/// How long before its end an access token is no longer used.
const TOKEN_MARGIN: Duration = Duration::from_secs(60);

fn not_configured() -> Error {
    Error::coded(
        ErrorKind::NotFound,
        "drive-not-configured",
        "Google Drive is not available in this build",
    )
}

/// An access token and when it stops being used.
struct Access {
    token: SecretString,
    until: Instant,
}

impl Access {
    fn new(token: SecretString, expires_in: u64) -> Access {
        let life = Duration::from_secs(expires_in).saturating_sub(TOKEN_MARGIN);
        Access {
            token,
            until: Instant::now() + life,
        }
    }

    fn fresh(&self) -> bool {
        Instant::now() < self.until
    }
}

/// A connection in memory.
struct Link {
    refresh: SecretString,
    email: Option<String>,
    stored: bool,
    access: Option<Access>,
}

impl Link {
    fn account(&self) -> DriveAccount {
        DriveAccount {
            email: self.email.clone(),
            stored: self.stored,
        }
    }
}

/// What the holder knows of the connection.
enum Known {
    /// The keyring was not read yet.
    Unread,
    /// Not connected.
    None,
    Linked(Link),
}

/// The keyring entry's value (ruling 10): `{"refresh_token":"…","email":"…"}`.
fn entry_value(refresh: &SecretString, email: Option<&str>) -> SecretString {
    let mut value = serde_json::Map::new();
    value.insert(
        "refresh_token".into(),
        serde_json::Value::String(refresh.expose_secret().to_owned()),
    );
    if let Some(email) = email {
        value.insert("email".into(), serde_json::Value::String(email.to_owned()));
    }
    SecretString::from(serde_json::Value::Object(value).to_string())
}

/// The refresh token and email of an entry; `None` when it is not that JSON.
fn parse_entry(value: &SecretString) -> Option<(SecretString, Option<String>)> {
    let serde_json::Value::Object(object) =
        serde_json::from_str::<serde_json::Value>(value.expose_secret()).ok()?
    else {
        return None;
    };
    let refresh = object
        .get("refresh_token")?
        .as_str()
        .filter(|token| !token.is_empty())?;
    let email = match object.get("email") {
        None => None,
        Some(email) => Some(email.as_str()?.to_owned()),
    };
    Some((SecretString::from(refresh.to_owned()), email))
}

/// Whether this build (or, in a debug build, its environment) has an OAuth client.
#[cfg(test)]
pub fn client_configured() -> bool {
    config::client().is_some()
}

/// The connection holder (see the module docs).
pub struct Drive {
    /// Built on first use (reading the system roots is not free); `None`: no OAuth client
    /// in this build.
    client: OnceLock<Option<Client>>,
    store: Arc<dyn AppSecretStore>,
    known: Mutex<Known>,
    /// The cancel flag of the connect that waits for the browser, if one does.
    attempt: Mutex<Option<Arc<AtomicBool>>>,
}

impl Drive {
    /// The client comes from [`config::client`] and [`config::Endpoints`] on first use.
    pub fn new(store: Arc<dyn AppSecretStore>) -> Drive {
        Drive {
            client: OnceLock::new(),
            store,
            known: Mutex::new(Known::Unread),
            attempt: Mutex::new(None),
        }
    }

    /// A holder over `client` (`None`: no OAuth client), for the tests.
    #[cfg(test)]
    pub fn with_client(client: Option<Client>, store: Arc<dyn AppSecretStore>) -> Drive {
        let drive = Drive::new(store);
        let _ = drive.client.set(client);
        drive
    }

    fn client(&self) -> Result<&Client, Error> {
        self.client
            .get_or_init(|| config::client().map(|oauth| Client::system(config::current(), oauth)))
            .as_ref()
            .ok_or_else(not_configured)
    }

    fn known(&self) -> MutexGuard<'_, Known> {
        self.known.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The stored connection: one keyring read. A value that is not an entry is deleted.
    /// An unreachable keyring reads as "not connected".
    fn read_entry(&self) -> Result<Option<Link>, Error> {
        let value = match self.store.get_app(AppSecret::GoogleDrive) {
            Ok(value) => value,
            Err(err) if is_keyring_unavailable(&err) => return Ok(None),
            Err(err) => return Err(err),
        };
        let Some(value) = value else {
            return Ok(None);
        };
        match parse_entry(&value) {
            Some((refresh, email)) => Ok(Some(Link {
                refresh,
                email,
                stored: true,
                access: None,
            })),
            None => {
                tracing::warn!("Google Drive: the keyring entry is unusable; deleting it");
                if let Err(err) = self.store.delete_app(AppSecret::GoogleDrive) {
                    tracing::warn!(error = %err, "Google Drive: cannot delete the keyring entry");
                }
                Ok(None)
            }
        }
    }

    /// Fills `known` from the keyring if it was not read yet.
    fn load(&self, known: &mut Known) -> Result<(), Error> {
        if matches!(known, Known::Unread) {
            *known = match self.read_entry()? {
                Some(link) => Known::Linked(link),
                None => Known::None,
            };
        }
        Ok(())
    }

    /// The account, from memory, else from the keyring (once).
    pub fn status(&self) -> Result<Option<DriveAccount>, Error> {
        self.client()?;
        let mut known = self.known();
        self.load(&mut known)?;
        Ok(match &*known {
            Known::Linked(link) => Some(link.account()),
            Known::Unread | Known::None => None,
        })
    }

    /// Signs in through the browser: sends `Waiting { url }`, waits for the browser (up
    /// to [`loopback::CONNECT_TIMEOUT`], or [`Drive::cancel`], or another `connect`),
    /// trades the code for tokens, reads the email, stores the entry, then sends exactly
    /// one `Done`, last.
    pub fn connect(&self, pages: &Pages, send: &mut dyn FnMut(ConnectEvent)) {
        let result = self
            .client()
            .and_then(|client| self.sign_in(client, pages, send));
        send(ConnectEvent::Done(result));
    }

    fn sign_in(
        &self,
        client: &Client,
        pages: &Pages,
        send: &mut dyn FnMut(ConnectEvent),
    ) -> Result<DriveAccount, Error> {
        let flag = Arc::new(AtomicBool::new(false));
        {
            let mut attempt = self.attempt.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(previous) = attempt.replace(flag.clone()) {
                previous.store(true, Ordering::Relaxed);
            }
        }
        let code = self.wait_for_code(client, &flag, pages, send);
        {
            let mut attempt = self.attempt.lock().unwrap_or_else(PoisonError::into_inner);
            if attempt.as_ref().is_some_and(|a| Arc::ptr_eq(a, &flag)) {
                *attempt = None;
            }
        }
        let (code, verifier, redirect_uri) = code?;
        let tokens = client.exchange(code.expose_secret(), &verifier, &redirect_uri)?;
        let refresh = tokens
            .refresh
            .ok_or_else(|| oauth::connect_failed("no refresh token in the answer"))?;
        let email = client.email(&tokens.access);
        let stored = match self.store.set_app(
            AppSecret::GoogleDrive,
            &entry_value(&refresh, email.as_deref()),
        ) {
            Ok(()) => true,
            Err(err) => {
                tracing::warn!(error = %err, "Google Drive: cannot store the connection; it lasts for this session");
                false
            }
        };
        let link = Link {
            refresh,
            email,
            stored,
            access: Some(Access::new(tokens.access, tokens.expires_in)),
        };
        let account = link.account();
        *self.known() = Known::Linked(link);
        tracing::info!("Google Drive: connected");
        Ok(account)
    }

    /// Binds the listener, sends `Waiting` and waits: the code, the PKCE verifier and the
    /// redirect URI.
    fn wait_for_code(
        &self,
        client: &Client,
        cancel: &AtomicBool,
        pages: &Pages,
        send: &mut dyn FnMut(ConnectEvent),
    ) -> Result<(SecretString, String, String), Error> {
        let listener = loopback::Listener::bind()?;
        let redirect_uri = listener.redirect_uri();
        let verifier = oauth::verifier(oauth::random()?);
        let state = oauth::state_token(oauth::random()?);
        let url = client.auth_url(&redirect_uri, &oauth::challenge(&verifier), &state);
        send(ConnectEvent::Waiting { url });
        let code = listener.wait(&state, cancel, loopback::CONNECT_TIMEOUT, pages)?;
        Ok((code, verifier, redirect_uri))
    }

    /// Ends the wait of a running [`Drive::connect`] (its `Done` is
    /// `drive-connect-cancelled`); nothing when none waits.
    pub fn cancel(&self) {
        let attempt = self
            .attempt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(flag) = attempt {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// Revokes the token at Google (a failure is only logged), deletes the keyring entry
    /// (its error is returned) and forgets the connection.
    pub fn disconnect(&self) -> Result<(), Error> {
        let client = self.client()?;
        let held = std::mem::replace(&mut *self.known(), Known::None);
        let link = match held {
            Known::Linked(link) => Some(link),
            Known::Unread | Known::None => match self.read_entry() {
                Ok(link) => link,
                Err(err) => {
                    tracing::warn!(error = %err, "Google Drive: cannot read the keyring entry");
                    None
                }
            },
        };
        let session_only = link.as_ref().is_some_and(|link| !link.stored);
        if let Some(link) = &link
            && let Err(err) = client.revoke(&link.refresh)
        {
            tracing::warn!(error = %err, "Google Drive: revoking the token failed");
        }
        match self.store.delete_app(AppSecret::GoogleDrive) {
            Ok(()) => {}
            // Nothing was stored, and the keyring is still out of reach.
            Err(err) if session_only && is_keyring_unavailable(&err) => {}
            Err(err) => return Err(err),
        }
        tracing::info!("Google Drive: disconnected");
        Ok(())
    }

    /// An access token: the one in memory while it has more than 60 s left, else a new one
    /// (`force`: always a new one). A refused refresh drops the connection.
    fn access(&self, client: &Client, force: bool) -> Result<SecretString, Error> {
        let refresh = {
            let mut known = self.known();
            self.load(&mut known)?;
            let Known::Linked(link) = &*known else {
                return Err(not_connected());
            };
            if !force
                && let Some(access) = &link.access
                && access.fresh()
            {
                return Ok(access.token.clone());
            }
            link.refresh.clone()
        };
        match client.refresh(&refresh) {
            Ok(tokens) => {
                let mut known = self.known();
                if let Known::Linked(link) = &mut *known
                    && link.refresh.expose_secret() == refresh.expose_secret()
                {
                    link.access = Some(Access::new(tokens.access.clone(), tokens.expires_in));
                }
                Ok(tokens.access)
            }
            Err(err) if err.code == Some("drive-not-connected") => {
                tracing::warn!("Google Drive: the connection was revoked or expired");
                *self.known() = Known::None;
                if let Err(err) = self.store.delete_app(AppSecret::GoogleDrive) {
                    tracing::warn!(error = %err, "Google Drive: cannot delete the keyring entry");
                }
                Err(err)
            }
            Err(err) => Err(err),
        }
    }

    /// Uploads `bytes` as `name` into the `hoprs` folder (looked up every time). A 401
    /// from Drive gets one new token and one retry.
    pub fn upload(&self, name: &str, bytes: &[u8]) -> Result<DriveFile, Error> {
        let client = self.client()?;
        let send = |access: &SecretString| {
            let folder = client.folder(access)?;
            client.upload(access, &folder, name, bytes)
        };
        let access = self.access(client, false)?;
        let file = match send(&access) {
            Err(err) if client::unauthorized(&err) => send(&self.access(client, true)?),
            other => other,
        }?;
        tracing::info!("Google Drive: uploaded {}", file.name);
        Ok(file)
    }
}

/// Shows whether it is connected, nothing else.
impl fmt::Debug for Drive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let connected = matches!(&*self.known(), Known::Linked(_));
        f.debug_struct("Drive")
            .field("connected", &connected)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_https::{Answer, Seen, answer, identity, serve};
    use hoprs_core::{ErrorKind, Result as CoreResult};
    use hoprs_vault::{AppSecret, MemoryStore};
    use percent_encoding::percent_decode_str;
    use secrecy::{ExposeSecret, SecretString};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    const CLIENT_SECRET: &str = "CLIENT-SECRET-VALUE";
    const CODE: &str = "CODE-SECRET-VALUE";
    const REFRESH: &str = "REFRESH-SECRET-VALUE";
    const ACCESS: &str = "ACCESS-SECRET-VALUE";
    const SECRETS: [&str; 4] = [CLIENT_SECRET, CODE, REFRESH, ACCESS];
    const EMAIL: &str = "me@example.com";

    /// How the local "Google" answers.
    struct Google {
        /// `expires_in` of every token answer.
        expires_in: u64,
        /// A refresh answers `invalid_grant`.
        refuse_refresh: bool,
        /// `/revoke` answers 500.
        revoke_fails: bool,
        /// The next uploads answer 401.
        upload_401: u32,
    }

    impl Default for Google {
        fn default() -> Self {
            Google {
                expires_in: 3600,
                refuse_refresh: false,
                revoke_fails: false,
                upload_401: 0,
            }
        }
    }

    type Script = Arc<Mutex<Google>>;

    fn form(body: &[u8]) -> Vec<(String, String)> {
        let text = std::str::from_utf8(body).unwrap_or_default();
        text.split('&')
            .filter_map(|pair| {
                let (k, v) = pair.split_once('=')?;
                let d = |s: &str| percent_decode_str(s).decode_utf8().unwrap().into_owned();
                Some((d(k), d(v)))
            })
            .collect()
    }

    fn field(fields: &[(String, String)], name: &str) -> Option<String> {
        fields
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    }

    fn json(body: String) -> Answer {
        answer("200 OK", body)
    }

    fn route(script: &Script, method: &str, path: &str, body: &[u8]) -> Answer {
        let mut google = script.lock().unwrap();
        let path = path.split('?').next().unwrap();
        let expires = google.expires_in;
        match (method, path) {
            ("POST", "/token") => {
                let fields = form(body);
                match field(&fields, "grant_type").as_deref() {
                    Some("authorization_code")
                        if field(&fields, "code").as_deref() == Some(CODE)
                            && field(&fields, "client_secret").as_deref()
                                == Some(CLIENT_SECRET) =>
                    {
                        json(format!(
                            r#"{{"access_token":"{ACCESS}","expires_in":{expires},"refresh_token":"{REFRESH}"}}"#
                        ))
                    }
                    Some("refresh_token") if google.refuse_refresh => {
                        answer("400 Bad Request", r#"{"error":"invalid_grant"}"#)
                    }
                    Some("refresh_token")
                        if field(&fields, "refresh_token").as_deref() == Some(REFRESH) =>
                    {
                        json(format!(
                            r#"{{"access_token":"{ACCESS}","expires_in":{expires}}}"#
                        ))
                    }
                    _ => answer("400 Bad Request", r#"{"error":"invalid_request"}"#),
                }
            }
            ("GET", "/drive/v3/about") => json(format!(r#"{{"user":{{"emailAddress":"{EMAIL}"}}}}"#)),
            ("GET", "/drive/v3/files") => json(r#"{"files":[{"id":"F1"}]}"#.to_owned()),
            ("POST", "/upload/drive/v3/files") if google.upload_401 > 0 => {
                google.upload_401 -= 1;
                answer("401 Unauthorized", r#"{"error":{"code":401}}"#)
            }
            ("POST", "/upload/drive/v3/files") => json(
                r#"{"id":"X","name":"backup.hoprs","webViewLink":"https://drive.google.com/file/d/X/view"}"#
                    .to_owned(),
            ),
            ("POST", "/revoke") if google.revoke_fails => answer("500 Internal Server Error", ""),
            ("POST", "/revoke") => json("{}".to_owned()),
            _ => answer("404 Not Found", ""),
        }
    }

    /// A client of a local "Google", its script and what it saw.
    fn google() -> (Client, Script, Seen) {
        let (config, ca) = identity();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let script: Script = Arc::default();
        let routed = script.clone();
        serve(listener, config, seen.clone(), move |m, p, b| {
            route(&routed, m, p, b)
        });
        let endpoints = config::under(&format!("https://localhost:{port}")).unwrap();
        let oauth = config::OAuthClient {
            id: "the-client-id".into(),
            secret: CLIENT_SECRET.into(),
        };
        let client = Client::with_proxy(endpoints, oauth, vec![ca], None);
        (client, script, seen)
    }

    /// How many requests went to `path` (lowercase, without the query).
    fn count(seen: &Seen, method: &str, path: &str) -> usize {
        let prefix = format!("{method} {path}");
        seen.lock()
            .unwrap()
            .iter()
            .filter(|(head, _)| {
                let line = head.lines().next().unwrap_or_default();
                let target = line.split(' ').take(2).collect::<Vec<_>>().join(" ");
                target == prefix || target.starts_with(&format!("{prefix}?"))
            })
            .count()
    }

    fn pages() -> Pages {
        Pages {
            connected: "connected".into(),
            failed: "failed".into(),
        }
    }

    /// A store that counts every call.
    #[derive(Default)]
    struct Counting {
        inner: MemoryStore,
        calls: AtomicU32,
    }

    impl Counting {
        fn calls(&self) -> u32 {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl AppSecretStore for Counting {
        fn get_app(&self, which: AppSecret) -> CoreResult<Option<SecretString>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.inner.get_app(which)
        }
        fn set_app(&self, which: AppSecret, value: &SecretString) -> CoreResult<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.inner.set_app(which, value)
        }
        fn delete_app(&self, which: AppSecret) -> CoreResult<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.inner.delete_app(which)
        }
    }

    /// A keyring that cannot be reached.
    struct Down;

    fn down() -> Error {
        Error::coded(
            ErrorKind::Io,
            "vault-keyring-unavailable",
            "System keyring unavailable",
        )
    }

    impl AppSecretStore for Down {
        fn get_app(&self, _: AppSecret) -> CoreResult<Option<SecretString>> {
            Err(down())
        }
        fn set_app(&self, _: AppSecret, _: &SecretString) -> CoreResult<()> {
            Err(down())
        }
        fn delete_app(&self, _: AppSecret) -> CoreResult<()> {
            Err(down())
        }
    }

    fn entry(store: &dyn AppSecretStore) -> Option<String> {
        store
            .get_app(AppSecret::GoogleDrive)
            .unwrap()
            .map(|value| value.expose_secret().to_owned())
    }

    fn store_entry(store: &MemoryStore) {
        let value = format!(r#"{{"refresh_token":"{REFRESH}","email":"{EMAIL}"}}"#);
        store
            .set_app(AppSecret::GoogleDrive, &SecretString::from(value))
            .unwrap();
    }

    /// The query parameter `name` of `url`, decoded, and how often it occurs.
    fn param(url: &str, name: &str) -> (Option<String>, usize) {
        let query = url.split_once('?').map(|(_, q)| q).unwrap_or_default();
        let found: Vec<String> = form(query.as_bytes())
            .into_iter()
            .filter(|(k, _)| k == name)
            .map(|(_, v)| v)
            .collect();
        (found.first().cloned(), found.len())
    }

    /// Plays the browser: sends Google's redirect (with `code`) to the listener of `url`.
    fn browse(url: &str, code: &str) -> String {
        let redirect = param(url, "redirect_uri").0.unwrap();
        let state = param(url, "state").0.unwrap();
        let port: u16 = redirect.rsplit(':').next().unwrap().parse().unwrap();
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        write!(stream, "GET /?code={code}&state={state} HTTP/1.1\r\n\r\n").unwrap();
        let mut out = String::new();
        let _ = stream.read_to_string(&mut out);
        out
    }

    /// Connects `drive`, the test answering in the browser; returns every event.
    fn connect(drive: &Drive) -> Vec<ConnectEvent> {
        let mut events = Vec::new();
        let mut browser = None;
        drive.connect(&pages(), &mut |event| {
            if let ConnectEvent::Waiting { url } = &event {
                let url = url.clone();
                browser = Some(std::thread::spawn(move || browse(&url, CODE)));
            }
            events.push(event);
        });
        if let Some(browser) = browser {
            browser.join().unwrap();
        }
        events
    }

    fn connected(events: &[ConnectEvent]) -> DriveAccount {
        match events {
            [
                ConnectEvent::Waiting { .. },
                ConnectEvent::Done(Ok(account)),
            ] => account.clone(),
            other => panic!("{other:?}"),
        }
    }

    fn hidden(shown: &str) {
        for secret in SECRETS {
            assert!(!shown.contains(secret), "{secret} in {shown}");
        }
    }

    #[test]
    fn connect_stores_token_and_email_in_one_entry() {
        let (client, _, seen) = google();
        let store = Arc::new(MemoryStore::new());
        let drive = Drive::with_client(Some(client), store.clone());
        let events = connect(&drive);
        let account = connected(&events);
        assert_eq!(
            account,
            DriveAccount {
                email: Some(EMAIL.into()),
                stored: true
            }
        );
        let ConnectEvent::Waiting { url } = &events[0] else {
            unreachable!()
        };
        assert_eq!(param(url, "scope"), (Some(config::SCOPE.to_owned()), 1));
        let value: serde_json::Value =
            serde_json::from_str(&entry(store.as_ref()).unwrap()).unwrap();
        assert_eq!(
            value,
            serde_json::json!({ "refresh_token": REFRESH, "email": EMAIL })
        );
        assert_eq!(count(&seen, "post", "/token"), 1);
        assert_eq!(drive.status().unwrap(), Some(account));
    }

    #[test]
    fn keyring_down_keeps_the_connection_for_the_session() {
        let (client, _, seen) = google();
        let store: Arc<dyn AppSecretStore> = Arc::new(Down);
        let drive = Drive::with_client(Some(client.clone()), store.clone());
        let account = connected(&connect(&drive));
        assert_eq!(
            account,
            DriveAccount {
                email: Some(EMAIL.into()),
                stored: false
            }
        );
        assert_eq!(drive.status().unwrap(), Some(account));
        let file = drive.upload("backup.hoprs", b"sealed").unwrap();
        assert_eq!(file.name, "backup.hoprs");
        assert_eq!(count(&seen, "post", "/upload/drive/v3/files"), 1);
        let again = Drive::with_client(Some(client), store);
        assert_eq!(again.status().unwrap(), None);
    }

    #[test]
    fn status_reads_the_keyring_only_when_asked() {
        let (client, _, _) = google();
        let store = Arc::new(Counting::default());
        store_entry(&store.inner);
        let _lazy = Drive::new(store.clone());
        let drive = Drive::with_client(Some(client.clone()), store.clone());
        assert_eq!(store.calls(), 0);
        let account = drive.status().unwrap();
        assert_eq!(
            account,
            Some(DriveAccount {
                email: Some(EMAIL.into()),
                stored: true
            })
        );
        assert_eq!(store.calls(), 1);
        assert_eq!(drive.status().unwrap(), account);
        assert_eq!(store.calls(), 1);

        // Nothing stored: one read, then nothing more either.
        let empty = Arc::new(Counting::default());
        let drive = Drive::with_client(Some(client), empty.clone());
        assert_eq!(drive.status().unwrap(), None);
        assert_eq!(drive.status().unwrap(), None);
        assert_eq!(empty.calls(), 1);
    }

    #[test]
    fn missing_or_garbage_entry_is_not_connected() {
        let (client, _, _) = google();
        let store = Arc::new(MemoryStore::new());
        let drive = Drive::with_client(Some(client.clone()), store.clone());
        assert_eq!(drive.status().unwrap(), None);
        for garbage in [
            "not json",
            r#"{"email":"x"}"#,
            r#"{"refresh_token":""}"#,
            "[1]",
        ] {
            store
                .set_app(AppSecret::GoogleDrive, &SecretString::from(garbage))
                .unwrap();
            let drive = Drive::with_client(Some(client.clone()), store.clone());
            assert_eq!(drive.status().unwrap(), None, "{garbage}");
            assert_eq!(entry(store.as_ref()), None, "{garbage} is deleted");
        }
    }

    #[test]
    fn upload_refreshes_then_finds_the_folder_and_uploads() {
        let (client, script, seen) = google();
        let store = Arc::new(MemoryStore::new());
        store_entry(&store);
        let drive = Drive::with_client(Some(client.clone()), store.clone());
        let file = drive.upload("backup.hoprs", b"sealed").unwrap();
        assert_eq!(
            file,
            DriveFile {
                name: "backup.hoprs".into(),
                link: Some("https://drive.google.com/file/d/X/view".into())
            }
        );
        assert_eq!(count(&seen, "post", "/token"), 1);
        assert_eq!(count(&seen, "get", "/drive/v3/files"), 1);
        assert_eq!(count(&seen, "post", "/upload/drive/v3/files"), 1);
        // Within the token's life: no refresh.
        drive.upload("backup.hoprs", b"sealed").unwrap();
        assert_eq!(count(&seen, "post", "/token"), 1);
        assert_eq!(count(&seen, "get", "/drive/v3/files"), 2);

        // A 401 from Drive: one refresh, one retry.
        script.lock().unwrap().upload_401 = 1;
        drive.upload("backup.hoprs", b"sealed").unwrap();
        assert_eq!(count(&seen, "post", "/token"), 2);
        assert_eq!(count(&seen, "post", "/upload/drive/v3/files"), 4);
        // Two 401s: the error, after one retry only.
        script.lock().unwrap().upload_401 = 2;
        let err = drive.upload("backup.hoprs", b"sealed").unwrap_err();
        assert_eq!(err.code, Some("drive-upload-failed"));
        assert_eq!(count(&seen, "post", "/token"), 3);
        assert_eq!(count(&seen, "post", "/upload/drive/v3/files"), 6);

        // A token with 60 s or less left is refreshed first.
        script.lock().unwrap().expires_in = 60;
        let drive = Drive::with_client(Some(client), store);
        drive.upload("backup.hoprs", b"sealed").unwrap();
        drive.upload("backup.hoprs", b"sealed").unwrap();
        assert_eq!(count(&seen, "post", "/token"), 5);
    }

    #[test]
    fn refused_refresh_drops_the_connection() {
        let (client, script, _) = google();
        script.lock().unwrap().refuse_refresh = true;
        let store = Arc::new(MemoryStore::new());
        store_entry(&store);
        let drive = Drive::with_client(Some(client), store.clone());
        assert!(drive.status().unwrap().is_some());
        let err = drive.upload("backup.hoprs", b"sealed").unwrap_err();
        assert_eq!(err.code, Some("drive-not-connected"));
        assert_eq!(entry(store.as_ref()), None);
        assert_eq!(drive.status().unwrap(), None);
        let err = drive.upload("backup.hoprs", b"sealed").unwrap_err();
        assert_eq!(err.code, Some("drive-not-connected"));
    }

    #[test]
    fn disconnect_revokes_and_deletes() {
        let (client, script, seen) = google();
        let store = Arc::new(MemoryStore::new());
        store_entry(&store);
        let drive = Drive::with_client(Some(client.clone()), store.clone());
        drive.disconnect().unwrap();
        {
            let seen = seen.lock().unwrap();
            let (head, body) = seen.last().unwrap();
            assert!(head.starts_with("post /revoke "), "{head}");
            assert_eq!(field(&form(body), "token").as_deref(), Some(REFRESH));
        }
        assert_eq!(entry(store.as_ref()), None);
        assert_eq!(drive.status().unwrap(), None);

        // A failed revoke is only logged: the entry still goes.
        script.lock().unwrap().revoke_fails = true;
        store_entry(&store);
        let drive = Drive::with_client(Some(client.clone()), store.clone());
        drive.disconnect().unwrap();
        assert_eq!(entry(store.as_ref()), None);
        assert_eq!(count(&seen, "post", "/revoke"), 2);

        // After a connect, the token in memory is the one revoked.
        script.lock().unwrap().revoke_fails = false;
        let drive = Drive::with_client(Some(client), store.clone());
        connected(&connect(&drive));
        drive.disconnect().unwrap();
        assert_eq!(count(&seen, "post", "/revoke"), 3);
        assert_eq!(entry(store.as_ref()), None);
        assert_eq!(drive.status().unwrap(), None);
    }

    /// Starts a connect on another thread; returns once it is waiting.
    fn waiting(drive: &Arc<Drive>) -> std::thread::JoinHandle<Vec<ConnectEvent>> {
        let (tx, rx) = std::sync::mpsc::channel();
        let drive = drive.clone();
        let handle = std::thread::spawn(move || {
            let mut events = Vec::new();
            drive.connect(&pages(), &mut |event| {
                if matches!(event, ConnectEvent::Waiting { .. }) {
                    let _ = tx.send(());
                }
                events.push(event);
            });
            events
        });
        rx.recv_timeout(Duration::from_secs(10)).unwrap();
        handle
    }

    fn cancelled(events: &[ConnectEvent]) {
        match events {
            [ConnectEvent::Waiting { .. }, ConnectEvent::Done(Err(err))] => {
                assert_eq!(err.code, Some("drive-connect-cancelled"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn cancel_ends_the_wait() {
        let (client, _, _) = google();
        let drive = Arc::new(Drive::with_client(
            Some(client),
            Arc::new(MemoryStore::new()),
        ));
        let first = waiting(&drive);
        let started = Instant::now();
        drive.cancel();
        cancelled(&first.join().unwrap());
        assert!(started.elapsed() < Duration::from_secs(1));

        // A second connect ends the first one the same way.
        let first = waiting(&drive);
        let started = Instant::now();
        let second = waiting(&drive);
        cancelled(&first.join().unwrap());
        assert!(started.elapsed() < Duration::from_secs(1));
        drive.cancel();
        cancelled(&second.join().unwrap());
        assert_eq!(drive.status().unwrap(), None);
    }

    #[test]
    fn not_configured_answers_every_call() {
        let store = Arc::new(Counting::default());
        let drive = Drive::with_client(None, store.clone());
        let code = Some("drive-not-configured");
        assert_eq!(drive.status().unwrap_err().code, code);
        let mut events = Vec::new();
        drive.connect(&pages(), &mut |event| events.push(event));
        assert!(
            matches!(events.as_slice(), [ConnectEvent::Done(Err(err))] if err.code == code),
            "{events:?}"
        );
        assert_eq!(drive.upload("n", b"x").unwrap_err().code, code);
        assert_eq!(drive.disconnect().unwrap_err().code, code);
        drive.cancel();
        assert_eq!(store.calls(), 0);
        let err = drive.status().unwrap_err();
        assert_eq!(err.message, "Google Drive is not available in this build");
    }

    #[test]
    fn nothing_secret_in_debug_or_errors() {
        let (client, script, _) = google();
        let store = Arc::new(MemoryStore::new());
        let drive = Arc::new(Drive::with_client(Some(client.clone()), store.clone()));
        // Every access token is stale at once: each upload refreshes.
        script.lock().unwrap().expires_in = 0;
        let mut shown = Vec::new();
        let events = connect(&drive);
        shown.push(format!("{events:?}"));
        shown.push(format!("{drive:?}"));
        shown.push(format!("{client:?}"));
        shown.push(format!("{:?}", drive.status()));
        shown.push(format!("{:?}", drive.upload("backup.hoprs", b"sealed")));
        // Errors: a refused upload, a refused refresh, a cancelled wait, a bad code.
        script.lock().unwrap().upload_401 = 2;
        shown.push(format!("{:?}", drive.upload("backup.hoprs", b"sealed")));
        script.lock().unwrap().refuse_refresh = true;
        let refused = drive.upload("backup.hoprs", b"sealed");
        assert_eq!(
            refused.as_ref().unwrap_err().code,
            Some("drive-not-connected")
        );
        shown.push(format!("{refused:?}"));
        let first = waiting(&drive);
        drive.cancel();
        shown.push(format!("{:?}", first.join().unwrap()));
        let mut events = Vec::new();
        let mut browser = None;
        drive.connect(&pages(), &mut |event| {
            if let ConnectEvent::Waiting { url } = &event {
                let url = url.clone();
                browser = Some(std::thread::spawn(move || browse(&url, "WRONG-CODE")));
            }
            events.push(event);
        });
        browser.unwrap().join().unwrap();
        assert!(matches!(events.last(), Some(ConnectEvent::Done(Err(_)))));
        shown.push(format!("{events:?}"));
        shown.push(format!("{drive:?}"));
        shown.push(format!("{:?}", Sealed::new(REFRESH.as_bytes().to_vec())));
        for text in &shown {
            hidden(text);
        }
    }
}
