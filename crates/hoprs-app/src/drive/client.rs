//! The blocking HTTPS client of Google Drive: the token calls, the account, the `hoprs`
//! folder and the multipart upload. Callers run it on the blocking pool.
//!
//! Built like the update client (`ureq` on the tree's rustls with the aws-lc-rs provider,
//! the system roots and the proxy variables) but stricter, because tokens travel here:
//! - HTTPS only; **no redirect is ever followed** (any 3xx answer is an error); no cookies.
//! - A token, an authorisation code or the client secret is only sent to the host of the
//!   endpoint it belongs to: forms to `token` and `revoke`, `Authorization: Bearer` to
//!   `api` and `upload`. Every call goes through [`Client::send`], which checks the URL
//!   (scheme `https`, no user info, the endpoint's own host) before connecting.
//! - Sizes: answers are read up to 64 KiB; an upload is refused above 5 MB before any
//!   request is made.
//! - Time: token and API calls end within 10 s overall, an upload within 60 s.
//! - Nothing secret is shown: an [`Error`] carries only `HTTP <status>`, Google's short
//!   `error` code, a transport error or a fixed phrase, never a response body, a token, a
//!   code or a URL with a query; the `Debug` of [`Tokens`] and [`Client`] shows none.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use hoprs_core::{Error, ErrorKind};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use rustls::pki_types::CertificateDer;
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use ureq::http::Uri;
use ureq::tls::{Certificate, RootCerts, TlsConfig, TlsProvider};
use ureq::{Agent, Proxy};

use super::DriveFile;
use super::config::{Endpoints, FOLDER, OAuthClient};
use super::oauth::{connect_failed, random};

/// Connect, headers and the whole of a token or API call.
const TIMEOUT: Duration = Duration::from_secs(10);
/// The whole of an upload.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(60);
/// The most an answer is read for.
const ANSWER_LIMIT: u64 = 64 * 1024;
/// Google's limit for a multipart upload.
const UPLOAD_LIMIT: usize = 5 * 1024 * 1024;
const USER_AGENT: &str = concat!("hoprs/", env!("CARGO_PKG_VERSION"));
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";

/// RFC 3986 unreserved characters stay as they are; everything else is percent-encoded.
const ENCODE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

fn upload_failed(detail: impl Into<String>) -> Error {
    Error::coded(
        ErrorKind::Network,
        "drive-upload-failed",
        "Could not upload to Google Drive",
    )
    .with_detail(detail)
}

pub(crate) fn not_connected() -> Error {
    Error::coded(
        ErrorKind::Auth,
        "drive-not-connected",
        "Not connected to Google Drive",
    )
}

/// What a failed call says, never a body or a URL.
const UNEXPECTED: &str = "unexpected answer";
const REDIRECT: &str = "redirect refused";
/// The detail of a Drive call that Google refused with 401 (the access token is no good).
const UNAUTHORIZED: &str = "HTTP 401";

/// The Drive call failed because Google refused the access token (401): a refresh may
/// help.
pub fn unauthorized(err: &Error) -> bool {
    err.code == Some("drive-upload-failed") && err.detail.as_deref() == Some(UNAUTHORIZED)
}

/// The tokens of a sign-in or a refresh.
pub struct Tokens {
    pub access: SecretString,
    /// Seconds the access token lives.
    pub expires_in: u64,
    /// Present on a sign-in; a refresh does not return a new one.
    pub refresh: Option<SecretString>,
}

/// Shows no token.
impl fmt::Debug for Tokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tokens")
            .field("expires_in", &self.expires_in)
            .finish_non_exhaustive()
    }
}

/// The Google Drive client (see the module docs).
#[derive(Clone)]
pub struct Client {
    agent: Agent,
    endpoints: Endpoints,
    oauth: OAuthClient,
}

/// Shows neither the OAuth client nor the endpoints' query strings (there are none).
impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client").finish_non_exhaustive()
    }
}

/// A completed exchange: the status (never 3xx) and the answer's bytes (at most 64 KiB).
struct Reply {
    status: u16,
    body: Vec<u8>,
}

impl Reply {
    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The answer as a JSON object.
    fn object(&self) -> Option<serde_json::Map<String, Value>> {
        match serde_json::from_slice(&self.body) {
            Ok(Value::Object(map)) => Some(map),
            _ => None,
        }
    }

    /// Google's short `error` code of a failed token call (`invalid_grant`, ...): only
    /// when it is a short lowercase word, so the detail can never carry a body.
    fn error_code(&self) -> Option<String> {
        let code = self.object()?.get("error")?.as_str()?.to_owned();
        let word = !code.is_empty()
            && code.len() <= 40
            && code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
        word.then_some(code)
    }

