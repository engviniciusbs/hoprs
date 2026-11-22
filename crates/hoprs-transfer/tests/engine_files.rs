mod common;

use std::collections::{HashMap, HashSet};
use std::future::Future as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use hoprs_core::{ErrorKind, RemoteEntry, RemotePath, Result};
use hoprs_remote::{BoxAsyncRead, BoxAsyncWrite, FaultFs, MemFs, RemoteFs};
use hoprs_transfer::{
    ConflictInfo, ConflictPolicy, Direction, EngineConfig, EnqueueOptions, Event, EventKind,
    LocalGuard, Origin, QueueStore, SiteEndpoints, TransferEngine, TransferId, TransferState,
};
use tokio::sync::broadcast::Receiver;

/// Collects events until `n` items reached a terminal state. Returns the final
/// states and the maximum number of simultaneously `Active` items.
async fn collect_terminal(
    rx: &mut Receiver<Event>,
    n: usize,
) -> (HashMap<TransferId, TransferState>, usize) {
    let mut done = HashMap::new();
    let mut active = HashSet::new();
    let mut max_active = 0;
    while done.len() < n {
        if let EventKind::StateChanged { id, state } =
            wait_for(rx, |e| matches!(e, EventKind::StateChanged { .. })).await
        {
            if state == TransferState::Active {
                active.insert(id);
                max_active = max_active.max(active.len());
            } else {
                active.remove(&id);
            }
            if state.is_terminal() {
                done.insert(id, state);
            }
        }
    }
    (done, max_active)
}

#[tokio::test(flavor = "multi_thread")]
async fn uploads_single_file() {
    let (engine, local, remote, mut rx) = engine_with_memfs(4).await;
    let bytes = data(300 * 1024);
    local.add_file("/w/a.bin", &bytes, mtime());
    remote.add_dir("/srv");

    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Done);
    assert_eq!(remote.read_file("/srv/a.bin").unwrap(), bytes);
}

