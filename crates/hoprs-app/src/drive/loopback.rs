//! The one-shot HTTP listener the browser comes back to after Google's consent screen.
//!
//! It binds `127.0.0.1` only, serves nothing but two small static pages and never logs
//! or echoes the code, the `state` or the query string. Only a request that carries the
//! right `state` ends the wait; every other request gets a 400 and the wait goes on.

use std::io::{ErrorKind as IoKind, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use hoprs_core::{Error, ErrorKind};
use secrecy::SecretString;

use super::oauth::{Callback, connect_failed, parse_callback};

/// How long the browser has to come back.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5 * 60);

const POLL: Duration = Duration::from_millis(50);
const READ_LIMIT: usize = 8 * 1024;
const READ_TIME: Duration = Duration::from_secs(2);
const DENIED_DETAIL_MAX: usize = 100;

/// Whether an `accept` error leaves the listener usable. A connection that fell over
/// before it was taken (reset, aborted, protocol errors) says nothing about the next one,
/// and any local process can cause it, so only errors of a broken listener are fatal.
fn retryable(kind: IoKind) -> bool {
    !matches!(
        kind,
        IoKind::InvalidInput
            | IoKind::NotConnected
            | IoKind::PermissionDenied
            | IoKind::Unsupported
            | IoKind::OutOfMemory
    )
}

/// The texts of the two pages the browser sees (plain text; the module wraps them).
pub struct Pages {
    pub connected: String,
    pub failed: String,
}

/// A listener on a free port of `127.0.0.1`.
pub struct Listener {
    inner: TcpListener,
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// A small static page: no script, no external resource.
fn page(text: &str) -> String {
    format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>hoprs</title></head>\
         <body><p>{}</p></body></html>\n",
        escape(text)
    )
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Cache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    // The browser may already be gone; nothing to do about it.
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);
}

/// Reads the request head (up to the blank line, at most 8 KiB, at most 2 s in all).
/// `None` when it did not arrive complete.
fn read_head(stream: &mut TcpStream) -> Option<Vec<u8>> {
    let deadline = Instant::now() + READ_TIME;
    let mut buf = Vec::with_capacity(512);
    let mut chunk = [0_u8; 1024];
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            buf.truncate(end + 4);
            return Some(buf);
        }
        if buf.len() >= READ_LIMIT {
            return None;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        stream.set_read_timeout(Some(left)).ok()?;
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return None,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
}

fn handle(
    mut stream: TcpStream,
    state: &str,
    pages: &Pages,
) -> Option<Result<SecretString, Error>> {
    // Accepted sockets may inherit the listener's non-blocking mode.
    stream.set_nonblocking(false).ok()?;
    let Some(head) = read_head(&mut stream) else {
        respond(&mut stream, "400 Bad Request", &page(&pages.failed));
        return None;
    };
    match parse_callback(&head, state) {
        Callback::Code(code) => {
            respond(&mut stream, "200 OK", &page(&pages.connected));
            Some(Ok(SecretString::from(code)))
        }
        Callback::Denied(reason) => {
            respond(&mut stream, "200 OK", &page(&pages.failed));
            let reason: String = reason.chars().take(DENIED_DETAIL_MAX).collect();
            Some(Err(Error::coded(
                ErrorKind::Auth,
                "drive-connect-denied",
                "Access to Google Drive was not granted",
            )
            .with_detail(reason)))
        }
        Callback::Other => {
            respond(&mut stream, "400 Bad Request", &page(&pages.failed));
            None
        }
    }
}

