//! Tests of remote editing, over `MemFs` and a temporary directory.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use hoprs_core::{ErrorKind, RemoteEntry};
use hoprs_remote::{BoxAsyncRead, BoxAsyncWrite, MemFs};
use tokio::sync::{Notify, Semaphore};
use tokio::time::Instant;

use super::*;

/// `MemFs` that counts reads and writes, can lose the network, cut the next write after
/// truncating the file, and hold `stat` at a gate.
#[derive(Default)]
struct CountingFs {
    inner: MemFs,
    writes: AtomicUsize,
    reads: AtomicUsize,
    offline: AtomicBool,
    /// The next `stat`s that fail with a network error.
    stat_failures: AtomicUsize,
    /// The next `open_write` truncates the file, then fails with this.
    cut_next_write: Mutex<Option<ErrorKind>>,
    /// While set, `stat` waits for a permit (after telling `stat_entered`).
    stat_gate: Mutex<Option<Arc<Semaphore>>>,
    stat_entered: Notify,
    /// Every `stat` call, counted on entry.
    stats: AtomicUsize,
    /// While set, `open_write` truncates the file, then waits for a permit (a slow upload
    /// already under way).
    write_gate: Mutex<Option<Arc<Semaphore>>>,
}

impl CountingFs {
    fn new(inner: MemFs) -> Self {
        Self {
            inner,
            ..Self::default()
        }
    }

    fn check(&self) -> Result<()> {
        if self.offline.load(Ordering::SeqCst) {
            Err(Error::new(ErrorKind::Network, "Connection lost"))
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl RemoteFs for CountingFs {
    async fn list(&self, path: &RemotePath) -> Result<Vec<RemoteEntry>> {
        self.check()?;
        self.inner.list(path).await
    }
    async fn stat(&self, path: &RemotePath) -> Result<RemoteEntry> {
        self.stats.fetch_add(1, Ordering::SeqCst);
        let gate = self.stat_gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            self.stat_entered.notify_one();
            // Each permit lets one `stat` through.
            gate.acquire().await.unwrap().forget();
        }
        self.check()?;
        let failures = self.stat_failures.load(Ordering::SeqCst);
        if failures > 0 {
            self.stat_failures.store(failures - 1, Ordering::SeqCst);
            return Err(Error::new(ErrorKind::Network, "Connection lost"));
        }
        self.inner.stat(path).await
    }
    async fn open_read(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncRead> {
        self.check()?;
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.inner.open_read(path, offset).await
    }
    async fn open_write(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncWrite> {
        self.check()?;
        self.writes.fetch_add(1, Ordering::SeqCst);
        let cut = self.cut_next_write.lock().unwrap().take();
        if let Some(kind) = cut {
            // Truncated, then the connection (or the quota) gives out.
            drop(self.inner.open_write(path, offset).await?);
            return Err(Error::new(kind, "Write cut"));
        }
        let gate = self.write_gate.lock().unwrap().clone();
        let writer = self.inner.open_write(path, offset).await?;
        if let Some(gate) = gate {
            let _permit = gate.acquire().await.unwrap();
        }
        Ok(writer)
    }
    async fn mkdir(&self, path: &RemotePath) -> Result<()> {
        self.check()?;
        self.inner.mkdir(path).await
    }
    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()> {
        self.check()?;
        self.inner.rename(from, to).await
    }
    async fn remove_file(&self, path: &RemotePath) -> Result<()> {
        self.check()?;
        self.inner.remove_file(path).await
    }
    async fn remove_dir(&self, path: &RemotePath) -> Result<()> {
        self.check()?;
        self.inner.remove_dir(path).await
    }
    async fn chmod(&self, path: &RemotePath, mode: u32) -> Result<()> {
        self.check()?;
        self.inner.chmod(path, mode).await
    }
}

type Launched = Arc<Mutex<Vec<(String, PathBuf)>>>;

struct Harness {
    _dir: tempfile::TempDir,
    root: PathBuf,
    recovered: PathBuf,
    edits: EditManager,
    mem: MemFs,
    fs: Arc<CountingFs>,
    state: watch::Sender<ConnectionState>,
    launched: Launched,
    events: broadcast::Receiver<EditEvent>,
}

const SESSION: SessionId = SessionId(7);
const SITE: &str = "site-1";
const FILE: &str = "/home/alice/notes.txt";

fn rp(path: &str) -> RemotePath {
    RemotePath::new(path).unwrap()
}

fn harness(watch_files: bool) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("edit");
    let recovered = dir.path().join("data").join("recovered");
    let launched: Launched = Arc::default();
    let record = launched.clone();
    let launcher: Launcher = Arc::new(move |command: &str, path: &Path| {
        record
            .lock()
            .unwrap()
            .push((command.to_owned(), path.to_owned()));
        Ok(())
    });
    let edits = EditManager::with_options(root.clone(), recovered.clone(), watch_files, launcher);
    let mem = MemFs::new();
    mem.add_file(
        FILE,
        b"version 1\n",
        UNIX_EPOCH + Duration::from_secs(1_000),
    );
    let fs = Arc::new(CountingFs::new(mem.clone()));
    let events = edits.subscribe();
    Harness {
        _dir: dir,
        root,
        recovered,
        edits,
        mem,
        fs,
        state: watch::channel(ConnectionState::Connected).0,
        launched,
        events,
    }
}

impl Harness {
    async fn start(&self, path: &str) -> Result<EditId> {
        self.start_with(path, UploadMode::TempRename).await
    }

    async fn start_with(&self, path: &str, mode: UploadMode) -> Result<EditId> {
        let target = EditSession {
            session: SESSION,
            site: SITE.to_owned(),
            fs: self.fs.clone(),
            state: self.state.subscribe(),
            mode,
        };
        self.edits.start(target, rp(path), "my-editor --wait").await
    }

    fn writes(&self) -> usize {
        self.fs.writes.load(Ordering::SeqCst)
    }

    /// The editor saves `data` (a plain write) and the watcher reports it.
    fn save(&self, id: EditId, data: &[u8]) {
        let local = self.edits.local_path(id).unwrap();
        std::fs::write(&local, data).unwrap();
        self.edits.file_event(SESSION, local);
    }

    async fn next_event(&mut self) -> EditEvent {
        tokio::time::timeout(Duration::from_secs(30), self.events.recv())
            .await
            .expect("an edit event")
            .expect("the event channel is open")
    }

    /// The recovered copies of `FILE`.
    fn recovered_copies(&self) -> Vec<PathBuf> {
        let dir = self.recovered.join(SITE).join("home").join("alice");
        let mut copies: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|entries| entries.map(|e| e.unwrap().path()).collect())
            .unwrap_or_default();
        copies.sort();
        copies
    }
}

fn held(id: EditId) -> EditEvent {
    EditEvent::Pending {
        id,
        remote_may_be_truncated: false,
    }
}

/// Lets the worker take what was sent to it, without moving the paused clock.
async fn settle() {
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
}

#[cfg(unix)]
#[tokio::test]
async fn download_goes_to_private_dir_with_0700_0600() {
    use std::os::unix::fs::PermissionsExt;

    let h = harness(false);
    let id = h.start(FILE).await.unwrap();
    let local = h.edits.local_path(id).unwrap();

    let session_dir = h.root.join(SESSION.0.to_string());
    assert_eq!(
        local,
        std::fs::canonicalize(&session_dir)
            .unwrap()
            .join("home")
            .join("alice")
            .join("notes.txt")
    );
    assert_eq!(std::fs::read(&local).unwrap(), b"version 1\n");
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&h.root), 0o700);
    assert_eq!(mode(&session_dir), 0o700);
    assert_eq!(mode(&session_dir.join("home")), 0o700);
    assert_eq!(mode(&session_dir.join("home/alice")), 0o700);
    assert_eq!(mode(&local), 0o600);

