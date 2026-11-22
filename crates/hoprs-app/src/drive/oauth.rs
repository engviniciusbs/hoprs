//! The pure pieces of OAuth 2.0 for a desktop app: PKCE (RFC 7636, S256), the `state`,
//! the authorisation URL and the parser of the browser's callback request.
//!
//! Nothing here logs or prints a code, a verifier or a `state`.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hoprs_core::{Error, ErrorKind};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use sha2::{Digest, Sha256};

use super::config::{Endpoints, SCOPE};

/// RFC 3986 unreserved characters stay as they are; everything else is percent-encoded.
const ENCODE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// The longest request line the parser looks at (Google's codes are far shorter).
const MAX_REQUEST_LINE: usize = 4096;

/// The error of anything that goes wrong while connecting that has no better key; the
/// cause goes into the detail (never a code, state or token).
pub(super) fn connect_failed(cause: impl Into<String>) -> Error {
    Error::coded(
        ErrorKind::Network,
        "drive-connect-failed",
        "Could not connect to Google Drive",
    )
    .with_detail(cause)
}

/// `N` random bytes from the TLS library's system generator.
// TODO(task 9 or 10): the connect flow draws the verifier and the state; remove this allow then.
#[allow(dead_code)]
pub fn random<const N: usize>() -> Result<[u8; N], Error> {
    let mut bytes = [0_u8; N];
    rustls::crypto::aws_lc_rs::default_provider()
        .secure_random
        .fill(&mut bytes)
        .map_err(|_| connect_failed("no random numbers from the system"))?;
    Ok(bytes)
}

/// The PKCE code verifier: base64url without padding (43 characters for 32 bytes).
// TODO(task 9 or 10): the connect flow; remove this allow then.
#[allow(dead_code)]
pub fn verifier(random: [u8; 32]) -> String {
    URL_SAFE_NO_PAD.encode(random)
}

/// The PKCE code challenge (S256): base64url without padding of the verifier's SHA-256.
// TODO(task 9 or 10): the connect flow; remove this allow then.
#[allow(dead_code)]
pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// The `state` value that ties the callback to this attempt.
// TODO(task 9 or 10): the connect flow; remove this allow then.
#[allow(dead_code)]
pub fn state_token(random: [u8; 16]) -> String {
    URL_SAFE_NO_PAD.encode(random)
}

/// The URL the browser is opened at.
// TODO(task 9 or 10): the connect flow; remove this allow then.
#[allow(dead_code)]
pub fn auth_url(
    endpoints: &Endpoints,
    client_id: &str,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
) -> String {
    let params = [
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("response_type", "code"),
        ("scope", SCOPE),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
    ];
    let query = params
        .iter()
        .map(|(name, value)| format!("{name}={}", utf8_percent_encode(value, ENCODE)))
        .collect::<Vec<_>>()
        .join("&");
    let joiner = if endpoints.auth.contains('?') {
        '&'
    } else {
        '?'
    };
    format!("{}{joiner}{query}", endpoints.auth)
}

/// What a request to the loopback listener means.
#[derive(Debug, PartialEq, Eq)]
// TODO(task 9 or 10): the connect flow; remove this allow then.
#[allow(dead_code)]
pub enum Callback {
    /// Google sent the authorisation code.
    Code(String),
    /// Google (or the user) refused; the value is Google's `error`.
    Denied(String),
    /// Anything else: a stray request.
    Other,
}

/// Equal bytes, compared without stopping at the first difference (lengths may leak).
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0_u8, |diff, (x, y)| diff | (x ^ y))
            == 0
}