impl Listener {
    /// Binds `127.0.0.1` on a free port (never another address).
    pub fn bind() -> Result<Listener, Error> {
        let inner = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|e| connect_failed(format!("cannot listen on 127.0.0.1: {e}")))?;
        inner
            .set_nonblocking(true)
            .map_err(|e| connect_failed(format!("cannot set the listener up: {e}")))?;
        Ok(Listener { inner })
    }

    /// The address Google sends the browser back to.
    pub fn redirect_uri(&self) -> String {
        let port = self.inner.local_addr().map(|a| a.port()).unwrap_or(0);
        format!("http://127.0.0.1:{port}")
    }

    /// Waits for the callback of the attempt that carries `state`; the port is released
    /// when this returns, on every path.
    pub fn wait(
        self,
        state: &str,
        cancel: &AtomicBool,
        timeout: Duration,
        pages: &Pages,
    ) -> Result<SecretString, Error> {
        let deadline = Instant::now() + timeout;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::coded(
                    ErrorKind::Io,
                    "drive-connect-cancelled",
                    "Connection to Google Drive cancelled",
                ));
            }
            if Instant::now() >= deadline {
                return Err(Error::coded(
                    ErrorKind::Network,
                    "drive-connect-timed-out",
                    "The browser did not answer in 5 minutes",
                ));
            }
            match self.inner.accept() {
                Ok((stream, _)) => {
                    if let Some(done) = handle(stream, state, pages) {
                        return done;
                    }
                }
                Err(e) if matches!(e.kind(), IoKind::WouldBlock | IoKind::Interrupted) => {
                    std::thread::sleep(POLL);
                }
                Err(e) if retryable(e.kind()) => std::thread::sleep(POLL),
                Err(e) => return Err(connect_failed(format!("accept failed: {e}"))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;
    use std::sync::Arc;

    fn pages() -> Pages {
        Pages {
            connected: "Connected <ok>".into(),
            failed: "Failed".into(),
        }
    }

    /// Sends `request` to `port`, returns everything the server answered.
    fn send(port: u16, request: &[u8]) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream.write_all(request).unwrap();
        let mut out = Vec::new();
        let _ = stream.read_to_end(&mut out);
        String::from_utf8_lossy(&out).into_owned()
    }

    fn port_of(listener: &Listener) -> u16 {
        listener
            .redirect_uri()
            .rsplit(':')
            .next()
            .unwrap()
            .parse()
            .unwrap()
    }

    fn run(
        listener: Listener,
        state: &'static str,
        timeout: Duration,
    ) -> std::thread::JoinHandle<Result<SecretString, Error>> {
        std::thread::spawn(move || listener.wait(state, &AtomicBool::new(false), timeout, &pages()))
    }

    #[test]
    fn good_callback_returns_the_code_and_the_page() {
        let listener = Listener::bind().unwrap();
        assert!(listener.redirect_uri().starts_with("http://127.0.0.1:"));
        let port = port_of(&listener);
        let waiting = run(listener, "S", Duration::from_secs(20));
        let answer = send(port, b"GET /?code=abc&state=S HTTP/1.1\r\n\r\n");
        assert_eq!(waiting.join().unwrap().unwrap().expose_secret(), "abc");
        assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
        assert!(answer.contains("Content-Type: text/html; charset=utf-8"));
        assert!(answer.contains("Cache-Control: no-store"));
        assert!(answer.contains("Connection: close"));
        assert!(answer.contains("Connected &lt;ok&gt;"));
    }

    #[test]
    fn denied_callback_fails_with_the_failed_page() {
        let listener = Listener::bind().unwrap();
        let port = port_of(&listener);
        let waiting = run(listener, "S", Duration::from_secs(20));
        let answer = send(port, b"GET /?error=access_denied&state=S HTTP/1.1\r\n\r\n");
        let err = waiting.join().unwrap().unwrap_err();
        assert_eq!(err.code, Some("drive-connect-denied"));
        assert_eq!(err.detail.as_deref(), Some("access_denied"));
        assert!(answer.starts_with("HTTP/1.1 200") && answer.contains("Failed"));
    }

    #[test]
    fn stray_requests_do_not_end_the_wait() {
        let listener = Listener::bind().unwrap();
        let port = port_of(&listener);
        let waiting = run(listener, "S", Duration::from_secs(30));
        for request in [
            &b"GET /favicon.ico HTTP/1.1\r\n\r\n"[..],
            b"GET /?code=abc&state=WRONG HTTP/1.1\r\n\r\n",
            b"junk\n",
        ] {
            let answer = send(port, request);
            assert!(
                answer.is_empty() || answer.starts_with("HTTP/1.1 400"),
                "{answer}"
            );
        }
        // A connection that sends nothing is dropped after the read time-out.
        let silent = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let answer = send(port, b"GET /?code=abc&state=S HTTP/1.1\r\n\r\n");
        assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
        drop(silent);
        assert_eq!(waiting.join().unwrap().unwrap().expose_secret(), "abc");
    }

    #[test]
    fn cancel_and_timeout_free_the_port() {
        let listener = Listener::bind().unwrap();
        let port = port_of(&listener);
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        let started = Instant::now();
        let waiting = std::thread::spawn(move || {
            listener.wait("S", &flag, Duration::from_secs(30), &pages())
        });
        std::thread::sleep(Duration::from_millis(100));
        cancel.store(true, Ordering::Relaxed);
        let err = waiting.join().unwrap().unwrap_err();
        assert!(started.elapsed() < Duration::from_millis(500) + Duration::from_millis(100));
        assert_eq!(err.code, Some("drive-connect-cancelled"));
        TcpListener::bind(("127.0.0.1", port)).unwrap();

        let listener = Listener::bind().unwrap();
        let port = port_of(&listener);
        let err = listener
            .wait(
                "S",
                &AtomicBool::new(false),
                Duration::from_millis(200),
                &pages(),
            )
            .unwrap_err();
        assert_eq!(err.code, Some("drive-connect-timed-out"));
        TcpListener::bind(("127.0.0.1", port)).unwrap();
    }

    #[test]
    fn connections_that_fall_over_do_not_end_the_wait() {
        // std cannot set SO_LINGER on stable, so a hard reset cannot be forced here;
        // the closest reliable cases are a connect-and-close and a half-sent request.
        let listener = Listener::bind().unwrap();
        let port = port_of(&listener);
        let waiting = run(listener, "S", Duration::from_secs(30));
        drop(TcpStream::connect(("127.0.0.1", port)).unwrap());
        let mut half = TcpStream::connect(("127.0.0.1", port)).unwrap();
        half.write_all(b"GET /?code=abc&sta").unwrap();
        drop(half);
        let answer = send(port, b"GET /?code=abc&state=S HTTP/1.1\r\n\r\n");
        assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
        assert_eq!(waiting.join().unwrap().unwrap().expose_secret(), "abc");
    }

    #[test]
    fn only_a_broken_listener_ends_the_wait() {
        for kind in [
            IoKind::ConnectionReset,
            IoKind::ConnectionAborted,
            IoKind::WouldBlock,
            IoKind::Interrupted,
            IoKind::TimedOut,
            IoKind::Other,
            IoKind::HostUnreachable,
            IoKind::NetworkDown,
        ] {
            assert!(retryable(kind), "{kind:?}");
        }
        for kind in [
            IoKind::InvalidInput,
            IoKind::NotConnected,
            IoKind::PermissionDenied,
        ] {
            assert!(!retryable(kind), "{kind:?}");
        }
    }

    #[test]
    fn the_returned_code_is_not_in_debug_output() {
        let listener = Listener::bind().unwrap();
        let port = port_of(&listener);
        let waiting = run(listener, "S", Duration::from_secs(20));
        send(port, b"GET /?code=abc123secret&state=S HTTP/1.1\r\n\r\n");
        let result = waiting.join().unwrap();
        assert!(!format!("{result:?}").contains("abc123secret"));
        assert_eq!(result.unwrap().expose_secret(), "abc123secret");
    }

    #[test]
    fn pages_escape_their_text() {
        let html = page("<b>&\"'");
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("&lt;b&gt;&amp;&quot;&#39;"));
        assert!(!html.contains("<b>") && !html.contains("<script"));
    }
}
