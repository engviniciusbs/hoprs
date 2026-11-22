//! A local HTTPS server for tests: a test CA, a leaf for `localhost` and `127.0.0.1`, and
//! a thread that answers one request per connection from a route function. Shared by the
//! update client's and the Google Drive client's tests; no network, no fixed port.

use std::io::{BufRead, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

/// What the server recorded for each request: the head (request line and headers,
/// lowercase) and the body.
pub type Seen = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

/// One canned answer of the test server.
pub struct Answer {
    pub status: &'static str,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub chunked: bool,
    /// Sends only this many body bytes, then closes.
    pub cut: Option<usize>,
}

pub fn answer(status: &'static str, body: impl Into<Vec<u8>>) -> Answer {
    Answer {
        status,
        headers: Vec::new(),
        body: body.into(),
        chunked: false,
        cut: None,
    }
}

pub fn redirect(to: String) -> Answer {
    let mut a = answer("302 Found", Vec::new());
    a.headers.push(("Location".into(), to));
    a
}

/// A test CA and the server configuration of a leaf for `localhost` and `127.0.0.1` (the
/// latter to test the host rules alone) signed by it; also the CA's certificate, which
/// is the root a client under test trusts.
pub fn identity() -> (Arc<rustls::ServerConfig>, CertificateDer<'static>) {
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca =
        rcgen::CertifiedIssuer::self_signed(params, rcgen::KeyPair::generate().unwrap()).unwrap();
    let params =
        rcgen::CertificateParams::new(vec!["localhost".to_owned(), "127.0.0.1".to_owned()])
            .unwrap();
    let key = rcgen::KeyPair::generate().unwrap();
    let leaf = params.signed_by(&key, &ca).unwrap().der().clone();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![leaf],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
    )
    .unwrap();
    (Arc::new(config), ca.der().clone())
}

/// Serves `route(method, path_and_query, body)` over TLS on `listener`, one request per
/// connection, recording each request in `seen`. A request body is read up to its
/// `Content-Length`.
pub fn serve(
    listener: TcpListener,
    config: Arc<rustls::ServerConfig>,
    seen: Seen,
    route: impl Fn(&str, &str, &[u8]) -> Answer + Send + 'static,
) {
    std::thread::spawn(move || {
        for tcp in listener.incoming() {
            let Ok(tcp) = tcp else { continue };
            let Ok(conn) = rustls::ServerConnection::new(config.clone()) else {
                continue;
            };
            let mut tls = rustls::StreamOwned::new(conn, tcp);
            let mut head = String::new();
            let mut body = Vec::new();
            {
                let mut reader = std::io::BufReader::new(&mut tls);
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    if line == "\r\n" {
                        break;
                    }
                    head.push_str(&line);
                }
                let length = head
                    .to_ascii_lowercase()
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:")?.trim().parse().ok())
                    .unwrap_or(0u64);
                if length > 0 {
                    let _ = reader.take(length).read_to_end(&mut body);
                }
            }
            if head.is_empty() {
                continue; // The client refused the handshake.
            }
            let mut parts = head.split(' ');
            let method = parts.next().unwrap_or("").to_owned();
            let path = parts.next().unwrap_or("/").to_owned();
            seen.lock()
                .unwrap()
                .push((head.to_ascii_lowercase(), body.clone()));
            let a = route(&method, &path, &body);
            let mut out = format!("HTTP/1.1 {}\r\nConnection: close\r\n", a.status);
            for (name, value) in &a.headers {
                out.push_str(&format!("{name}: {value}\r\n"));
            }
            let mut bytes;
            if a.chunked {
                out.push_str("Transfer-Encoding: chunked\r\n\r\n");
                bytes = out.into_bytes();
                for chunk in a.body.chunks(100) {
                    bytes.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
                    bytes.extend_from_slice(chunk);
                    bytes.extend_from_slice(b"\r\n");
                }
                bytes.extend_from_slice(b"0\r\n\r\n");
            } else {
                out.push_str(&format!("Content-Length: {}\r\n\r\n", a.body.len()));
                bytes = out.into_bytes();
                bytes.extend_from_slice(&a.body[..a.cut.unwrap_or(a.body.len())]);
            }
            let _ = tls.write_all(&bytes);
            tls.conn.send_close_notify();
            let _ = tls.flush();
        }
    });
}