    // The editor got the command and the path, once.
    let launched = h.launched.lock().unwrap().clone();
    assert_eq!(launched, vec![("my-editor --wait".to_owned(), local)]);
    // Nothing went back up.
    assert_eq!(h.writes(), 0);
}

#[test]
fn path_traversal_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("7");
    for bad in [
        "/../etc/passwd",
        "/a/../../etc/passwd",
        "/a/./b",
        "/a//b",
        "relative/path",
        "/",
        "",
        "/a/..",
        "/a\\..\\..\\b",
        "/nul\0byte",
    ] {
        let err = local_path_for(&session, bad).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Permission, "{bad:?} must be refused");
    }
    assert_eq!(
        local_path_for(&session, "/srv/www/index.html").unwrap(),
        session.join("srv").join("www").join("index.html")
    );
    // A name that merely contains dots is fine.
    assert_eq!(
        local_path_for(&session, "/a/..b/c..").unwrap(),
        session.join("a").join("..b").join("c..")
    );
}

#[tokio::test(start_paused = true)]
async fn save_uploads_after_500ms_debounce() {
    let mut h = harness(false);
    let id = h.start(FILE).await.unwrap();

    // Three saves 200 ms apart: one upload, 500 ms after the last.
    h.save(id, b"draft a\n");
    settle().await;
    tokio::time::advance(Duration::from_millis(200)).await;
    h.save(id, b"draft b\n");
    settle().await;
    tokio::time::advance(Duration::from_millis(200)).await;
    h.save(id, b"final\n");
    let last = Instant::now();
    settle().await;
    tokio::time::advance(DEBOUNCE - Duration::from_millis(1)).await;
    settle().await;
    assert!(Instant::now() - last < DEBOUNCE);
    assert_eq!(h.writes(), 0, "no upload before 500 ms of quiet");
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"version 1\n");

    let event = h.next_event().await;
    assert!(
        matches!(event, EditEvent::Uploaded { id: got, .. } if got == id),
        "{event:?}"
    );
    assert_eq!(h.writes(), 1, "the three saves made one upload");
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"final\n");

    // A watcher event without a change of content uploads nothing.
    let local = h.edits.local_path(id).unwrap();
    h.edits.file_event(SESSION, local);
    tokio::time::advance(DEBOUNCE * 2).await;
    settle().await;
    // The worker handles its files in order: had that event uploaded, this save would
    // be the third write.
    h.save(id, b"later\n");
    tokio::time::advance(DEBOUNCE).await;
    assert!(matches!(h.next_event().await, EditEvent::Uploaded { .. }));
    assert_eq!(h.writes(), 2);
    // No temporary file left beside the original.
    let names: Vec<String> = h
        .mem
        .list(&rp("/home/alice"))
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, vec!["notes.txt".to_owned()]);
}

/// The real watcher (`notify`), real files, real time: an editor that saves through a
/// temporary file renamed over the original, then one that removes and recreates it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn atomic_rename_saves_are_seen() {
    let mut h = harness(true);
    let id = h.start(FILE).await.unwrap();
    let local = h.edits.local_path(id).unwrap();
    let dir = local.parent().unwrap().to_owned();

    // vim/gedit style: write a temporary file, rename it over the original.
    let temp = dir.join(".notes.txt.swp");
    std::fs::write(&temp, b"renamed over\n").unwrap();
    std::fs::rename(&temp, &local).unwrap();
    loop {
        if let EditEvent::Uploaded { id: got, .. } = h.next_event().await {
            assert_eq!(got, id);
            break;
        }
    }
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"renamed over\n");

    // Remove, then create anew.
    std::fs::remove_file(&local).unwrap();
    std::fs::write(&local, b"recreated\n").unwrap();
    loop {
        if let EditEvent::Uploaded { id: got, .. } = h.next_event().await {
            assert_eq!(got, id);
            break;
        }
    }
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"recreated\n");
}

/// Review Focus 2: the server's copy changed after the download; the save is not
/// uploaded, the UI is told, and nothing is overwritten until the user chooses.
#[tokio::test(start_paused = true)]
async fn remote_change_blocks_upload_and_offers_choices() {
    let mut h = harness(false);
    let id = h.start(FILE).await.unwrap();

    // Someone else changes the file on the server (same size, newer mtime).
    h.mem.add_file(
        FILE,
        b"version X\n",
        UNIX_EPOCH + Duration::from_secs(2_000),
    );
    h.save(id, b"my change\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, EditEvent::RemoteChanged { id });
    assert_eq!(h.writes(), 0);
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"version X\n");
    let pending = h.edits.pending(SESSION).await;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].state, EditState::Conflict);

    // Saving again still does not overwrite it.
    h.save(id, b"my change, again\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, EditEvent::RemoteChanged { id });
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"version X\n");

    // A change of size only is caught too.
    let other = harness(false);
    let id2 = other.start(FILE).await.unwrap();
    other.mem.add_file(
        FILE,
        b"longer v1\n!",
        UNIX_EPOCH + Duration::from_secs(1_000),
    );
    let mut events = other.edits.subscribe();
    other.save(id2, b"mine\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(
        events.recv().await.unwrap(),
        EditEvent::RemoteChanged { id: id2 }
    );

    // "Overwrite": the user's version goes up.
    h.edits.resolve(id, EditChoice::Overwrite).await.unwrap();
    assert!(matches!(h.next_event().await, EditEvent::Uploaded { .. }));
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"my change, again\n");
    assert!(h.edits.pending(SESSION).await.is_empty());

    // And later saves upload normally against the new stamp.
    h.save(id, b"after overwrite\n");
    tokio::time::advance(DEBOUNCE).await;
    assert!(matches!(h.next_event().await, EditEvent::Uploaded { .. }));
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"after overwrite\n");
}

