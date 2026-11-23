mod common;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use hoprs_core::{ErrorKind, SiteId};
use hoprs_remote::{FaultFs, MemFs, Op};
use hoprs_transfer::{
    ConflictPolicy, Direction, EngineConfig, EnqueueOptions, Event, EventKind, Origin, QueueStore,
    SiteEndpoints, TransferEngine, TransferId, TransferItem, TransferSpec, TransferState,
};
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::RecvError;

const MIB: usize = 1 << 20;

fn site_named(name: &str) -> SiteId {
    SiteId(name.to_string())
}

fn upload(site: &SiteId, local: &str, remote: &str) -> TransferSpec {
    TransferSpec {
        site: site.clone(),
        direction: Direction::Upload,
        local: path(local),
        remote: path(remote),
    }
}

fn endpoints(local: Arc<MemFs>, remote: Arc<FaultFs<MemFs>>, workers: usize) -> SiteEndpoints {
    SiteEndpoints {
        local,
        remote,
        workers,
    }
}

fn find(snap: &[TransferItem], id: TransferId) -> &TransferItem {
    snap.iter().find(|i| i.id == id).expect("item in snapshot")
}

/// Waits until every id in `ids` reached a terminal state; returns their states.
async fn wait_all_terminal(
    rx: &mut Receiver<Event>,
    ids: &[TransferId],
) -> HashMap<TransferId, TransferState> {
    let mut done = HashMap::new();
    while done.len() < ids.len() {
        if let EventKind::StateChanged { id, state } = wait_for(rx, |e| {
            matches!(e, EventKind::StateChanged { id, state } if ids.contains(id) && state.is_terminal())
        })
        .await
        {
            done.insert(id, state);
        }
    }
    done
}

