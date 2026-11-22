//! The update check and download over HTTPS: `ureq` (blocking, so callers run it on the
//! blocking pool) on the tree's rustls with the aws-lc-rs provider and the system roots.
//!
//! Policy:
//! - HTTPS only, on every hop, with certificate validation against the given roots (the
//!   system's in the app). No cookies (ureq's `cookies` feature is off), no tokens, and no
//!   header besides `User-Agent: hoprs/<version>` and `Accept`.
//! - Redirects: at most 5 hops, each HTTPS (ureq's `https_only`), and every URL of the
//!   chain must be on an allowed host: the API URL's own host, `github.com`, or a
//!   subdomain of `githubusercontent.com` (GitHub's asset storage). A chain through any
//!   other host is refused before its body is read. Nothing secret is ever sent, so the
//!   only thing such a hop could see is the request itself.
//! - Sizes: the release JSON at most 1 MiB, `SHA256SUMS` at most 64 KiB, an installer at
//!   most 1 GiB (and exactly its `Content-Length`, when given; `download` also checks the
//!   size the release lists).
//! - Time: the check and `SHA256SUMS` end within 10 s overall; a download has 10 s to
//!   connect and get its headers, then each read must make progress within 30 s.
//! - Proxy: the system proxy from the environment (`ALL_PROXY`, `HTTPS_PROXY`,
//!   `HTTP_PROXY`, with `NO_PROXY`) is honoured, on purpose: users behind a corporate
//!   proxy must still get updates. Requests go through it as `CONNECT host:443`, so TLS
//!   stays end to end with the same certificate checks; the proxy sees the host name
//!   (and its own credentials, if the user put them in the proxy URL), nothing else.
//! - Rate limits: 429, or 403 with `x-ratelimit-remaining: 0`, is reported as "rate
//!   limited"; any other error status as `HTTP <code>`.

use std::io::{Read, Write};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use hoprs_core::{Error, ErrorKind};
use rustls::pki_types::CertificateDer;
use ureq::http::Uri;
use ureq::tls::{Certificate, RootCerts, TlsConfig, TlsProvider};
use ureq::{Agent, Proxy, ResponseExt};

use super::download::{Fetch, failed};
use super::{API_URL, Release, parse_release};

/// Connect, headers, and the whole check or `SHA256SUMS` request.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Between two reads of a download's body.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const API_LIMIT: u64 = 1024 * 1024;
const TEXT_LIMIT: u64 = 64 * 1024;
/// Far above any installer (they are tens of MB); bounds a server that never stops.
const FILE_LIMIT: u64 = 1024 * 1024 * 1024;
const USER_AGENT: &str = concat!("hoprs/", env!("CARGO_PKG_VERSION"));

/// The release API to ask. Release builds always ask GitHub; debug builds only check when
/// `HOPRS_UPDATE_URL` is set, and ask that URL (ruling 2). `cfg!` makes this a property of
/// the build, not something a release build's environment can change.
pub fn api_url() -> Option<String> {
    if cfg!(debug_assertions) {
        std::env::var("HOPRS_UPDATE_URL")
            .ok()
            .filter(|url| !url.is_empty())
    } else {
        Some(API_URL.to_owned())
    }
}

fn check_failed(detail: impl Into<String>) -> Error {
    Error::coded(
        ErrorKind::Network,
        "update-check-failed",
        "Could not check for updates",
    )
    .with_detail(detail)
}

/// A readable reason for a ureq error.
fn describe(err: ureq::Error) -> String {
    match err {
        ureq::Error::StatusCode(code) => format!("HTTP {code}"),
        other => other.to_string(),
    }
}

/// A readable reason for an error status: GitHub's rate limit is 429, or 403 with no
/// requests remaining; other 403s (blocked, forbidden) are plain errors.
fn status_detail(code: u16, remaining: Option<&str>) -> String {
    let limited = code == 429 || (code == 403 && remaining.map(str::trim) == Some("0"));
    if limited {
        format!("rate limited (HTTP {code})")
    } else {
        format!("HTTP {code}")
    }
}

/// The system's root certificates, read once per process.
pub fn system_roots() -> Vec<CertificateDer<'static>> {
    static ROOTS: OnceLock<Vec<CertificateDer<'static>>> = OnceLock::new();
    ROOTS
        .get_or_init(|| {
            let loaded = rustls_native_certs::load_native_certs();
            for error in &loaded.errors {
                tracing::warn!(%error, "a system root certificate could not be read");
            }
            loaded.certs
        })
        .clone()
}

