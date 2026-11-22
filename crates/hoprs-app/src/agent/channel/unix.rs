//! The channel on Linux and macOS: a Unix domain socket in a directory only this user
//! can enter.
//!
//! **The order that keeps it closed.** `bind` first makes the directory exist, be owned
//! by this user (`geteuid`) and be `0700` (it is created `0700`, and an existing one is
//! closed to `0700`; one owned by another user, or one that stays open, is refused). Only
//! then is the socket bound, so even while the socket's own mode is still what the umask
//! gave it, no other user can reach it: they cannot enter the directory. It is then set
//! to `0600`. A socket left by a crash is replaced (this run holds the instance lock, so
//! nobody listens on it); anything else at that path (a file, a directory, a link, a
//! socket of another user) is never deleted or bound over: `bind` fails instead.
//!
//! The directory itself may not be a symbolic link (on either side): its owner and mode
//! are those of the directory the socket really is in.
//!
//! `accept` drops, without answering, a peer whose user id is not this process's.
//! `connect` (the relay) refuses a directory or a socket that another user owns, and,
//! once connected, a peer that is not this user: the checks by name and the connect are
//! separate lookups, so the peer itself is what decides.

use std::fs::{self, DirBuilder, Permissions};
use std::io::{Error, ErrorKind, Result};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use tokio::net::{UnixListener, UnixStream};

pub(super) type Stream = UnixStream;

/// This process's effective user id.
fn me() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// Whether a directory owned by `owner` with mode bits `mode` may hold the channel of
/// user `me`: it is `me`'s and closed to group and others.
fn usable(owner: u32, mode: u32, me: u32) -> bool {
    owner == me && mode & 0o077 == 0
}

/// Whether the other end of a connection, whose user id the system reported as
/// `peer`, may be talked to by user `me`: only the same user, and never an end whose
/// user id could not be read.
fn same_user(peer: Result<u32>, me: u32) -> bool {
    matches!(peer, Ok(uid) if uid == me)
}

fn peer_uid(stream: &UnixStream) -> Result<u32> {
    stream.peer_cred().map(|cred| cred.uid())
}

/// The directory's own metadata, refusing a symbolic link in its place.
fn dir_metadata(dir: &Path) -> Result<fs::Metadata> {
    let meta = fs::symlink_metadata(dir)?;
    if meta.file_type().is_symlink() {
        return Err(refused("the channel's directory is a symbolic link"));
    }
    Ok(meta)
}

fn refused(what: &str) -> Error {
    Error::new(ErrorKind::PermissionDenied, what.to_owned())
}

/// Makes `dir` exist, be this user's and be `0700`; refuses it otherwise.
fn secure_dir(dir: &Path) -> Result<()> {
    // Created closed: the mode is 0700 minus the umask, never more.
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let meta = dir_metadata(dir)?;
    if !meta.is_dir() {
        return Err(Error::new(
            ErrorKind::NotADirectory,
            "the channel's directory is not a directory",
        ));
    }
    if meta.uid() != me() {
        return Err(refused("the channel's directory belongs to another user"));
    }
    if meta.mode() & 0o777 != 0o700 {
        fs::set_permissions(dir, Permissions::from_mode(0o700))?;
    }
    let meta = dir_metadata(dir)?;
    if !usable(meta.uid(), meta.mode(), me()) {
        return Err(refused("the channel's directory is open to other users"));
    }
    Ok(())
}

/// Clears the way for a new socket at `path`: nothing there, or a socket of this user
/// left by an earlier run (removed). Anything else is refused and left as it is.
fn clear_stale(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
        Ok(meta) if meta.file_type().is_socket() && meta.uid() == me() => {
            match fs::remove_file(path) {
                Err(err) if err.kind() != ErrorKind::NotFound => Err(err),
                _ => Ok(()),
            }
        }
        Ok(_) => Err(Error::new(
            ErrorKind::AlreadyExists,
            "something that is not this user's socket is in the channel's place",
        )),
    }
}

/// The listening socket and what it made.
#[derive(Debug)]
pub(super) struct Listener {
    listener: UnixListener,
    path: PathBuf,
    /// Device and inode of the socket file it bound: `Drop` removes only that one.
    identity: (u64, u64),
}