#[tokio::test(start_paused = true)]
async fn keep_remote_preserves_local_copy() {
    let mut h = harness(false);
    let id = h.start(FILE).await.unwrap();
    let local = h.edits.local_path(id).unwrap();
    h.mem.add_file(
        FILE,
        b"their version\n",
        UNIX_EPOCH + Duration::from_secs(2_000),
    );
    h.save(id, b"my version\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, EditEvent::RemoteChanged { id });

    h.edits.resolve(id, EditChoice::KeepRemote).await.unwrap();
    assert_eq!(std::fs::read(&local).unwrap(), b"their version\n");
    let copies = h.recovered_copies();
    assert_eq!(copies.len(), 1, "one recovered copy: {copies:?}");
    let name = copies[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(name.starts_with("notes.txt.local-"), "{name}");
    assert_eq!(std::fs::read(&copies[0]).unwrap(), b"my version\n");
    assert_eq!(
        h.next_event().await,
        EditEvent::ReplacedByRemote {
            id,
            recovered: Some(copies[0].clone()),
            reload_editor: true,
        }
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&copies[0]), 0o600);
        assert_eq!(mode(&h.recovered), 0o700);
        assert_eq!(mode(copies[0].parent().unwrap()), 0o700);
    }
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"their version\n");
    assert_eq!(h.writes(), 0);
    assert!(h.edits.pending(SESSION).await.is_empty());

    // The next save goes up against the stamp of what was kept.
    h.save(id, b"merged\n");
    tokio::time::advance(DEBOUNCE).await;
    assert!(matches!(h.next_event().await, EditEvent::Uploaded { .. }));
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"merged\n");

    // A second conflict in the same second: a new copy, never over the first.
    h.mem
        .add_file(FILE, b"theirs 2\n", UNIX_EPOCH + Duration::from_secs(3_000));
    h.save(id, b"mine 2\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, EditEvent::RemoteChanged { id });
    h.edits.resolve(id, EditChoice::KeepRemote).await.unwrap();
    let copies = h.recovered_copies();
    assert_eq!(copies.len(), 2, "{copies:?}");
    let contents: Vec<Vec<u8>> = copies.iter().map(|c| std::fs::read(c).unwrap()).collect();
    assert!(contents.contains(&b"my version\n".to_vec()));
    assert!(contents.contains(&b"mine 2\n".to_vec()));

    // Finishing the edit (nothing unsent) removes the edit directory, not the copies.
    assert!(h.edits.finish(SESSION, false).await.unwrap().is_empty());
    assert!(!local.exists());
    for copy in &copies {
        assert!(copy.exists(), "{copy:?} survives finish");
    }
}

#[tokio::test(start_paused = true)]
async fn save_copy_uploads_beside() {
    let mut h = harness(false);
    let id = h.start(FILE).await.unwrap();
    let local = h.edits.local_path(id).unwrap();
    h.mem.add_file(
        FILE,
        b"their version\n",
        UNIX_EPOCH + Duration::from_secs(2_000),
    );
    h.save(id, b"my version\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, EditEvent::RemoteChanged { id });

    h.edits.resolve(id, EditChoice::SaveCopy).await.unwrap();
    let entries = h.mem.list(&rp("/home/alice")).await.unwrap();
    let copy = entries
        .iter()
        .find(|e| e.name.starts_with("notes.txt.hoprs-"))
        .expect("a copy beside the original");
    assert_eq!(entries.len(), 2, "{entries:?}");
    assert_eq!(
        h.mem.read_file(copy.path.as_str()).unwrap(),
        b"my version\n"
    );
    // The original is untouched, and the local file now follows it.
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"their version\n");
    assert_eq!(std::fs::read(&local).unwrap(), b"their version\n");
    assert!(h.edits.pending(SESSION).await.is_empty());
    // The user's version is also kept locally, durably.
    let copies = h.recovered_copies();
    assert_eq!(copies.len(), 1, "{copies:?}");
    assert_eq!(std::fs::read(&copies[0]).unwrap(), b"my version\n");
    assert_eq!(
        h.next_event().await,
        EditEvent::ReplacedByRemote {
            id,
            recovered: Some(copies[0].clone()),
            reload_editor: true,
        }
    );

    // Later saves still target the original, with the conflict check.
    h.save(id, b"continued\n");
    tokio::time::advance(DEBOUNCE).await;
    assert!(matches!(h.next_event().await, EditEvent::Uploaded { .. }));
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"continued\n");

    assert!(h.edits.finish(SESSION, false).await.unwrap().is_empty());
    assert!(!local.exists());
    assert!(copies[0].exists(), "the recovered copy survives finish");
}

/// Review Focus 3: a save while the session is down is held, then uploaded by itself
/// once the session is connected again.
#[tokio::test(start_paused = true)]
async fn pending_edit_uploads_after_reconnect() {
    let mut h = harness(false);
    let id = h.start(FILE).await.unwrap();

    // The connection drops (idle timeout); the transport is reconnecting.
    h.fs.offline.store(true, Ordering::SeqCst);
    h.state
        .send_replace(ConnectionState::Reconnecting { attempt: 1 });
    h.save(id, b"saved while down\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, held(id));
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"version 1\n");
    let pending = h.edits.pending(SESSION).await;
    assert_eq!(pending[0].state, EditState::Pending);

    // Gave up, then the user (or the monitor) reconnects: still pending meanwhile.
    h.state
        .send_replace(ConnectionState::Disconnected(Error::new(
            ErrorKind::Network,
            "Connection lost",
        )));
    settle().await;
    assert_eq!(h.writes(), 0);

    h.fs.offline.store(false, Ordering::SeqCst);
    h.state.send_replace(ConnectionState::Connected);
    assert!(matches!(
        h.next_event().await,
        EditEvent::Uploaded { id: got, .. } if got == id
    ));
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"saved while down\n");
    assert!(h.edits.pending(SESSION).await.is_empty());

    // The state said connected but the link was dead: the failed upload is held too.
    h.fs.offline.store(true, Ordering::SeqCst);
    h.save(id, b"second\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, held(id));
    h.state
        .send_replace(ConnectionState::Reconnecting { attempt: 1 });
    settle().await;
    h.fs.offline.store(false, Ordering::SeqCst);
    h.state.send_replace(ConnectionState::Connected);
    assert!(matches!(h.next_event().await, EditEvent::Uploaded { .. }));
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"second\n");
}

#[tokio::test(start_paused = true)]
async fn finish_keeps_files_with_pending_changes() {
    let mut h = harness(false);
    let id = h.start(FILE).await.unwrap();
    let local = h.edits.local_path(id).unwrap();
    let session_dir = local.parent().unwrap().parent().unwrap().parent().unwrap();
    assert_eq!(session_dir.file_name().unwrap(), "7");

    // Saved, but not uploaded yet (the watcher has not even said so).
    std::fs::write(&local, b"unsent\n").unwrap();
    let kept = h.edits.finish(SESSION, false).await.unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].id, id);
    assert_eq!(kept[0].state, EditState::Modified);
    assert_eq!(kept[0].remote, rp(FILE));
    assert!(local.exists(), "pending changes keep the file");

    // Discarding removes them.
    let kept = h.edits.finish(SESSION, true).await.unwrap();
    assert!(kept.is_empty());
    assert!(!session_dir.exists());
    assert_eq!(h.next_event().await, EditEvent::Closed { id });
    assert!(h.edits.local_path(id).is_none());

    // Without pending changes the files go without asking.
    let id = h.start(FILE).await.unwrap();
    let local = h.edits.local_path(id).unwrap();
    assert!(h.edits.finish(SESSION, false).await.unwrap().is_empty());
    assert!(!local.exists());
    assert_eq!(h.next_event().await, EditEvent::Closed { id });
}