/// The update client (see the module docs).
pub struct Client {
    agent: Agent,
    api_url: String,
    /// The API URL's host, lowercase, also allowed in download chains.
    api_host: Option<String>,
}

impl Client {
    /// A client that trusts `roots` only, through the system proxy if one is set.
    pub fn new(api_url: String, roots: Vec<CertificateDer<'static>>) -> Client {
        Client::with_proxy(api_url, roots, Proxy::try_from_env())
    }

    fn with_proxy(
        api_url: String,
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
            .http_status_as_error(true)
            .max_redirects(5)
            .save_redirect_history(true)
            .user_agent(USER_AGENT)
            .timeout_connect(Some(TIMEOUT))
            .timeout_recv_response(Some(TIMEOUT))
            .build()
            .new_agent();
        let api_host = api_url
            .parse::<Uri>()
            .ok()
            .and_then(|uri| uri.host().map(str::to_ascii_lowercase));
        Client {
            agent,
            api_url,
            api_host,
        }
    }

    /// A client that trusts the system's roots (read once per process).
    pub fn system(api_url: String) -> Client {
        Client::new(api_url, system_roots())
    }

    /// The latest stable release.
    pub fn latest(&self) -> Result<Release, Error> {
        let body = self
            .small(
                &self.api_url,
                Some("application/vnd.github+json"),
                API_LIMIT,
            )
            .map_err(check_failed)?;
        parse_release(&body)
    }

    /// An HTTPS URL on an allowed host (see the module docs), without user info.
    fn allowed(&self, uri: &Uri) -> bool {
        let Some(authority) = uri.authority() else {
            return false;
        };
        if uri.scheme_str() != Some("https") || authority.as_str().contains('@') {
            return false;
        }
        let host = authority.host().to_ascii_lowercase();
        host == "github.com"
            || host.ends_with(".githubusercontent.com")
            || self.api_host.as_deref() == Some(host.as_str())
    }

    fn check_url(&self, url: &str) -> Result<(), String> {
        let uri: Uri = url.parse().map_err(|_| format!("not a URL: {url}"))?;
        if self.allowed(&uri) {
            Ok(())
        } else {
            Err(format!("not an allowed HTTPS URL: {url}"))
        }
    }

    /// Every URL the request went through (redirects included) is allowed.
    fn check_chain(&self, response: &ureq::http::Response<ureq::Body>) -> Result<(), String> {
        let history = response.get_redirect_history().unwrap_or_default();
        match history
            .iter()
            .chain([response.get_uri()])
            .find(|uri| !self.allowed(uri))
        {
            Some(uri) => Err(format!("redirected to a host that is not allowed: {uri}")),
            None => Ok(()),
        }
    }

    /// A small body, within `limit` bytes and [`TIMEOUT`] overall; the error is a detail.
    fn small(&self, url: &str, accept: Option<&str>, limit: u64) -> Result<Vec<u8>, String> {
        self.check_url(url)?;
        let mut request = self.agent.get(url);
        if let Some(accept) = accept {
            request = request.header("Accept", accept);
        }
        // Statuses read here, to see the rate-limit header.
        let response = request
            .config()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .build()
            .call()
            .map_err(describe)?;
        self.check_chain(&response)?;
        let status = response.status();
        if !status.is_success() {
            let remaining = response
                .headers()
                .get("x-ratelimit-remaining")
                .and_then(|value| value.to_str().ok());
            return Err(status_detail(status.as_u16(), remaining));
        }
        response
            .into_body()
            .into_with_config()
            .limit(limit)
            .read_to_vec()
            .map_err(describe)
    }
}

impl Fetch for Client {
    fn text(&self, url: &str) -> Result<String, Error> {
        let body = self.small(url, None, TEXT_LIMIT).map_err(failed)?;
        String::from_utf8(body).map_err(|_| failed(format!("{url} is not text")))
    }