/// Reads the request `head` (the request up to the blank line). Only `GET /?...` with a
/// `state` equal to `state` is a `Code` (non-empty, percent-decoded) or a `Denied`.
// TODO(task 9 or 10): the listener's `wait`, itself unused until then; remove this allow then.
#[allow(dead_code)]
pub fn parse_callback(head: &[u8], state: &str) -> Callback {
    let line = head.split(|&b| b == b'\n').next().unwrap_or_default();
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.len() > MAX_REQUEST_LINE {
        return Callback::Other;
    }
    let Ok(line) = std::str::from_utf8(line) else {
        return Callback::Other;
    };
    let mut parts = line.split(' ');
    let (Some("GET"), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Callback::Other;
    };
    let Some(query) = target.strip_prefix("/?") else {
        return Callback::Other;
    };
    if !version.starts_with("HTTP/") {
        return Callback::Other;
    }
    // The first occurrence of each name counts.
    let value = |wanted: &str| -> Option<String> {
        query.split('&').find_map(|pair| {
            let (name, raw) = pair.split_once('=')?;
            if name != wanted {
                return None;
            }
            percent_decode_str(raw)
                .decode_utf8()
                .ok()
                .map(|v| v.into_owned())
        })
    };
    match value("state") {
        Some(got) if same(&got, state) => {}
        _ => return Callback::Other,
    }
    if let Some(code) = value("code").filter(|code| !code.is_empty()) {
        return Callback::Code(code);
    }
    match value("error") {
        Some(error) if !error.is_empty() => Callback::Denied(error),
        _ => Callback::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(query: &str) -> Vec<u8> {
        format!("GET /?{query} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").into_bytes()
    }

    #[test]
    fn pkce_matches_rfc_7636() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let v = verifier([0; 32]);
        assert_eq!(v.len(), 43);
        assert!(
            v.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        );
    }

    #[test]
    fn state_and_verifier_differ_each_time() {
        assert_ne!(random::<32>().unwrap(), random::<32>().unwrap());
        assert_ne!(random::<16>().unwrap(), random::<16>().unwrap());
        assert_eq!(state_token([0; 16]).len(), 22);
    }

    #[test]
    fn auth_url_has_exactly_the_parameters() {
        let endpoints = super::super::config::google();
        let url = auth_url(
            &endpoints,
            "id &x=1 ü",
            "http://127.0.0.1:49152",
            "chal-_",
            "st",
        );
        let (base, query) = url.split_once('?').unwrap();
        assert_eq!(base, endpoints.auth);
        let pairs: Vec<(String, String)> = query
            .split('&')
            .map(|p| {
                let (k, v) = p.split_once('=').unwrap();
                (
                    k.to_owned(),
                    percent_decode_str(v).decode_utf8().unwrap().into_owned(),
                )
            })
            .collect();
        let expected = [
            ("client_id", "id &x=1 ü"),
            ("redirect_uri", "http://127.0.0.1:49152"),
            ("response_type", "code"),
            ("scope", SCOPE),
            ("code_challenge", "chal-_"),
            ("code_challenge_method", "S256"),
            ("state", "st"),
        ];
        assert_eq!(pairs.len(), expected.len());
        for (k, v) in expected {
            assert!(
                pairs.iter().any(|(pk, pv)| pk == k && pv == v),
                "missing {k}"
            );
        }
        assert!(!query.contains(' ') && !query.contains("&x=1"));
    }

    #[test]
    fn callback_parser() {
        let s = "right-state";
        assert_eq!(
            parse_callback(&get("code=abc&state=right-state"), s),
            Callback::Code("abc".into())
        );
        assert_eq!(
            parse_callback(&get("state=right-state&code=a%2Fb"), s),
            Callback::Code("a/b".into())
        );
        for query in [
            "code=abc&state=wrong",
            "code=abc",
            "code=abc&state=right-stat",
            "code=abc&state=right-state2",
            "code=&state=right-state",
            "state=right-state",
        ] {
            assert_eq!(parse_callback(&get(query), s), Callback::Other, "{query}");
        }
        assert_eq!(
            parse_callback(&get("error=access_denied&state=right-state"), s),
            Callback::Denied("access_denied".into())
        );
        assert_eq!(
            parse_callback(&get("error=access_denied&state=nope"), s),
            Callback::Other
        );
        let others: [Vec<u8>; 5] = [
            b"GET /favicon.ico HTTP/1.1\r\n\r\n".to_vec(),
            b"POST /?code=abc&state=right-state HTTP/1.1\r\n\r\n".to_vec(),
            b"GET /?code=\xff\xfe&state=right-state HTTP/1.1\r\n\r\n".to_vec(),
            Vec::new(),
            format!(
                "GET /?code={}&state=right-state HTTP/1.1\r\n\r\n",
                "a".repeat(10_000)
            )
            .into_bytes(),
        ];
        for head in others {
            assert_eq!(parse_callback(&head, s), Callback::Other);
        }
        let long = vec![b'a'; 10_000];
        assert_eq!(parse_callback(&long, s), Callback::Other);
    }
}