/// Collects every event received during `window` (the one timing-based
/// negative check: nothing must happen).
async fn events_during(rx: &mut Receiver<Event>, window: Duration) -> Vec<EventKind> {
    let mut out = Vec::new();
    let _ = tokio::time::timeout(window, async {
        loop {
            match rx.recv().await {
                Ok(ev) => out.push(ev.kind),
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
    })
    .await;
    out
}

fn is_failed(e: &EventKind) -> bool {
    matches!(
        e,
        EventKind::StateChanged {
            state: TransferState::Failed(_),
            ..
        }
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn network_error_pauses_site() {
    let local = MemFs::new();
    let remote_mem = MemFs::new();
    let remote = FaultFs::new(remote_mem.clone());
    let (engine, mut rx) = engine_with(
        Arc::new(local.clone()),
        Arc::new(remote.clone()),
        2,
        EngineConfig::default(),
    )
    .await;
    remote_mem.add_dir("/srv");
    let mut files = Vec::new();
    for (i, name) in ["a", "b", "c"].into_iter().enumerate() {
        let bytes = data(MIB + i * 1000);
        local.add_file(&format!("/w/{name}"), &bytes, mtime());
        files.push((name, bytes));
    }
    remote.fail_io_after(200_000, ErrorKind::Network);

    let mut ids = Vec::new();
    for (name, _) in &files {
        let id = engine
            .enqueue(spec(
                Direction::Upload,
                &format!("/w/{name}"),
                &format!("/srv/{name}"),
            ))
            .await
            .unwrap();
        ids.push(id);
    }

    let mut before = Vec::new();
    let paused = wait_for(&mut rx, |e| {
        before.push(e.clone());
        matches!(e, EventKind::SitePaused { .. })
    })
    .await;
    match paused {
        EventKind::SitePaused { site: s, error } => {
            assert_eq!(s, site());
            assert_eq!(error.kind, ErrorKind::Network);
        }
        _ => unreachable!(),
    }
    let after = events_during(&mut rx, Duration::from_millis(300)).await;
    assert!(
        !after.iter().any(|e| matches!(
            e,
            EventKind::StateChanged {
                state: TransferState::Active,
                ..
            }
        )),
        "no item is dispatched while the site is paused: {after:?}"
    );
    assert!(!before.iter().chain(&after).any(is_failed));
    assert!(
        !after
            .iter()
            .any(|e| matches!(e, EventKind::SitePaused { .. })),
        "exactly one SitePaused"
    );
    assert!(
        engine
            .snapshot()
            .await
            .iter()
            .all(|i| !matches!(i.state, TransferState::Failed(_)))
    );

    // An item already running when the site paused may have finished meanwhile.
    let mut done: HashMap<TransferId, TransferState> = before
        .iter()
        .chain(&after)
        .filter_map(|e| match e {
            EventKind::StateChanged { id, state } if state.is_terminal() => {
                Some((*id, state.clone()))
            }
            _ => None,
        })
        .collect();
    let pending: Vec<TransferId> = ids
        .iter()
        .copied()
        .filter(|id| !done.contains_key(id))
        .collect();
    assert!(!pending.is_empty());

    engine.resume_site(site()).await;
    done.extend(wait_all_terminal(&mut rx, &pending).await);
    for id in &ids {
        assert_eq!(done.get(id), Some(&TransferState::Done));
    }
    for (name, bytes) in &files {
        assert_eq!(
            remote_mem.read_file(&format!("/srv/{name}")).as_ref(),
            Some(bytes)
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn other_sites_keep_running() {
    let (a, b) = (site_named("a"), site_named("b"));
    let (engine, mut rx) = start_engine(EngineConfig::default());
    let local = MemFs::new();
    let remote_a = MemFs::new();
    let remote_b = MemFs::new();
    let fault_a = FaultFs::new(remote_a.clone());
    let fault_b = FaultFs::new(remote_b.clone());
    remote_a.add_dir("/srv");
    remote_b.add_dir("/srv");
    for name in ["x", "y", "z"] {
        local.add_file(&format!("/w/{name}"), &data(MIB), mtime());
    }
    engine
        .register_site(
            a.clone(),
            endpoints(Arc::new(local.clone()), Arc::new(fault_a.clone()), 1),
        )
        .await;
    engine
        .register_site(
            b.clone(),
            endpoints(Arc::new(local.clone()), Arc::new(fault_b), 2),
        )
        .await;
    fault_a.fail_io_after(200_000, ErrorKind::Network);

    let mut ids_a = Vec::new();
    for name in ["x", "y", "z"] {
        let spec = upload(&a, &format!("/w/{name}"), &format!("/srv/{name}"));
        ids_a.push(engine.enqueue(spec).await.unwrap());
    }
    wait_for(
        &mut rx,
        |e| matches!(e, EventKind::SitePaused { site, .. } if *site == a),
    )
    .await;

    let mut ids_b = Vec::new();
    for name in ["x", "y", "z"] {
        let spec = upload(&b, &format!("/w/{name}"), &format!("/srv/{name}"));
        ids_b.push(engine.enqueue(spec).await.unwrap());
    }
    let done = wait_all_terminal(&mut rx, &ids_b).await;
    for id in &ids_b {
        assert_eq!(done.get(id), Some(&TransferState::Done));
    }
    for name in ["x", "y", "z"] {
        assert_eq!(
            remote_b.read_file(&format!("/srv/{name}")),
            local.read_file(&format!("/w/{name}"))
        );
    }

    let snap = engine.snapshot().await;
    for id in &ids_a {
        assert_eq!(
            find(&snap, *id).state,
            TransferState::Queued,
            "site a stays paused"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn manual_pause_site_holds_dispatch_until_resume() {
    let (engine, local, remote, mut rx) = engine_with_memfs(2).await;
    remote.add_dir("/srv");
    local.add_file("/w/a", &data(10_000), mtime());

    engine.pause_site(site()).await;
    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a", "/srv/a"))
        .await
        .unwrap();
    let during = events_during(&mut rx, Duration::from_millis(300)).await;
    assert!(
        !during.iter().any(|e| matches!(
            e,
            EventKind::StateChanged { .. } | EventKind::SitePaused { .. }
        )),
        "a manually paused site dispatches nothing and emits no SitePaused: {during:?}"
    );

    // Unknown or unpaused sites are no-ops.
    engine.pause_site(site_named("nope")).await;
    engine.resume_site(site_named("nope")).await;

    engine.resume_site(site()).await;
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Done);
    engine.resume_site(site()).await;
}

/// Starts an engine over the queue database at `db`, registered as site `"s"`.
async fn engine_on_file(
    db: &Path,
    local: Arc<FaultFs<MemFs>>,
    remote: MemFs,
    workers: usize,
    config: EngineConfig,
) -> (TransferEngine, Receiver<Event>) {
    let store = QueueStore::open(db).unwrap();
    let engine = TransferEngine::start(store, config).unwrap();
    let rx = engine.subscribe();
    engine
        .register_site(
            site(),
            SiteEndpoints {
                local,
                remote: Arc::new(remote),
                workers,
            },
        )
        .await;
    (engine, rx)
}

/// Waits for a `Progress` update reporting bytes copied for `id`.
async fn wait_bytes(rx: &mut Receiver<Event>, id: TransferId) {
    wait_for(
        rx,
        |e| matches!(e, EventKind::Progress(us) if us.iter().any(|u| u.id == id && u.bytes_done > 0)),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_recovers_as_paused() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("queue.db");
    let local = MemFs::new();
    let remote = MemFs::new();
    let bytes = data(4 * MIB);
    local.add_file("/w/slow.bin", &bytes, mtime());
    local.add_file("/w/ask.bin", &data(3_000), mtime());
    remote.add_file("/srv/ask.bin", b"old contents", mtime());

    let slow = FaultFs::new(local.clone());
    slow.set_stream_latency(Duration::from_millis(20));
    let (engine, mut rx) = engine_on_file(
        &db,
        Arc::new(slow),
        remote.clone(),
        2,
        EngineConfig::default(),
    )
    .await;
    let active = engine
        .enqueue(spec(Direction::Upload, "/w/slow.bin", "/srv/slow.bin"))
        .await
        .unwrap();
    let asking = engine
        .enqueue(spec(Direction::Upload, "/w/ask.bin", "/srv/ask.bin"))
        .await
        .unwrap();
    wait_for(&mut rx, |e| {
        matches!(e, EventKind::StateChanged { id, state: TransferState::AwaitingDecision(_) } if *id == asking)
    })
    .await;
    wait_bytes(&mut rx, active).await;
    engine.shutdown().await;

    let fault = FaultFs::new(local.clone());
    let (engine, mut rx) = engine_on_file(
        &db,
        Arc::new(fault.clone()),
        remote.clone(),
        2,
        EngineConfig::default(),
    )
    .await;
    let snap = engine.snapshot().await;
    assert_eq!(snap.len(), 2);
    assert_eq!(find(&snap, active).state, TransferState::Paused);
    assert_eq!(find(&snap, asking).state, TransferState::Paused);

    engine.resume(active).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, active).await, TransferState::Done);
    assert_eq!(remote.read_file("/srv/slow.bin").unwrap(), bytes);
    let offsets = fault.read_offsets();
    assert!(
        offsets.len() == 1 && offsets[0] > 0,
        "resumed from the partial destination: {offsets:?}"
    );

    engine.resume(asking).await.unwrap();
    wait_for(&mut rx, |e| {
        matches!(e, EventKind::StateChanged { id, state: TransferState::AwaitingDecision(_) } if *id == asking)
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_keeps_expanded_folder_and_pauses_queued_items() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("queue.db");
    let local = MemFs::new();
    let remote = MemFs::new();
    remote.add_dir("/srv");
    for i in 0..6 {
        local.add_file(&format!("/w/d/f{i}.bin"), &data(MIB), mtime());
    }
    let config = EngineConfig {
        buf_size: 64 * 1024,
        ..EngineConfig::default()
    };

    let slow = FaultFs::new(local.clone());
    slow.set_stream_latency(Duration::from_millis(5));
    let (engine, mut rx) =
        engine_on_file(&db, Arc::new(slow), remote.clone(), 1, config.clone()).await;
    let root = engine
        .enqueue(spec(Direction::Upload, "/w/d", "/srv/d"))
        .await
        .unwrap();
    // The first child finishes while the others wait in the queue.
    wait_for(
        &mut rx,
        |e| matches!(e, EventKind::StateChanged { id, state: TransferState::Done } if *id != root),
    )
    .await;
    engine.shutdown().await;

    let (engine, mut rx) = engine_on_file(
        &db,
        Arc::new(FaultFs::new(local.clone())),
        remote.clone(),
        2,
        config,
    )
    .await;
    let snap = engine.snapshot().await;
    assert_eq!(snap.len(), 7, "the folder and its six children");
    assert!(
        snap.iter()
            .all(|i| i.state == TransferState::Paused || i.state == TransferState::Done),
        "{snap:?}"
    );
    assert!(snap.iter().any(|i| i.state == TransferState::Done));

    engine.resume(root).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, root).await, TransferState::Done);
    let snap = engine.snapshot().await;
    assert_eq!(snap.len(), 7, "the folder is not listed again");
    assert!(snap.iter().all(|i| i.state == TransferState::Done));
    for i in 0..6 {
        assert_eq!(
            remote.read_file(&format!("/srv/d/f{i}.bin")),
            local.read_file(&format!("/w/d/f{i}.bin"))
        );
    }
}

/// Collects every event received until `pred` matches one, then (after a
/// `snapshot` round trip, so the actor has emitted everything caused by
/// what it already handled) every event still buffered.
async fn events_until(
    rx: &mut Receiver<Event>,
    engine: &TransferEngine,
    mut pred: impl FnMut(&[EventKind]) -> bool,
) -> Vec<EventKind> {
    let mut seen = Vec::new();
    while !pred(&seen) {
        let ev = wait_for(rx, |_| true).await;
        seen.push(ev);
    }
    engine.snapshot().await;
    while let Ok(Event { kind, .. }) = rx.try_recv() {
        seen.push(kind);
    }
    seen
}

fn requeued(events: &[EventKind], id: TransferId) -> bool {
    events.iter().any(
        |e| matches!(e, EventKind::StateChanged { id: i, state: TransferState::Queued } if *i == id),
    )
}

fn site_paused_count(events: &[EventKind]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, EventKind::SitePaused { .. }))
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn network_requeued_file_resumes_after_manual_pause() {
    let local = MemFs::new();
    let remote_mem = MemFs::new();
    let remote = FaultFs::new(remote_mem.clone());
    let (engine, mut rx) = engine_with(
        Arc::new(local.clone()),
        Arc::new(remote.clone()),
        1,
        EngineConfig::default(),
    )
    .await;
    remote_mem.add_dir("/srv");
    let bytes = data(MIB);
    local.add_file("/w/a", &bytes, mtime());
    remote.fail_io_after(200_000, ErrorKind::Network);

    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a", "/srv/a"))
        .await
        .unwrap();
    wait_for(&mut rx, |e| matches!(e, EventKind::SitePaused { .. })).await;
    assert_eq!(
        remote_mem.read_file("/srv/a").map(|f| f.len()),
        Some(200_000),
        "a partial destination is left behind"
    );

    // Under `Skip` the partial destination would be skipped as existing.
    engine
        .set_policy(hoprs_transfer::ConflictPolicy::Skip)
        .await;
    engine.pause(id).await.unwrap();
    engine.resume_site(site()).await;
    engine.resume(id).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Done);
    assert_eq!(remote_mem.read_file("/srv/a").unwrap(), bytes);
}

#[tokio::test(flavor = "multi_thread")]
async fn network_requeued_file_resumes_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("queue.db");
    let local = MemFs::new();
    let remote = MemFs::new();
    remote.add_dir("/srv");
    let bytes = data(MIB);
    local.add_file("/w/a", &bytes, mtime());

    let fault = FaultFs::new(local.clone());
    fault.fail_io_after(200_000, ErrorKind::Network);
    let (engine, mut rx) = engine_on_file(
        &db,
        Arc::new(fault),
        remote.clone(),
        1,
        EngineConfig::default(),
    )
    .await;
    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a", "/srv/a"))
        .await
        .unwrap();
    wait_for(&mut rx, |e| matches!(e, EventKind::SitePaused { .. })).await;
    engine.shutdown().await;
    assert_eq!(
        remote.read_file("/srv/a").map(|f| f.len()),
        Some(200_000),
        "a partial destination is left behind"
    );

    let config = EngineConfig {
        default_policy: hoprs_transfer::ConflictPolicy::Skip,
        ..EngineConfig::default()
    };
    let fault = FaultFs::new(local.clone());
    let (engine, mut rx) =
        engine_on_file(&db, Arc::new(fault.clone()), remote.clone(), 1, config).await;
    assert_eq!(
        find(&engine.snapshot().await, id).state,
        TransferState::Paused
    );
    engine.resume(id).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Done);
    assert_eq!(remote.read_file("/srv/a").unwrap(), bytes);
    assert_eq!(fault.read_offsets(), vec![200_000]);
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_network_failures_pause_site_once() {
    let local_mem = MemFs::new();
    let remote_mem = MemFs::new();
    let local = FaultFs::new(local_mem.clone());
    let remote = FaultFs::new(remote_mem.clone());
    remote_mem.add_dir("/srv");
    local_mem.add_file("/w/a", &data(MIB), mtime());
    local_mem.add_file("/w/b", &data(MIB), mtime());
    // Whichever copy opens first fails reading; the other then fails writing.
    local.fail_next(Op::OpenRead, ErrorKind::Network);
    remote.fail_next(Op::OpenWrite, ErrorKind::Network);

    let (engine, mut rx) = start_engine(EngineConfig::default());
    let a = engine
        .enqueue(spec(Direction::Upload, "/w/a", "/srv/a"))
        .await
        .unwrap();
    let b = engine
        .enqueue(spec(Direction::Upload, "/w/b", "/srv/b"))
        .await
        .unwrap();
    // Both are dispatched together, so both are in flight when one fails.
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
    let events = events_until(&mut rx, &engine, |seen| {
        requeued(seen, a) && requeued(seen, b)
    })
    .await;
    assert_eq!(site_paused_count(&events), 1, "{events:?}");
    assert!(!events.iter().any(is_failed));
    let snap = engine.snapshot().await;
    assert_eq!(find(&snap, a).state, TransferState::Queued);
    assert_eq!(find(&snap, b).state, TransferState::Queued);
}

#[tokio::test(flavor = "multi_thread")]
async fn site_pauses_again_after_resume() {
    let local = MemFs::new();
    let remote_mem = MemFs::new();
    let remote = FaultFs::new(remote_mem.clone());
    let (engine, mut rx) = engine_with(
        Arc::new(local.clone()),
        Arc::new(remote.clone()),
        1,
        EngineConfig::default(),
    )
    .await;
    remote_mem.add_dir("/srv");
    let bytes = data(10_000);
    local.add_file("/w/a", &bytes, mtime());
    remote.fail_next(Op::OpenWrite, ErrorKind::Network);

    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a", "/srv/a"))
        .await
        .unwrap();
    wait_for(&mut rx, |e| matches!(e, EventKind::SitePaused { .. })).await;

    remote.fail_next(Op::OpenWrite, ErrorKind::Network);
    engine.resume_site(site()).await;
    wait_for(&mut rx, |e| matches!(e, EventKind::SitePaused { .. })).await;

    engine.resume_site(site()).await;
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Done);
    assert_eq!(remote_mem.read_file("/srv/a").unwrap(), bytes);
}

/// Waits until `id` reaches exactly `state`.
async fn wait_state(rx: &mut Receiver<Event>, id: TransferId, state: TransferState) {
    wait_for(
        rx,
        |e| matches!(e, EventKind::StateChanged { id: i, state: s } if *i == id && *s == state),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn overwrite_stopped_while_preparing_overwrites_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("queue.db");
    let local = MemFs::new();
    let remote = MemFs::new();
    let new = data(300_000);
    let old = vec![0x11; 100_000];
    local.add_file("/w/a", &new, mtime());
    remote.add_file("/srv/a", &old, mtime());
    let config = EngineConfig {
        default_policy: hoprs_transfer::ConflictPolicy::Overwrite,
        ..EngineConfig::default()
    };

    // Every call on the source is slow: the item is still preparing (it has
    // not opened the destination) when the engine stops.
    let slow = FaultFs::new(local.clone());
    slow.set_latency(Duration::from_millis(500));
    let (engine, mut rx) =
        engine_on_file(&db, Arc::new(slow), remote.clone(), 1, config.clone()).await;
    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a", "/srv/a"))
        .await
        .unwrap();
    wait_state(&mut rx, id, TransferState::Active).await;
    engine.shutdown().await;
    assert!(remote.read_file("/srv/a").unwrap() == old, "not opened yet");

    let (engine, mut rx) = engine_on_file(
        &db,
        Arc::new(FaultFs::new(local.clone())),
        remote.clone(),
        1,
        config,
    )
    .await;
    assert_eq!(
        find(&engine.snapshot().await, id).state,
        TransferState::Paused
    );
    engine.resume(id).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Done);
    let got = remote.read_file("/srv/a").unwrap();
    assert!(
        got == new,
        "{} bytes, head replaced: {}",
        got.len(),
        got[..16] == new[..16]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn network_override_survives_pause_while_preparing() {
    let local = MemFs::new();
    let remote_mem = MemFs::new();
    let remote = FaultFs::new(remote_mem.clone());
    let (engine, mut rx) = engine_with(
        Arc::new(local.clone()),
        Arc::new(remote.clone()),
        1,
        EngineConfig::default(),
    )
    .await;
    remote_mem.add_dir("/srv");
    let bytes = data(MIB);
    local.add_file("/w/a", &bytes, mtime());
    remote.fail_io_after(200_000, ErrorKind::Network);

    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a", "/srv/a"))
        .await
        .unwrap();
    wait_for(&mut rx, |e| matches!(e, EventKind::SitePaused { .. })).await;
    assert_eq!(
        remote_mem.read_file("/srv/a").map(|f| f.len()),
        Some(200_000)
    );

    // Under `Skip` the partial destination would be skipped as existing.
    engine
        .set_policy(hoprs_transfer::ConflictPolicy::Skip)
        .await;
    remote.set_latency(Duration::from_millis(500));
    engine.resume_site(site()).await;
    wait_state(&mut rx, id, TransferState::Active).await;
    // Paused while preparing (the destination stat is slow).
    engine.pause(id).await.unwrap();
    wait_state(&mut rx, id, TransferState::Paused).await;
    remote.set_latency(Duration::ZERO);

    engine.resume(id).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Done);
    assert!(remote_mem.read_file("/srv/a").unwrap() == bytes);
}

#[tokio::test(flavor = "multi_thread")]
async fn requeued_folder_child_resumes_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("queue.db");
    let local = MemFs::new();
    let remote = MemFs::new();
    remote.add_dir("/srv");
    let bytes = data(4 * MIB);
    local.add_file("/w/d/big.bin", &bytes, mtime());
    let config = EngineConfig {
        buf_size: 64 * 1024,
        ..EngineConfig::default()
    };

    let slow = FaultFs::new(local.clone());
    slow.set_stream_latency(Duration::from_millis(50));
    let (engine, mut rx) =
        engine_on_file(&db, Arc::new(slow), remote.clone(), 1, config.clone()).await;
    let root = engine
        .enqueue(spec(Direction::Upload, "/w/d", "/srv/d"))
        .await
        .unwrap();
    let child = match wait_for(
        &mut rx,
        |e| matches!(e, EventKind::Progress(us) if us.iter().any(|u| u.id != root && u.bytes_done > 0)),
    )
    .await
    {
        EventKind::Progress(us) => us.iter().find(|u| u.id != root).unwrap().id,
        _ => unreachable!(),
    };
    // Paused, then resumed before the copy stopped: the child is requeued,
    // but nothing is dispatched on the paused site.
    engine.pause_site(site()).await;
    engine.pause(root).await.unwrap();
    engine.resume(root).await.unwrap();
    wait_state(&mut rx, child, TransferState::Queued).await;
    engine.shutdown().await;
    let partial = remote.read_file("/srv/d/big.bin").map_or(0, |f| f.len());
    assert!(partial > 0 && partial < bytes.len(), "partial: {partial}");

    let config = EngineConfig {
        default_policy: hoprs_transfer::ConflictPolicy::Skip,
        ..config
    };
    let (engine, mut rx) = engine_on_file(
        &db,
        Arc::new(FaultFs::new(local.clone())),
        remote.clone(),
        1,
        config,
    )
    .await;
    assert_eq!(
        find(&engine.snapshot().await, child).state,
        TransferState::Paused
    );
    engine.resume(root).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, child).await, TransferState::Done);
    assert!(remote.read_file("/srv/d/big.bin").unwrap() == bytes);
}

/// A paused-by-hand site whose running copy then hits a network error.
struct UserPausedOutage {
    engine: TransferEngine,
    rx: Receiver<Event>,
    remote_mem: MemFs,
    /// The copy that failed, then a queued item that never started.
    ids: [TransferId; 2],
    files: [Vec<u8>; 2],
    /// Events up to and including the `SitePaused`.
    events: Vec<EventKind>,
}

/// Starts a slow upload, pauses its site by hand while it copies, then
/// fails its next write with a network error.
async fn network_failure_on_user_paused_site() -> UserPausedOutage {
    let local = MemFs::new();
    let remote_mem = MemFs::new();
    let remote = FaultFs::new(remote_mem.clone());
    remote.set_stream_latency(Duration::from_millis(20));
    let config = EngineConfig {
        buf_size: 64 * 1024,
        ..EngineConfig::default()
    };
    let (engine, mut rx) =
        engine_with(Arc::new(local.clone()), Arc::new(remote.clone()), 1, config).await;
    remote_mem.add_dir("/srv");
    let files = [data(MIB), data(MIB + 1)];
    local.add_file("/w/a", &files[0], mtime());
    local.add_file("/w/b", &files[1], mtime());
    let a = engine
        .enqueue(spec(Direction::Upload, "/w/a", "/srv/a"))
        .await
        .unwrap();
    let b = engine
        .enqueue(spec(Direction::Upload, "/w/b", "/srv/b"))
        .await
        .unwrap();
    wait_bytes(&mut rx, a).await;

    engine.pause_site(site()).await;
    remote.fail_io_after(0, ErrorKind::Network);
    let mut events = Vec::new();
    wait_for(&mut rx, |e| {
        events.push(e.clone());
        matches!(e, EventKind::SitePaused { .. })
    })
    .await;
    UserPausedOutage {
        engine,
        rx,
        remote_mem,
        ids: [a, b],
        files,
        events,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn network_failure_while_user_paused_emits_site_paused() {
    let UserPausedOutage {
        engine,
        mut rx,
        remote_mem,
        ids,
        files,
        events,
    } = network_failure_on_user_paused_site().await;
    match events.last() {
        Some(EventKind::SitePaused { site: s, error }) => {
            assert_eq!(*s, site());
            assert_eq!(error.kind, ErrorKind::Network);
        }
        other => panic!("expected SitePaused, got {other:?}"),
    }
    assert!(!events.iter().any(is_failed));
    let snap = engine.snapshot().await;
    assert_eq!(find(&snap, ids[0]).state, TransferState::Queued);
    assert_eq!(find(&snap, ids[1]).state, TransferState::Queued);

    // An explicit resume clears both pauses.
    engine.resume_site(site()).await;
    let done = wait_all_terminal(&mut rx, &ids).await;
    for (id, bytes) in ids.iter().zip(&files) {
        assert_eq!(done.get(id), Some(&TransferState::Done));
        let name = if *id == ids[0] { "a" } else { "b" };
        assert!(remote_mem.read_file(&format!("/srv/{name}")).as_ref() == Some(bytes));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn user_pause_survives_network_restore() {
    let UserPausedOutage {
        engine,
        mut rx,
        remote_mem,
        ids,
        files,
        ..
    } = network_failure_on_user_paused_site().await;

    engine.network_restored(site()).await;
    let during = events_during(&mut rx, Duration::from_millis(300)).await;
    assert!(
        !during.iter().any(|e| matches!(
            e,
            EventKind::StateChanged {
                state: TransferState::Active,
                ..
            }
        )),
        "the site is still paused by the user: {during:?}"
    );

    engine.resume_site(site()).await;
    let done = wait_all_terminal(&mut rx, &ids).await;
    for id in &ids {
        assert_eq!(done.get(id), Some(&TransferState::Done));
    }
    assert!(remote_mem.read_file("/srv/a").unwrap() == files[0]);
    assert!(remote_mem.read_file("/srv/b").unwrap() == files[1]);
}

/// A failed copy is retried by overwriting its partial destination, and a
/// cancelled one by continuing it, also after a restart and whatever the
/// global policy is then.
#[tokio::test(flavor = "multi_thread")]
async fn failed_and_cancelled_continuations_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("queue.db");
    let local = MemFs::new();
    let remote = MemFs::new();
    remote.add_dir("/srv");
    let files = [
        ("a.bin", data(MIB)),
        ("b.bin", data(4 * MIB)),
        ("c.bin", data(MIB + 7)),
    ];
    for (name, bytes) in &files {
        local.add_file(&format!("/w/d/{name}"), bytes, mtime());
    }
    let config = EngineConfig {
        buf_size: 64 * 1024,
        ..EngineConfig::default()
    };

    let slow = FaultFs::new(local.clone());
    slow.set_stream_latency(Duration::from_millis(20));
    let failing = FaultFs::new(remote.clone());
    // a.bin fails writing after 300 000 bytes (not a network error).
    failing.fail_io_after(300_000, ErrorKind::Io);
    let engine = TransferEngine::start(QueueStore::open(&db).unwrap(), config.clone()).unwrap();
    let mut rx = engine.subscribe();
    engine
        .register_site(
            site(),
            SiteEndpoints {
                local: Arc::new(slow),
                remote: Arc::new(failing),
                workers: 1,
            },
        )
        .await;
    let root = engine
        .enqueue(spec(Direction::Upload, "/w/d", "/srv/d"))
        .await
        .unwrap();
    let a = match wait_for(
        &mut rx,
        |e| matches!(e, EventKind::StateChanged { id, state: TransferState::Failed(_) } if *id != root),
    )
    .await
    {
        EventKind::StateChanged {
            id,
            state: TransferState::Failed(e),
        } => {
            assert_eq!(e.kind, ErrorKind::Io);
            id
        }
        _ => unreachable!(),
    };
    let snap = engine.snapshot().await;
    let child = |name: &str| {
        snap.iter()
            .find(|i| i.spec.local.as_str() == format!("/w/d/{name}"))
            .unwrap()
            .id
    };
    assert_eq!(child("a.bin"), a);
    let b = child("b.bin");
    wait_bytes(&mut rx, b).await;
    // c.bin stays queued.
    engine.pause_site(site()).await;
    engine.cancel(b).await.unwrap();
    wait_state(&mut rx, b, TransferState::Cancelled).await;
    engine.shutdown().await;
    for (name, full) in [("a.bin", MIB), ("b.bin", 4 * MIB)] {
        let partial = remote
            .read_file(&format!("/srv/d/{name}"))
            .map_or(0, |f| f.len());
        assert!(partial > 0 && partial < full, "{name}: {partial}");
    }

    let config = EngineConfig {
        default_policy: hoprs_transfer::ConflictPolicy::Skip,
        ..config
    };
    let fault = FaultFs::new(local.clone());
    let (engine, mut rx) =
        engine_on_file(&db, Arc::new(fault.clone()), remote.clone(), 1, config).await;
    let snap = engine.snapshot().await;
    assert!(matches!(find(&snap, a).state, TransferState::Failed(_)));
    assert_eq!(find(&snap, b).state, TransferState::Cancelled);

    engine.retry(a).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, a).await, TransferState::Done);
    engine.retry(b).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, b).await, TransferState::Done);
    for (name, bytes) in &files[..2] {
        assert!(
            remote.read_file(&format!("/srv/d/{name}")).as_ref() == Some(bytes),
            "{name}"
        );
    }
    let offsets = fault.read_offsets();
    assert_eq!(offsets.len(), 2, "{offsets:?}");
    assert_eq!(offsets[0], 0, "a.bin starts over");
    assert!(offsets[1] > 0, "b.bin continues: {offsets:?}");
}

/// A copy stopped by the engine shutting down whose write error only
/// surfaces when its destination is shut down is never continued from that
/// destination after a restart, even under `Skip`: it is rewritten.
#[tokio::test(flavor = "multi_thread")]
async fn shutdown_with_deferred_write_error_overwrites_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("queue.db");
    let local = MemFs::new();
    let remote_mem = MemFs::new();
    remote_mem.add_dir("/srv");
    let bytes = data(4 * MIB);
    local.add_file("/w/a.bin", &bytes, mtime());
    let config = EngineConfig {
        buf_size: 64 * 1024,
        ..EngineConfig::default()
    };

    let slow = FaultFs::new(local.clone());
    slow.set_stream_latency(Duration::from_millis(20));
    let remote = FaultFs::new(remote_mem.clone());
    remote.lose_write_after(100_000, ErrorKind::Io);
    let engine = TransferEngine::start(QueueStore::open(&db).unwrap(), config.clone()).unwrap();
    engine
        .register_site(site(), endpoints_fault(slow, remote, 1))
        .await;
    let id = engine
        .enqueue(spec(Direction::Upload, "/w/a.bin", "/srv/a.bin"))
        .await
        .unwrap();
    // Wait until a chunk written after the lost one has landed.
    tokio::time::timeout(Duration::from_secs(5), async {
        while remote_mem
            .read_file("/srv/a.bin")
            .is_none_or(|f| f.len() < 100_000)
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("chunks after the lost one land");
    engine.shutdown().await;

    let fault = FaultFs::new(local.clone());
    let engine = TransferEngine::start(QueueStore::open(&db).unwrap(), config).unwrap();
    let mut rx = engine.subscribe();
    engine
        .register_site(
            site(),
            endpoints_fault(fault.clone(), FaultFs::new(remote_mem.clone()), 1),
        )
        .await;
    engine
        .set_policy(hoprs_transfer::ConflictPolicy::Skip)
        .await;
    engine.resume(id).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, id).await, TransferState::Done);
    assert!(remote_mem.read_file("/srv/a.bin").unwrap() == bytes);
    assert_eq!(fault.read_offsets(), vec![0], "the resume starts over");
}