    fn file(
        &self,
        url: &str,
        out: &mut dyn Write,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<u64, Error> {
        self.check_url(url).map_err(failed)?;
        let response = self
            .agent
            .get(url)
            .config()
            .timeout_per_read(Some(READ_TIMEOUT))
            .build()
            .call()
            .map_err(|err| failed(describe(err)))?;
        self.check_chain(&response).map_err(failed)?;
        let total = response.body().content_length();
        if total.is_some_and(|total| total > FILE_LIMIT) {
            return Err(failed(format!(
                "the server announced {} bytes",
                total.unwrap_or_default()
            )));
        }
        let mut reader = response
            .into_body()
            .into_with_config()
            .limit(FILE_LIMIT)
            .reader();
        let io = |err: std::io::Error| failed(err.to_string());
        let mut buffer = vec![0u8; 64 * 1024];
        let mut done = 0u64;
        loop {
            let n = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(io(err)),
            };
            out.write_all(&buffer[..n]).map_err(io)?;
            done += n as u64;
            progress(done, total);
        }
        out.flush().map_err(io)?;
        match total {
            Some(total) if done != total => Err(failed(format!(
                "the body ended after {done} of {total} bytes"
            ))),
            _ => Ok(done),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_https::{Answer, answer, identity, redirect, serve};
    use crate::update::download::{self, DownloadEvent, DownloadJob};
    use crate::update::{Asset, SUMS_NAME};
    use sha2::{Digest, Sha256};
    use std::net::TcpListener;
    use std::sync::Mutex;

    #[test]
    fn hosts_allowed_in_a_chain() {
        let client = Client::new(
            "https://api.github.com/repos/a/b/releases/latest".into(),
            vec![],
        );
        let ok = |url: &str| client.check_url(url).is_ok();
        assert!(ok("https://api.github.com/x"));
        assert!(ok("https://github.com/a/b/releases/download/v1/x.deb"));
        assert!(ok("https://GitHub.com/a"));
        assert!(ok("https://objects.githubusercontent.com/x"));
        assert!(ok("https://release-assets.githubusercontent.com/x?sig=1"));
        assert!(!ok("http://github.com/a"));
        assert!(!ok("https://githubusercontent.com.evil.example/x"));
        assert!(!ok("https://evilgithubusercontent.com/x"));
        assert!(!ok("https://github.com.evil.example/x"));
        assert!(!ok("https://user@github.com/x"));
        assert!(!ok("https://example.com/x"));
        assert!(!ok("not a url"));
        assert!(!ok("/relative"));
    }

    #[test]
    fn api_url_follows_the_build() {
        if cfg!(debug_assertions) {
            // Debug: only what HOPRS_UPDATE_URL says (the test runner does not set it).
            assert_eq!(api_url(), std::env::var("HOPRS_UPDATE_URL").ok());
        } else {
            assert_eq!(api_url().as_deref(), Some(API_URL));
        }
        assert!(USER_AGENT.starts_with("hoprs/"));
    }

    fn hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn run(client: &Client, job: &DownloadJob, dir: &std::path::Path) -> Vec<DownloadEvent> {
        let mut events = Vec::new();
        download::run(client, job, dir, &mut |e| events.push(e));
        events
    }

    fn result(events: &[DownloadEvent]) -> Result<std::path::PathBuf, Error> {
        match events.last() {
            Some(DownloadEvent::Done(result)) => result.clone(),
            other => panic!("last event {other:?}"),
        }
    }

    #[test]
    fn rate_limit_only_when_github_says_so() {
        assert_eq!(status_detail(429, None), "rate limited (HTTP 429)");
        assert_eq!(status_detail(403, Some("0")), "rate limited (HTTP 403)");
        assert_eq!(status_detail(403, Some("12")), "HTTP 403");
        assert_eq!(status_detail(403, None), "HTTP 403");
        assert_eq!(status_detail(404, Some("0")), "HTTP 404");
    }

    /// Runs in the normal suite: a local server in a thread, no network, ~0.1 s.
    #[test]
    fn local_https_server_redirect_chunked_and_checksum() {
        let (config, ca) = identity();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let base = format!("https://localhost:{port}");
        const NAME: &str = "hoprs_9.9.9-1_amd64.deb";
        let good: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let bad = b"tampered installer".to_vec();
        let sums = format!("{}  {NAME}\n{}  other.msi\n", hex(&good), hex(b"x"));
        let fixture = include_str!("fixtures/latest-release.json");
        let seen = Arc::new(Mutex::new(Vec::new()));
        {
            let (good, bad) = (good.clone(), bad.clone());
            serve(
                listener,
                config,
                seen.clone(),
                move |_, path, _| match path {
                    "/latest" => Answer {
                        chunked: true,
                        ..answer("200 OK", fixture)
                    },
                    "/limited" => answer("429 Too Many Requests", "slow down"),
                    "/limited403" => {
                        let mut a = answer("403 Forbidden", "slow down");
                        a.headers.push(("X-RateLimit-Remaining".into(), "0".into()));
                        a
                    }
                    "/forbidden" => answer("403 Forbidden", "no"),
                    p if p == format!("/dl/{SUMS_NAME}") => answer("200 OK", sums.clone()),
                    p if p == format!("/dl/{NAME}") => redirect("/objects/1".into()),
                    p if p == format!("/dl/bad/{NAME}") => redirect("/objects/bad".into()),
                    p if p == format!("/dl/short/{NAME}") => redirect("/objects/short".into()),
                    p if p == format!("/dl/http/{NAME}") => {
                        redirect(format!("http://localhost:{port}/objects/1"))
                    }
                    p if p == format!("/dl/ip/{NAME}") => {
                        redirect(format!("https://127.0.0.1:{port}/objects/1"))
                    }
                    "/objects/1" => answer("200 OK", good.clone()),
                    "/objects/bad" => answer("200 OK", bad.clone()),
                    "/objects/short" => Answer {
                        cut: Some(1000),
                        ..answer("200 OK", good.clone())
                    },
                    _ => answer("404 Not Found", "no"),
                },
            );
        }

        // No proxy: a proxy set in the developer's environment must not see this server.
        let direct = |path: &str, roots: Vec<CertificateDer<'static>>| {
            Client::with_proxy(format!("{base}{path}"), roots, None)
        };
        let client = direct("/latest", vec![ca.clone()]);

        // The release JSON, chunked. Its asset URLs stay on github.com: `parse_release`
        // accepts no other host, so the downloads below name the local server directly.
        let release = client.latest().unwrap();
        assert_eq!(release.version, semver::Version::new(1, 3, 1));
        assert_eq!(release.assets.len(), 5);

        let job = |prefix: &str, size: usize| DownloadJob {
            asset: Asset {
                name: NAME.to_owned(),
                url: format!("{base}/dl/{prefix}{NAME}"),
                size: size as u64,
            },
            sums_url: format!("{base}/dl/{SUMS_NAME}"),
        };

        // Redirect, then the bytes; verified and renamed.
        let dir = tempfile::tempdir().unwrap();
        let events = run(&client, &job("", good.len()), dir.path());
        let path = result(&events).unwrap();
        assert_eq!(path, dir.path().join(NAME));
        assert_eq!(std::fs::read(&path).unwrap(), good);
        assert!(events.iter().any(|e| matches!(e, DownloadEvent::Verifying)));
        assert!(events.iter().any(|e| matches!(
            e,
            DownloadEvent::Progress {
                total: Some(300_000),
                ..
            }
        )));

        // The wrong bytes: a mismatch, and no file.
        let dir = tempfile::tempdir().unwrap();
        let err = result(&run(&client, &job("bad/", bad.len()), dir.path())).unwrap_err();
        assert_eq!(err.code, Some("update-checksum-mismatch"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);

        // A body shorter than its Content-Length; a redirect to http; a redirect to a host
        // outside the allowed ones (a valid certificate, so only the host rule stops it).
        for prefix in ["short/", "http/", "ip/"] {
            let dir = tempfile::tempdir().unwrap();
            let err = result(&run(&client, &job(prefix, good.len()), dir.path())).unwrap_err();
            assert_eq!(
                err.code,
                Some("update-download-failed"),
                "{prefix}: {err:?}"
            );
            assert_eq!(
                std::fs::read_dir(dir.path()).unwrap().count(),
                0,
                "{prefix}"
            );
        }

        // Rate limited (429, or 403 with no requests remaining); a plain 403 is not.
        for (path, detail) in [
            ("/limited", "rate limited (HTTP 429)"),
            ("/limited403", "rate limited (HTTP 403)"),
            ("/forbidden", "HTTP 403"),
        ] {
            let err = direct(path, vec![ca.clone()]).latest().unwrap_err();
            assert_eq!(err.code, Some("update-check-failed"));
            assert_eq!(err.detail.as_deref(), Some(detail), "{path}");
        }

        // An unknown certificate authority: the system roots do not know the test CA.
        let untrusted = direct("/latest", vec![]);
        assert_eq!(
            untrusted.latest().unwrap_err().code,
            Some("update-check-failed")
        );

        // What the server saw: our User-Agent, and nothing identifying.
        let seen = seen.lock().unwrap();
        assert!(seen.len() >= 8, "{seen:?}");
        for (head, _) in seen.iter() {
            assert!(head.contains("\r\nuser-agent: hoprs/"), "{head}");
            assert!(!head.contains("\r\ncookie:"), "{head}");
            assert!(!head.contains("\r\nauthorization:"), "{head}");
        }
        assert!(seen.iter().any(|(h, _)| h.starts_with("get /latest ")
            && h.contains("\r\naccept: application/vnd.github+json")));
    }
}