    /// A failed Drive API call: [`UNAUTHORIZED`] for a 401, else [`Reply::failure`].
    fn drive_failure(&self) -> String {
        if self.status == 401 {
            UNAUTHORIZED.to_owned()
        } else {
            self.failure()
        }
    }

    /// `Google's error code`, else `HTTP <status>`.
    fn failure(&self) -> String {
        self.error_code()
            .unwrap_or_else(|| format!("HTTP {}", self.status))
    }
}

/// What a request is for: which endpoint it belongs to.
#[derive(Clone, Copy)]
enum Endpoint {
    Token,
    Revoke,
    Api,
    Upload,
}

/// A readable reason for a transport error. Never a URL: errors that may carry one are
/// replaced by a fixed phrase.
fn describe(err: ureq::Error) -> String {
    match err {
        ureq::Error::StatusCode(code) => format!("HTTP {code}"),
        ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => REDIRECT.to_owned(),
        ureq::Error::BodyExceedsLimit(_) => UNEXPECTED.to_owned(),
        ureq::Error::Timeout(_) => "timed out".to_owned(),
        ureq::Error::Http(_) | ureq::Error::BadUri(_) | ureq::Error::InvalidProxyUrl => {
            "invalid request".to_owned()
        }
        other => other.to_string(),
    }
}

/// The URL is `https`, has no user info and is on the host of `endpoint` (which must be
/// `https` itself). The error is a detail without the URL.
fn check_url(endpoint: &str, url: &str) -> Result<(), String> {
    let parse = |text: &str| -> Option<Uri> { text.parse().ok() };
    let (Some(base), Some(uri)) = (parse(endpoint), parse(url)) else {
        return Err("invalid endpoint".to_owned());
    };
    let plain = |uri: &Uri| {
        uri.scheme_str() == Some("https")
            && uri
                .authority()
                .is_some_and(|authority| !authority.as_str().contains('@'))
    };
    if !plain(&base) || !plain(&uri) {
        return Err("endpoint is not a plain https URL".to_owned());
    }
    let host = |uri: &Uri| uri.host().map(str::to_ascii_lowercase);
    if host(&base).is_none() || host(&base) != host(&uri) {
        return Err("endpoint host mismatch".to_owned());
    }
    Ok(())
}

fn encode(value: &str) -> String {
    utf8_percent_encode(value, ENCODE).to_string()
}