#[tokio::test(flavor = "multi_thread")]
async fn downloads_single_file() {
    let (engine, local, remote, mut rx) = engine_with_memfs(4).await;
    let bytes = data(300 * 1024);
    remote.add_file("/srv/a.bin", &bytes, mtime());
    local.add_dir("/w");

    let id = engine
        .enqueue(spec(Direction::Download, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Done);
    assert_eq!(local.read_file("/w/a.bin").unwrap(), bytes);
}

#[tokio::test(flavor = "multi_thread")]
async fn respects_worker_limit() {
    let local = MemFs::new();
    let remote_mem = MemFs::new();
    remote_mem.add_dir("/srv");
    let remote = FaultFs::new(remote_mem.clone());
    remote.set_latency(Duration::from_millis(20));
    for i in 0..6 {
        local.add_file(&format!("/w/{i}.bin"), &data(100_000), mtime());
    }

    // Enqueue before the site is registered: everything stays Queued.
    let (engine, mut rx) = start_engine(EngineConfig::default());
    for i in 0..6 {
        engine
            .enqueue(spec(
                Direction::Upload,
                &format!("/w/{i}.bin"),
                &format!("/srv/{i}.bin"),
            ))
            .await
            .unwrap();
    }
    let snap = engine.snapshot().await;
    assert_eq!(snap.len(), 6);
    assert!(snap.iter().all(|i| i.state == TransferState::Queued));
    assert!(snap.windows(2).all(|w| w[0].id < w[1].id));

    engine
        .register_site(
            site(),
            SiteEndpoints {
                local: Arc::new(local),
                remote: Arc::new(remote),
                workers: 2,
            },
        )
        .await;

    let (states, max_active) = collect_terminal(&mut rx, 6).await;
    assert!(states.values().all(|s| *s == TransferState::Done));
    assert_eq!(max_active, 2);
    for i in 0..6 {
        assert_eq!(
            remote_mem.read_file(&format!("/srv/{i}.bin")).unwrap(),
            data(100_000)
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_is_throttled() {
    let local = MemFs::new();
    let remote_mem = MemFs::new();
    remote_mem.add_dir("/srv");
    let remote = FaultFs::new(remote_mem.clone());
    remote.set_latency(Duration::from_millis(20));
    let bytes = data(8 << 20);
    local.add_file("/w/big.bin", &bytes, mtime());
    let config = EngineConfig {
        buf_size: 4 * 1024,
        ..EngineConfig::default()
    };
    let (engine, mut rx) = engine_with(Arc::new(local), Arc::new(remote), 1, config).await;

    let start = Instant::now();
    let id = engine
        .enqueue(spec(Direction::Upload, "/w/big.bin", "/srv/big.bin"))
        .await
        .unwrap();
    let mut progress_events = 0u128;
    loop {
        match wait_for(&mut rx, |_| true).await {
            EventKind::Progress(_) => progress_events += 1,
            EventKind::StateChanged { id: i, state } if i == id && state.is_terminal() => {
                assert_eq!(state, TransferState::Done);
                break;
            }
            _ => {}
        }
    }
    let elapsed = start.elapsed();
    assert!(
        progress_events <= elapsed.as_millis() / 100 + 2,
        "{progress_events} progress events in {elapsed:?}"
    );
    assert_eq!(remote_mem.read_file("/srv/big.bin").unwrap(), bytes);
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_source_fails_only_that_item() {
    let (engine, local, remote, mut rx) = engine_with_memfs(2).await;
    local.add_file("/w/a.bin", &data(10_000), mtime());
    local.add_file("/w/c.bin", &data(20_000), mtime());
    remote.add_dir("/srv");

    let a = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    let b = engine
        .enqueue(spec(Direction::Upload, "/w/missing.bin", "/srv/b.bin"))
        .await
        .unwrap();
    let c = engine
        .enqueue(spec(Direction::Upload, "/w/c.bin", "/srv/c.bin"))
        .await
        .unwrap();

    let (states, _) = collect_terminal(&mut rx, 3).await;
    assert_eq!(states[&a], TransferState::Done);
    assert_eq!(states[&c], TransferState::Done);
    match &states[&b] {
        TransferState::Failed(e) => assert_eq!(e.kind, ErrorKind::NotFound),
        other => panic!("expected Failed(NotFound), got {other:?}"),
    }
    assert_eq!(remote.read_file("/srv/c.bin").unwrap(), data(20_000));
}

#[tokio::test(flavor = "multi_thread")]
async fn read_error_mid_transfer_fails_item() {
    let local_mem = MemFs::new();
    local_mem.add_file("/w/a.bin", &data(200_000), mtime());
    local_mem.add_file("/w/b.bin", &data(150_000), mtime());
    let local = FaultFs::new(local_mem);
    let remote = MemFs::new();
    remote.add_dir("/srv");
    local.fail_io_after(50_000, ErrorKind::Io);
    let (engine, mut rx) = engine_with(
        Arc::new(local),
        Arc::new(remote.clone()),
        1,
        EngineConfig::default(),
    )
    .await;

    let a = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    let b = engine
        .enqueue(spec(Direction::Upload, "/w/b.bin", "/srv/b.bin"))
        .await
        .unwrap();

    match wait_terminal(&mut rx, a).await {
        TransferState::Failed(e) => assert_eq!(e.kind, ErrorKind::Io),
        other => panic!("expected Failed(Io), got {other:?}"),
    }
    assert_eq!(wait_terminal(&mut rx, b).await, TransferState::Done);
    assert_eq!(remote.read_file("/srv/b.bin").unwrap(), data(150_000));
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_policy_awaits_decision_and_frees_worker() {
    let (engine, local, remote, mut rx) = engine_with_memfs(1).await;
    local.add_file("/w/a.bin", &data(3_000), mtime());
    local.add_file("/w/b.bin", &data(4_000), mtime());
    remote.add_file("/srv/a.bin", b"old", mtime());

    let a = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    let b = engine
        .enqueue(spec(Direction::Upload, "/w/b.bin", "/srv/b.bin"))
        .await
        .unwrap();

    let ev = wait_for(&mut rx, |e| {
        matches!(e, EventKind::StateChanged { id, state: TransferState::AwaitingDecision(_) } if *id == a)
    })
    .await;
    let EventKind::StateChanged {
        state: TransferState::AwaitingDecision(info),
        ..
    } = ev
    else {
        unreachable!()
    };
    assert_eq!(
        info,
        ConflictInfo {
            src_size: 3_000,
            dst_size: 3,
            src_modified: Some(mtime()),
            dst_modified: Some(mtime()),
        }
    );
    // The single worker was freed: the next item runs.
    assert_eq!(wait_terminal(&mut rx, b).await, TransferState::Done);
    assert_eq!(remote.read_file("/srv/a.bin").unwrap(), b"old");
}

#[tokio::test(flavor = "multi_thread")]
async fn set_policy_skip_skips_existing_destination() {
    let (engine, local, remote, mut rx) = engine_with_memfs(1).await;
    local.add_file("/w/a.bin", &data(3_000), mtime());
    remote.add_file("/srv/a.bin", b"old", mtime());
    engine.set_policy(ConflictPolicy::Skip).await;

    let a = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    assert_eq!(
        wait_terminal(&mut rx, a).await,
        TransferState::Skipped("exists".to_string())
    );
    assert_eq!(remote.read_file("/srv/a.bin").unwrap(), b"old");
}

#[tokio::test(flavor = "multi_thread")]
async fn persists_items_and_emits_added() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("queue.db");
    let local = MemFs::new();
    let remote = MemFs::new();
    local.add_file("/w/a.bin", &data(70_000), mtime());
    remote.add_dir("/srv");
    let engine =
        TransferEngine::start(QueueStore::open(&db).unwrap(), EngineConfig::default()).unwrap();
    let mut rx = engine.subscribe();
    engine
        .register_site(
            site(),
            SiteEndpoints {
                local: Arc::new(local),
                remote: Arc::new(remote),
                workers: 0, // clamped to 1
            },
        )
        .await;

    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    match wait_for(&mut rx, |e| matches!(e, EventKind::Added(_))).await {
        EventKind::Added(item) => {
            assert_eq!(item.id, id);
            assert_eq!(item.state, TransferState::Queued);
            assert_eq!(item.spec, spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"));
        }
        _ => unreachable!(),
    }
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Done);

    let stored = QueueStore::open(&db).unwrap().get(id).unwrap().unwrap();
    assert_eq!(stored.state, TransferState::Done);
    assert_eq!(stored.is_dir, Some(false));
    assert_eq!(stored.size, Some(70_000));
    assert_eq!(stored.bytes_done, 70_000);

    let snap = engine.snapshot().await;
    assert_eq!(snap.len(), 1);
    assert_eq!(snap[0], stored);
}

/// `MemFs` wrapper for tests: read streams sleep `per_read` before every poll
/// (so a copy spans several progress intervals), and `open_read` of
/// `panic_on_read` panics.
struct TestFs {
    inner: MemFs,
    per_read: Duration,
    panic_on_read: Option<RemotePath>,
}

const INJECTED_PANIC: &str = "injected test panic";

struct SlowRead {
    inner: BoxAsyncRead,
    per_read: Duration,
    sleep: Option<std::pin::Pin<Box<tokio::time::Sleep>>>,
}

impl tokio::io::AsyncRead for SlowRead {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let per_read = self.per_read;
        let sleep = self
            .sleep
            .get_or_insert_with(|| Box::pin(tokio::time::sleep(per_read)));
        if sleep.as_mut().poll(cx).is_pending() {
            return std::task::Poll::Pending;
        }
        self.sleep = None;
        std::pin::Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

#[async_trait::async_trait]
impl RemoteFs for TestFs {
    async fn list(&self, path: &RemotePath) -> Result<Vec<RemoteEntry>> {
        self.inner.list(path).await
    }
    async fn stat(&self, path: &RemotePath) -> Result<RemoteEntry> {
        self.inner.stat(path).await
    }
    async fn open_read(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncRead> {
        if self.panic_on_read.as_ref() == Some(path) {
            panic!("{INJECTED_PANIC}");
        }
        let inner = self.inner.open_read(path, offset).await?;
        Ok(Box::new(SlowRead {
            inner,
            per_read: self.per_read,
            sleep: None,
        }))
    }
    async fn open_write(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncWrite> {
        self.inner.open_write(path, offset).await
    }
    async fn mkdir(&self, path: &RemotePath) -> Result<()> {
        self.inner.mkdir(path).await
    }
    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()> {
        self.inner.rename(from, to).await
    }
    async fn remove_file(&self, path: &RemotePath) -> Result<()> {
        self.inner.remove_file(path).await
    }
    async fn remove_dir(&self, path: &RemotePath) -> Result<()> {
        self.inner.remove_dir(path).await
    }
    async fn chmod(&self, path: &RemotePath, mode: u32) -> Result<()> {
        self.inner.chmod(path, mode).await
    }
}

/// A copy spanning several intervals emits progress, at most once per tick,
/// with monotonically increasing byte counts.
#[tokio::test(flavor = "multi_thread")]
async fn slow_copy_emits_bounded_monotonic_progress() {
    let local_mem = MemFs::new();
    let bytes = data(1 << 20);
    local_mem.add_file("/w/a.bin", &bytes, mtime());
    let local = TestFs {
        inner: local_mem,
        per_read: Duration::from_millis(2),
        panic_on_read: None,
    };
    let remote = MemFs::new();
    remote.add_dir("/srv");
    let config = EngineConfig {
        buf_size: 4 * 1024,
        ..EngineConfig::default()
    };
    let (engine, mut rx) = engine_with(Arc::new(local), Arc::new(remote.clone()), 1, config).await;

    let start = Instant::now();
    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    let mut seen = Vec::new();
    loop {
        match wait_for(&mut rx, |_| true).await {
            EventKind::Progress(updates) => {
                assert_eq!(updates.len(), 1);
                assert_eq!(updates[0].id, id);
                assert_eq!(updates[0].total, Some(1 << 20));
                seen.push(updates[0].bytes_done);
            }
            EventKind::StateChanged { id: i, state } if i == id && state.is_terminal() => {
                assert_eq!(state, TransferState::Done);
                break;
            }
            _ => {}
        }
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed >= Duration::from_millis(500),
        "copy too fast: {elapsed:?}"
    );
    assert!(seen.len() >= 2, "only {} progress events", seen.len());
    assert!(
        seen.len() as u128 <= elapsed.as_millis() / 100 + 2,
        "{} progress events in {elapsed:?}",
        seen.len()
    );
    assert!(seen.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(remote.read_file("/srv/a.bin").unwrap(), bytes);
}

/// Silences the default panic message for `INJECTED_PANIC` only; every other
/// panic still goes to the previous hook.
fn silence_injected_panics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied());
        if msg != Some(INJECTED_PANIC) {
            previous(info);
        }
    }));
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_panic_fails_item_and_frees_slot() {
    silence_injected_panics();
    let local_mem = MemFs::new();
    local_mem.add_file("/w/a.bin", &data(10_000), mtime());
    local_mem.add_file("/w/b.bin", &data(20_000), mtime());
    let local = TestFs {
        inner: local_mem,
        per_read: Duration::ZERO,
        panic_on_read: Some(path("/w/a.bin")),
    };
    let remote = MemFs::new();
    remote.add_dir("/srv");
    let (engine, mut rx) = engine_with(
        Arc::new(local),
        Arc::new(remote.clone()),
        1,
        EngineConfig::default(),
    )
    .await;

    let a = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    let b = engine
        .enqueue(spec(Direction::Upload, "/w/b.bin", "/srv/b.bin"))
        .await
        .unwrap();

    match wait_terminal(&mut rx, a).await {
        TransferState::Failed(e) => {
            assert_eq!(e.kind, ErrorKind::Io);
            assert_eq!(e.message, "Worker panicked");
        }
        other => panic!("expected Failed(Io), got {other:?}"),
    }
    assert_eq!(wait_terminal(&mut rx, b).await, TransferState::Done);
    assert_eq!(remote.read_file("/srv/b.bin").unwrap(), data(20_000));
}

#[tokio::test(flavor = "multi_thread")]
async fn uploads_zero_byte_file() {
    let (engine, local, remote, mut rx) = engine_with_memfs(1).await;
    local.add_file("/w/empty", b"", mtime());
    remote.add_dir("/srv");

    let id = engine
        .enqueue(spec(Direction::Upload, "/w/empty", "/srv/empty"))
        .await
        .unwrap();
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Done);
    assert_eq!(remote.read_file("/srv/empty").unwrap(), b"");
    let snap = engine.snapshot().await;
    assert_eq!((snap[0].size, snap[0].bytes_done), (Some(0), 0));
}

const NOT_A_FILE: &str = "Destination is not a regular file";

#[tokio::test(flavor = "multi_thread")]
async fn destination_directory_fails_item() {
    let (engine, local, remote, mut rx) = engine_with_memfs(1).await;
    engine.set_policy(ConflictPolicy::Overwrite).await;
    local.add_file("/w/a.bin", &data(1_000), mtime());
    remote.add_file("/srv/a.bin/keep.txt", b"keep", mtime());

    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    match wait_terminal(&mut rx, id).await {
        TransferState::Failed(e) => {
            assert_eq!((e.kind, e.message.as_str()), (ErrorKind::Io, NOT_A_FILE));
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    let dir = remote.stat(&path("/srv/a.bin")).await.unwrap();
    assert_eq!(dir.kind, hoprs_core::EntryKind::Dir);
    assert_eq!(remote.read_file("/srv/a.bin/keep.txt").unwrap(), b"keep");
}

#[tokio::test(flavor = "multi_thread")]
async fn destination_symlink_fails_item() {
    let (engine, local, remote, mut rx) = engine_with_memfs(1).await;
    engine.set_policy(ConflictPolicy::Overwrite).await;
    local.add_file("/w/a.bin", &data(1_000), mtime());
    remote.add_file("/srv/target", b"target", mtime());
    remote.add_symlink("/srv/a.bin", "/srv/target");

    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    match wait_terminal(&mut rx, id).await {
        TransferState::Failed(e) => {
            assert_eq!((e.kind, e.message.as_str()), (ErrorKind::Io, NOT_A_FILE));
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    let link = remote.stat(&path("/srv/a.bin")).await.unwrap();
    assert_eq!(link.kind, hoprs_core::EntryKind::Symlink);
    assert_eq!(remote.read_file("/srv/target").unwrap(), b"target");
}

/// Spec §9: progress reaches the UI at 10 Hz at most, aggregated over every worker. Four
/// workers copy a folder of small files (each a few chunks, each chunk slowed down), so
/// copies keep finishing all through the 3 s window; the engine sends at most one
/// `Progress` per 100 ms interval: 31 in 3 s, counting both ends.
#[tokio::test(flavor = "multi_thread")]
async fn progress_rate_at_most_10hz() {
    const WINDOW: Duration = Duration::from_secs(3);
    const FILES: usize = 600;
    let local_mem = MemFs::new();
    let file = data(64 * 1024);
    local_mem.add_dir("/w/tree");
    for i in 0..FILES {
        local_mem.add_file(&format!("/w/tree/f{i:04}.bin"), &file, mtime());
    }
    let local = FaultFs::new(local_mem);
    // 4 chunks of 16 KiB per file, 10 ms each: about 25 files done per second per worker.
    local.set_stream_latency(Duration::from_millis(10));
    let remote = MemFs::new();
    remote.add_dir("/srv");
    let config = EngineConfig {
        buf_size: 16 * 1024,
        ..EngineConfig::default()
    };
    let (engine, mut rx) = engine_with(Arc::new(local), Arc::new(remote), 4, config).await;

    let root = engine
        .enqueue(spec(Direction::Upload, "/w/tree", "/srv/tree"))
        .await
        .unwrap();
    let start = Instant::now();
    let mut progress = 0usize;
    let mut finished = 0usize;
    let mut root_done = false;
    while let Ok(event) = tokio::time::timeout_at((start + WINDOW).into(), rx.recv()).await {
        match event.expect("no lag, no close").kind {
            EventKind::Progress(_) => progress += 1,
            EventKind::StateChanged {
                id,
                state: TransferState::Done,
            } => {
                if id == root {
                    root_done = true;
                } else {
                    finished += 1;
                }
            }
            _ => {}
        }
    }
    assert!(
        !root_done,
        "the transfer must outlast the {WINDOW:?} window"
    );
    assert!(
        finished >= 60,
        "copies keep finishing during the window: {finished}"
    );
    assert!(
        progress <= 31,
        "{progress} Progress events in {WINDOW:?} ({finished} files finished)"
    );
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn enqueue_with_overwrite_replaces_existing_without_asking() {
    let (engine, local, remote, mut rx) = engine_with_memfs(1).await;
    let new = data(3_000);
    local.add_file("/w/a.bin", &new, mtime());
    local.add_file("/w/b.bin", &data(10), mtime());
    remote.add_file("/srv/a.bin", b"old", mtime());
    remote.add_file("/srv/b.bin", b"old", mtime());
    // Global policy stays `Ask`.

    let a = engine
        .enqueue_with_policy(
            spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"),
            ConflictPolicy::Overwrite,
        )
        .await
        .unwrap();
    let mut seen = Vec::new();
    let end = wait_for(&mut rx, |e| {
        seen.push(e.clone());
        matches!(e, EventKind::StateChanged { id, state } if *id == a && state.is_terminal())
    })
    .await;
    assert!(matches!(
        end,
        EventKind::StateChanged {
            state: TransferState::Done,
            ..
        }
    ));
    assert!(!seen.iter().any(|e| matches!(
        e,
        EventKind::StateChanged {
            state: TransferState::AwaitingDecision(_),
            ..
        }
    )));
    assert_eq!(remote.read_file("/srv/a.bin").unwrap(), new);

    // A plain `enqueue` still uses the global policy (`Ask`).
    let b = engine
        .enqueue(spec(Direction::Upload, "/w/b.bin", "/srv/b.bin"))
        .await
        .unwrap();
    wait_for(&mut rx, |e| {
        matches!(e, EventKind::StateChanged { id, state: TransferState::AwaitingDecision(_) } if *id == b)
    })
    .await;
    assert_eq!(remote.read_file("/srv/b.bin").unwrap(), b"old");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_item_policy_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("queue.db");
    let local = MemFs::new();
    let remote = MemFs::new();
    let new = data(3_000);
    local.add_file("/w/a.bin", &new, mtime());
    remote.add_file("/srv/a.bin", b"old", mtime());

    // No site registered: the item stays queued and never runs.
    let engine =
        TransferEngine::start(QueueStore::open(&db).unwrap(), EngineConfig::default()).unwrap();
    let id = engine
        .enqueue_with_policy(
            spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"),
            ConflictPolicy::Overwrite,
        )
        .await
        .unwrap();
    engine.shutdown().await;

    let engine =
        TransferEngine::start(QueueStore::open(&db).unwrap(), EngineConfig::default()).unwrap();
    let mut rx = engine.subscribe();
    engine
        .register_site(
            site(),
            SiteEndpoints {
                local: Arc::new(local),
                remote: Arc::new(remote.clone()),
                workers: 1,
            },
        )
        .await;
    engine.resume(id).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Done);
    assert_eq!(remote.read_file("/srv/a.bin").unwrap(), new);
}

#[tokio::test(flavor = "multi_thread")]
async fn enqueue_with_overwrite_after_earlier_transfer_to_same_destination() {
    let (engine, local, remote, mut rx) = engine_with_memfs(1).await;
    local.add_file("/w/a.bin", &data(100), mtime());
    remote.add_dir("/srv");

    let first = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    assert_eq!(wait_terminal(&mut rx, first).await, TransferState::Done);

    // The file is edited and compared again; the user chooses overwrite.
    let new = data(3_000);
    local.add_file("/w/a.bin", &new, mtime());
    let mut seen = Vec::new();
    let second = engine
        .enqueue_with_policy(
            spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"),
            ConflictPolicy::Overwrite,
        )
        .await
        .unwrap();
    wait_for(&mut rx, |e| {
        seen.push(e.clone());
        matches!(e, EventKind::StateChanged { id, state } if *id == second && state.is_terminal())
    })
    .await;
    assert!(!seen.iter().any(|e| matches!(
        e,
        EventKind::StateChanged {
            state: TransferState::AwaitingDecision(_),
            ..
        }
    )));
    assert_eq!(remote.read_file("/srv/a.bin").unwrap(), new);
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_guard_runs_for_every_agent_item() {
    let checked: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let recorder = Arc::clone(&checked);
    let guard = LocalGuard::new(move |path: RemotePath| {
        let recorder = Arc::clone(&recorder);
        Box::pin(async move {
            recorder.lock().unwrap().push(path.as_str().to_owned());
            if path.as_str() == "/w/site/sub/secret.txt" {
                return Err(hoprs_core::Error::new(
                    ErrorKind::Permission,
                    format!("Protected path: {path}"),
                ));
            }
            Ok(())
        })
    });
    let config = EngineConfig {
        agent_guard: Some(guard),
        ..EngineConfig::default()
    };
    let local = MemFs::new();
    let remote = MemFs::new();
    local.add_file("/w/site/a.txt", b"a", mtime());
    local.add_file("/w/site/sub/b.txt", b"b", mtime());
    local.add_file("/w/site/sub/secret.txt", b"secret", mtime());
    remote.add_dir("/srv");
    let (engine, mut rx) = engine_with(Arc::new(local), Arc::new(remote.clone()), 2, config).await;

    let agent = engine
        .enqueue_with(
            spec(Direction::Upload, "/w/site", "/srv/agent"),
            EnqueueOptions {
                policy: Some(ConflictPolicy::Skip),
                origin: Origin::Agent,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        wait_terminal(&mut rx, agent).await,
        TransferState::Failed(_)
    ));
    let snap = engine.snapshot().await;
    for i in &snap {
        if i.spec.local.as_str() == "/w/site/sub/secret.txt" {
            match &i.state {
                TransferState::Failed(e) => assert_eq!(
                    (e.kind, e.message.as_str()),
                    (
                        ErrorKind::Permission,
                        "Protected path: /w/site/sub/secret.txt"
                    )
                ),
                other => panic!("secret.txt ended {other:?}"),
            }
        } else if i.is_dir == Some(false) {
            assert_eq!(i.state, TransferState::Done, "{i:?}");
        }
    }
    assert_eq!(remote.read_file("/srv/agent/a.txt").unwrap(), b"a");
    assert_eq!(remote.read_file("/srv/agent/sub/b.txt").unwrap(), b"b");
    assert!(remote.read_file("/srv/agent/sub/secret.txt").is_none());
    let mut seen = checked.lock().unwrap().clone();
    seen.sort();
    assert_eq!(
        seen,
        [
            "/w/site",
            "/w/site/a.txt",
            "/w/site/sub",
            "/w/site/sub/b.txt",
            "/w/site/sub/secret.txt",
        ]
    );

    // The user's own transfer of the same tree never meets the guard.
    checked.lock().unwrap().clear();
    let user = engine
        .enqueue(spec(Direction::Upload, "/w/site", "/srv/user"))
        .await
        .unwrap();
    assert_eq!(wait_terminal(&mut rx, user).await, TransferState::Done);
    assert!(checked.lock().unwrap().is_empty());
    assert_eq!(
        remote.read_file("/srv/user/sub/secret.txt").unwrap(),
        b"secret"
    );
}
