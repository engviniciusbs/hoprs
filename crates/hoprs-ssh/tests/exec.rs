//! Exec channel against the container in `docker/compose.yml`.

use std::sync::Arc;
use std::time::Duration;

use hoprs_core::ErrorKind;
use hoprs_ssh::testing::{fixture, test_config};
use hoprs_ssh::{
    ExecEnd, ExecLimits, KeySource, Prompter, PtyRequest, ScriptedPrompter, SshConfig, SshSession,
};
use tempfile::TempDir;
use tokio::time::Instant;

const LIMITS: ExecLimits = ExecLimits {
    stream_bytes: 256 * 1024,
    timeout: Duration::from_secs(10),
};

fn alice(dir: &TempDir) -> SshConfig {
    let mut cfg = test_config("alice", dir.path());
    cfg.auth.key = KeySource::File {
        path: fixture("id_ed25519"),
        passphrase: None,
    };
    cfg
}

async fn connect(cfg: SshConfig) -> SshSession {
    let p = Arc::new(ScriptedPrompter::new().accept_host(true));
    SshSession::connect(cfg, p as Arc<dyn Prompter>)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn exec_returns_output_and_exit_code() {
    let dir = TempDir::new().unwrap();
    let session = connect(alice(&dir)).await;
    let out = session
        .exec("printf out; printf err >&2; exit 3", LIMITS)
        .await
        .unwrap();
    assert_eq!(out.stdout, b"out");
    assert_eq!(out.stderr, b"err");
    assert_eq!(out.end, ExecEnd::Exit(3));
    assert!(!out.stdout_truncated && !out.stderr_truncated);
    assert_eq!(session.open_channels(), 0, "the slot is released");
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn exec_has_no_stdin_and_no_terminal() {
    let dir = TempDir::new().unwrap();
    let session = connect(alice(&dir)).await;
    // `cat` ends at once because stdin is at end of input; stdout is not a terminal.
    let out = tokio::time::timeout(
        Duration::from_secs(5),
        session.exec("cat; test -t 1 || echo notty", LIMITS),
    )
    .await
    .expect("cat waited for input")
    .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "notty");
    assert_eq!(out.end, ExecEnd::Exit(0));
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn exec_time_limit_closes_the_channel() {
    let dir = TempDir::new().unwrap();
    let session = connect(alice(&dir)).await;
    let before = session.open_channels();
    let limits = ExecLimits {
        timeout: Duration::from_secs(1),
        ..LIMITS
    };
    let start = Instant::now();
    let out = session.exec("sleep 30", limits).await.unwrap();
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(out.end, ExecEnd::TimedOut);
    assert_eq!(session.open_channels(), before);
    // The connection is still good.
    let again = session.exec("echo ok", LIMITS).await.unwrap();
    assert_eq!(again.stdout, b"ok\n");
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn exec_large_output_is_capped() {
    let dir = TempDir::new().unwrap();
    let session = connect(alice(&dir)).await;
    let out = session
        .exec("head -c 1000000 /dev/zero | tr '\\0' x", LIMITS)
        .await
        .unwrap();
    assert_eq!(out.stdout.len(), 262_144);
    assert!(out.stdout_truncated);
    assert!(!out.stderr_truncated);
    assert_eq!(out.end, ExecEnd::Exit(0));
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn exec_counts_against_the_channel_budget() {
    // As `too_many_channels_is_a_clear_error`: a limit of 3 takes two channels (the last
    // slot is never a PTY's, nor an exec's).
    let dir = TempDir::new().unwrap();
    let mut cfg = alice(&dir);
    cfg.max_channels = 3;
    let session = connect(cfg).await;
    let first = session.open_shell(PtyRequest::default()).await.unwrap();
    let second = session.open_shell(PtyRequest::default()).await.unwrap();
    let start = Instant::now();
    let e = session.exec("echo hi", LIMITS).await.unwrap_err();
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "the refusal waited"
    );
    assert_eq!(e.kind, ErrorKind::Protocol, "{e:?}");
    assert!(hoprs_ssh::is_channel_limit(&e), "{e:?}");
    // The shells are unaffected, and a freed slot serves an exec (the shell gives its
    // slot back when its pump ends, a moment after the close).
    let (handle, _output) = first.into_output();
    handle.close();
    let deadline = Instant::now() + Duration::from_secs(2);
    while session.open_channels() > 1 {
        assert!(Instant::now() < deadline, "the slot was never freed");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let out = session.exec("echo hi", LIMITS).await.unwrap();
    assert_eq!(out.stdout, b"hi\n");
    drop(second);
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn exec_when_not_connected_fails_at_once() {
    let dir = TempDir::new().unwrap();
    let session = connect(alice(&dir)).await;
    session.disconnect().await;
    let start = Instant::now();
    let e = session.exec("echo hi", LIMITS).await.unwrap_err();
    assert!(start.elapsed() < Duration::from_millis(500));
    assert_eq!(e.kind, ErrorKind::Network);
    assert_eq!(e.message, "Not connected", "{e:?}");
}

#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d"]
async fn dropping_exec_frees_the_channel() {
    let dir = TempDir::new().unwrap();
    let session = connect(alice(&dir)).await;
    let before = session.open_channels();
    let call = session.exec("sleep 30", LIMITS);
    // Cancelled while the command runs.
    assert!(
        tokio::time::timeout(Duration::from_millis(500), call)
            .await
            .is_err()
    );
    assert_eq!(session.open_channels(), before, "the slot is free at once");
    // The server closed the channel too, so its own MaxSessions (10) is not used up. A
    // command that keeps writing dies of the closed pipe (a silent one, like the `sleep`
    // above, would keep its server-side channel until it ends).
    for _ in 0..12 {
        let call = session.exec("yes", LIMITS);
        let _ = tokio::time::timeout(Duration::from_millis(300), call).await;
    }
    let out = session.exec("echo ok", LIMITS).await.unwrap();
    assert_eq!(out.stdout, b"ok\n");
}