/// `a=b&c=d`, percent-encoded.
fn form(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// A Google id (file or folder): short, URL-safe characters only.
fn is_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The link of an uploaded file, kept only when it is `https://` on `google.com` or a
/// subdomain of it, without user info.
fn safe_link(link: Option<&str>) -> Option<String> {
    let link = link?;
    let uri: Uri = link.parse().ok()?;
    let authority = uri.authority()?;
    let host = authority.host().to_ascii_lowercase();
    let google = host == "google.com" || host.ends_with(".google.com");
    (uri.scheme_str() == Some("https") && !authority.as_str().contains('@') && google)
        .then(|| link.to_owned())
}

impl Client {
    /// A client that trusts `roots` only, through the system proxy if one is set.
    pub fn new(
        endpoints: Endpoints,
        oauth: OAuthClient,
        roots: Vec<CertificateDer<'static>>,
    ) -> Client {
        Client::with_proxy(endpoints, oauth, roots, Proxy::try_from_env())
    }

    /// [`Client::new`] with an explicit proxy (`None`: none, whatever the environment
    /// says); the tests use it.
    pub(super) fn with_proxy(
        endpoints: Endpoints,
        oauth: OAuthClient,
        roots: Vec<CertificateDer<'static>>,
        proxy: Option<Proxy>,
    ) -> Client {
        let roots: Vec<Certificate<'static>> = roots
            .iter()
            .map(|der| Certificate::from_der(der.as_ref()).to_owned())
            .collect();
        let tls = TlsConfig::builder()
            .provider(TlsProvider::Rustls)
            .root_certs(RootCerts::Specific(Arc::new(roots)))
            .unversioned_rustls_crypto_provider(Arc::new(
                rustls::crypto::aws_lc_rs::default_provider(),
            ))
            .build();
        let agent = Agent::config_builder()
            .tls_config(tls)
            .proxy(proxy)
            .https_only(true)
            // Statuses are read here (Google says why in the body); 3xx is an error.
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .user_agent(USER_AGENT)
            .timeout_connect(Some(TIMEOUT))
            .timeout_recv_response(Some(TIMEOUT))
            .build()
            .new_agent();
        Client {
            agent,
            endpoints,
            oauth,
        }
    }

    /// A client that trusts the system's roots.
    pub fn system(endpoints: Endpoints, oauth: OAuthClient) -> Client {
        Client::new(endpoints, oauth, crate::update::http::system_roots())
    }

    /// The URL the browser is opened at, for this client and the listener at
    /// `redirect_uri`.
    pub fn auth_url(&self, redirect_uri: &str, challenge: &str, state: &str) -> String {
        super::oauth::auth_url(
            &self.endpoints,
            &self.oauth.id,
            redirect_uri,
            challenge,
            state,
        )
    }

    fn base(&self, endpoint: Endpoint) -> &str {
        match endpoint {
            Endpoint::Token => &self.endpoints.token,
            Endpoint::Revoke => &self.endpoints.revoke,
            Endpoint::Api => &self.endpoints.api,
            Endpoint::Upload => &self.endpoints.upload,
        }
    }

    /// The one path every call takes: `url` is `endpoint`'s base plus a suffix; it is
    /// checked against that base before anything connects. A bearer token is attached
    /// only for [`Endpoint::Api`] and [`Endpoint::Upload`]. A 3xx answer is an error.
    fn send(
        &self,
        endpoint: Endpoint,
        suffix: &str,
        bearer: Option<&SecretString>,
        content_type: &str,
        body: Option<&[u8]>,
        timeout: Duration,
    ) -> Result<Reply, String> {
        let base = self.base(endpoint);
        let url = format!("{base}{suffix}");
        check_url(base, &url)?;
        let bearer = match endpoint {
            Endpoint::Api | Endpoint::Upload => bearer,
            Endpoint::Token | Endpoint::Revoke => None,
        };
        let response = match body {
            Some(body) => {
                let mut request = self.agent.post(&url).content_type(content_type);
                if let Some(token) = bearer {
                    request = request
                        .header("Authorization", format!("Bearer {}", token.expose_secret()));
                }
                request
                    .config()
                    .timeout_global(Some(timeout))
                    .build()
                    .send(body)
            }
            None => {
                let mut request = self.agent.get(&url);
                if let Some(token) = bearer {
                    request = request
                        .header("Authorization", format!("Bearer {}", token.expose_secret()));
                }
                request
                    .config()
                    .timeout_global(Some(timeout))
                    .build()
                    .call()
            }
        }
        .map_err(describe)?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            return Err(REDIRECT.to_owned());
        }
        let body = response
            .into_body()
            .into_with_config()
            .limit(ANSWER_LIMIT)
            .read_to_vec()
            .map_err(describe)?;
        Ok(Reply { status, body })
    }

    /// A form POST to the token or revoke endpoint.
    fn post_form(&self, endpoint: Endpoint, fields: &[(&str, &str)]) -> Result<Reply, String> {
        self.send(
            endpoint,
            "",
            None,
            "application/x-www-form-urlencoded",
            Some(form(fields).as_bytes()),
            TIMEOUT,
        )
    }

    /// The tokens in a successful token answer.
    fn tokens(reply: &Reply) -> Result<Tokens, String> {
        let object = reply.object().ok_or(UNEXPECTED)?;
        let access = object
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .ok_or(UNEXPECTED)?;
        let expires_in = object
            .get("expires_in")
            .and_then(Value::as_u64)
            .ok_or(UNEXPECTED)?;
        let refresh = object
            .get("refresh_token")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .map(|token| SecretString::from(token.to_owned()));
        Ok(Tokens {
            access: SecretString::from(access.to_owned()),
            expires_in,
            refresh,
        })
    }

    /// Trades the authorisation `code` (with its PKCE `verifier`) for tokens.
    pub fn exchange(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> Result<Tokens, Error> {
        let reply = self
            .post_form(
                Endpoint::Token,
                &[
                    ("grant_type", "authorization_code"),
                    ("code", code),
                    ("code_verifier", verifier),
                    ("client_id", &self.oauth.id),
                    ("client_secret", &self.oauth.secret),
                    ("redirect_uri", redirect_uri),
                ],
            )
            .map_err(connect_failed)?;
        if !reply.ok() {
            return Err(connect_failed(reply.failure()));
        }
        Self::tokens(&reply).map_err(connect_failed)
    }

    /// A new access token from the refresh token. Google saying the grant is gone
    /// (revoked, expired) is [`not_connected`]; any other failure is a connect failure.
    pub fn refresh(&self, refresh: &SecretString) -> Result<Tokens, Error> {
        let reply = self
            .post_form(
                Endpoint::Token,
                &[
                    ("grant_type", "refresh_token"),
                    ("refresh_token", refresh.expose_secret()),
                    ("client_id", &self.oauth.id),
                    ("client_secret", &self.oauth.secret),
                ],
            )
            .map_err(connect_failed)?;
        if matches!(reply.status, 400 | 401)
            && reply.error_code().as_deref() == Some("invalid_grant")
        {
            return Err(not_connected());
        }
        if !reply.ok() {
            return Err(connect_failed(reply.failure()));
        }
        Self::tokens(&reply).map_err(connect_failed)
    }

    /// Revokes the refresh token at Google.
    pub fn revoke(&self, refresh: &SecretString) -> Result<(), Error> {
        let reply = self
            .post_form(Endpoint::Revoke, &[("token", refresh.expose_secret())])
            .map_err(connect_failed)?;
        if reply.ok() {
            Ok(())
        } else {
            Err(connect_failed(reply.failure()))
        }
    }

    /// The signed-in account's e-mail address; any failure means "no email".
    pub fn email(&self, access: &SecretString) -> Option<String> {
        let reply = self
            .send(
                Endpoint::Api,
                "/about?fields=user(emailAddress)",
                Some(access),
                "",
                None,
                TIMEOUT,
            )
            .ok()?;
        if !reply.ok() {
            return None;
        }
        let object = reply.object()?;
        let email = object.get("user")?.get("emailAddress")?.as_str()?;
        let plain =
            !email.is_empty() && email.len() <= 320 && email.chars().all(|c| !c.is_control());
        plain.then(|| email.to_owned())
    }

    /// The id of the `hoprs` folder, created when there is none.
    pub fn folder(&self, access: &SecretString) -> Result<String, Error> {
        self.folder_id(access).map_err(upload_failed)
    }

    fn folder_id(&self, access: &SecretString) -> Result<String, String> {
        let q = format!("name = '{FOLDER}' and mimeType = '{FOLDER_MIME}' and trashed = false");
        let suffix = format!("/files?q={}&fields=files(id)&pageSize=1", encode(&q));
        let reply = self.send(Endpoint::Api, &suffix, Some(access), "", None, TIMEOUT)?;
        if !reply.ok() {
            return Err(reply.drive_failure());
        }
        let object = reply.object().ok_or(UNEXPECTED)?;
        let files = object
            .get("files")
            .and_then(Value::as_array)
            .ok_or(UNEXPECTED)?;
        if let Some(first) = files.first() {
            let id = first.get("id").and_then(Value::as_str).ok_or(UNEXPECTED)?;
            return if is_id(id) {
                Ok(id.to_owned())
            } else {
                Err(UNEXPECTED.to_owned())
            };
        }
        let metadata = serde_json::json!({ "name": FOLDER, "mimeType": FOLDER_MIME });
        let reply = self.send(
            Endpoint::Api,
            "/files?fields=id",
            Some(access),
            "application/json",
            Some(metadata.to_string().as_bytes()),
            TIMEOUT,
        )?;
        if !reply.ok() {
            return Err(reply.drive_failure());
        }
        let id = reply
            .object()
            .and_then(|object| object.get("id")?.as_str().map(str::to_owned))
            .filter(|id| is_id(id))
            .ok_or(UNEXPECTED)?;
        Ok(id)
    }

    /// Uploads `bytes` as `name` into `folder` (multipart, at most 5 MB).
    pub fn upload(
        &self,
        access: &SecretString,
        folder: &str,
        name: &str,
        bytes: &[u8],
    ) -> Result<DriveFile, Error> {
        self.upload_file(access, folder, name, bytes)
            .map_err(upload_failed)
    }

    fn upload_file(
        &self,
        access: &SecretString,
        folder: &str,
        name: &str,
        bytes: &[u8],
    ) -> Result<DriveFile, String> {
        if bytes.len() > UPLOAD_LIMIT {
            return Err("larger than 5 MB".to_owned());
        }
        let (boundary, body) = multipart(folder, name, bytes)?;
        let reply = self.send(
            Endpoint::Upload,
            "/files?uploadType=multipart&fields=id,name,webViewLink",
            Some(access),
            &format!("multipart/related; boundary={boundary}"),
            Some(&body),
            UPLOAD_TIMEOUT,
        )?;
        if !reply.ok() {
            return Err(reply.drive_failure());
        }
        let object = reply.object().ok_or(UNEXPECTED)?;
        object
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| is_id(id))
            .ok_or(UNEXPECTED)?;
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .filter(|reported| !reported.is_empty() && reported.len() <= 1024)
            .unwrap_or(name);
        Ok(DriveFile {
            name: name.to_owned(),
            link: safe_link(object.get("webViewLink").and_then(Value::as_str)),
        })
    }
}

