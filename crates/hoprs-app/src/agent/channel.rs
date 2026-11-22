//! The private local channel between `hoprs mcp` (the relay) and the open hoprs app.
//!
//! **Only the same user, only this machine.** On Linux and macOS it is a Unix domain
//! socket, `agent.sock`, in the directory whose `instance.lock` this run holds: the
//! directory is `0700` and owned by this user before the socket exists, the socket is
//! `0600`, and the app drops, without a word, a peer whose user id is not its own (see
//! `unix`). On Windows it is a named pipe whose name carries the user's SID, created
//! first (another process cannot squat the name) with a security descriptor that admits
//! that user only, and that refuses remote clients (see `windows`). Nothing here ever
//! listens on a network socket.
//!
//! What travels on it is in [`super::proto`]; the app's side is [`super::server`].

use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as sys;
#[cfg(windows)]
use windows as sys;

/// The socket's file name in its directory (Unix).
#[cfg(unix)]
pub const SOCKET_NAME: &str = "agent.sock";

/// Debug builds only: the directory that replaces the channel's for both halves, so
/// tests never meet the author's running hoprs.
const DIR_OVERRIDE: &str = "HOPRS_AGENT_DIR";

/// Where the channel is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    /// A Unix domain socket at this path.
    #[cfg(unix)]
    Socket(PathBuf),
    /// A named pipe of this name (`\\.\pipe\…`).
    #[cfg(windows)]
    Pipe(String),
}

/// The channel of a run whose directory is `dir`: `dir/agent.sock` on Unix; on Windows
/// `\\.\pipe\hoprs-agent-<SID>` for the standard directory, and that name plus `-` and
/// the first 16 hex digits of the SHA-256 of `dir` for any other one (so channels in
/// different directories never meet).
pub fn endpoint_in(dir: &Path) -> Endpoint {
    #[cfg(unix)]
    {
        Endpoint::Socket(dir.join(SOCKET_NAME))
    }
    #[cfg(windows)]
    {
        Endpoint::Pipe(windows::pipe_name(dir))
    }
}

/// `HOPRS_AGENT_DIR` in a debug build (when set and not empty); `None` in a release
/// build, whatever the environment says.
pub fn override_dir() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var_os(DIR_OVERRIDE)
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
}

/// The channels a relay tries, in order: the override's alone when there is one;
/// otherwise on Unix the socket in each directory of the instance lock's locations (the
/// app listens in the one it locked), and on Windows the standard pipe.
// TODO(Task 13): remove
#[allow(dead_code)]
pub fn candidates() -> Vec<Endpoint> {
    if let Some(dir) = override_dir() {
        return vec![endpoint_in(&dir)];
    }
    #[cfg(unix)]
    {
        crate::instance_lock_paths()
            .iter()
            .filter_map(|lock| lock.parent())
            .map(endpoint_in)
            .collect()
    }
    #[cfg(windows)]
    {
        vec![Endpoint::Pipe(windows::standard_pipe_name())]
    }
}

/// The app's end of the channel. Dropping it stops listening (and on Unix removes the
/// socket file, if it is still the one this listener made).
pub struct Listener(sys::Listener);

impl Listener {
    /// Opens the channel at `endpoint` (see the module docs for the rules on each
    /// system). Fails rather than open anything wider than the same user.
    pub fn bind(endpoint: &Endpoint) -> io::Result<Listener> {
        match endpoint {
            #[cfg(unix)]
            Endpoint::Socket(path) => sys::Listener::bind(path).map(Listener),
            #[cfg(windows)]
            Endpoint::Pipe(name) => sys::Listener::bind(name).map(Listener),
        }
    }

    /// The next relay. A peer that is not this user never comes out of here.
    pub async fn accept(&mut self) -> io::Result<Stream> {
        self.0.accept().await.map(Stream)
    }
}

/// The relay's end: connects to the app's channel at `endpoint`, after checking that it
/// belongs to this user.
// TODO(Task 13): remove
#[allow(dead_code)]
pub async fn connect(endpoint: &Endpoint) -> io::Result<Stream> {
    match endpoint {
        #[cfg(unix)]
        Endpoint::Socket(path) => sys::connect(path).await.map(Stream),
        #[cfg(windows)]
        Endpoint::Pipe(name) => sys::connect(name).await.map(Stream),
    }
}

/// One connection of the channel, either end.
pub struct Stream(sys::Stream);

impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_differ_per_directory() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        assert_eq!(endpoint_in(a.path()), endpoint_in(a.path()));
        assert_ne!(endpoint_in(a.path()), endpoint_in(b.path()));
        #[cfg(unix)]
        assert_eq!(
            endpoint_in(a.path()),
            Endpoint::Socket(a.path().join("agent.sock"))
        );
    }

    #[test]
    fn candidates_follow_the_lock_locations() {
        let found = candidates();
        match override_dir() {
            Some(dir) => assert_eq!(found, [endpoint_in(&dir)]),
            None => {
                #[cfg(unix)]
                {
                    let dirs: Vec<Endpoint> = crate::instance_lock_paths()
                        .iter()
                        .map(|lock| endpoint_in(lock.parent().unwrap()))
                        .collect();
                    assert_eq!(found, dirs);
                }
                #[cfg(windows)]
                assert_eq!(found.len(), 1);
            }
        }
    }
}