impl Listener {
    pub(super) fn bind(path: &Path) -> Result<Listener> {
        let dir = path
            .parent()
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "the channel has no directory"))?;
        secure_dir(dir)?;
        clear_stale(path)?;
        let listener = UnixListener::bind(path)?;
        let meta = fs::symlink_metadata(path)?;
        let identity = (meta.dev(), meta.ino());
        let made = Listener {
            listener,
            path: path.to_owned(),
            identity,
        };
        // From here on a failure drops `made`, which removes the socket.
        fs::set_permissions(path, Permissions::from_mode(0o600))?;
        Ok(made)
    }

    pub(super) async fn accept(&mut self) -> Result<Stream> {
        loop {
            let (stream, _) = self.listener.accept().await?;
            if same_user(peer_uid(&stream), me()) {
                return Ok(stream);
            }
            // Not this user (or unknown): dropped without a word.
            tracing::warn!("refused a connection to the agent channel from another user");
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let ours = fs::symlink_metadata(&self.path)
            .is_ok_and(|meta| (meta.dev(), meta.ino()) == self.identity);
        if ours {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// The relay's side: the directory and the socket must be this user's.
pub(super) async fn connect(path: &Path) -> Result<Stream> {
    let dir = path
        .parent()
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "the channel has no directory"))?;
    if dir_metadata(dir)?.uid() != me() {
        return Err(refused("the channel's directory belongs to another user"));
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_socket() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "the channel's place holds something that is not a socket",
        ));
    }
    if meta.uid() != me() {
        return Err(refused("the channel's socket belongs to another user"));
    }
    let stream = UnixStream::connect(path).await?;
    if !same_user(peer_uid(&stream), me()) {
        return Err(refused("the agent channel is held by another user"));
    }
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    const WAIT: Duration = Duration::from_secs(10);

    fn mode(path: &Path) -> u32 {
        fs::symlink_metadata(path).unwrap().mode() & 0o777
    }

    async fn talk(listener: &mut Listener, path: &Path) {
        let client = tokio::time::timeout(WAIT, connect(path));
        let server = tokio::time::timeout(WAIT, listener.accept());
        let (client, server) = tokio::join!(client, server);
        let (mut client, mut server) = (client.unwrap().unwrap(), server.unwrap().unwrap());
        client.write_all(b"ping\n").await.unwrap();
        let mut got = [0u8; 5];
        tokio::time::timeout(WAIT, server.read_exact(&mut got))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&got, b"ping\n");
    }

    #[tokio::test]
    async fn socket_and_directory_modes() {
        let base = tempfile::tempdir().unwrap();
        // Missing, wide open, and closed already: always 0700 and 0600 after.
        for before in [None, Some(0o777), Some(0o755), Some(0o700)] {
            let dir = base.path().join(format!("d{}", before.unwrap_or(0)));
            if let Some(before) = before {
                fs::create_dir(&dir).unwrap();
                fs::set_permissions(&dir, Permissions::from_mode(before)).unwrap();
            }
            let path = dir.join("agent.sock");
            let listener = Listener::bind(&path).unwrap();
            assert_eq!(mode(&dir), 0o700, "{before:?}");
            assert_eq!(mode(&path), 0o600, "{before:?}");
            assert!(fs::symlink_metadata(&path).unwrap().file_type().is_socket());
            drop(listener);
        }
    }

    #[test]
    fn directory_rule() {
        let me = 1000;
        assert!(usable(me, 0o700, me));
        assert!(usable(me, 0o40700, me), "the file type bits do not count");
        assert!(!usable(me + 1, 0o700, me), "another owner");
        assert!(!usable(me, 0o750, me));
        assert!(!usable(me, 0o701, me));
        assert!(!usable(me, 0o707, me));
    }

    #[test]
    fn peer_rule() {
        assert!(same_user(Ok(1000), 1000));
        assert!(!same_user(Ok(1001), 1000), "another user");
        assert!(!same_user(Ok(0), 1000), "root is another user too");
        assert!(
            !same_user(Err(Error::other("no credentials")), 1000),
            "unknown"
        );
    }

    #[tokio::test]
    async fn a_symlinked_directory_is_refused() {
        let base = tempfile::tempdir().unwrap();
        let real = base.path().join("real");
        fs::create_dir(&real).unwrap();
        fs::set_permissions(&real, Permissions::from_mode(0o700)).unwrap();
        let link = base.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let err = Listener::bind(&link.join("agent.sock")).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied, "{err}");
        assert!(
            fs::read_dir(&real).unwrap().next().is_none(),
            "nothing made"
        );
        // The relay refuses it too, even with a socket behind it.
        let _listener = Listener::bind(&real.join("agent.sock")).unwrap();
        let err = connect(&link.join("agent.sock")).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied, "{err}");
    }

    #[tokio::test]
    async fn a_path_too_long_fails_cleanly() {
        let base = tempfile::tempdir().unwrap();
        // Over `sun_path` (108 bytes on Linux, 104 on macOS) whatever the base.
        let dir = base.path().join("d".repeat(120));
        let path = dir.join("agent.sock");
        assert!(Listener::bind(&path).is_err());
        assert!(fs::symlink_metadata(&path).is_err(), "no socket left");
        assert!(connect(&path).await.is_err());
    }

    #[tokio::test]
    async fn stale_socket_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.sock");
        // A socket bound and dropped without removing its file (a crash).
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        let mut listener = Listener::bind(&path).unwrap();
        talk(&mut listener, &path).await;
        drop(listener);

        // A leftover socket file of a listener that was never dropped (its process died).
        let leftover = tokio::net::UnixListener::bind(&path).unwrap();
        std::mem::forget(leftover);
        let mut listener = Listener::bind(&path).unwrap();
        talk(&mut listener, &path).await;
    }

    #[tokio::test]
    async fn what_is_not_a_socket_is_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.sock");
        fs::write(&path, b"keep me").unwrap();
        assert!(Listener::bind(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"keep me");
        fs::remove_file(&path).unwrap();

        fs::create_dir(&path).unwrap();
        assert!(Listener::bind(&path).is_err());
        assert!(path.is_dir());
        fs::remove_dir(&path).unwrap();

        // A link to elsewhere: neither the link nor its target is touched.
        let target = dir.path().join("target");
        fs::write(&target, b"t").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(Listener::bind(&path).is_err());
        assert!(
            fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&target).unwrap(), b"t");
    }

    #[tokio::test]
    async fn a_directory_of_another_user_is_refused() {
        // Root's directory: never ours unless the tests run as root.
        if me() == 0 {
            return;
        }
        let path = Path::new("/").join("agent.sock");
        let err = Listener::bind(&path).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied, "{err}");
        let err = connect(&path).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied, "{err}");
    }

    #[tokio::test]
    async fn drop_removes_the_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.sock");
        let listener = Listener::bind(&path).unwrap();
        assert!(path.exists());
        drop(listener);
        assert!(fs::symlink_metadata(&path).is_err());
        assert!(dir.path().is_dir(), "the directory stays");
    }

    #[tokio::test]
    async fn drop_leaves_a_newer_socket_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.sock");
        let old = Listener::bind(&path).unwrap();
        let mut new = Listener::bind(&path).unwrap();
        drop(old);
        talk(&mut new, &path).await;
    }

    #[tokio::test]
    async fn connect_refuses_a_missing_socket() {
        let dir = tempfile::tempdir().unwrap();
        let err = connect(&dir.path().join("agent.sock")).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        let err = connect(&dir.path().join("missing").join("agent.sock"))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        // A file in its place is not a socket.
        fs::write(dir.path().join("agent.sock"), b"").unwrap();
        assert!(connect(&dir.path().join("agent.sock")).await.is_err());
    }

    #[tokio::test]
    async fn same_user_connects() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.sock");
        let mut listener = Listener::bind(&path).unwrap();
        let (client, server) = tokio::join!(
            tokio::time::timeout(WAIT, connect(&path)),
            tokio::time::timeout(WAIT, listener.accept())
        );
        let (_client, server) = (client.unwrap().unwrap(), server.unwrap().unwrap());
        assert_eq!(server.peer_cred().unwrap().uid(), me());
    }
}