/// The `multipart/related` body of an upload and its boundary: the JSON metadata part,
/// then the bytes. The boundary is random and checked not to occur in the bytes.
fn multipart(folder: &str, name: &str, bytes: &[u8]) -> Result<(String, Vec<u8>), String> {
    let metadata = serde_json::json!({ "name": name, "parents": [folder] });
    let mut boundary = String::new();
    for _ in 0..4 {
        let random = random::<16>().map_err(|_| "no random numbers".to_owned())?;
        let candidate = format!(
            "hoprs-{}",
            random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        if !bytes
            .windows(candidate.len())
            .any(|window| window == candidate.as_bytes())
        {
            boundary = candidate;
            break;
        }
    }
    if boundary.is_empty() {
        return Err("no free boundary".to_owned());
    }
    let mut body = Vec::with_capacity(bytes.len() + 512);
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n--{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    Ok((boundary, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_https::{Answer, Seen, answer, identity, redirect, serve};
    use percent_encoding::percent_decode_str;
    use secrecy::ExposeSecret;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    const FOLDER_Q: &str =
        "name = 'hoprs' and mimeType = 'application/vnd.google-apps.folder' and trashed = false";

    fn oauth() -> OAuthClient {
        OAuthClient {
            id: "the-id".into(),
            secret: "the-secret".into(),
        }
    }

    fn secret(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    /// A client of a local server answering by `route(method, path_and_query, body)`.
    fn server(route: impl Fn(&str, &str, &[u8]) -> Answer + Send + 'static) -> (Client, Seen, u16) {
        let (config, ca) = identity();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        serve(listener, config, seen.clone(), route);
        let endpoints = crate::drive::config::under(&format!("https://localhost:{port}")).unwrap();
        // No proxy: one set in the developer's environment must not see this server.
        let client = Client::with_proxy(endpoints, oauth(), vec![ca], None);
        (client, seen, port)
    }

    fn json(body: &str) -> Answer {
        answer("200 OK", body)
    }

    /// The fields of a form body, decoded.
    fn form(body: &[u8]) -> Vec<(String, String)> {
        let text = std::str::from_utf8(body).unwrap();
        text.split('&')
            .map(|pair| {
                let (k, v) = pair.split_once('=').unwrap();
                let d = |s: &str| percent_decode_str(s).decode_utf8().unwrap().into_owned();
                (d(k), d(v))
            })
            .collect()
    }

    fn field<'a>(fields: &'a [(String, String)], name: &str) -> &'a str {
        fields
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .unwrap_or_else(|| panic!("no field {name}"))
    }

    fn query(path: &str, name: &str) -> String {
        let q = path.split_once('?').unwrap().1;
        field(&form(q.as_bytes()), name).to_owned()
    }

    #[test]
    fn exchange_sends_the_pkce_form() {
        let (client, seen, _) = server(|m, p, _| match (m, p) {
            ("POST", "/token") => {
                json(r#"{"access_token":"A","expires_in":3599,"refresh_token":"R"}"#)
            }
            _ => answer("404 Not Found", ""),
        });
        let tokens = client
            .exchange("the-code", "the-verifier", "http://127.0.0.1:5/cb")
            .unwrap();
        assert_eq!(tokens.access.expose_secret(), "A");
        assert_eq!(tokens.expires_in, 3599);
        assert_eq!(tokens.refresh.unwrap().expose_secret(), "R");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        let (head, body) = &seen[0];
        assert!(head.starts_with("post /token "), "{head}");
        assert!(!head.contains("\r\nauthorization:"), "{head}");
        assert!(head.contains("\r\ncontent-type: application/x-www-form-urlencoded"));
        assert!(head.contains("\r\nuser-agent: hoprs/"));
        let mut fields = form(body);
        fields.sort();
        let names: Vec<&str> = fields.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            names,
            [
                "client_id",
                "client_secret",
                "code",
                "code_verifier",
                "grant_type",
                "redirect_uri"
            ]
        );
        assert_eq!(field(&fields, "grant_type"), "authorization_code");
        assert_eq!(field(&fields, "code"), "the-code");
        assert_eq!(field(&fields, "code_verifier"), "the-verifier");
        assert_eq!(field(&fields, "client_id"), "the-id");
        assert_eq!(field(&fields, "client_secret"), "the-secret");
        assert_eq!(field(&fields, "redirect_uri"), "http://127.0.0.1:5/cb");
    }

    #[test]
    fn refresh_and_revoke() {
        let (client, seen, _) = server(|m, p, body| match (m, p) {
            ("POST", "/token") => {
                let fields = form(body);
                match field(&fields, "refresh_token") {
                    "R" => json(r#"{"access_token":"A2","expires_in":3600}"#),
                    "gone" => answer("400 Bad Request", r#"{"error":"invalid_grant"}"#),
                    "gone401" => answer("401 Unauthorized", r#"{"error":"invalid_grant"}"#),
                    _ => answer("500 Internal Server Error", "boom"),
                }
            }
            ("POST", "/revoke") => json("{}"),
            _ => answer("404 Not Found", ""),
        });
        let tokens = client.refresh(&secret("R")).unwrap();
        assert_eq!(tokens.access.expose_secret(), "A2");
        assert!(tokens.refresh.is_none());
        {
            let seen = seen.lock().unwrap();
            let fields = form(&seen[0].1);
            assert_eq!(field(&fields, "grant_type"), "refresh_token");
            assert_eq!(field(&fields, "refresh_token"), "R");
            assert_eq!(field(&fields, "client_id"), "the-id");
            assert_eq!(field(&fields, "client_secret"), "the-secret");
            assert_eq!(fields.len(), 4);
        }
        for gone in ["gone", "gone401"] {
            let err = client.refresh(&secret(gone)).unwrap_err();
            assert_eq!(err.code, Some("drive-not-connected"), "{gone}");
        }
        let err = client.refresh(&secret("other")).unwrap_err();
        assert_ne!(err.code, Some("drive-not-connected"));
        assert_eq!(err.detail.as_deref(), Some("HTTP 500"));
        assert!(!format!("{err:?}").contains("boom"));

        client.revoke(&secret("R")).unwrap();
        let seen = seen.lock().unwrap();
        let (head, body) = seen.last().unwrap();
        assert!(head.starts_with("post /revoke "), "{head}");
        assert_eq!(form(body), [("token".to_owned(), "R".to_owned())]);
    }

    #[test]
    fn folder_is_found_or_created() {
        let found = Arc::new(Mutex::new(true));
        let flag = found.clone();
        let (client, seen, _) = server(move |m, p, _| match (m, p.split('?').next().unwrap()) {
            ("GET", "/drive/v3/files") if *flag.lock().unwrap() => {
                json(r#"{"files":[{"id":"F1"}]}"#)
            }
            ("GET", "/drive/v3/files") => json(r#"{"files":[]}"#),
            ("POST", "/drive/v3/files") => json(r#"{"id":"NEW"}"#),
            _ => answer("404 Not Found", ""),
        });
        assert_eq!(client.folder(&secret("a")).unwrap(), "F1");
        {
            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), 1, "no POST when the folder exists");
            let (head, _) = &seen[0];
            assert!(head.starts_with("get /drive/v3/files?"), "{head}");
            assert!(head.contains("\r\nauthorization: bearer a\r\n"), "{head}");
            let path = head.split(' ').nth(1).unwrap();
            // The head is lowercase; the query is compared case-insensitively.
            assert_eq!(query(path, "q"), FOLDER_Q.to_ascii_lowercase());
            assert_eq!(query(path, "fields"), "files(id)");
            assert_eq!(query(path, "pagesize"), "1");
        }
        *found.lock().unwrap() = false;
        assert_eq!(client.folder(&secret("a")).unwrap(), "NEW");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        let (head, body) = &seen[2];
        assert!(
            head.starts_with("post /drive/v3/files?fields=id "),
            "{head}"
        );
        assert!(
            head.contains("\r\ncontent-type: application/json"),
            "{head}"
        );
        let value: serde_json::Value = serde_json::from_slice(body).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"name":"hoprs","mimeType":"application/vnd.google-apps.folder"})
        );
    }

    #[test]
    fn folder_query_is_exact() {
        // The request line keeps the case of the query: checked without lowercasing.
        let (client, seen, _) = server(|_, _, _| json(r#"{"files":[{"id":"F1"}]}"#));
        client.folder(&secret("a")).unwrap();
        let seen = seen.lock().unwrap();
        let line = seen[0].0.lines().next().unwrap();
        let path = line.split(' ').nth(1).unwrap();
        assert!(
            path.contains("%27hoprs%27") || path.contains("%27HOPRS%27"),
            "{path}"
        );
    }

    #[test]
    fn upload_sends_metadata_and_bytes() {
        let (client, seen, _) = server(|m, p, _| match (m, p.split('?').next().unwrap()) {
            ("POST", "/upload/drive/v3/files") => json(
                r#"{"id":"X","name":"backup.hoprs","webViewLink":"https://drive.google.com/file/d/X/view"}"#,
            ),
            _ => answer("404 Not Found", ""),
        });
        let bytes: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
        let file = client
            .upload(&secret("a"), "FOLDER1", "backup.hoprs", &bytes)
            .unwrap();
        assert_eq!(file.name, "backup.hoprs");
        assert_eq!(
            file.link.as_deref(),
            Some("https://drive.google.com/file/d/X/view")
        );
        let seen = seen.lock().unwrap();
        let (head, body) = &seen[0];
        assert!(head.starts_with("post /upload/drive/v3/files?"), "{head}");
        let path = head.split(' ').nth(1).unwrap();
        assert_eq!(query(path, "uploadtype"), "multipart");
        assert_eq!(query(path, "fields"), "id,name,webviewlink");
        assert!(head.contains("\r\nauthorization: bearer a\r\n"), "{head}");
        let boundary = head
            .split("boundary=")
            .nth(1)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .trim()
            .to_owned();
        assert!(head.contains("\r\ncontent-type: multipart/related; boundary="));
        // The boundary of the lowercase head is lowercase; find the real one in the body.
        let text_start = body.windows(2).position(|w| w == b"--").unwrap();
        let line_end = body.windows(2).position(|w| w == b"\r\n").unwrap();
        let real = std::str::from_utf8(&body[text_start + 2..line_end]).unwrap();
        assert_eq!(real.to_ascii_lowercase(), boundary);
        assert!(real.len() >= 16);
        let marker = format!("--{real}");
        let mut expected = Vec::new();
        let json_part = serde_json::json!({"name":"backup.hoprs","parents":["FOLDER1"]});
        expected.extend_from_slice(
            format!(
                "{marker}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{json_part}\r\n{marker}\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        expected.extend_from_slice(&bytes);
        expected.extend_from_slice(format!("\r\n{marker}--\r\n").as_bytes());
        assert_eq!(body, &expected);
    }

    #[test]
    fn unsafe_view_link_is_left_out() {
        for (link, kept) in [
            (Some("http://drive.google.com/x"), false),
            (Some("https://evil.example/x"), false),
            (Some("https://google.com.evil.example/x"), false),
            (Some("https://evilgoogle.com/x"), false),
            (Some("https://user@drive.google.com/x"), false),
            (None, false),
            (Some("https://drive.google.com/file/d/1/view"), true),
            (Some("https://google.com/x"), true),
        ] {
            let body = match link {
                Some(link) => format!(r#"{{"id":"X","name":"n","webViewLink":"{link}"}}"#),
                None => r#"{"id":"X","name":"n"}"#.to_owned(),
            };
            let (client, _, _) = server(move |_, _, _| json(&body));
            let file = client.upload(&secret("a"), "F", "n", b"x").unwrap();
            assert_eq!(file.link.is_some(), kept, "{link:?}");
            if kept {
                assert_eq!(file.link.as_deref(), link);
            }
        }
        assert_eq!(
            safe_link(Some("https://a.google.com/x")).as_deref(),
            Some("https://a.google.com/x")
        );
    }

    #[test]
    fn unexpected_answers_are_errors_not_panics() {
        let big = "x".repeat(1024 * 1024);
        let cases: Vec<(&'static str, Answer)> = vec![
            ("html", answer("200 OK", "<html>hi</html>")),
            ("empty object", answer("200 OK", "{}")),
            ("empty body", answer("200 OK", "")),
            (
                "401",
                answer("401 Unauthorized", r#"{"error":{"message":"SECRET-BODY"}}"#),
            ),
            ("1 MiB", answer("200 OK", big.clone())),
            ("1 MiB 401", answer("401 Unauthorized", big)),
            ("not an object", answer("200 OK", "[1,2]")),
            (
                "wrong types",
                answer(
                    "200 OK",
                    r#"{"access_token":5,"expires_in":"x","id":7,"files":"no"}"#,
                ),
            ),
        ];
        for (label, canned) in cases {
            let canned = Arc::new(Mutex::new(Some(canned)));
            let (client, _, _) = server(move |_, _, _| {
                // The same answer for every request of this case (one per call below).
                let mut guard = canned.lock().unwrap();
                guard.take().unwrap_or_else(|| answer("500 Error", ""))
            });
            let err = client.exchange("c", "v", "r").unwrap_err();
            assert_eq!(err.code, Some("drive-connect-failed"), "{label}: {err:?}");
            assert!(!format!("{err:?}").contains("SECRET-BODY"), "{label}");
        }
        for (label, make) in [
            (
                "html",
                (|| answer("200 OK", "<html>hi</html>")) as fn() -> Answer,
            ),
            ("empty object", || answer("200 OK", "{}")),
            ("empty body", || answer("200 OK", "")),
            ("401", || {
                answer("401 Unauthorized", r#"{"error":{"message":"SECRET-BODY"}}"#)
            }),
            ("1 MiB", || answer("200 OK", "x".repeat(1024 * 1024))),
            ("wrong types", || {
                answer("200 OK", r#"{"id":7,"files":"no"}"#)
            }),
            ("odd id", || {
                answer("200 OK", r#"{"files":[{"id":"a b\"c"}]}"#)
            }),
        ] {
            let (client, _, _) = server(move |_, _, _| make());
            let err = client.folder(&secret("tok-SECRET")).unwrap_err();
            assert_eq!(
                err.code,
                Some("drive-upload-failed"),
                "folder {label}: {err:?}"
            );
            let shown = format!("{err:?}");
            assert!(
                !shown.contains("tok-SECRET") && !shown.contains("SECRET-BODY"),
                "{shown}"
            );
            let err = client
                .upload(&secret("tok-SECRET"), "F", "n", b"x")
                .unwrap_err();
            assert_eq!(
                err.code,
                Some("drive-upload-failed"),
                "upload {label}: {err:?}"
            );
            let shown = format!("{err:?}");
            assert!(
                !shown.contains("tok-SECRET") && !shown.contains("SECRET-BODY"),
                "{shown}"
            );
        }
    }

    #[test]
    fn redirects_are_refused() {
        let (client, seen, _) = server(|_, p, _| match p.split('?').next().unwrap() {
            "/elsewhere" => {
                json(r#"{"access_token":"A","expires_in":1,"id":"X","files":[{"id":"F"}]}"#)
            }
            _ => redirect("/elsewhere".into()),
        });
        let err = client.exchange("c", "v", "r").unwrap_err();
        assert_eq!(err.code, Some("drive-connect-failed"));
        assert_eq!(err.detail.as_deref(), Some("redirect refused"));
        let err = client.folder(&secret("a")).unwrap_err();
        assert_eq!(err.code, Some("drive-upload-failed"));
        assert_eq!(err.detail.as_deref(), Some("redirect refused"));
        let err = client.upload(&secret("a"), "F", "n", b"x").unwrap_err();
        assert_eq!(err.code, Some("drive-upload-failed"));
        assert_eq!(err.detail.as_deref(), Some("redirect refused"));
        let err = client.revoke(&secret("R")).unwrap_err();
        assert_eq!(err.detail.as_deref(), Some("redirect refused"));
        assert!(client.email(&secret("a")).is_none());
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 5);
        assert!(
            seen.iter().all(|(head, _)| !head.contains("/elsewhere")),
            "{seen:?}"
        );
    }

    #[test]
    fn tokens_go_only_to_their_endpoint() {
        let (client, seen, port) = server(|_, _, _| json("{}"));
        // An endpoint that is not https, or has user info: refused before any request.
        let mut endpoints = client.endpoints.clone();
        endpoints.upload = format!("http://localhost:{port}/upload/drive/v3");
        endpoints.api = format!("https://user@localhost:{port}/drive/v3");
        endpoints.token = format!("https://user:pw@localhost:{port}/token");
        let bad = Client {
            endpoints,
            ..client.clone()
        };
        let err = bad.upload(&secret("a"), "F", "n", b"x").unwrap_err();
        assert_eq!(err.code, Some("drive-upload-failed"));
        assert!(bad.folder(&secret("a")).is_err());
        assert!(bad.exchange("c", "v", "r").is_err());
        assert!(bad.email(&secret("a")).is_none());
        assert!(seen.lock().unwrap().is_empty());
        // The host rule itself.
        let base = format!("https://localhost:{port}/drive/v3");
        assert!(check_url(&base, &format!("{base}/files?q=1")).is_ok());
        for url in [
            "https://other.example/drive/v3/files",
            "https://localhost.evil.example/drive/v3/files",
            "http://localhost/drive/v3/files",
            "https://user@localhost/drive/v3/files",
            "not a url",
        ] {
            assert!(check_url(&base, url).is_err(), "{url}");
        }
        assert!(check_url("http://localhost/x", "http://localhost/x").is_err());
        // A failing account lookup is "no email", not an error.
        let (client, _, _) = server(|_, _, _| answer("403 Forbidden", "{}"));
        assert!(client.email(&secret("a")).is_none());
        let (client, _, _) =
            server(|_, _, _| json(r#"{"user":{"emailAddress":"me@example.com"}}"#));
        assert_eq!(
            client.email(&secret("a")).as_deref(),
            Some("me@example.com")
        );
    }

    #[test]
    fn upload_over_five_megabytes_is_refused_without_a_request() {
        let (client, seen, _) = server(|_, _, _| json(r#"{"id":"X","name":"n"}"#));
        let big = vec![0u8; 5 * 1024 * 1024 + 1];
        let err = client.upload(&secret("a"), "F", "n", &big).unwrap_err();
        assert_eq!(err.code, Some("drive-upload-failed"));
        assert_eq!(err.detail.as_deref(), Some("larger than 5 MB"));
        assert!(seen.lock().unwrap().is_empty());
        let ok = vec![0u8; 5 * 1024 * 1024];
        client.upload(&secret("a"), "F", "n", &ok).unwrap();
    }
}