fn endpoints_fault(local: FaultFs<MemFs>, remote: FaultFs<MemFs>, workers: usize) -> SiteEndpoints {
    SiteEndpoints {
        local: Arc::new(local),
        remote: Arc::new(remote),
        workers,
    }
}

/// RF5: a folder's inherited policy and origin are stored with its children.
#[tokio::test(flavor = "multi_thread")]
async fn inherited_policy_and_origin_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("queue.db");
    let local = MemFs::new();
    let remote = MemFs::new();
    local.add_file("/w/d/a.txt", b"new a", mtime());
    local.add_file("/w/d/b.txt", b"new b", mtime());
    local.add_file("/w/d/c.txt", b"new c", mtime());
    remote.add_file("/srv/d/a.txt", b"old a", mtime());
    remote.add_file("/srv/d/b.txt", b"old b", mtime());

    // Slow enough that the pause below reaches the folder while it is listed.
    let slow = FaultFs::new(local.clone());
    slow.set_latency(Duration::from_millis(100));
    let (engine, mut rx) = engine_on_file(
        &db,
        Arc::new(slow),
        remote.clone(),
        4,
        EngineConfig::default(),
    )
    .await;
    let root = engine
        .enqueue_with(
            spec(Direction::Upload, "/w/d", "/srv/d"),
            EnqueueOptions {
                policy: Some(ConflictPolicy::Skip),
                origin: Origin::Agent,
            },
        )
        .await
        .unwrap();
    // Paused while listing: the children are created paused and never run.
    engine.pause(root).await.unwrap();
    wait_for(
        &mut rx,
        |e| matches!(e, EventKind::StateChanged { id, state: TransferState::Paused } if *id == root),
    )
    .await;
    assert_eq!(engine.snapshot().await.len(), 4);
    engine.shutdown().await;

    let (engine, mut rx) = engine_on_file(
        &db,
        Arc::new(FaultFs::new(local.clone())),
        remote.clone(),
        4,
        EngineConfig::default(),
    )
    .await;
    let snap = engine.snapshot().await;
    assert_eq!(snap.len(), 4, "the folder and its three children");
    for i in &snap {
        assert_eq!(i.origin, Origin::Agent, "{i:?}");
        assert_eq!(i.tree_policy, Some(ConflictPolicy::Skip), "{i:?}");
        assert_eq!(i.state, TransferState::Paused, "{i:?}");
    }

    engine.resume(root).await.unwrap();
    let mut seen = Vec::new();
    wait_for(&mut rx, |e| {
        seen.push(e.clone());
        matches!(e, EventKind::StateChanged { id, state } if *id == root && state.is_terminal())
    })
    .await;
    assert!(
        !seen.iter().any(|e| matches!(
            e,
            EventKind::StateChanged {
                state: TransferState::AwaitingDecision(_),
                ..
            }
        )),
        "{seen:?}"
    );
    let snap = engine.snapshot().await;
    let state_of = |p: &str| {
        snap.iter()
            .find(|i| i.spec.local.as_str() == p)
            .map(|i| i.state.clone())
            .unwrap()
    };
    assert_eq!(state_of("/w/d"), TransferState::Done);
    for skipped in ["/w/d/a.txt", "/w/d/b.txt"] {
        assert_eq!(
            state_of(skipped),
            TransferState::Skipped("exists".to_string()),
            "{skipped}"
        );
    }
    assert_eq!(state_of("/w/d/c.txt"), TransferState::Done);
    assert_eq!(remote.read_file("/srv/d/a.txt").unwrap(), b"old a");
    assert_eq!(remote.read_file("/srv/d/c.txt").unwrap(), b"new c");
}