/// The argv of `editor_launch` (panics on `System` or an error).
fn launch_argv(command: &str, path: &Path) -> Vec<String> {
    match editor_launch(command, path).unwrap() {
        EditorLaunch::Argv(argv) => argv.into_iter().map(|a| a.into_string().unwrap()).collect(),
        EditorLaunch::System => panic!("{command:?} is not a command line"),
    }
}

#[test]
fn editor_command_is_split_without_shell() {
    let path = Path::new("/run/user/1000/hoprs/edit/7/my file; rm -rf ~.txt");
    let argv = |command: &str| launch_argv(command, path);
    let last = path.to_str().unwrap().to_owned();
    // A saved `xdg-open` (the default before `system`) keeps working.
    assert_eq!(argv("xdg-open"), vec!["xdg-open".to_owned(), last.clone()]);
    assert_eq!(argv("code --wait"), vec!["code", "--wait", last.as_str()]);
    assert_eq!(
        argv("code --wait --new-window"),
        vec!["code", "--wait", "--new-window", last.as_str()]
    );
    let message = |command: &str| editor_launch(command, path).unwrap_err().message;
    assert_eq!(
        message("\"unterminated"),
        "The editor command has an unmatched quote"
    );
    for blank in ["", "   "] {
        assert_eq!(message(blank), "No editor command is set", "{blank:?}");
    }
}

/// POSIX quoting (Linux, macOS): single quotes, and shell syntax stays literal.
#[cfg(unix)]
#[test]
fn editor_command_uses_posix_quoting_on_unix() {
    let path = Path::new("/run/user/1000/hoprs/edit/7/my file; rm -rf ~.txt");
    let argv = |command: &str| launch_argv(command, path);
    let last = path.to_str().unwrap();
    assert_eq!(
        argv("'/opt/My Editor/bin/edit' -n \"two words\""),
        vec!["/opt/My Editor/bin/edit", "-n", "two words", last]
    );
    // No expansion, no command substitution, no pipes.
    assert_eq!(
        argv("vim $HOME $(id) | tee"),
        vec!["vim", "$HOME", "$(id)", "|", "tee", last]
    );
    assert!(editor_launch("'unterminated", path).is_err());
}

#[test]
fn system_is_the_os_default_editor() {
    let path = Path::new("/run/user/1000/hoprs/edit/7/notes.conf");
    for command in ["system", " System ", "SYSTEM"] {
        assert!(
            matches!(editor_launch(command, path), Ok(EditorLaunch::System)),
            "{command:?}"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn system_argv_on_linux_is_xdg_open() {
    let path = Path::new("/run/user/1000/hoprs/edit/7/notes.conf");
    assert_eq!(
        system_argv(path),
        vec![OsString::from("xdg-open"), path.as_os_str().to_owned()]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn system_argv_on_macos_is_open_text() {
    let path = Path::new("/private/var/folders/x/hoprs/edit/7/notes.conf");
    assert_eq!(
        system_argv(path),
        vec![
            OsString::from("open"),
            OsString::from("-t"),
            path.as_os_str().to_owned()
        ]
    );
}

/// Windows command-line rules (as `CommandLineToArgvW`); pure, so tested on every OS.
#[test]
fn windows_rules_split_like_command_line_to_argv() {
    let split = |command: &str| split_windows(command).map(|w| w.join("|"));
    let some = |s: &str| Some(s.to_owned());
    // Backslashes are path separators, not escapes.
    assert_eq!(split(r"C:\Tools\vim.exe"), some(r"C:\Tools\vim.exe"));
    assert_eq!(
        split(r#""C:\Program Files\Notepad++\notepad++.exe" -multiInst"#),
        some(r"C:\Program Files\Notepad++\notepad++.exe|-multiInst")
    );
    // The program name: quotes delimit it, a trailing backslash stays.
    assert_eq!(split(r#""C:\My Editor\" -n"#), some(r"C:\My Editor\|-n"));
    assert_eq!(split("  code\t --wait  "), some("code|--wait"));
    // Arguments: 2n backslashes + quote are n backslashes and a quote toggle; 2n+1 are
    // n backslashes and a literal quote; `""` inside quotes is a literal quote.
    assert_eq!(split(r#"e "a b" c\d"#), some(r"e|a b|c\d"));
    assert_eq!(split(r#"e a\"b"#), some(r#"e|a"b"#));
    assert_eq!(split(r#"e "a\\" b"#), some(r"e|a\|b"));
    assert_eq!(split(r#"e "a""b" """#), some(r#"e|a"b|"#));
    assert_eq!(
        split(r#"e 'single quotes' stay"#),
        some("e|'single|quotes'|stay")
    );
    assert_eq!(split(""), some(""));
    assert_eq!(split("   ").map(|s| s.is_empty()), Some(true));
    // An unmatched quote is refused (CommandLineToArgvW would close it at the end).
    assert_eq!(split(r#""C:\Program Files\x.exe"#), None);
    assert_eq!(split(r#"e "open"#), None);
}

#[cfg(windows)]
#[test]
fn split_command_uses_windows_rules_on_windows() {
    let words = |command: &str| split_command(command).unwrap();
    assert_eq!(
        words(r#""C:\Program Files\Notepad++\notepad++.exe" -multiInst"#),
        vec![r"C:\Program Files\Notepad++\notepad++.exe", "-multiInst"]
    );
    assert_eq!(words(r"C:\Tools\vim.exe"), vec![r"C:\Tools\vim.exe"]);
}

/// The real launcher: the path reaches the program as one argument, shell characters
/// and all, and nothing in it runs.
#[cfg(unix)]
#[test]
fn launcher_passes_the_path_as_one_argument() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("argv");
    let pwned = dir.path().join("pwned");
    let path = dir
        .path()
        .join(format!("x; touch {}", pwned.to_str().unwrap()));
    // `sh -c SCRIPT NAME ARG`: the script writes its first argument ($1) to `out`.
    let command = format!(
        "sh -c 'printf %s \"$1\" > \"$0\"' {}",
        shlex::try_quote(out.to_str().unwrap()).unwrap()
    );
    launch_editor(&command, &path).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !out.exists() || std::fs::read(&out).unwrap().is_empty() {
        assert!(std::time::Instant::now() < deadline, "the editor never ran");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        path.to_str().unwrap()
    );
    assert!(!pwned.exists());
}

/// Important 1 (fix round 1): the drop and the reconnect arrive back to back while the
/// worker is busy with the sync that hits the dead link; the held edit still goes up,
/// exactly once.
#[tokio::test(start_paused = true)]
async fn coalesced_reconnect_still_uploads_the_held_edit() {
    let mut h = harness(false);
    let id = h.start(FILE).await.unwrap();
    let gate = Arc::new(Semaphore::new(0));
    *h.fs.stat_gate.lock().unwrap() = Some(gate.clone());
    h.fs.stat_failures.store(1, Ordering::SeqCst);

    // The state still says connected; the link is dead: the sync's stat will fail.
    h.save(id, b"saved on a dead link\n");
    tokio::time::advance(DEBOUNCE).await;
    h.fs.stat_entered.notified().await;
    // While the worker waits on that stat: the drop and the reconnect, back to back.
    h.state
        .send_replace(ConnectionState::Reconnecting { attempt: 1 });
    h.state.send_replace(ConnectionState::Connected);
    *h.fs.stat_gate.lock().unwrap() = None;
    gate.add_permits(1);

    assert_eq!(h.next_event().await, held(id));
    assert!(matches!(
        h.next_event().await,
        EditEvent::Uploaded { id: got, .. } if got == id
    ));
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"saved on a dead link\n");
    // The backoff retry that was scheduled finds nothing left to send.
    tokio::time::advance(RETRY_MAX * 2).await;
    settle().await;
    h.save(id, b"next\n");
    tokio::time::advance(DEBOUNCE).await;
    assert!(matches!(h.next_event().await, EditEvent::Uploaded { .. }));
    assert_eq!(
        h.writes(),
        2,
        "the held edit went up once, then the next save"
    );
}

/// A save held while the session still says `Connected` is retried by itself.
#[tokio::test(start_paused = true)]
async fn held_save_on_a_connected_session_is_retried() {
    let mut h = harness(false);
    let id = h.start(FILE).await.unwrap();
    h.fs.stat_failures.store(2, Ordering::SeqCst);
    h.save(id, b"retried\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, held(id));
    // First retry after RETRY_MIN: fails again; the next waits twice as long.
    assert_eq!(h.next_event().await, held(id));
    assert!(matches!(h.next_event().await, EditEvent::Uploaded { .. }));
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"retried\n");
}

/// Important 2, network path: an in-place write cut by the network is reported as
/// possibly truncated; the first retry after the reconnect skips the stamp check (the
/// damage was ours), and the check is back afterwards.
#[tokio::test(start_paused = true)]
async fn cut_in_place_upload_by_network_skips_the_check_once() {
    let mut h = harness(false);
    let id = h.start_with(FILE, UploadMode::InPlace).await.unwrap();
    *h.fs.cut_next_write.lock().unwrap() = Some(ErrorKind::Network);
    h.save(id, b"mine\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(
        h.next_event().await,
        EditEvent::Pending {
            id,
            remote_may_be_truncated: true
        }
    );
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"", "truncated by the cut");

    h.state
        .send_replace(ConnectionState::Reconnecting { attempt: 1 });
    settle().await;
    h.state.send_replace(ConnectionState::Connected);
    assert!(matches!(h.next_event().await, EditEvent::Uploaded { .. }));
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"mine\n");

    // Not a lasting pass: someone else's change is caught again.
    h.mem
        .add_file(FILE, b"theirs\n", UNIX_EPOCH + Duration::from_secs(5_000));
    h.save(id, b"mine again\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, EditEvent::RemoteChanged { id });
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"theirs\n");
}

/// Important 2, other failures (quota, permission): the damaged file becomes the
/// baseline at once, and later saves are checked against it.
#[tokio::test(start_paused = true)]
async fn cut_in_place_upload_by_quota_adopts_the_damage_as_baseline() {
    let mut h = harness(false);
    let id = h.start_with(FILE, UploadMode::InPlace).await.unwrap();
    *h.fs.cut_next_write.lock().unwrap() = Some(ErrorKind::Permission);
    h.save(id, b"mine\n");
    tokio::time::advance(DEBOUNCE).await;
    match h.next_event().await {
        EditEvent::Failed {
            id: got,
            error,
            remote_may_be_truncated,
            recovered: None,
        } => {
            assert_eq!(got, id);
            assert_eq!(error.kind, ErrorKind::Permission);
            assert!(remote_may_be_truncated);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"");

    // The next save is checked against the truncated file, which is unchanged: it goes up.
    h.save(id, b"mine, retried\n");
    tokio::time::advance(DEBOUNCE).await;
    assert!(matches!(h.next_event().await, EditEvent::Uploaded { .. }));
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"mine, retried\n");

    // And the check still works after a quota cut.
    *h.fs.cut_next_write.lock().unwrap() = Some(ErrorKind::Permission);
    h.save(id, b"cut again\n");
    tokio::time::advance(DEBOUNCE).await;
    assert!(matches!(h.next_event().await, EditEvent::Failed { .. }));
    h.mem
        .add_file(FILE, b"theirs\n", UNIX_EPOCH + Duration::from_secs(5_000));
    h.save(id, b"after theirs\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, EditEvent::RemoteChanged { id });
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"theirs\n");
}

/// A temporary upload file left on the server by a cut is removed after the next
/// successful upload; temporary names are random.
#[tokio::test(start_paused = true)]
async fn orphaned_temp_upload_is_cleaned_after_the_next_upload() {
    let mut h = harness(false);
    let id = h.start(FILE).await.unwrap();
    *h.fs.cut_next_write.lock().unwrap() = Some(ErrorKind::Network);
    h.save(id, b"mine\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, held(id));
    let names = |mem: MemFs| async move {
        mem.list(&rp("/home/alice"))
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect::<Vec<String>>()
    };
    let left = names(h.mem.clone()).await;
    let temp = left
        .iter()
        .find(|n| n.ends_with(".hoprs-upload"))
        .expect("the cut left the temporary file")
        .clone();
    assert!(temp.starts_with(".notes.txt."), "{temp}");
    assert!(
        temp.len() > ".notes.txt..hoprs-upload".len() + 8,
        "random part: {temp}"
    );
    // The original was never touched by a temporary-file upload.
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"version 1\n");

    h.state
        .send_replace(ConnectionState::Reconnecting { attempt: 1 });
    settle().await;
    h.state.send_replace(ConnectionState::Connected);
    assert!(matches!(h.next_event().await, EditEvent::Uploaded { .. }));
    assert_eq!(names(h.mem.clone()).await, vec!["notes.txt".to_owned()]);
}

/// Two `start`s of the same file at once make one edit (one download).
#[tokio::test]
async fn concurrent_starts_make_one_edit() {
    let h = harness(false);
    let (a, b) = tokio::join!(h.start(FILE), h.start(FILE));
    assert_eq!(a.unwrap(), b.unwrap());
    assert_eq!(h.fs.reads.load(Ordering::SeqCst), 1, "downloaded once");
    assert_eq!(
        h.launched.lock().unwrap().len(),
        2,
        "the editor opened for each"
    );
}

/// Carried minor: the "may be truncated" warning lasts until the server's copy is whole
/// again; a retry that fails on the way does not clear it.
#[tokio::test(start_paused = true)]
async fn truncation_warning_lasts_until_an_upload_succeeds() {
    let mut h = harness(false);
    let id = h.start_with(FILE, UploadMode::InPlace).await.unwrap();
    *h.fs.cut_next_write.lock().unwrap() = Some(ErrorKind::Network);
    h.save(id, b"mine\n");
    tokio::time::advance(DEBOUNCE).await;
    let truncated = EditEvent::Pending {
        id,
        remote_may_be_truncated: true,
    };
    assert_eq!(h.next_event().await, truncated);

    // The retry after the reconnect fails again (its stat): still possibly truncated.
    h.fs.stat_failures.store(1, Ordering::SeqCst);
    h.state
        .send_replace(ConnectionState::Reconnecting { attempt: 1 });
    settle().await;
    h.state.send_replace(ConnectionState::Connected);
    assert_eq!(h.next_event().await, truncated);
    // A failure for another reason says so too.
    *h.fs.cut_next_write.lock().unwrap() = Some(ErrorKind::Permission);
    tokio::time::advance(RETRY_MIN).await;
    match h.next_event().await {
        EditEvent::Failed {
            remote_may_be_truncated,
            ..
        } => assert!(remote_may_be_truncated),
        other => panic!("{other:?}"),
    }
    // A whole upload ends the warning.
    h.save(id, b"mine, whole\n");
    tokio::time::advance(DEBOUNCE).await;
    assert!(matches!(h.next_event().await, EditEvent::Uploaded { .. }));
    h.state
        .send_replace(ConnectionState::Reconnecting { attempt: 1 });
    h.save(id, b"later\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, held(id));
}

/// Carried minor: after a cut truncated the server's copy, reverting the local file to
/// the bytes last synced must still upload them (the server no longer holds them).
#[tokio::test(start_paused = true)]
async fn reverting_to_the_synced_bytes_still_repairs_a_truncated_remote() {
    let mut h = harness(false);
    let id = h.start_with(FILE, UploadMode::InPlace).await.unwrap();
    *h.fs.cut_next_write.lock().unwrap() = Some(ErrorKind::Network);
    h.save(id, b"mine\n");
    tokio::time::advance(DEBOUNCE).await;
    assert!(matches!(h.next_event().await, EditEvent::Pending { .. }));
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"");

    h.state
        .send_replace(ConnectionState::Reconnecting { attempt: 1 });
    // The user undoes the change: the file holds what was downloaded.
    h.save(id, b"version 1\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(
        h.next_event().await,
        EditEvent::Pending {
            id,
            remote_may_be_truncated: true
        }
    );
    assert_eq!(h.edits.pending(SESSION).await.len(), 1, "still unsent");
    h.state.send_replace(ConnectionState::Connected);
    assert!(matches!(h.next_event().await, EditEvent::Uploaded { .. }));
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"version 1\n");
    assert!(h.edits.pending(SESSION).await.is_empty());
}

/// Carried minor: "Keep remote" or "Save a copy" that fails after the recovered copy was
/// written says where that copy is.
#[tokio::test(start_paused = true)]
async fn failed_resolution_reports_the_recovered_copy() {
    for choice in [EditChoice::KeepRemote, EditChoice::SaveCopy] {
        let mut h = harness(false);
        let id = h.start(FILE).await.unwrap();
        h.mem
            .add_file(FILE, b"theirs\n", UNIX_EPOCH + Duration::from_secs(2_000));
        h.save(id, b"mine\n");
        tokio::time::advance(DEBOUNCE).await;
        assert_eq!(h.next_event().await, EditEvent::RemoteChanged { id });

        h.fs.offline.store(true, Ordering::SeqCst);
        let error = h.edits.resolve(id, choice).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::Network, "{choice:?}");
        let copies = h.recovered_copies();
        assert_eq!(copies.len(), 1, "{choice:?}: {copies:?}");
        assert_eq!(std::fs::read(&copies[0]).unwrap(), b"mine\n");
        match h.next_event().await {
            EditEvent::Failed {
                id: got, recovered, ..
            } => {
                assert_eq!(got, id);
                assert_eq!(recovered, Some(copies[0].clone()), "{choice:?}");
            }
            other => panic!("{choice:?}: {other:?}"),
        }
        // The edit still waits for a choice.
        let pending = h.edits.pending(SESSION).await;
        assert_eq!(pending[0].state, EditState::Conflict, "{choice:?}");
    }
}

/// Carried minor: a recovered copy whose write fails is not left half written.
#[test]
fn recovered_copy_is_removed_when_its_write_fails() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("site").join("notes.txt");
    let failing = |_: &mut std::fs::File, _: &[u8]| -> std::io::Result<()> {
        Err(std::io::Error::other("disk full"))
    };
    let error = write_new_private_with(&base, ".local-1", b"data", failing).unwrap_err();
    assert!(error.detail.is_some());
    let left: Vec<_> = std::fs::read_dir(base.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "nothing half written: {left:?}");
    // The plain write still works.
    let path = write_new_private(&base, ".local-1", b"data").unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"data");
}

/// Carried minor: once the session is closed (its state sender is gone), the worker stops
/// retrying; saves are held without touching the network until the close flow decides.
#[tokio::test(start_paused = true)]
async fn closing_the_session_stops_the_retries() {
    let mut h = harness(false);
    let id = h.start(FILE).await.unwrap();
    h.fs.stat_failures.store(1_000, Ordering::SeqCst);
    h.save(id, b"held\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, held(id));
    // A backoff retry is scheduled (the state still says connected)...
    tokio::time::advance(RETRY_MIN).await;
    assert_eq!(h.next_event().await, held(id));

    // ...then the session closes.
    drop(std::mem::replace(
        &mut h.state,
        watch::channel(ConnectionState::Connected).0,
    ));
    settle().await;
    let left = h.fs.stat_failures.load(Ordering::SeqCst);
    tokio::time::advance(RETRY_MAX * 4).await;
    settle().await;
    assert_eq!(
        h.fs.stat_failures.load(Ordering::SeqCst),
        left,
        "no retry after the close"
    );
    // A later save is held at once, without a stat.
    h.save(id, b"held again\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, held(id));
    assert_eq!(h.fs.stat_failures.load(Ordering::SeqCst), left);
    // A flush cannot send it either: it is reported, not lost.
    let unsent = h.edits.flush(SESSION, Duration::from_secs(10)).await;
    assert_eq!(unsent.len(), 1);
    assert_eq!(h.fs.stat_failures.load(Ordering::SeqCst), left);
    assert_eq!(h.writes(), 0);
}

/// Carried minor: the backoff of an edit goes once it is no longer held on a connected
/// session, and the start locks go once the start is done.
#[tokio::test]
async fn bookkeeping_maps_shrink() {
    let mut due = HashMap::new();
    let mut backoff = HashMap::new();
    let id = EditId(3);
    schedule_retry(id, true, &Outcome::Pending, &mut due, &mut backoff);
    assert_eq!(backoff.get(&id), Some(&RETRY_MIN));
    schedule_retry(id, true, &Outcome::Pending, &mut due, &mut backoff);
    assert_eq!(backoff.get(&id), Some(&(RETRY_MIN * 2)));
    // Held while the session is down: it waits for `Connected`, no backoff kept.
    schedule_retry(id, false, &Outcome::Pending, &mut due, &mut backoff);
    assert!(backoff.is_empty());
    schedule_retry(id, true, &Outcome::Pending, &mut due, &mut backoff);
    schedule_retry(id, true, &Outcome::Uploaded, &mut due, &mut backoff);
    assert!(backoff.is_empty());

    let h = harness(false);
    let (a, b) = tokio::join!(h.start(FILE), h.start(FILE));
    assert_eq!(a.unwrap(), b.unwrap());
    h.start("/home/alice/missing.txt").await.unwrap_err();
    assert_eq!(h.edits.starting_len(), 0);
}

/// Task 8: "Upload pending changes" sends what it can within the limit (here once the
/// session is back) and reports what is still unsent.
#[tokio::test(start_paused = true)]
async fn flush_sends_unsent_edits_within_the_limit() {
    let mut h = harness(false);
    let id = h.start(FILE).await.unwrap();
    h.state
        .send_replace(ConnectionState::Reconnecting { attempt: 1 });
    h.save(id, b"unsent\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, held(id));

    // Never back within the limit: reported after 10 s.
    let started = Instant::now();
    let unsent = h.edits.flush(SESSION, Duration::from_secs(10)).await;
    assert_eq!(unsent.len(), 1);
    assert_eq!(unsent[0].state, EditState::Pending);
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_secs(10) && waited < Duration::from_secs(11),
        "{waited:?}"
    );

    // Back during the flush: sent, nothing left.
    let edits = h.edits.clone();
    let flush = tokio::spawn(async move { edits.flush(SESSION, Duration::from_secs(10)).await });
    tokio::time::advance(Duration::from_secs(2)).await;
    h.state.send_replace(ConnectionState::Connected);
    let unsent = flush.await.unwrap();
    assert!(unsent.is_empty(), "{unsent:?}");
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"unsent\n");

    // A save not seen by the worker yet is sent too.
    let local = h.edits.local_path(id).unwrap();
    std::fs::write(&local, b"just saved\n").unwrap();
    assert!(
        h.edits
            .flush(SESSION, Duration::from_secs(10))
            .await
            .is_empty()
    );
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"just saved\n");

    // A conflict is never overwritten by a flush: reported at once.
    h.mem
        .add_file(FILE, b"theirs\n", UNIX_EPOCH + Duration::from_secs(9_000));
    std::fs::write(&local, b"mine\n").unwrap();
    let started = Instant::now();
    let unsent = h.edits.flush(SESSION, Duration::from_secs(10)).await;
    assert_eq!(unsent.len(), 1);
    assert_eq!(unsent[0].state, EditState::Conflict);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"theirs\n");
}

/// Task 8: "Close and keep local copies" saves each unsent edit as a recovered copy,
/// stops the edits and deletes nothing.
#[tokio::test(start_paused = true)]
async fn keep_saves_recovered_copies_and_deletes_nothing() {
    let mut h = harness(false);
    let unsent = h.start(FILE).await.unwrap();
    h.mem.add_file("/home/alice/synced.txt", b"s\n", UNIX_EPOCH);
    let synced = h.start("/home/alice/synced.txt").await.unwrap();
    let unsent_local = h.edits.local_path(unsent).unwrap();
    let synced_local = h.edits.local_path(synced).unwrap();
    h.state
        .send_replace(ConnectionState::Reconnecting { attempt: 1 });
    h.save(unsent, b"mine\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, held(unsent));

    let kept = h.edits.keep(SESSION).await.unwrap();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0].id, unsent);
    assert_eq!(kept[0].remote, rp(FILE));
    assert_eq!(std::fs::read(&kept[0].recovered).unwrap(), b"mine\n");
    assert!(kept[0].recovered.starts_with(&h.recovered));
    // Nothing deleted: the local copies are where they were.
    assert_eq!(std::fs::read(&unsent_local).unwrap(), b"mine\n");
    assert!(synced_local.exists());
    let mut closed = vec![h.next_event().await, h.next_event().await];
    closed.sort_by_key(|e| format!("{e:?}"));
    assert_eq!(
        closed,
        vec![
            EditEvent::Closed { id: unsent },
            EditEvent::Closed { id: synced }
        ]
    );
    assert!(h.edits.pending(SESSION).await.is_empty());
    // The edits are over: a reconnect uploads nothing.
    h.state.send_replace(ConnectionState::Connected);
    settle().await;
    assert_eq!(h.writes(), 0);
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"version 1\n");
}

/// Task 8: "Retry" on a failed upload tries again now.
#[tokio::test(start_paused = true)]
async fn retry_uploads_a_failed_edit() {
    let mut h = harness(false);
    let id = h.start_with(FILE, UploadMode::InPlace).await.unwrap();
    *h.fs.cut_next_write.lock().unwrap() = Some(ErrorKind::Permission);
    h.save(id, b"mine\n");
    tokio::time::advance(DEBOUNCE).await;
    assert!(matches!(h.next_event().await, EditEvent::Failed { .. }));
    h.edits.retry(id).await.unwrap();
    assert!(matches!(h.next_event().await, EditEvent::Uploaded { .. }));
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"mine\n");
    assert!(h.edits.retry(EditId(999)).await.is_err());
}

/// Fix round 1, Important 1: a flush whose deadline passes while an in-place upload is
/// under way lets it finish: the server's copy is never left truncated unflagged.
#[tokio::test(start_paused = true)]
async fn slow_upload_crossing_the_flush_deadline_is_never_left_half_done() {
    let h = harness(false);
    let id = h.start_with(FILE, UploadMode::InPlace).await.unwrap();
    let local = h.edits.local_path(id).unwrap();
    std::fs::write(&local, b"slow but whole\n").unwrap();
    let gate = Arc::new(Semaphore::new(0));
    *h.fs.write_gate.lock().unwrap() = Some(gate.clone());

    let edits = h.edits.clone();
    let flush = tokio::spawn(async move { edits.flush(SESSION, Duration::from_secs(1)).await });
    // The write has started (the file is truncated) and outlives the deadline.
    for _ in 0..200 {
        if h.mem.read_file(FILE).unwrap().is_empty() {
            break;
        }
        settle().await;
    }
    assert_eq!(
        h.mem.read_file(FILE).unwrap(),
        b"",
        "the upload is under way"
    );
    tokio::time::advance(Duration::from_secs(5)).await;
    settle().await;
    gate.add_permits(1);
    let unsent = flush.await.unwrap();
    assert!(unsent.is_empty(), "{unsent:?}");
    assert_eq!(h.mem.read_file(FILE).unwrap(), b"slow but whole\n");
}

/// Fix round 1, Important 2: the local-only pass before a shutdown saves each unsent
/// edit as a recovered copy at once (no network, no waiting for uploads), and a later
/// `keep` / `end_all` of the same bytes does not write a second copy.
#[tokio::test(start_paused = true)]
async fn preserve_saves_unsent_edits_locally_once() {
    let mut h = harness(false);
    let id = h.start(FILE).await.unwrap();
    h.state
        .send_replace(ConnectionState::Reconnecting { attempt: 1 });
    h.save(id, b"unsent\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, held(id));
    // An upload in flight holds the turn: the pass does not wait for it.
    let turn = h.edits.inner.edit(id).unwrap().turn.clone();
    let _held = turn.lock().await;

    let kept = h.edits.preserve_unsent().await;
    assert_eq!(kept.len(), 1);
    assert_eq!(std::fs::read(&kept[0].recovered).unwrap(), b"unsent\n");
    assert_eq!(h.recovered_copies(), vec![kept[0].recovered.clone()]);
    drop(_held);

    // The window closes for good: nothing deleted, still one copy.
    let ended = h.edits.end_all().await;
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].recovered, kept[0].recovered);
    assert_eq!(h.recovered_copies().len(), 1);
    assert!(h.edits.local_path(id).is_none());
    let local = h.root.join(SESSION.0.to_string());
    assert!(local.exists(), "the edit directory is kept");
}

/// Fix round 2, Important 1: "Stop and keep local copies" while a flush runs. Two
/// unsent edits; the flush has sent the first and its sync of the second waits on the
/// server. `keep` returns at once (it does not wait for that upload), saying the second's
/// server copy may be incomplete; the waiting sync then writes nothing, the flush stops,
/// and a sync that still holds the old handle afterwards writes nothing either.
#[tokio::test]
async fn stop_and_keep_during_a_flush_never_uploads_an_ended_edit() {
    let h = harness(false);
    h.mem.add_file(
        "/home/alice/b.txt",
        b"b v1\n",
        UNIX_EPOCH + Duration::from_secs(1_000),
    );
    let a = h.start_with(FILE, UploadMode::InPlace).await.unwrap();
    let b = h
        .start_with("/home/alice/b.txt", UploadMode::InPlace)
        .await
        .unwrap();
    std::fs::write(h.edits.local_path(a).unwrap(), b"a unsent\n").unwrap();
    std::fs::write(h.edits.local_path(b).unwrap(), b"b unsent\n").unwrap();
    let old_b = h.edits.inner.edit(b).unwrap();
    // The first edit's sync takes two stats (before and after its upload); the third,
    // the second edit's, waits.
    let gate = Arc::new(Semaphore::new(2));
    *h.fs.stat_gate.lock().unwrap() = Some(gate.clone());
    let base = h.fs.stats.load(Ordering::SeqCst);

    let edits = h.edits.clone();
    let flush = tokio::spawn(async move { edits.flush(SESSION, Duration::from_secs(60)).await });
    tokio::time::timeout(Duration::from_secs(10), async {
        while h.fs.stats.load(Ordering::SeqCst) < base + 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the second sync reaches the server");
    assert_eq!(
        h.mem.read_file(FILE).unwrap(),
        b"a unsent\n",
        "the first went up"
    );
    assert_eq!(h.writes(), 1);

    // Stop: keep returns while the second sync still waits.
    let kept = tokio::time::timeout(Duration::from_secs(5), h.edits.keep(SESSION))
        .await
        .expect("keep does not wait for the upload under way")
        .unwrap();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0].id, b);
    assert!(
        kept[0].remote_may_be_truncated,
        "the busy upload is flagged"
    );
    assert_eq!(std::fs::read(&kept[0].recovered).unwrap(), b"b unsent\n");

    // The waiting sync goes on: it must not write the ended edit.
    *h.fs.stat_gate.lock().unwrap() = None;
    gate.add_permits(10);
    let left = flush.await.unwrap();
    assert!(left.is_empty(), "{left:?}");
    assert_eq!(h.writes(), 1, "the ended edit was never uploaded");
    assert_eq!(h.mem.read_file("/home/alice/b.txt").unwrap(), b"b v1\n");

    // The flush's old handle syncing after `keep` (the race of the review) writes nothing.
    assert!(matches!(
        h.edits.inner.sync(&old_b, false).await,
        Outcome::Unchanged
    ));
    assert_eq!(h.writes(), 1);
    assert_eq!(h.mem.read_file("/home/alice/b.txt").unwrap(), b"b v1\n");
}

/// Fix round 2 (minor): two passes racing (a forced exit during the shutdown's) copy the
/// same bytes once.
#[tokio::test(start_paused = true)]
async fn racing_preserve_passes_copy_once() {
    let mut h = harness(false);
    let id = h.start(FILE).await.unwrap();
    h.state
        .send_replace(ConnectionState::Reconnecting { attempt: 1 });
    h.save(id, b"unsent\n");
    tokio::time::advance(DEBOUNCE).await;
    assert_eq!(h.next_event().await, held(id));
    let (one, two) = tokio::join!(h.edits.preserve_unsent(), h.edits.preserve_unsent());
    assert_eq!(one[0].recovered, two[0].recovered);
    assert_eq!(h.recovered_copies().len(), 1);
}

/// `put` (an agent's `write_file`): `TempRename` writes a temporary file and renames it
/// over the path, keeping the old file's mode and leaving no temporary file; `InPlace`
/// writes the path itself.
#[tokio::test]
async fn put_writes_by_mode() {
    /// Records the paths `open_write` and `rename` were called with.
    struct Recording {
        inner: MemFs,
        calls: Mutex<Vec<String>>,
    }
    #[async_trait]
    impl RemoteFs for Recording {
        async fn list(&self, path: &RemotePath) -> Result<Vec<RemoteEntry>> {
            self.inner.list(path).await
        }
        async fn stat(&self, path: &RemotePath) -> Result<RemoteEntry> {
            self.inner.stat(path).await
        }
        async fn open_read(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncRead> {
            self.inner.open_read(path, offset).await
        }
        async fn open_write(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncWrite> {
            self.calls.lock().unwrap().push(format!("write {path}"));
            self.inner.open_write(path, offset).await
        }
        async fn mkdir(&self, path: &RemotePath) -> Result<()> {
            self.inner.mkdir(path).await
        }
        async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("rename {from} {to}"));
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

    let mem = MemFs::new();
    mem.add_file("/srv/app.conf", b"old", UNIX_EPOCH);
    mem.chmod(&rp("/srv/app.conf"), 0o640).await.unwrap();
    let fs = Recording {
        inner: mem.clone(),
        calls: Mutex::new(Vec::new()),
    };

    put(
        &fs,
        &rp("/srv/app.conf"),
        UploadMode::TempRename,
        b"new text",
    )
    .await
    .unwrap();
    assert_eq!(mem.read_file("/srv/app.conf").unwrap(), b"new text");
    let names: Vec<String> = mem
        .list(&rp("/srv"))
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    assert_eq!(names, ["app.conf"], "no temporary file left");
    assert_eq!(
        mem.stat(&rp("/srv/app.conf")).await.unwrap().mode,
        Some(0o640)
    );
    {
        let calls = fs.calls.lock().unwrap();
        assert_eq!(calls.len(), 2, "{calls:?}");
        assert!(calls[0].starts_with("write /srv/.app.conf."), "{calls:?}");
        assert!(calls[0].ends_with(".hoprs-upload"), "{calls:?}");
        assert!(calls[1].ends_with(" /srv/app.conf"), "{calls:?}");
    }

    // A new file by temporary file and rename too.
    put(&fs, &rp("/srv/new.txt"), UploadMode::TempRename, b"fresh")
        .await
        .unwrap();
    assert_eq!(mem.read_file("/srv/new.txt").unwrap(), b"fresh");

    fs.calls.lock().unwrap().clear();
    put(&fs, &rp("/srv/app.conf"), UploadMode::InPlace, b"in place")
        .await
        .unwrap();
    assert_eq!(mem.read_file("/srv/app.conf").unwrap(), b"in place");
    assert_eq!(*fs.calls.lock().unwrap(), ["write /srv/app.conf"]);
}
