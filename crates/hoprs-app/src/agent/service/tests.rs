use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use hoprs_core::{AgentAccess, Error, ErrorKind, Protocol, RemotePath, Site, SiteId};
use hoprs_remote::{MemFs, RemoteFs};
use hoprs_ssh::{ExecEnd, ExecOutput};
use hoprs_transfer::{Direction, Origin, TransferId, TransferItem, TransferSpec, TransferState};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::*;
use crate::agent::paths::engine_path;
use crate::agent::testing::{FakeHost, FsOp};
use crate::agent::tools::{self, Need};
use crate::agent::{
    HOST_KEY_CHANGED, NO_SUCH_TRANSFER, SITE_UNAVAILABLE, TURNED_OFF, WAITING_FOR_USER, protected,
    read_only, ssh_only,
};

const EPOCH: SystemTime = SystemTime::UNIX_EPOCH;

/// Four sites: `ssh` (All), `ftp` (All), `ro` (Read, SSH) and `closed` (None).
struct Fx {
    host: Arc<FakeHost>,
    service: Arc<Service>,
    events: mpsc::UnboundedReceiver<AgentEvent>,
    ssh: Site,
    ftp: Site,
    ro: Site,
    closed: Site,
    ssh_fs: MemFs,
    ftp_fs: MemFs,
}

fn site(name: &str, host: &str, protocol: Protocol, level: AgentAccess) -> Site {
    let mut site = Site::new(name, host, "u", protocol);
    site.agent_access = level;
    site
}

fn fixture() -> Fx {
    let host = FakeHost::new();
    let ssh = site("Acme", "ssh.example", Protocol::Ssh, AgentAccess::All);
    let ftp = site("Acme FTP", "ftp.example", Protocol::Ftp, AgentAccess::All);
    let ro = site(
        "Acme staging",
        "staging.example",
        Protocol::Ssh,
        AgentAccess::Read,
    );
    let closed = site(
        "Hidden Corp",
        "hidden.example",
        Protocol::Ssh,
        AgentAccess::None,
    );
    let ssh_fs = host.add_site(ssh.clone());
    let ftp_fs = host.add_site(ftp.clone());
    let ro_fs = host.add_site(ro.clone());
    let closed_fs = host.add_site(closed.clone());
    for fs in [&ssh_fs, &ro_fs, &closed_fs] {
        fs.add_file("/var/www/a.txt", b"hello", EPOCH);
    }
    let (tx, events) = mpsc::unbounded_channel();
    let service = Service::new(host.clone(), Limits::default(), tx);
    Fx {
        host,
        service,
        events,
        ssh,
        ftp,
        ro,
        closed,
        ssh_fs,
        ftp_fs,
    }
}

impl Fx {
    async fn call(&self, tool: &str, args: Value) -> Reply {
        self.service.call(tool, args).await
    }

    /// The four sites, `which` at `level`.
    fn set_level(&self, which: &Site, level: AgentAccess) {
        let sites = [&self.ssh, &self.ftp, &self.ro, &self.closed]
            .map(|s| {
                if s.id == which.id {
                    with_level(s, level)
                } else {
                    s.clone()
                }
            })
            .to_vec();
        self.host.set_sites(sites);
    }

    fn events(&mut self) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            out.push(event);
        }
        out
    }
}

fn with_level(site: &Site, level: AgentAccess) -> Site {
    let mut site = site.clone();
    site.agent_access = level;
    site
}

fn err(reply: Reply) -> String {
    match reply {
        Reply::Err(text) => text,
        Reply::Ok(blocks) => panic!("expected an error, got {blocks:?}"),
    }
}

fn ok(reply: Reply) -> Vec<String> {
    match reply {
        Reply::Ok(blocks) => blocks,
        Reply::Err(text) => panic!("expected a result, got the error: {text}"),
    }
}

/// The first block, parsed.
fn head(reply: Reply) -> Value {
    serde_json::from_str(&ok(reply)[0]).unwrap()
}

/// Arguments that pass the argument step of `tool` on `site`.
fn valid_args(tool: &str, site: &SiteId) -> Value {
    let site = site.0.clone();
    match tool {
        "list_directory" | "stat" | "make_directory" | "delete" => {
            json!({"site": site, "path": "/var/www"})
        }
        "read_file" => json!({"site": site, "path": "/var/www/a.txt"}),
        "download" => json!({"site": site, "remote_path": "/var/www/a.txt",
            "local_path": "/home/u/a.txt"}),
        "upload" => json!({"site": site, "local_path": "/home/u/a.txt",
            "remote_path": "/var/www/b.txt"}),
        "write_file" => json!({"site": site, "path": "/var/www/b.txt", "content": "x"}),
        "rename" => json!({"site": site, "from": "/var/www/a.txt", "to": "/var/www/c.txt"}),
        "set_permissions" => json!({"site": site, "path": "/var/www/a.txt", "mode": "644"}),
        "run_command" => json!({"site": site, "command": "true"}),
        "transfer_status" => json!({"transfer": 1}),
        "list_sites" => json!({}),
        other => panic!("no arguments for {other}"),
    }
}

fn sited_tools() -> impl Iterator<Item = &'static tools::ToolSpec> {
    tools::all().iter().filter(|t| t.need != Need::Nothing)
}

/// Log lines written while a [`capture_logs`] guard lives (this thread's tasks).
#[derive(Clone, Default)]
struct LogBuf(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl LogBuf {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }

    /// The agent lines written so far.
    fn agent_lines(&self) -> Vec<String> {
        self.text()
            .lines()
            .filter(|l| l.contains(" agent: "))
            .map(str::to_owned)
            .collect()
    }
}

struct LogCapture {
    _default: tracing::subscriber::DefaultGuard,
    _other: tracing::Dispatch,
}

/// Captures what is logged at `info` on this thread, without timestamps (as
/// `backend/sessions.rs`'s tests do; the second, silent dispatcher keeps another
/// thread's cached interest from hiding a callsite).
fn capture_logs() -> (LogBuf, LogCapture) {
    let buf = LogBuf::default();
    let writer = buf.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::INFO)
        .finish();
    let capture = LogCapture {
        _default: tracing::subscriber::set_default(subscriber),
        _other: tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()),
    };
    (buf, capture)
}

/// The access step of every tool that takes a site, at every level, on SSH and FTP.
/// (Paused time: a transfer that passes waits its minute.)
#[tokio::test(start_paused = true)]
async fn access_matrix() {
    let fx = fixture();
    for spec in sited_tools() {
        for base in [&fx.ssh, &fx.ftp] {
            for level in [AgentAccess::None, AgentAccess::Read, AgentAccess::All] {
                let site = with_level(base, level);
                fx.host.set_sites(vec![site.clone()]);
                let reply = fx.call(spec.name, valid_args(spec.name, &site.id)).await;
                let case = format!("{} on {:?} at {level:?}", spec.name, site.protocol);
                let want = match (level, spec.need) {
                    (AgentAccess::None, _) => Some(SITE_UNAVAILABLE.to_owned()),
                    (AgentAccess::Read, Need::All | Need::AllSsh) => Some(read_only(&site.name)),
                    (AgentAccess::All, Need::AllSsh) if site.protocol != Protocol::Ssh => {
                        Some(ssh_only(&site.name))
                    }
                    _ => None,
                };
                match (want, reply) {
                    (Some(want), reply) => assert_eq!(err(reply), want, "{case}"),
                    (None, Reply::Ok(_)) => {}
                    (None, Reply::Err(text)) => {
                        for refused in [
                            SITE_UNAVAILABLE.to_owned(),
                            read_only(&site.name),
                            ssh_only(&site.name),
                            TURNED_OFF.to_owned(),
                        ] {
                            assert_ne!(text, refused, "{case} passes the access step");
                        }
                    }
                }
            }
        }
    }
}

/// A site at "No access" and an id no site has: the same reply, the same log line,
/// no connection, no filesystem call, no event, and `list_sites` does not name it.
#[tokio::test]
async fn no_access_and_no_such_site_read_the_same() {
    let (logs, _guard) = capture_logs();
    let mut fx = fixture();
    let nobody = SiteId::new();
    for spec in sited_tools() {
        let before = logs.agent_lines().len();
        let closed = err(fx
            .call(spec.name, valid_args(spec.name, &fx.closed.id))
            .await);
        let missing = err(fx.call(spec.name, valid_args(spec.name, &nobody)).await);
        assert_eq!(closed, missing, "{}", spec.name);
        assert_eq!(closed, SITE_UNAVAILABLE, "{}", spec.name);
        assert!(!closed.contains(&fx.closed.name), "{closed}");
        assert!(!closed.contains(&fx.closed.host), "{closed}");
        let lines = logs.agent_lines()[before..].to_vec();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[0], lines[1], "{}", spec.name);
        for word in [&fx.closed.name, &fx.closed.host, &fx.closed.id.0, &nobody.0] {
            assert!(!lines[0].contains(word.as_str()), "{}", lines[0]);
        }
    }
    assert_eq!(fx.host.connects(&fx.closed.id), 0);
    assert_eq!(fx.host.connects(&nobody), 0);
    for op in [
        FsOp::List,
        FsOp::Stat,
        FsOp::Read,
        FsOp::Write,
        FsOp::Mkdir,
        FsOp::Rename,
        FsOp::Remove,
        FsOp::Chmod,
    ] {
        assert_eq!(fx.host.calls(&fx.closed.id, op), 0, "{op:?}");
    }
    assert!(fx.host.execs().is_empty());
    assert!(fx.host.agent_sites().is_empty());
    assert!(fx.events().is_empty());
    let listed = ok(fx.call("list_sites", json!({})).await).join("\n");
    assert!(!listed.contains(&fx.closed.id.0), "{listed}");
    assert!(!listed.contains(&fx.closed.name), "{listed}");
}

#[tokio::test]
async fn level_change_applies_on_the_next_call() {
    let fx = fixture();
    let read = json!({"site": fx.ro.id.0, "path": "/var/www/a.txt"});
    assert_eq!(ok(fx.call("read_file", read.clone()).await)[1], "hello");
    assert_eq!(
        err(fx
            .call("make_directory", valid_args("make_directory", &fx.ro.id))
            .await),
        read_only(&fx.ro.name)
    );

    fx.set_level(&fx.ro, AgentAccess::None);
    assert_eq!(
        err(fx.call("read_file", read.clone()).await),
        SITE_UNAVAILABLE
    );

    fx.set_level(&fx.ro, AgentAccess::All);
    let reply = fx
        .call("make_directory", valid_args("make_directory", &fx.ro.id))
        .await;
    if let Reply::Err(text) = reply {
        assert_ne!(text, read_only(&fx.ro.name));
        assert_ne!(text, SITE_UNAVAILABLE);
    }

    // A call already past its check finishes, whatever the level becomes.
    let hold = fx.host.hold(&fx.ro.id, FsOp::List);
    let running = tokio::spawn({
        let service = fx.service.clone();
        let args = json!({"site": fx.ro.id.0, "path": "/var/www"});
        async move { service.call("list_directory", args).await }
    });
    hold.entered().await;
    fx.set_level(&fx.ro, AgentAccess::None);
    hold.release();
    let listing = head(running.await.unwrap());
    assert_eq!(listing["entries"][0]["name"], "a.txt");
    assert_eq!(err(fx.call("read_file", read).await), SITE_UNAVAILABLE);
}

#[tokio::test]
async fn deleted_site_reads_as_unavailable() {
    let fx = fixture();
    let read = json!({"site": fx.ro.id.0, "path": "/var/www/a.txt"});
    ok(fx.call("read_file", read.clone()).await);
    fx.host
        .set_sites(vec![fx.ssh.clone(), fx.ftp.clone(), fx.closed.clone()]);
    assert_eq!(err(fx.call("read_file", read).await), SITE_UNAVAILABLE);
    let listed = ok(fx.call("list_sites", json!({})).await).join("\n");
    assert!(!listed.contains(&fx.ro.id.0), "{listed}");
}

#[tokio::test]
async fn switch_off_refuses_everything() {
    let fx = fixture();
    fx.host.set_enabled(false);
    for spec in tools::all() {
        let reply = fx.call(spec.name, valid_args(spec.name, &fx.ssh.id)).await;
        assert_eq!(err(reply), TURNED_OFF, "{}", spec.name);
    }
    // Even with arguments that would be refused.
    assert_eq!(
        err(fx.call("list_directory", json!({"site": 7})).await),
        TURNED_OFF
    );
    assert_eq!(fx.host.connects(&fx.ssh.id), 0);
    fx.host.set_enabled(true);
    ok(fx.call("list_sites", json!({})).await);
}

#[tokio::test]
async fn list_sites_has_no_key_file_and_no_secret_flags() {
    let fx = fixture();
    let mut ssh = fx.ssh.clone();
    ssh.ssh.key_file = Some(PathBuf::from("/home/me/.ssh/deploy"));
    ssh.has_password = true;
    ssh.has_key_passphrase = true;
    ssh.local_dir = Some(PathBuf::from("/home/me/work"));
    ssh.remote_dir = Some("/srv/app".to_owned());
    let mut ftp = fx.ftp.clone();
    ftp.ftp.pinned_fingerprint = Some("SHA256:pinnedprint".to_owned());
    ftp.has_password = true;
    let ftps = site(
        "Acme FTPS",
        "ftps.example",
        Protocol::FtpsImplicit,
        AgentAccess::Read,
    );
    fx.host.set_sites(vec![
        ssh.clone(),
        ftp.clone(),
        fx.ro.clone(),
        fx.closed.clone(),
        ftps.clone(),
    ]);
    ok(fx
        .call("list_directory", json!({"site": ftp.id.0, "path": "/"}))
        .await);

    let text = ok(fx.call("list_sites", json!({})).await).join("\n");
    for word in [
        "deploy",
        "key_file",
        "has_password",
        "has_key_passphrase",
        ".ssh",
        "pinned",
        "SHA256",
        "local_dir",
        "/home/me",
    ] {
        assert!(!text.contains(word), "{word} in {text}");
    }
    let v: Value = serde_json::from_str(&text).unwrap();
    let sites = v["sites"].as_array().unwrap();
    let ids: Vec<&str> = sites.iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(ids, [&ssh.id.0, &ftp.id.0, &fx.ro.id.0, &ftps.id.0]);
    for s in sites {
        let mut keys: Vec<&str> = s.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "connected",
                "host",
                "id",
                "level",
                "name",
                "port",
                "protocol",
                "start_folder",
                "user"
            ]
        );
    }
    assert_eq!(
        sites[0],
        json!({"id": ssh.id.0, "name": "Acme", "protocol": "ssh", "host": "ssh.example",
            "port": 22, "user": "u", "level": "all", "connected": false,
            "start_folder": "/srv/app"})
    );
    assert_eq!(sites[1]["protocol"], "ftp");
    assert_eq!(sites[1]["connected"], true);
    assert_eq!(sites[1]["start_folder"], Value::Null);
    assert_eq!(sites[2]["level"], "read");
    assert_eq!(sites[3]["protocol"], "ftps-implicit");
    assert_eq!(sites[3]["port"], 990);
}

#[tokio::test]
async fn connects_once_and_reuses() {
    let mut fx = fixture();
    let args = json!({"site": fx.ssh.id.0, "path": "/var/www"});
    for _ in 0..3 {
        ok(fx.call("list_directory", args.clone()).await);
    }
    assert_eq!(fx.host.connects(&fx.ssh.id), 1);
    let session = fx.host.connected(&fx.ssh.id).unwrap();
    assert_eq!(
        fx.events(),
        [
            AgentEvent::Connected(session),
            AgentEvent::Sites(vec![fx.ssh.id.clone()])
        ]
    );
}

#[tokio::test]
async fn connect_errors_are_fixed_sentences() {
    let fx = fixture();
    let read = |site: &Site| json!({"site": site.id.0, "path": "/var/www/a.txt"});

    fx.host.fail_connect(
        &fx.ssh.id,
        Error::new(ErrorKind::Auth, "Key /home/me/.ssh/id_ed25519 was refused")
            .with_detail("tried /home/me/.ssh/id_ed25519"),
    );
    let text = err(fx.call("read_file", read(&fx.ssh)).await);
    for word in ["/home", "id_ed25519", ".ssh", "refused"] {
        assert!(!text.contains(word), "{word} in {text}");
    }
    assert!(text.contains("credentials"), "{text}");
    assert!(text.contains("\"Acme\""), "{text}");

    fx.host.fail_connect(
        &fx.ssh.id,
        hoprs_ssh::testing::changed_host_key_error("ssh.example", 4),
    );
    assert_eq!(
        err(fx.call("read_file", read(&fx.ssh)).await),
        HOST_KEY_CHANGED
    );

    for error in [
        Error::new(ErrorKind::HostKey, "Unknown host key for ftp.example"),
        Error::new(ErrorKind::Tls, "Certificate of ftp.example was rejected"),
    ] {
        fx.host.fail_connect(&fx.ftp.id, error);
        let text = err(fx
            .call("list_directory", json!({"site": fx.ftp.id.0, "path": "/"}))
            .await);
        assert!(text.contains("hoprs window"), "{text}");
        assert!(
            !text.contains("rejected") && !text.contains("Unknown"),
            "{text}"
        );
    }

    fx.host.fail_connect(
        &fx.ro.id,
        Error::new(ErrorKind::Network, "Connection refused (os error 111)")
            .with_detail("connect 10.1.2.3:22"),
    );
    let text = err(fx.call("read_file", read(&fx.ro)).await);
    assert!(text.contains("staging.example:22"), "{text}");
    assert!(
        !text.contains("os error") && !text.contains("10.1.2.3"),
        "{text}"
    );

    fx.host.fail_connect(
        &fx.ro.id,
        Error::new(ErrorKind::Protocol, "530 no shell for /home/me"),
    );
    let text = err(fx.call("read_file", read(&fx.ro)).await);
    assert!(text.contains("Could not connect"), "{text}");
    assert!(!text.contains("/home") && !text.contains("530"), "{text}");
}

#[tokio::test]
async fn failed_agent_connect_closes_its_session() {
    let mut fx = fixture();
    fx.host.fail_connect(
        &fx.ssh.id,
        Error::new(ErrorKind::Network, "Connection refused"),
    );
    err(fx
        .call("list_directory", json!({"site": fx.ssh.id.0, "path": "/"}))
        .await);
    assert_eq!(fx.host.closed(), std::slice::from_ref(&fx.ssh.id));
    assert!(fx.host.agent_sites().is_empty());
    assert_eq!(fx.host.connected(&fx.ssh.id), None);
    assert!(
        !fx.events()
            .iter()
            .any(|e| matches!(e, AgentEvent::Connected(_)))
    );

    // A tab's session that is down is the user's: answered, never closed.
    fx.host
        .disconnect_user(&fx.ftp.id, Error::new(ErrorKind::Network, "gone"))
        .await;
    err(fx
        .call("list_directory", json!({"site": fx.ftp.id.0, "path": "/"}))
        .await);
    assert_eq!(fx.host.closed(), std::slice::from_ref(&fx.ssh.id));
}

#[tokio::test(start_paused = true)]
async fn prompt_wait_ends_with_the_right_text() {
    let fx = fixture();
    fx.host.hang_connect(&fx.ssh.id);
    fx.host.set_prompt_open(&fx.ssh.id, true);
    let mut call = tokio::spawn({
        let service = fx.service.clone();
        let args = json!({"site": fx.ssh.id.0, "path": "/"});
        async move { service.call("list_directory", args).await }
    });
    assert!(
        tokio::time::timeout(Duration::from_secs(119), &mut call)
            .await
            .is_err(),
        "still waiting before 120 s"
    );
    assert!(fx.host.closed().is_empty());
    assert_eq!(err(call.await.unwrap()), WAITING_FOR_USER);
    assert_eq!(fx.host.closed(), std::slice::from_ref(&fx.ssh.id));

    // No dialog open: the connection took too long.
    fx.host.hang_connect(&fx.ftp.id);
    let started = Instant::now();
    let text = err(fx
        .call("list_directory", json!({"site": fx.ftp.id.0, "path": "/"}))
        .await);
    assert_eq!(started.elapsed(), Duration::from_secs(120));
    assert!(text.contains("took more than 2 minutes"), "{text}");
    assert!(text.contains("ftp.example:21"), "{text}");
    assert_eq!(fx.host.closed(), [fx.ssh.id.clone(), fx.ftp.id.clone()]);
}

async fn list(fx: &Fx, args: Value) -> Value {
    head(fx.call("list_directory", args).await)
}

fn names(listing: &Value) -> Vec<String> {
    listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn huge_listing_is_capped_and_paged() {
    let fx = fixture();
    for i in (0..100_000).rev() {
        fx.ssh_fs.add_file(&format!("/big/f{i:06}"), b"", EPOCH);
    }
    let site = &fx.ssh.id.0;

    let first = list(&fx, json!({"site": site, "path": "/big"})).await;
    let first_names = names(&first);
    assert_eq!(first_names.len(), 1000);
    assert_eq!(first["total"], 100_000);
    assert_eq!(first["offset"], 0);
    assert_eq!(first["truncated"], true);
    assert_eq!(first["path"], "/big");
    assert_eq!(first_names[0], "f000000");
    assert_eq!(first_names[999], "f000999");

    let second = list(&fx, json!({"site": site, "path": "/big", "offset": 1000})).await;
    assert_eq!(names(&second)[0], "f001000");
    assert!(first_names.last().unwrap() < &names(&second)[0]);

    let last = list(
        &fx,
        json!({"site": site, "path": "/big", "offset": 99_500, "limit": 5000}),
    )
    .await;
    assert_eq!(names(&last).len(), 500);
    assert_eq!(last["truncated"], false);
    assert_eq!(names(&last)[499], "f099999");

    let capped = list(&fx, json!({"site": site, "path": "/big", "limit": 9000})).await;
    assert_eq!(names(&capped).len(), 5000);
    assert_eq!(capped["truncated"], true);

    let past = list(
        &fx,
        json!({"site": site, "path": "/big", "offset": 200_000}),
    )
    .await;
    assert!(names(&past).is_empty());
    assert_eq!(past["total"], 100_000);
    assert_eq!(past["truncated"], false);
}

#[tokio::test]
async fn listing_and_stat_shapes() {
    let fx = fixture();
    let modified = EPOCH + Duration::from_secs(1_700_000_000);
    fx.ssh_fs.add_file("/srv/b.txt", b"12345", modified);
    fx.ssh_fs.add_dir("/srv/a-dir");
    fx.ssh_fs.add_symlink("/srv/c-link", "/srv/b.txt");
    let site = &fx.ssh.id.0;

    let listing = list(&fx, json!({"site": site, "path": "/srv/"})).await;
    assert_eq!(listing["path"], "/srv");
    assert_eq!(names(&listing), ["a-dir", "b.txt", "c-link"]);
    let kinds: Vec<&str> = listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["dir", "file", "symlink"]);
    assert_eq!(
        listing["entries"][1],
        json!({"name": "b.txt", "kind": "file", "size": 5,
            "modified": "2023-11-14T22:13:20Z", "permissions": "0644"})
    );

    let stat = head(
        fx.call("stat", json!({"site": site, "path": "/srv/b.txt"}))
            .await,
    );
    assert_eq!(
        stat,
        json!({"path": "/srv/b.txt", "name": "b.txt", "kind": "file", "size": 5,
            "modified": "2023-11-14T22:13:20Z", "permissions": "0644"})
    );
    let missing = err(fx
        .call("stat", json!({"site": site, "path": "/srv/nope"}))
        .await);
    assert!(missing.contains("/srv/nope"), "{missing}");
    let not_a_dir = err(fx
        .call(
            "list_directory",
            json!({"site": site, "path": "/srv/b.txt"}),
        )
        .await);
    assert!(not_a_dir.contains("/srv/b.txt"), "{not_a_dir}");
}

async fn read(fx: &Fx, args: Value) -> (Value, String) {
    let blocks = ok(fx.call("read_file", args).await);
    assert_eq!(blocks.len(), 2, "a header and the text");
    (serde_json::from_str(&blocks[0]).unwrap(), blocks[1].clone())
}

#[tokio::test]
async fn read_file_ranges() {
    let fx = fixture();
    fx.ssh_fs.add_file("/t.txt", b"0123456789", EPOCH);
    // "abé!": the é is two bytes, 2 and 3.
    fx.ssh_fs.add_file("/e.txt", "abé!".as_bytes(), EPOCH);
    let site = &fx.ssh.id.0;

    let (h, text) = read(&fx, json!({"site": site, "path": "/t.txt"})).await;
    assert_eq!(text, "0123456789");
    assert_eq!(
        h,
        json!({"path": "/t.txt", "size": 10, "offset": 0, "length": 10,
            "next_offset": 10, "eof": true})
    );

    let (h, text) = read(
        &fx,
        json!({"site": site, "path": "/t.txt", "offset": 4, "length": 3}),
    )
    .await;
    assert_eq!(text, "456");
    assert_eq!(h["length"], 3);
    assert_eq!(h["next_offset"], 7);
    assert_eq!(h["eof"], false);

    let (h, text) = read(&fx, json!({"site": site, "path": "/t.txt", "offset": 20})).await;
    assert_eq!(text, "");
    assert_eq!(h["length"], 0);
    assert_eq!(h["eof"], true);

    // The range ends inside the é: it is left out and read next time.
    let (h, text) = read(
        &fx,
        json!({"site": site, "path": "/e.txt", "offset": 0, "length": 3}),
    )
    .await;
    assert_eq!(text, "ab");
    assert_eq!(h["offset"], 0);
    assert_eq!(h["length"], 2);
    assert_eq!(h["next_offset"], 2);
    assert_eq!(h["eof"], false);

    // The range starts inside the é: its second half is left out.
    let (h, text) = read(&fx, json!({"site": site, "path": "/e.txt", "offset": 3})).await;
    assert_eq!(text, "!");
    assert_eq!(h["offset"], 4);
    assert_eq!(h["length"], 1);
    assert_eq!(h["next_offset"], 5);
    assert_eq!(h["eof"], true);
}

#[tokio::test]
async fn read_file_refuses_binary_and_oversized_ranges() {
    let fx = fixture();
    fx.ssh_fs
        .add_file("/big.txt", &vec![b'a'; 2 * 1024 * 1024], EPOCH);
    fx.ssh_fs.add_file("/bin", &[0xff, 0xfe], EPOCH);
    fx.ssh_fs.add_dir("/dir");
    let site = &fx.ssh.id.0;

    let text = err(fx
        .call("read_file", json!({"site": site, "path": "/big.txt"}))
        .await);
    for word in ["2097152", "offset", "length", "download"] {
        assert!(text.contains(word), "{word} in {text}");
    }
    assert_eq!(fx.host.streamed(&fx.ssh.id), 0, "nothing was read");

    let (h, text) = read(
        &fx,
        json!({"site": site, "path": "/big.txt", "length": 1_048_576}),
    )
    .await;
    assert_eq!(text.len(), 1_048_576);
    assert_eq!(h["next_offset"], 1_048_576);
    assert_eq!(h["eof"], false);
    assert_eq!(fx.host.streamed(&fx.ssh.id), 1_048_576, "only the range");

    let text = err(fx
        .call("read_file", json!({"site": site, "path": "/bin"}))
        .await);
    assert!(text.contains("download"), "{text}");
    assert!(text.contains("UTF-8"), "{text}");

    let text = err(fx
        .call("read_file", json!({"site": site, "path": "/dir"}))
        .await);
    assert!(text.contains("/dir"), "{text}");
    assert!(text.contains("folder"), "{text}");
}

#[tokio::test]
async fn odd_arguments_are_tool_errors() {
    let fx = fixture();
    let mut ssh = fx.ssh.clone();
    ssh.remote_dir = Some("/srv/app".to_owned());
    fx.host
        .set_sites(vec![ssh, fx.ftp.clone(), fx.ro.clone(), fx.closed.clone()]);
    let site = &fx.ssh.id.0;
    let cases = [
        (
            "read_file",
            json!({"site": site, "path": "var/www/a.txt"}),
            "path",
        ),
        ("read_file", json!({"site": site, "path": ""}), "path"),
        ("read_file", json!({"site": 7, "path": "/a"}), "site"),
        ("read_file", json!({"site": site}), "path"),
        (
            "read_file",
            json!({"site": site, "path": "/a", "colour": "red"}),
            "colour",
        ),
        (
            "list_directory",
            json!({"site": site, "path": "/", "limit": 0}),
            "limit",
        ),
        (
            "list_directory",
            json!({"site": site, "path": "/", "offset": -1}),
            "offset",
        ),
        (
            "read_file",
            json!({"site": site, "path": "/a", "length": 1.5}),
            "length",
        ),
        ("stat", json!({"site": site, "path": "/../etc"}), "path"),
        ("stat", json!([site, "/"]), "object"),
    ];
    for (tool, args, named) in cases {
        let text = err(fx.call(tool, args.clone()).await);
        assert!(text.contains(named), "{tool} {args}: {text}");
    }
    let relative = err(fx
        .call("read_file", json!({"site": site, "path": "var/www/a.txt"}))
        .await);
    assert!(relative.contains("/srv/app"), "{relative}");
    assert_eq!(
        err(fx.call("shell", json!({})).await),
        "Unknown tool: shell"
    );
    assert_eq!(fx.host.calls(&fx.ssh.id, FsOp::Read), 0);
}

fn transfer(site: &SiteId, state: TransferState) -> TransferItem {
    TransferItem {
        id: TransferId(1),
        parent: None,
        spec: TransferSpec {
            site: site.clone(),
            direction: Direction::Download,
            local: RemotePath::new("/home/u/a.txt").unwrap(),
            remote: RemotePath::new("/var/www/a.txt").unwrap(),
        },
        is_dir: Some(false),
        size: Some(5),
        bytes_done: 0,
        state,
        origin: Origin::Agent,
        tree_policy: None,
    }
}

#[tokio::test(start_paused = true)]
async fn idle_sessions_close_after_ten_minutes() {
    let mut fx = fixture();
    let minute = Duration::from_secs(60);
    let touch = |site: &Site| json!({"site": site.id.0, "path": "/"});
    ok(fx.call("list_directory", touch(&fx.ssh)).await);
    ok(fx.call("list_directory", touch(&fx.ftp)).await);
    ok(fx.call("list_directory", touch(&fx.ro)).await);
    fx.host
        .set_transfers(vec![transfer(&fx.ro.id, TransferState::Active)]);
    // A tab's session is never the agent's to close.
    let tab = site("Tab", "tab.example", Protocol::Ssh, AgentAccess::All);
    fx.host.add_site(tab.clone());
    let mut sites = vec![fx.ssh.clone(), fx.ftp.clone(), fx.ro.clone(), tab.clone()];
    sites.push(fx.closed.clone());
    fx.host.set_sites(sites);
    fx.host.connect_user(&tab.id).await;
    fx.events();

    tokio::time::advance(9 * minute).await;
    ok(fx.call("list_directory", touch(&fx.ftp)).await);
    tokio::time::advance(minute).await;
    fx.service.reap().await;
    assert_eq!(fx.host.closed(), std::slice::from_ref(&fx.ssh.id));
    assert_eq!(
        fx.events().last(),
        Some(&AgentEvent::Sites(fx.host.agent_sites()))
    );
    let mut left = vec![fx.ftp.id.clone(), fx.ro.id.clone()];
    left.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(fx.host.agent_sites(), left);

    tokio::time::advance(9 * minute).await;
    fx.service.reap().await;
    assert_eq!(fx.host.closed(), [fx.ssh.id.clone(), fx.ftp.id.clone()]);
    assert_eq!(fx.host.agent_sites(), [fx.ro.id.clone()], "a transfer runs");

    fx.host
        .set_transfers(vec![transfer(&fx.ro.id, TransferState::Done)]);
    fx.service.reap().await;
    assert_eq!(
        fx.host.closed(),
        [fx.ssh.id.clone(), fx.ftp.id.clone(), fx.ro.id.clone()]
    );
    assert!(fx.host.connected(&tab.id).is_some());

    // A session no call was ever made on closes at the first look.
    fx.host.mark_agent_only(&fx.ssh.id);
    fx.service.reap().await;
    assert_eq!(fx.host.closed().last(), Some(&fx.ssh.id));
}

#[tokio::test(start_paused = true)]
async fn session_of_a_site_that_lost_access_is_closed() {
    let fx = fixture();
    ok(fx
        .call("list_directory", json!({"site": fx.ro.id.0, "path": "/"}))
        .await);
    ok(fx
        .call("list_directory", json!({"site": fx.ssh.id.0, "path": "/"}))
        .await);
    fx.set_level(&fx.ro, AgentAccess::None);
    fx.service.reap().await;
    assert_eq!(fx.host.closed(), std::slice::from_ref(&fx.ro.id));
    // A deleted site too.
    fx.host
        .set_sites(vec![fx.ftp.clone(), fx.ro.clone(), fx.closed.clone()]);
    fx.service.reap().await;
    assert_eq!(fx.host.closed(), [fx.ro.id.clone(), fx.ssh.id.clone()]);
}

/// The master switch off: no site is open to agents, so their sessions close at the
/// next look, idle or not; a tab's session stays.
#[tokio::test(start_paused = true)]
async fn switch_off_closes_agent_sessions_at_once() {
    let mut fx = fixture();
    ok(fx
        .call("list_directory", json!({"site": fx.ssh.id.0, "path": "/"}))
        .await);
    fx.host.connect_user(&fx.ro.id).await;
    fx.events();
    fx.service.reap().await;
    assert!(fx.host.closed().is_empty(), "just used, switch on: kept");
    fx.host.set_enabled(false);
    fx.service.reap().await;
    assert_eq!(fx.host.closed(), std::slice::from_ref(&fx.ssh.id));
    assert!(
        fx.host.connected(&fx.ro.id).is_some(),
        "a tab's session stays"
    );
    assert_eq!(fx.events().last(), Some(&AgentEvent::Sites(Vec::new())));
}

#[tokio::test]
async fn one_log_line_per_call() {
    let (logs, _guard) = capture_logs();
    let fx = fixture();
    let site = &fx.ssh.id.0;
    ok(fx
        .call("read_file", json!({"site": site, "path": "/var/www/a.txt"}))
        .await);
    let lines = logs.agent_lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains("INFO agent: read /var/www/a.txt on \"Acme\""),
        "{lines:?}"
    );

    err(fx
        .call(
            "read_file",
            json!({"site": site, "path": "/var/www/nope.txt"}),
        )
        .await);
    let lines = logs.agent_lines();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(
        lines[1].contains("read /var/www/nope.txt on \"Acme\" (failed: "),
        "{lines:?}"
    );

    err(fx
        .call(
            "read_file",
            json!({"site": fx.closed.id.0, "path": "/var/www/a.txt"}),
        )
        .await);
    let lines = logs.agent_lines();
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(
        lines[2].ends_with("refused read_file: site not available"),
        "{lines:?}"
    );

    fx.ssh_fs.add_file("/var/www/a\nb.txt", b"x", EPOCH);
    ok(fx
        .call(
            "read_file",
            json!({"site": site, "path": "/var/www/a\nb.txt"}),
        )
        .await);
    let lines = logs.agent_lines();
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(lines[3].contains("read /var/www/a b.txt on"), "{lines:?}");
    assert_eq!(logs.text().lines().count(), 4, "one line per call");

    ok(fx.call("list_sites", json!({})).await);
    ok(fx
        .call("list_directory", json!({"site": site, "path": "/var/www"}))
        .await);
    ok(fx
        .call("stat", json!({"site": site, "path": "/var/www/a.txt"}))
        .await);
    err(fx.call("stat", json!({"site": site})).await);
    assert_eq!(logs.agent_lines().len(), 8, "{:?}", logs.agent_lines());
}

#[tokio::test]
async fn log_has_no_content_and_no_secret() {
    let (logs, _guard) = capture_logs();
    let fx = fixture();
    fx.ssh_fs.add_file("/var/www/s.txt", b"TOP-SECRET", EPOCH);
    let text = ok(fx
        .call(
            "read_file",
            json!({"site": fx.ssh.id.0, "path": "/var/www/s.txt"}),
        )
        .await);
    assert_eq!(text[1], "TOP-SECRET");
    fx.host.fail_connect(
        &fx.ro.id,
        Error::new(ErrorKind::Auth, "Key /home/me/.ssh/id_ed25519 was refused")
            .with_detail("tried /home/me/.ssh/id_ed25519"),
    );
    err(fx
        .call(
            "read_file",
            json!({"site": fx.ro.id.0, "path": "/var/www/a.txt"}),
        )
        .await);
    let log = logs.text();
    assert_eq!(logs.agent_lines().len(), 2, "{log}");
    assert!(!log.contains("TOP-SECRET"), "{log}");
    assert!(!log.contains("/home/me/.ssh"), "{log}");
    assert!(log.contains("on \"Acme staging\" (failed: "), "{log}");
}

/// The service runs in the channel's tasks: its calls must be `Send`.
#[test]
fn calls_are_send() {
    fn send<T: Send>(_: &T) {}
    let fx = fixture();
    let call = fx.service.call("list_sites", json!({}));
    send(&call);
    let reap = fx.service.reap();
    send(&reap);
}

/// A call in flight keeps its site's session through the reaper, idle or not, open to
/// agents or not; the idle time counts from the end of the last call.
#[tokio::test(start_paused = true)]
async fn reap_leaves_sites_with_calls_in_flight() {
    let fx = fixture();
    let minute = Duration::from_secs(60);
    let hold = fx.host.hold(&fx.ssh.id, FsOp::List);
    let running = tokio::spawn({
        let service = fx.service.clone();
        let args = json!({"site": fx.ssh.id.0, "path": "/var/www"});
        async move { service.call("list_directory", args).await }
    });
    hold.entered().await;
    tokio::time::advance(11 * minute).await;
    fx.service.reap().await;
    assert!(fx.host.closed().is_empty(), "idle, but a call runs");
    fx.set_level(&fx.ssh, AgentAccess::None);
    fx.service.reap().await;
    assert!(fx.host.closed().is_empty(), "calls already running finish");
    hold.release();
    assert_eq!(names(&head(running.await.unwrap())), ["a.txt"]);
    fx.service.reap().await;
    assert_eq!(fx.host.closed(), std::slice::from_ref(&fx.ssh.id));

    // A call that ran 11 minutes ended just now: not idle.
    let hold = fx.host.hold(&fx.ftp.id, FsOp::List);
    let running = tokio::spawn({
        let service = fx.service.clone();
        let args = json!({"site": fx.ftp.id.0, "path": "/"});
        async move { service.call("list_directory", args).await }
    });
    hold.entered().await;
    tokio::time::advance(11 * minute).await;
    hold.release();
    ok(running.await.unwrap());
    fx.service.reap().await;
    assert_eq!(fx.host.closed(), std::slice::from_ref(&fx.ssh.id));
    tokio::time::advance(10 * minute).await;
    fx.service.reap().await;
    assert_eq!(fx.host.closed(), [fx.ssh.id.clone(), fx.ftp.id.clone()]);
}

/// A call dropped after its check writes its line, ending in "(cancelled)", and no
/// longer counts as running.
#[tokio::test]
async fn a_cancelled_call_is_logged() {
    let (logs, _guard) = capture_logs();
    let fx = fixture();
    let hold = fx.host.hold(&fx.ssh.id, FsOp::List);
    let running = tokio::spawn({
        let service = fx.service.clone();
        let args = json!({"site": fx.ssh.id.0, "path": "/var/www"});
        async move { service.call("list_directory", args).await }
    });
    hold.entered().await;
    assert!(logs.agent_lines().is_empty());
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());
    assert!(hold.dropped());
    let lines = logs.agent_lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].ends_with("listed /var/www on \"Acme\" (cancelled)"),
        "{lines:?}"
    );
    assert!(fx.service.activity().running.is_empty());
}

/// Two calls on a site that is not connected wait on one attempt.
#[tokio::test]
async fn concurrent_calls_share_one_connection() {
    let mut fx = fixture();
    let hold = fx.host.hold_connect(&fx.ssh.id);
    let spawn = |fx: &Fx| {
        let service = fx.service.clone();
        let args = json!({"site": fx.ssh.id.0, "path": "/var/www"});
        tokio::spawn(async move { service.call("list_directory", args).await })
    };
    let first = spawn(&fx);
    let second = spawn(&fx);
    hold.entered().await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    hold.release();
    ok(first.await.unwrap());
    ok(second.await.unwrap());
    assert_eq!(fx.host.connects(&fx.ssh.id), 1);
    let session = fx.host.connected(&fx.ssh.id).unwrap();
    assert_eq!(
        fx.events(),
        [
            AgentEvent::Connected(session),
            AgentEvent::Sites(vec![fx.ssh.id.clone()])
        ]
    );
}

/// The switch turned off while a call runs: that call finishes, the next is refused.
#[tokio::test]
async fn switch_off_lets_the_running_call_finish() {
    let fx = fixture();
    let hold = fx.host.hold(&fx.ssh.id, FsOp::List);
    let args = json!({"site": fx.ssh.id.0, "path": "/var/www"});
    let running = tokio::spawn({
        let service = fx.service.clone();
        let args = args.clone();
        async move { service.call("list_directory", args).await }
    });
    hold.entered().await;
    fx.host.set_enabled(false);
    hold.release();
    assert_eq!(names(&head(running.await.unwrap())), ["a.txt"]);
    assert_eq!(err(fx.call("list_directory", args).await), TURNED_OFF);
}

fn schema_without_site() -> Value {
    json!({"type": "object", "properties": {
        "remote_path": {"type": "string", "description": "x"},
        "local_path": {"type": "string", "description": "x"},
    }, "required": [], "additionalProperties": false})
}

/// Whether a call is checked against a site comes from the tool's need: a tool that
/// needs a site and arrives without one is refused, whatever its schema says.
#[tokio::test]
async fn admission_follows_the_need_not_the_arguments() {
    let (logs, _guard) = capture_logs();
    let fx = fixture();
    for need in [Need::Read, Need::All, Need::AllSsh] {
        let spec = tools::ToolSpec {
            name: "download",
            title: "t",
            description: "d",
            schema: schema_without_site,
            need,
            read_only: false,
            destructive: true,
            idempotent: true,
            open_world: true,
        };
        let (reply, line) = fx
            .service
            .run_spec(&spec, json!({"remote_path": "/a", "local_path": "/b"}))
            .await;
        // Every tool's arguments are now typed, and a sited tool's type wants `site`:
        // the call is refused before any site is looked at, never let through.
        assert!(matches!(reply, Reply::Err(_)), "{need:?}");
        assert!(line.starts_with("refused download: "), "{line}");
    }
    assert!(logs.agent_lines().is_empty(), "run_spec does not log");
    assert!(fx.host.agent_sites().is_empty());
    assert!(fx.host.enqueued().is_empty());
    assert_eq!(fx.host.connects(&fx.ssh.id), 0);
}

/// `/proc` files say size 0 over SFTP: they are read for what they hold.
#[tokio::test]
async fn zero_size_files_are_read_by_content() {
    let fx = fixture();
    fx.ssh_fs
        .add_file("/proc/loadavg", b"0.42 0.30 0.20\n", EPOCH);
    fx.host.stat_zero_size(&fx.ssh.id, "/proc/loadavg");
    fx.ssh_fs.add_file("/empty", b"", EPOCH);
    let site = &fx.ssh.id.0;

    let (h, text) = read(&fx, json!({"site": site, "path": "/proc/loadavg"})).await;
    assert_eq!(text, "0.42 0.30 0.20\n");
    assert_eq!(h["size"], 15);
    assert_eq!(h["eof"], true);

    let (h, text) = read(
        &fx,
        json!({"site": site, "path": "/proc/loadavg", "length": 4}),
    )
    .await;
    assert_eq!(text, "0.42");
    assert_eq!(h["eof"], false);
    assert_eq!(h["size"], Value::Null, "not known before the end");
    assert_eq!(h["next_offset"], 4);

    let (h, text) = read(&fx, json!({"site": site, "path": "/empty"})).await;
    assert_eq!(text, "");
    assert_eq!(h["size"], 0);
    assert_eq!(h["eof"], true);

    // More than the cap behind a size of 0: the too-long answer, after at most one
    // byte past the cap was read.
    fx.ssh_fs
        .add_file("/proc/big", &vec![b'a'; 1_048_576 + 10], EPOCH);
    fx.host.stat_zero_size(&fx.ssh.id, "/proc/big");
    let before = fx.host.streamed(&fx.ssh.id);
    let text = err(fx
        .call("read_file", json!({"site": site, "path": "/proc/big"}))
        .await);
    for word in ["more than 1048576 bytes", "offset", "length", "download"] {
        assert!(text.contains(word), "{word} in {text}");
    }
    assert_eq!(fx.host.streamed(&fx.ssh.id) - before, 1_048_577);
}

/// A link at the end of the path is refused before anything is opened: `RemoteFs`
/// cannot resolve it, and SFTP's `open_read` refuses it.
#[tokio::test]
async fn links_are_refused_up_front() {
    let fx = fixture();
    fx.ssh_fs.add_symlink(
        "/etc/nginx/sites-enabled/app",
        "/etc/nginx/sites-available/app",
    );
    let text = err(fx
        .call(
            "read_file",
            json!({"site": fx.ssh.id.0, "path": "/etc/nginx/sites-enabled/app"}),
        )
        .await);
    assert!(text.contains("symbolic link"), "{text}");
    assert!(text.contains("stat"), "{text}");
    // Nothing the agent could paste into run_command.
    assert!(
        !text.contains("readlink") && !text.contains("run_command"),
        "{text}"
    );
    assert!(text.contains("/etc/nginx/sites-enabled/app"), "{text}");
    assert_eq!(fx.host.calls(&fx.ssh.id, FsOp::Read), 0, "never opened");
}

/// Offsets and lengths at `u64::MAX`: answers, never a panic.
#[tokio::test]
async fn numbers_at_u64_max_do_not_panic() {
    let fx = fixture();
    fx.ssh_fs.add_file("/proc/stat", b"cpu 1 2 3\n", EPOCH);
    fx.host.stat_zero_size(&fx.ssh.id, "/proc/stat");
    let site = &fx.ssh.id.0;
    let max = u64::MAX;
    for path in ["/var/www/a.txt", "/proc/stat"] {
        let (h, text) = read(
            &fx,
            json!({"site": site, "path": path, "offset": max, "length": max}),
        )
        .await;
        assert_eq!(text, "", "{path}");
        assert_eq!(h["eof"], true, "{path}");
        assert_eq!(h["offset"], max, "{path}");
        assert_eq!(h["next_offset"], max, "{path}");
    }
    // The size from `stat`; none for a size-0 file read past its end.
    let (h, _) = read(
        &fx,
        json!({"site": site, "path": "/var/www/a.txt", "offset": max}),
    )
    .await;
    assert_eq!(h["size"], 5);
    let (h, _) = read(
        &fx,
        json!({"site": site, "path": "/proc/stat", "offset": max}),
    )
    .await;
    assert_eq!(h["size"], Value::Null);
    let (h, _) = read(
        &fx,
        json!({"site": site, "path": "/proc/stat", "offset": 3}),
    )
    .await;
    assert_eq!(h["size"], 10, "read to the end: known");
    let (h, _) = read(
        &fx,
        json!({"site": site, "path": "/proc/stat", "offset": 50}),
    )
    .await;
    assert_eq!(h["size"], Value::Null, "past the end: not known");
    let listing = list(
        &fx,
        json!({"site": site, "path": "/var/www", "offset": max, "limit": max}),
    )
    .await;
    assert!(names(&listing).is_empty());
    assert_eq!(listing["truncated"], false);
}

/// Line separators and bidirectional controls in a path cannot split or reorder a log
/// line; an unknown tool's name never reaches the log.
#[tokio::test]
async fn log_lines_cannot_be_split_or_reordered() {
    let (logs, _guard) = capture_logs();
    let fx = fixture();
    let odd = "/var/www/a\u{2028}b\u{2029}c\u{202e}d\u{2066}e\u{200f}f.txt";
    fx.ssh_fs.add_file(odd, b"x", EPOCH);
    ok(fx
        .call("read_file", json!({"site": fx.ssh.id.0, "path": odd}))
        .await);
    let lines = logs.agent_lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains("read /var/www/a b c d e f.txt on"),
        "{lines:?}"
    );
    for c in ['\u{2028}', '\u{2029}', '\u{202e}', '\u{2066}', '\u{200f}'] {
        assert!(!logs.text().contains(c), "{c:?}");
    }
    for c in [
        '\u{200e}', '\u{202a}', '\u{202d}', '\u{2067}', '\u{2069}', '\u{85}',
    ] {
        assert_eq!(super::printable(&format!("a{c}b")), "a b", "{c:?}");
    }

    assert_eq!(
        err(fx.call("evil\ntool", json!({})).await),
        "Unknown tool: evil\ntool"
    );
    let lines = logs.agent_lines();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[1].ends_with("refused an unknown tool"), "{lines:?}");
    assert!(!logs.text().contains("evil"));
}

/// `Sites` is sent when the list changed, not on every look.
#[tokio::test]
async fn sites_events_only_when_the_list_changes() {
    let mut fx = fixture();
    fx.service.reap().await;
    assert_eq!(fx.events(), [AgentEvent::Sites(vec![])], "the first one");
    fx.service.reap().await;
    assert!(fx.events().is_empty());
    ok(fx
        .call("list_directory", json!({"site": fx.ssh.id.0, "path": "/"}))
        .await);
    let events = fx.events();
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[1], AgentEvent::Sites(vec![fx.ssh.id.clone()]));
    fx.service.reap().await;
    assert!(fx.events().is_empty(), "nothing changed");
}

/// One agent call waits on a connect with a dialog open and is cancelled: the attempt
/// goes at once (its connect dropped, the site's connect lock free), the agent-only
/// session is closed, and the user's own connect proceeds.
#[tokio::test(start_paused = true)]
async fn an_abandoned_attempt_is_out_of_the_users_way() {
    let fx = fixture();
    fx.host.hang_connect(&fx.ssh.id);
    fx.host.set_prompt_open(&fx.ssh.id, true);
    let call = tokio::spawn({
        let service = fx.service.clone();
        let args = json!({"site": fx.ssh.id.0, "path": "/var/www"});
        async move { service.call("list_directory", args).await }
    });
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(fx.host.connects(&fx.ssh.id), 1);
    assert!(fx.host.prompt_open(&fx.ssh.id), "a dialog waits");
    call.abort();
    let _ = call.await;
    assert!(fx.service.connecting().is_empty(), "the attempt is gone");
    assert!(fx.service.activity().running.is_empty());
    tokio::time::timeout(Duration::from_secs(5), async {
        // Sleeps, not yields: paused time must move for the bound to fire.
        while fx.host.closed().is_empty() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the agent-only session is closed");
    assert_eq!(fx.host.closed(), std::slice::from_ref(&fx.ssh.id));
    let tab = tokio::time::timeout(Duration::from_secs(5), fx.host.connect_user(&fx.ssh.id))
        .await
        .expect("the user's connect waits on nothing the agent left");
    assert_eq!(fx.host.connected(&fx.ssh.id), Some(tab));
}

/// Two calls wait on one attempt; one is cancelled: the other still gets its answer.
#[tokio::test]
async fn a_cancelled_waiter_leaves_the_others_their_answer() {
    let fx = fixture();
    let hold = fx.host.hold_connect(&fx.ssh.id);
    let spawn = |fx: &Fx| {
        let service = fx.service.clone();
        let args = json!({"site": fx.ssh.id.0, "path": "/var/www"});
        tokio::spawn(async move { service.call("list_directory", args).await })
    };
    let first = spawn(&fx);
    let second = spawn(&fx);
    hold.entered().await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    first.abort();
    let _ = first.await;
    assert!(!hold.dropped(), "the attempt still has a waiter");
    hold.release();
    assert_eq!(names(&head(second.await.unwrap())), ["a.txt"]);
    assert_eq!(fx.host.connects(&fx.ssh.id), 1);
    assert!(fx.service.connecting().is_empty());
}

/// The two minutes belong to the attempt: a call that joins 90 s in is answered 30 s
/// later with the first; the next call after the time-out starts a fresh attempt.
#[tokio::test(start_paused = true)]
async fn the_limit_is_the_attempts() {
    let fx = fixture();
    fx.host.hang_connect(&fx.ssh.id);
    fx.host.set_prompt_open(&fx.ssh.id, true);
    let spawn = |fx: &Fx| {
        let service = fx.service.clone();
        let args = json!({"site": fx.ssh.id.0, "path": "/var/www"});
        tokio::spawn(async move { service.call("list_directory", args).await })
    };
    let first = spawn(&fx);
    tokio::time::sleep(Duration::from_secs(90)).await;
    let joined = Instant::now();
    let second = spawn(&fx);
    assert_eq!(err(first.await.unwrap()), WAITING_FOR_USER);
    assert_eq!(err(second.await.unwrap()), WAITING_FOR_USER);
    assert_eq!(joined.elapsed(), Duration::from_secs(30));
    assert_eq!(fx.host.connects(&fx.ssh.id), 1);
    assert_eq!(fx.host.closed(), std::slice::from_ref(&fx.ssh.id));
    assert!(fx.service.connecting().is_empty());

    let started = Instant::now();
    assert_eq!(err(spawn(&fx).await.unwrap()), WAITING_FOR_USER);
    assert_eq!(started.elapsed(), Duration::from_secs(120));
    assert_eq!(fx.host.connects(&fx.ssh.id), 2, "a fresh attempt");
}

// ---- Task 10: writing, deleting, commands ----

/// `more` with the site's id as `site`.
fn on(site: &Site, more: Value) -> Value {
    let mut args = more;
    args["site"] = json!(site.id.0);
    args
}

/// The names in a folder of `fs`, sorted.
async fn children(fs: &MemFs, path: &str) -> Vec<String> {
    let mut names: Vec<String> = fs
        .list(&RemotePath::new(path).unwrap())
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    names.sort();
    names
}

/// Waits (yielding, no timer) until `done` holds.
async fn until(mut done: impl FnMut() -> bool) {
    for _ in 0..10_000 {
        if done() {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("never happened");
}

fn exec_output(stdout: &[u8], stderr: &[u8], end: ExecEnd) -> ExecOutput {
    ExecOutput {
        stdout: stdout.to_vec(),
        stderr: stderr.to_vec(),
        stdout_truncated: false,
        stderr_truncated: false,
        end,
    }
}

#[tokio::test]
async fn write_file_rules() {
    let fx = fixture();
    let write = |site: &Site, path: &str, content: &str, overwrite: Option<bool>| {
        let mut args = on(site, json!({"path": path, "content": content}));
        if let Some(overwrite) = overwrite {
            args["overwrite"] = json!(overwrite);
        }
        let service = fx.service.clone();
        async move { service.call("write_file", args).await }
    };

    // A new file.
    let reply = write(&fx.ssh, "/var/www/new.txt", "hello world!", None).await;
    assert_eq!(
        head(reply),
        json!({"ok": true, "path": "/var/www/new.txt", "bytes": 12})
    );
    assert_eq!(
        fx.ssh_fs.read_file("/var/www/new.txt").as_deref(),
        Some(&b"hello world!"[..])
    );
    // Over SFTP by a temporary file and a rename, and none is left behind.
    assert!(fx.host.calls(&fx.ssh.id, FsOp::Rename) >= 1);
    assert_eq!(children(&fx.ssh_fs, "/var/www").await, ["a.txt", "new.txt"]);

    // An existing file without `overwrite`: an error naming it, the old bytes stay.
    let writes = fx.host.calls(&fx.ssh.id, FsOp::Write);
    for overwrite in [None, Some(false)] {
        let text = err(write(&fx.ssh, "/var/www/a.txt", "other", overwrite).await);
        assert!(text.contains("overwrite"), "{text}");
        assert!(text.contains("/var/www/a.txt"), "{text}");
    }
    assert_eq!(
        fx.ssh_fs.read_file("/var/www/a.txt").as_deref(),
        Some(&b"hello"[..])
    );
    assert_eq!(fx.host.calls(&fx.ssh.id, FsOp::Write), writes);
    // With it: replaced.
    ok(write(&fx.ssh, "/var/www/a.txt", "other", Some(true)).await);
    assert_eq!(
        fx.ssh_fs.read_file("/var/www/a.txt").as_deref(),
        Some(&b"other"[..])
    );
    assert_eq!(children(&fx.ssh_fs, "/var/www").await, ["a.txt", "new.txt"]);

    // One byte over the cap: an error, nothing written, no connection needed.
    let writes = fx.host.calls(&fx.ssh.id, FsOp::Write);
    let big = "x".repeat(1_048_577);
    fx.host.close_agent_session(&fx.ssh.id).await;
    let connects = fx.host.connects(&fx.ssh.id);
    let text = err(write(&fx.ssh, "/var/www/big.txt", &big, Some(true)).await);
    assert!(text.contains("1048576"), "{text}");
    assert_eq!(
        fx.host.connects(&fx.ssh.id),
        connects,
        "no connection for it"
    );
    assert!(fx.ssh_fs.read_file("/var/www/big.txt").is_none());
    assert_eq!(fx.host.calls(&fx.ssh.id, FsOp::Write), writes);
    // The cap itself fits.
    let full = "x".repeat(1_048_576);
    assert_eq!(
        head(write(&fx.ssh, "/var/www/big.txt", &full, None).await)["bytes"],
        1_048_576
    );

    // A folder or a link at the path: an error, with or without `overwrite`.
    fx.ssh_fs.add_dir("/var/www/dir");
    fx.ssh_fs.add_symlink("/var/www/link", "/etc/passwd");
    let writes = fx.host.calls(&fx.ssh.id, FsOp::Write);
    for overwrite in [None, Some(true)] {
        let text = err(write(&fx.ssh, "/var/www/dir", "x", overwrite).await);
        assert!(text.contains("folder"), "{text}");
        let text = err(write(&fx.ssh, "/var/www/link", "x", overwrite).await);
        assert!(text.contains("symbolic link"), "{text}");
    }
    assert_eq!(fx.host.calls(&fx.ssh.id, FsOp::Write), writes);
    assert!(
        fx.ssh_fs
            .stat(&RemotePath::new("/var/www/dir").unwrap())
            .await
            .is_ok()
    );

    // Over FTP in place: one write, no rename.
    ok(write(&fx.ftp, "/new.txt", "ftp text", None).await);
    assert_eq!(
        fx.ftp_fs.read_file("/new.txt").as_deref(),
        Some(&b"ftp text"[..])
    );
    assert_eq!(fx.host.calls(&fx.ftp.id, FsOp::Write), 1);
    assert_eq!(fx.host.calls(&fx.ftp.id, FsOp::Rename), 0);

    // An in-place write cut by the network: the file may now be incomplete.
    fx.host.fail(
        &fx.ftp.id,
        FsOp::Write,
        Error::new(ErrorKind::Network, "Connection lost"),
    );
    let text = err(write(&fx.ftp, "/new.txt", "again", Some(true)).await);
    assert!(text.contains("Connection lost"), "{text}");
    assert!(text.contains("may now be incomplete"), "{text}");
    // Over SFTP the cut write is the temporary file's: the file itself is untouched.
    fx.host.fail(
        &fx.ssh.id,
        FsOp::Write,
        Error::new(ErrorKind::Network, "Connection lost"),
    );
    let text = err(write(&fx.ssh, "/var/www/a.txt", "again", Some(true)).await);
    assert!(text.contains("Connection lost"), "{text}");
    assert!(!text.contains("incomplete"), "{text}");
    assert_eq!(
        fx.ssh_fs.read_file("/var/www/a.txt").as_deref(),
        Some(&b"other"[..])
    );
}

#[tokio::test]
async fn make_rename_chmod() {
    let fx = fixture();
    let call = |tool: &'static str, args: Value| {
        let service = fx.service.clone();
        async move { service.call(tool, args).await }
    };

    // A folder; again (it exists: not an error); a missing parent is the error.
    let mkdir = on(&fx.ssh, json!({"path": "/var/www/new"}));
    assert_eq!(
        head(call("make_directory", mkdir.clone()).await),
        json!({"ok": true, "path": "/var/www/new"})
    );
    let made = fx
        .ssh_fs
        .stat(&RemotePath::new("/var/www/new").unwrap())
        .await;
    assert_eq!(made.unwrap().kind, hoprs_core::EntryKind::Dir);
    ok(call("make_directory", mkdir).await);
    let text = err(call(
        "make_directory",
        on(&fx.ssh, json!({"path": "/no/such/dir"})),
    )
    .await);
    assert!(text.contains("/no/such/dir"), "{text}");
    assert!(text.contains("parent"), "{text}");
    assert!(
        fx.ssh_fs
            .stat(&RemotePath::new("/no").unwrap())
            .await
            .is_err()
    );
    // A file at the path is not a folder.
    err(call(
        "make_directory",
        on(&fx.ssh, json!({"path": "/var/www/a.txt"})),
    )
    .await);

    // Rename onto an existing name: refused, both files intact.
    fx.ssh_fs.add_file("/var/www/b.txt", b"bee", EPOCH);
    let text = err(call(
        "rename",
        on(
            &fx.ssh,
            json!({"from": "/var/www/a.txt", "to": "/var/www/b.txt"}),
        ),
    )
    .await);
    assert!(text.contains("/var/www/b.txt"), "{text}");
    assert!(text.contains("already"), "{text}");
    assert_eq!(fx.host.calls(&fx.ssh.id, FsOp::Rename), 0);
    assert_eq!(
        fx.ssh_fs.read_file("/var/www/a.txt").as_deref(),
        Some(&b"hello"[..])
    );
    assert_eq!(
        fx.ssh_fs.read_file("/var/www/b.txt").as_deref(),
        Some(&b"bee"[..])
    );
    // Onto a free name: moved.
    assert_eq!(
        head(
            call(
                "rename",
                on(
                    &fx.ssh,
                    json!({"from": "/var/www/a.txt", "to": "/var/www/new/c.txt"})
                ),
            )
            .await
        ),
        json!({"ok": true, "from": "/var/www/a.txt", "to": "/var/www/new/c.txt"})
    );
    assert!(fx.ssh_fs.read_file("/var/www/a.txt").is_none());
    assert_eq!(
        fx.ssh_fs.read_file("/var/www/new/c.txt").as_deref(),
        Some(&b"hello"[..])
    );
    // The root is neither renamed nor replaced; both paths are checked.
    let renames = fx.host.calls(&fx.ssh.id, FsOp::Rename);
    for (from, to) in [
        ("/", "/x"),
        ("/var/www/b.txt", "/"),
        ("/var/www/b.txt", "rel"),
    ] {
        err(call("rename", on(&fx.ssh, json!({"from": from, "to": to}))).await);
    }
    let text = err(call(
        "rename",
        on(&fx.ssh, json!({"from": "/a/../..", "to": "/x"})),
    )
    .await);
    assert!(text.contains("`from`"), "{text}");
    assert_eq!(fx.host.calls(&fx.ssh.id, FsOp::Rename), renames);

    // Permissions: three or four octal digits.
    let chmod = |mode: Value| on(&fx.ssh, json!({"path": "/var/www/b.txt", "mode": mode}));
    let mode = || async {
        fx.ssh_fs
            .stat(&RemotePath::new("/var/www/b.txt").unwrap())
            .await
            .unwrap()
            .mode
    };
    assert_eq!(
        head(call("set_permissions", chmod(json!("644"))).await),
        json!({"ok": true, "path": "/var/www/b.txt", "mode": "0644"})
    );
    assert_eq!(mode().await, Some(0o644));
    ok(call("set_permissions", chmod(json!("0755"))).await);
    assert_eq!(mode().await, Some(0o755));
    let chmods = fx.host.calls(&fx.ssh.id, FsOp::Chmod);
    for bad in ["9z", "77777", "", "64", "0x1f", "+755", "７５５"] {
        let text = err(call("set_permissions", chmod(json!(bad))).await);
        assert!(text.contains("octal"), "{bad}: {text}");
    }
    let text = err(call("set_permissions", chmod(json!(644))).await);
    assert!(text.contains("`mode`"), "{text}");
    assert_eq!(fx.host.calls(&fx.ssh.id, FsOp::Chmod), chmods);
    assert_eq!(mode().await, Some(0o755));

    // An FTP server without SITE CHMOD answers 502, which the FTP backend reports as an
    // unexpected reply (`map_reply`): a clear error.
    fx.ftp_fs.add_file("/f.txt", b"f", EPOCH);
    fx.host.fail(
        &fx.ftp.id,
        FsOp::Chmod,
        Error::new(ErrorKind::Protocol, "Unexpected FTP reply: /f.txt")
            .with_detail("502 SITE CHMOD not understood"),
    );
    let text = err(call(
        "set_permissions",
        on(&fx.ftp, json!({"path": "/f.txt", "mode": "600"})),
    )
    .await);
    assert!(text.contains("did not accept"), "{text}");
    assert!(text.contains("0600"), "{text}");
    assert!(!text.contains("502"), "{text}");
    // A link at the path: refused before any chmod (SFTP's would change its target).
    fx.ssh_fs.add_symlink("/var/www/link", "/etc/passwd");
    let chmods = fx.host.calls(&fx.ssh.id, FsOp::Chmod);
    let text = err(call(
        "set_permissions",
        on(&fx.ssh, json!({"path": "/var/www/link", "mode": "777"})),
    )
    .await);
    assert!(text.contains("symbolic link"), "{text}");
    assert_eq!(fx.host.calls(&fx.ssh.id, FsOp::Chmod), chmods);
}

#[tokio::test]
async fn delete_needs_recursive_for_a_full_folder() {
    let fx = fixture();
    let fs = &fx.ssh_fs;
    let delete = |path: &str, recursive: Option<bool>| {
        let mut args = on(&fx.ssh, json!({"path": path}));
        if let Some(recursive) = recursive {
            args["recursive"] = json!(recursive);
        }
        let service = fx.service.clone();
        async move { service.call("delete", args).await }
    };

    // A file.
    assert_eq!(
        head(delete("/var/www/a.txt", None).await),
        json!({"ok": true, "path": "/var/www/a.txt", "deleted": 1})
    );
    assert!(fs.read_file("/var/www/a.txt").is_none());
    // An empty folder.
    fs.add_dir("/var/www/empty");
    assert_eq!(head(delete("/var/www/empty", None).await)["deleted"], 1);
    assert!(children(fs, "/var/www").await.is_empty());

    // A folder with content: without `recursive`, an error and nothing removed.
    fs.add_file("/srv/tree/a.txt", b"a", EPOCH);
    fs.add_file("/srv/tree/sub/b.txt", b"b", EPOCH);
    fs.add_file("/keep/k.txt", b"k", EPOCH);
    fs.add_symlink("/srv/tree/link", "/keep");
    let removes = fx.host.calls(&fx.ssh.id, FsOp::Remove);
    for recursive in [None, Some(false)] {
        let text = err(delete("/srv/tree", recursive).await);
        assert!(text.contains("recursive"), "{text}");
        assert!(text.contains("/srv/tree"), "{text}");
    }
    assert_eq!(fx.host.calls(&fx.ssh.id, FsOp::Remove), removes);
    assert_eq!(children(fs, "/srv/tree").await, ["a.txt", "link", "sub"]);
    // With it: gone, every item counted (a.txt, link, b.txt, sub, tree), and the link
    // is removed, never followed.
    assert_eq!(head(delete("/srv/tree", Some(true)).await)["deleted"], 5);
    assert!(children(fs, "/srv").await.is_empty());
    assert_eq!(fs.read_file("/keep/k.txt").as_deref(), Some(&b"k"[..]));
    // A link at the path itself: the link goes, its target stays.
    fs.add_symlink("/srv/l", "/keep");
    assert_eq!(head(delete("/srv/l", Some(true)).await)["deleted"], 1);
    assert_eq!(fs.read_file("/keep/k.txt").as_deref(), Some(&b"k"[..]));

    // The root, and paths that are not absolute: refused before anything is touched.
    let (lists, removes) = (
        fx.host.calls(&fx.ssh.id, FsOp::List),
        fx.host.calls(&fx.ssh.id, FsOp::Remove),
    );
    for recursive in [None, Some(true)] {
        let text = err(delete("/", recursive).await);
        assert!(text.contains("root"), "{text}");
        err(delete("", recursive).await);
        err(delete("keep", recursive).await);
        err(delete("/keep/..", recursive).await);
    }
    assert_eq!(fx.host.calls(&fx.ssh.id, FsOp::List), lists);
    assert_eq!(fx.host.calls(&fx.ssh.id, FsOp::Remove), removes);
    assert_eq!(fs.read_file("/keep/k.txt").as_deref(), Some(&b"k"[..]));

    // A failure half way says how much went and that the rest is still there.
    for i in 0..3 {
        fs.add_file(&format!("/part/a{i}"), b"", EPOCH);
    }
    fs.add_file("/part/z/locked", b"", EPOCH);
    fs.deny_writes_under("/part/z");
    let text = err(delete("/part", Some(true)).await);
    assert!(text.contains("3 items were deleted"), "{text}");
    assert!(text.contains("still there"), "{text}");
    assert_eq!(children(fs, "/part").await, ["z"]);
    // Missing: an error.
    err(delete("/nothing/here", Some(true)).await);
}

#[tokio::test]
async fn run_command_reply_and_limits() {
    let fx = fixture();
    let run = |site: &Site, more: Value| {
        let service = fx.service.clone();
        let args = on(site, more);
        async move { service.call("run_command", args).await }
    };
    let last = || fx.host.execs().last().cloned().unwrap();

    let mut out = exec_output(b"ok\n", b"warn", ExecEnd::Exit(3));
    out.stdout_truncated = true;
    fx.host.set_exec(Ok(out));
    let blocks = ok(run(&fx.ssh, json!({"command": "make test"})).await);
    let header: Value = serde_json::from_str(&blocks[0]).unwrap();
    assert_eq!(
        header,
        json!({"exit_code": 3, "signal": null, "timed_out": false, "lost": false,
            "stdout_truncated": true, "stderr_truncated": false})
    );
    assert_eq!(blocks[1..], ["stdout:\nok\n", "stderr:\nwarn"]);
    let (site, command, limits) = last();
    assert_eq!((site, command.as_str()), (fx.ssh.id.clone(), "make test"));
    assert_eq!(limits.timeout, Duration::from_secs(60), "the default");
    assert_eq!(limits.stream_bytes, 262_144);

    // The time limit: clamped to 600 s, at least 1 s.
    ok(run(&fx.ssh, json!({"command": "x", "timeout_seconds": 5000})).await);
    assert_eq!(last().2.timeout, Duration::from_secs(600));
    ok(run(&fx.ssh, json!({"command": "x", "timeout_seconds": 5})).await);
    assert_eq!(last().2.timeout, Duration::from_secs(5));
    let runs = fx.host.execs().len();
    err(run(&fx.ssh, json!({"command": "x", "timeout_seconds": 0})).await);
    err(run(&fx.ssh, json!({"command": "x", "timeout_seconds": "5"})).await);
    assert_eq!(fx.host.execs().len(), runs);

    // Timed out: no exit code, and the process may still be running.
    fx.host
        .set_exec(Ok(exec_output(b"", b"", ExecEnd::TimedOut)));
    let blocks = ok(run(&fx.ssh, json!({"command": "sleep 900"})).await);
    assert_eq!(blocks.len(), 1, "no empty stream blocks: {blocks:?}");
    let header: Value = serde_json::from_str(&blocks[0]).unwrap();
    assert_eq!(header["timed_out"], true);
    assert_eq!(header["exit_code"], Value::Null);
    assert_eq!(header["lost"], false);
    assert!(
        header["note"]
            .as_str()
            .unwrap()
            .contains("may still be running"),
        "{header}"
    );
    // A signal; a lost connection.
    fx.host.set_exec(Ok(exec_output(
        b"",
        b"",
        ExecEnd::Signal("KILL".to_owned()),
    )));
    let header = head(run(&fx.ssh, json!({"command": "x"})).await);
    assert_eq!(
        (header["signal"].clone(), header["exit_code"].clone()),
        (json!("KILL"), Value::Null)
    );
    fx.host
        .set_exec(Ok(exec_output(b"part", b"", ExecEnd::Lost)));
    let blocks = ok(run(&fx.ssh, json!({"command": "x"})).await);
    let header: Value = serde_json::from_str(&blocks[0]).unwrap();
    assert_eq!(header["lost"], true);
    assert_eq!(header["timed_out"], false);
    assert!(
        header["note"].as_str().unwrap().contains("lost"),
        "{header}"
    );
    assert_eq!(blocks[1], "stdout:\npart");
    // Output that is not UTF-8 is decoded lossily.
    fx.host
        .set_exec(Ok(exec_output(&[0xff, b'a'], b"", ExecEnd::Exit(0))));
    assert_eq!(
        ok(run(&fx.ssh, json!({"command": "x"})).await)[1],
        "stdout:\n\u{fffd}a"
    );
    // A command that could not start: a clear error.
    fx.host.set_exec(Err(Error::new(
        ErrorKind::Protocol,
        "Server refused the command",
    )));
    let text = err(run(&fx.ssh, json!({"command": "x"})).await);
    assert!(text.contains("Server refused the command"), "{text}");
    assert!(text.contains("\"Acme\""), "{text}");

    // On the FTP site the host is never called.
    let runs = fx.host.execs().len();
    assert_eq!(
        err(run(&fx.ftp, json!({"command": "ls"})).await),
        ssh_only(&fx.ftp.name)
    );
    assert_eq!(fx.host.execs().len(), runs);
    assert_eq!(fx.host.connects(&fx.ftp.id), 0);
}

#[tokio::test]
async fn commands_run_one_at_a_time_per_site() {
    let fx = fixture();
    fx.set_level(&fx.ro, AgentAccess::All);
    let spawn = |site: &Site, command: &str| {
        let service = fx.service.clone();
        let args = on(site, json!({"command": command}));
        tokio::spawn(async move { service.call("run_command", args).await })
    };
    let running = |site: &Site| {
        fx.service
            .activity()
            .running
            .get(&site.id)
            .copied()
            .unwrap_or(0)
    };

    let hold = fx.host.hold_exec();
    let first = spawn(&fx.ssh, "one");
    hold.entered().await;
    let second = spawn(&fx.ssh, "two");
    // The second is in flight (the reaper leaves the site alone) but waits its turn.
    until(|| running(&fx.ssh) == 2).await;
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert_eq!(fx.host.execs().len(), 1, "the second waits for the first");
    // Another site does not wait.
    let other = spawn(&fx.ro, "three");
    until(|| fx.host.execs().len() == 2).await;
    let commands: Vec<String> = fx.host.execs().into_iter().map(|e| e.1).collect();
    assert_eq!(commands, ["one", "three"]);
    assert!(!first.is_finished() && !second.is_finished() && !other.is_finished());

    hold.release();
    ok(first.await.unwrap());
    ok(second.await.unwrap());
    ok(other.await.unwrap());
    let commands: Vec<String> = fx.host.execs().into_iter().map(|e| e.1).collect();
    assert_eq!(commands, ["one", "three", "two"]);
    assert!(fx.service.activity().running.is_empty());
    assert!(fx.service.commands().is_empty(), "turns are not kept");
}

#[tokio::test]
async fn write_tools_log_what_they_did() {
    let (logs, _guard) = capture_logs();
    let fx = fixture();
    let call = |tool: &'static str, more: Value| {
        let service = fx.service.clone();
        let args = on(&fx.ssh, more);
        async move { service.call(tool, args).await }
    };
    let last = || logs.agent_lines().last().cloned().unwrap();

    ok(call(
        "write_file",
        json!({"path": "/a.txt", "content": "hello world!"}),
    )
    .await);
    assert!(
        last().ends_with("wrote /a.txt (12 bytes) on \"Acme\""),
        "{}",
        last()
    );
    fx.host.set_exec(Ok(exec_output(
        b"SECRET-OUTPUT",
        b"SECRET-ERR",
        ExecEnd::Exit(0),
    )));
    ok(call("run_command", json!({"command": "uptime"})).await);
    assert!(
        last().ends_with("ran `uptime` on \"Acme\" (exit 0)"),
        "{}",
        last()
    );
    ok(call("make_directory", json!({"path": "/d"})).await);
    assert!(
        last().ends_with("created the folder /d on \"Acme\""),
        "{}",
        last()
    );
    ok(call("rename", json!({"from": "/a.txt", "to": "/d/b.txt"})).await);
    assert!(
        last().ends_with("renamed /a.txt to /d/b.txt on \"Acme\""),
        "{}",
        last()
    );
    ok(call(
        "set_permissions",
        json!({"path": "/d/b.txt", "mode": "600"}),
    )
    .await);
    assert!(
        last().ends_with("changed the permissions of /d/b.txt to 0600 on \"Acme\""),
        "{}",
        last()
    );
    ok(call("delete", json!({"path": "/d", "recursive": true})).await);
    assert!(
        last().ends_with("deleted /d on \"Acme\" (2 items)"),
        "{}",
        last()
    );
    err(call(
        "write_file",
        json!({"path": "/var/www/a.txt", "content": "x"}),
    )
    .await);
    assert!(
        last().contains("wrote /var/www/a.txt (1 byte) on \"Acme\" (failed: "),
        "{}",
        last()
    );

    for (end, says) in [
        (ExecEnd::TimedOut, "(timed out)"),
        (ExecEnd::Signal("TERM".to_owned()), "(signal TERM)"),
        (ExecEnd::Lost, "(connection lost)"),
    ] {
        fx.host.set_exec(Ok(exec_output(b"", b"", end)));
        ok(call("run_command", json!({"command": "x"})).await);
        assert!(
            last().ends_with(&format!("ran `x` on \"Acme\" {says}")),
            "{}",
            last()
        );
    }

    // A command cannot forge a line.
    let before = logs.text().lines().count();
    ok(call(
        "run_command",
        json!({"command": "uptime\nINFO agent: forged\r"}),
    )
    .await);
    assert_eq!(logs.text().lines().count(), before + 1);
    assert!(
        last().contains("ran `uptime INFO agent: forged ` on"),
        "{}",
        last()
    );

    // A cancelled command is on record.
    let hold = fx.host.hold_exec();
    let running = tokio::spawn({
        let service = fx.service.clone();
        let args = on(&fx.ssh, json!({"command": "sleep 100"}));
        async move { service.call("run_command", args).await }
    });
    hold.entered().await;
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());
    assert!(hold.dropped());
    assert!(
        last().ends_with("ran `sleep 100` on \"Acme\" (cancelled)"),
        "{}",
        last()
    );
    assert!(fx.service.activity().running.is_empty());

    let log = logs.text();
    for secret in ["hello world!", "SECRET-OUTPUT", "SECRET-ERR"] {
        assert!(!log.contains(secret), "{log}");
    }
}

/// A `..` component is refused in every path argument, reading and writing alike: the
/// server would resolve it through links, the log by its text.
#[tokio::test]
async fn dot_dot_paths_are_refused() {
    let fx = fixture();
    fx.ssh_fs.add_file("/var/x.txt", b"x", EPOCH);
    let call = |tool: &'static str, more: Value| {
        let service = fx.service.clone();
        let args = on(&fx.ssh, more);
        async move { service.call(tool, args).await }
    };
    let text = err(call("read_file", json!({"path": "/var/www/../x.txt"})).await);
    assert!(text.contains("`path`") && text.contains("`..`"), "{text}");
    for recursive in [false, true] {
        let text = err(call(
            "delete",
            json!({"path": "/var/www/..", "recursive": recursive}),
        )
        .await);
        assert!(text.contains("`..`"), "{text}");
    }
    let text = err(call(
        "rename",
        json!({"from": "/var/www/../x.txt", "to": "/var/y.txt"}),
    )
    .await);
    assert!(text.contains("`from`") && text.contains("`..`"), "{text}");
    let text = err(call(
        "rename",
        json!({"from": "/var/x.txt", "to": "/var/www/../y.txt"}),
    )
    .await);
    assert!(text.contains("`to`") && text.contains("`..`"), "{text}");
    // Nothing reached the server, and everything is still there.
    for op in [
        FsOp::Read,
        FsOp::Stat,
        FsOp::List,
        FsOp::Remove,
        FsOp::Rename,
    ] {
        assert_eq!(fx.host.calls(&fx.ssh.id, op), 0, "{op:?}");
    }
    assert_eq!(fx.host.connects(&fx.ssh.id), 0);
    assert_eq!(children(&fx.ssh_fs, "/var").await, ["www", "x.txt"]);
    // A name that only contains dots is a name.
    fx.ssh_fs.add_file("/var/www/..a", b"dots", EPOCH);
    assert_eq!(
        ok(call("read_file", json!({"path": "/var/www/..a"})).await)[1],
        "dots"
    );
}

/// Without `recursive` a folder is only removed as an empty folder: one filled between
/// the check and the removal fails, and what was put in it stays.
#[tokio::test]
async fn delete_without_recursive_never_walks() {
    let fx = fixture();
    fx.ssh_fs.add_dir("/srv/empty");
    let hold = fx.host.hold(&fx.ssh.id, FsOp::Remove);
    let running = tokio::spawn({
        let service = fx.service.clone();
        let args = on(&fx.ssh, json!({"path": "/srv/empty"}));
        async move { service.call("delete", args).await }
    });
    hold.entered().await;
    fx.ssh_fs.add_file("/srv/empty/new.txt", b"new", EPOCH);
    hold.release();
    let text = err(running.await.unwrap());
    assert!(text.contains("Nothing was deleted"), "{text}");
    assert_eq!(
        fx.ssh_fs.read_file("/srv/empty/new.txt").as_deref(),
        Some(&b"new"[..])
    );
    assert_eq!(fx.host.calls(&fx.ssh.id, FsOp::Remove), 1, "one remove_dir");
    // The check's listing only: no walk lists the folder again (a walk would remove
    // whatever its own listing found).
    assert_eq!(fx.host.calls(&fx.ssh.id, FsOp::List), 1);
}

/// A command waiting its turn that is cancelled leaves the queue, logged as cancelled;
/// the one behind it still runs.
#[tokio::test]
async fn a_cancelled_waiting_command_leaves_the_queue() {
    let (logs, _guard) = capture_logs();
    let fx = fixture();
    let spawn = |command: &str| {
        let service = fx.service.clone();
        let args = on(&fx.ssh, json!({"command": command}));
        tokio::spawn(async move { service.call("run_command", args).await })
    };
    let running = || {
        fx.service
            .activity()
            .running
            .get(&fx.ssh.id)
            .copied()
            .unwrap_or(0)
    };
    let hold = fx.host.hold_exec();
    let first = spawn("one");
    hold.entered().await;
    let waiting = spawn("two");
    let third = spawn("three");
    until(|| running() == 3).await;
    waiting.abort();
    assert!(waiting.await.unwrap_err().is_cancelled());
    assert_eq!(running(), 2);
    assert!(
        logs.agent_lines()
            .iter()
            .any(|l| l.ends_with("ran `two` on \"Acme\" (cancelled)")),
        "{:?}",
        logs.agent_lines()
    );
    hold.release();
    ok(first.await.unwrap());
    ok(third.await.unwrap());
    let commands: Vec<String> = fx.host.execs().into_iter().map(|e| e.1).collect();
    assert_eq!(commands, ["one", "three"]);
    assert!(fx.service.commands().is_empty());
}

/// An exec error of network kind says the connection may have been lost.
#[tokio::test]
async fn a_network_exec_error_says_the_connection_may_be_lost() {
    let fx = fixture();
    fx.host.set_exec(Err(Error::new(
        ErrorKind::Network,
        "Timed out opening a channel for the command",
    )));
    let text = err(fx
        .call("run_command", on(&fx.ssh, json!({"command": "uptime"})))
        .await);
    assert!(
        text.contains("Timed out opening a channel for the command"),
        "{text}"
    );
    assert!(text.contains("connection may have been lost"), "{text}");
}

/// A command of megabytes is cut in the log line, which keeps its outcome.
#[tokio::test]
async fn a_long_command_keeps_its_outcome_in_the_log() {
    let (logs, _guard) = capture_logs();
    let fx = fixture();
    fx.host
        .set_exec(Ok(exec_output(b"", b"", ExecEnd::Exit(7))));
    let command = format!("echo {}", "y".repeat(3_000_000));
    ok(fx
        .call("run_command", on(&fx.ssh, json!({"command": command})))
        .await);
    let lines = logs.agent_lines();
    let line = lines.last().unwrap();
    assert!(
        line.ends_with("on \"Acme\" (exit 7)"),
        "{}",
        &line[line.len() - 80..]
    );
    assert!(line.contains("… (2999505 more bytes)"), "{}", &line[..600]);
    assert!(line.len() < 1024, "{}", line.len());
    // The host got the whole command.
    assert_eq!(fx.host.execs().last().unwrap().1, command);
}

// ---- Task 11: transfers ----

/// A tempdir the engine's local side can name: on Windows the temporary folder may be
/// on another drive than the one `LocalFs::new("/")` reaches.
fn local_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    if engine_path(dir.path()).is_ok() {
        return dir;
    }
    tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()
}

/// `rel` (written with `/`) under `dir`, built with `Path::join`.
fn at(dir: &tempfile::TempDir, rel: &str) -> PathBuf {
    rel.split('/')
        .filter(|s| !s.is_empty())
        .fold(dir.path().to_path_buf(), |path, comp| path.join(comp))
}

fn text_of(path: &std::path::Path) -> String {
    path.to_str().expect("a Unicode path").to_owned()
}

/// `conf/hoprs/sites.toml`, `home/.ssh/known_hosts`, `keys/deploy.pem`,
/// `keys/other.pem` and `work/n.txt`, with `conf/hoprs`, `home/.ssh` and
/// `keys/deploy.pem` protected in the fake.
fn protected_tree(fx: &Fx) -> tempfile::TempDir {
    let dir = local_dir();
    for folder in ["conf/hoprs", "home/.ssh", "keys", "work"] {
        std::fs::create_dir_all(at(&dir, folder)).unwrap();
    }
    for file in [
        "conf/hoprs/sites.toml",
        "home/.ssh/known_hosts",
        "keys/deploy.pem",
        "keys/other.pem",
        "work/n.txt",
    ] {
        std::fs::write(at(&dir, file), b"x").unwrap();
    }
    fx.host.set_protected(vec![
        at(&dir, "conf/hoprs"),
        at(&dir, "home/.ssh"),
        at(&dir, "keys/deploy.pem"),
    ]);
    dir
}

fn download_args(site: &Site, remote: &str, local: &std::path::Path) -> Value {
    on(
        site,
        json!({"remote_path": remote, "local_path": text_of(local)}),
    )
}

fn upload_args(site: &Site, local: &std::path::Path, remote: &str) -> Value {
    on(
        site,
        json!({"local_path": text_of(local), "remote_path": remote}),
    )
}

/// The call, running on its own task.
fn spawn_call(fx: &Fx, tool: &'static str, args: Value) -> tokio::task::JoinHandle<Reply> {
    let service = fx.service.clone();
    tokio::spawn(async move { service.call(tool, args).await })
}

/// The task's reply, within 30 s (of paused time too): a call that waits longer fails
/// its test instead of hanging it.
async fn joined(task: tokio::task::JoinHandle<Reply>) -> Reply {
    tokio::time::timeout(Duration::from_secs(30), task)
        .await
        .expect("the call answers within 30 s")
        .unwrap()
}

/// Waits (1 ms steps, at most 10 s) until `n` transfers are enqueued.
async fn until_enqueued(fx: &Fx, n: usize) {
    for _ in 0..10_000 {
        if fx.host.enqueued().len() >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("{n} transfers were never enqueued");
}

/// The fake's item `id` became a file of `bytes` bytes, done.
async fn finish_file(fx: &Fx, id: TransferId, bytes: u64) {
    let mut items = fx.host.transfers().await;
    let item = items.iter_mut().find(|item| item.id == id).unwrap();
    item.is_dir = Some(false);
    item.size = Some(bytes);
    item.bytes_done = bytes;
    fx.host.set_transfers(items);
    fx.host.set_state(id, TransferState::Done);
}

/// Both tools enqueue one item of origin Agent (the host's `enqueue`) under Skip
/// without `overwrite` and Overwrite with it, never the user's default; the spec's local
/// side is `engine_path` of the argument.
#[tokio::test(start_paused = true)]
async fn download_and_upload_enqueue_with_their_policy() {
    let fx = fixture();
    let dir = local_dir();
    let landing = at(&dir, "a.txt");
    let source = at(&dir, "up.txt");
    std::fs::write(&source, b"up").unwrap();
    let mut args = Vec::new();
    for overwrite in [None, Some(false), Some(true)] {
        let mut down = download_args(&fx.ro, "/var/www/a.txt", &landing);
        let mut up = upload_args(&fx.ssh, &source, "/var/www/up.txt");
        if let Some(overwrite) = overwrite {
            down["overwrite"] = json!(overwrite);
            up["overwrite"] = json!(overwrite);
        }
        args.push(("download", down));
        args.push(("upload", up));
    }
    for (n, (tool, args)) in args.into_iter().enumerate() {
        let reply = head(fx.call(tool, args).await);
        assert_eq!(reply["transfer"], json!(n + 1), "{reply}");
        assert_eq!(reply["state"], "running", "{reply}");
    }
    let download = TransferSpec {
        site: fx.ro.id.clone(),
        direction: Direction::Download,
        local: engine_path(&landing).unwrap(),
        remote: RemotePath::new("/var/www/a.txt").unwrap(),
    };
    let upload = TransferSpec {
        site: fx.ssh.id.clone(),
        direction: Direction::Upload,
        local: engine_path(&source).unwrap(),
        remote: RemotePath::new("/var/www/up.txt").unwrap(),
    };
    use hoprs_transfer::ConflictPolicy::{Overwrite, Skip};
    assert_eq!(
        fx.host.enqueued(),
        [
            (download.clone(), Skip),
            (upload.clone(), Skip),
            (download.clone(), Skip),
            (upload.clone(), Skip),
            (download, Overwrite),
            (upload, Overwrite),
        ]
    );
    // The upload connected its site first: an item for a site that is not connected
    // would stay queued.
    assert_eq!(fx.host.connects(&fx.ssh.id), 1);
}

#[tokio::test(start_paused = true)]
async fn upload_needs_everything_download_needs_reading() {
    let fx = fixture();
    let dir = local_dir();
    let file = at(&dir, "a.txt");
    std::fs::write(&file, b"a").unwrap();
    assert_eq!(
        err(fx
            .call("upload", upload_args(&fx.ro, &file, "/var/www/b.txt"))
            .await),
        read_only(&fx.ro.name)
    );
    for tool in ["download", "upload"] {
        let args = match tool {
            "download" => download_args(&fx.closed, "/var/www/a.txt", &file),
            _ => upload_args(&fx.closed, &file, "/var/www/b.txt"),
        };
        assert_eq!(err(fx.call(tool, args).await), SITE_UNAVAILABLE, "{tool}");
    }
    assert!(fx.host.enqueued().is_empty());
    let landing = at(&dir, "b.txt");
    let reply = head(
        fx.call(
            "download",
            download_args(&fx.ro, "/var/www/a.txt", &landing),
        )
        .await,
    );
    assert_eq!(reply["state"], "running");
    assert_eq!(fx.host.enqueued().len(), 1);
    assert_eq!(fx.host.connects(&fx.closed.id), 0);
}

/// `protected(path)` for the path as given, and nothing enqueued.
async fn refused_as_protected(fx: &Fx, tool: &str, args: Value, given: &std::path::Path) {
    let before = fx.host.enqueued().len();
    let text = err(fx.call(tool, args).await);
    assert_eq!(text, protected(&text_of(given)), "{tool} {given:?}");
    assert_eq!(fx.host.enqueued().len(), before, "{tool} {given:?}");
}

/// Each protected root, a path inside it and, for a folder, a folder that holds it: one
/// refusal naming only the path given, before anything is enqueued. A neighbour is
/// enqueued.
#[tokio::test(start_paused = true)]
async fn protected_paths_stop_the_transfer() {
    let fx = fixture();
    let dir = protected_tree(&fx);
    fx.ssh_fs.add_file("/var/www/sub/b.txt", b"b", EPOCH);
    // Downloads of a file: the root itself, inside a root (existing or not).
    for rel in [
        "keys/deploy.pem",
        "home/.ssh",
        "home/.ssh/known_hosts",
        "conf/hoprs/sites.toml",
        "conf/hoprs/new/deep.txt",
    ] {
        let local = at(&dir, rel);
        let args = download_args(&fx.ssh, "/var/www/a.txt", &local);
        refused_as_protected(&fx, "download", args, &local).await;
    }
    // Downloads of a folder: also a folder that holds a root.
    for rel in ["home", "conf", "keys", "", "home/.ssh/sub"] {
        let local = at(&dir, rel);
        let args = download_args(&fx.ssh, "/var/www", &local);
        refused_as_protected(&fx, "download", args, &local).await;
    }
    // Uploads of a file, of a root folder, of a folder that holds one.
    for rel in [
        "keys/deploy.pem",
        "home/.ssh/known_hosts",
        "conf/hoprs/sites.toml",
        "home/.ssh",
        "home",
        "conf",
        "keys",
        "",
    ] {
        let local = at(&dir, rel);
        let args = upload_args(&fx.ssh, &local, "/var/www/x");
        refused_as_protected(&fx, "upload", args, &local).await;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(at(&dir, "home/.ssh"), at(&dir, "work/ssh-link")).unwrap();
        symlink(at(&dir, "keys/deploy.pem"), at(&dir, "work/key-link")).unwrap();
        symlink(at(&dir, "conf"), at(&dir, "work/conf-link")).unwrap();
        for rel in ["work/ssh-link/x.txt", "work/key-link", "work/ssh-link"] {
            let local = at(&dir, rel);
            let args = download_args(&fx.ssh, "/var/www/a.txt", &local);
            refused_as_protected(&fx, "download", args, &local).await;
        }
        // A link as an upload's source is judged by where it leads.
        for rel in ["work/ssh-link", "work/key-link", "work/conf-link"] {
            let local = at(&dir, rel);
            let args = upload_args(&fx.ssh, &local, "/var/www/x");
            refused_as_protected(&fx, "upload", args, &local).await;
        }
    }
    assert!(fx.host.enqueued().is_empty());
    // Neighbours go through.
    let neighbours = [
        (
            "download",
            download_args(&fx.ssh, "/var/www/a.txt", &at(&dir, "work/new.txt")),
        ),
        (
            "download",
            download_args(&fx.ssh, "/var/www", &at(&dir, "work/site")),
        ),
        (
            "download",
            download_args(&fx.ssh, "/var/www/a.txt", &at(&dir, "keys/deploy.pem.bak")),
        ),
        (
            "upload",
            upload_args(&fx.ssh, &at(&dir, "keys/other.pem"), "/var/www/o"),
        ),
        (
            "upload",
            upload_args(&fx.ssh, &at(&dir, "work"), "/var/www/work"),
        ),
    ];
    let count = neighbours.len();
    for (tool, args) in neighbours {
        let reply = head(fx.call(tool, args).await);
        assert_eq!(reply["state"], "running", "{reply}");
    }
    assert_eq!(fx.host.enqueued().len(), count);
}

/// What can be refused before connecting is: an upload from a protected path, a download
/// into one, never connect the site. A folder download needs the server to tell it is a
/// folder first, and is still refused before anything is enqueued.
#[tokio::test]
async fn protected_is_checked_before_connecting_when_it_can_be() {
    let fx = fixture();
    let dir = protected_tree(&fx);
    let key = at(&dir, "keys/deploy.pem");
    refused_as_protected(&fx, "upload", upload_args(&fx.ssh, &key, "/k"), &key).await;
    let inside = at(&dir, "home/.ssh/known_hosts");
    let args = download_args(&fx.ssh, "/var/www/a.txt", &inside);
    refused_as_protected(&fx, "download", args, &inside).await;
    assert_eq!(fx.host.connects(&fx.ssh.id), 0);
    assert_eq!(fx.host.calls(&fx.ssh.id, FsOp::Stat), 0);
    let home = at(&dir, "home");
    refused_as_protected(
        &fx,
        "download",
        download_args(&fx.ssh, "/var/www", &home),
        &home,
    )
    .await;
    assert_eq!(fx.host.connects(&fx.ssh.id), 1);
    assert!(fx.host.enqueued().is_empty());
}

/// The local path's own rules: `engine_path`'s sentence; an upload's source must exist
/// and be a regular file or a folder, not a link; a download's remote path must not be a
/// link. Nothing is enqueued, and an upload refused here never connects.
#[tokio::test]
async fn local_path_rules() {
    let fx = fixture();
    let dir = local_dir();
    std::fs::write(at(&dir, "n.txt"), b"n").unwrap();
    let relative = std::path::Path::new("a.txt");
    let dotted = at(&dir, "work").join("..").join("n.txt");
    for local in [relative, dotted.as_path()] {
        let sentence = engine_path(local).unwrap_err();
        let down = download_args(&fx.ssh, "/var/www/a.txt", local);
        assert_eq!(err(fx.call("download", down).await), sentence, "{local:?}");
        let up = upload_args(&fx.ssh, local, "/var/www/b.txt");
        assert_eq!(err(fx.call("upload", up).await), sentence, "{local:?}");
    }
    let missing = at(&dir, "nothing.txt");
    let text = err(fx
        .call("upload", upload_args(&fx.ssh, &missing, "/var/www/b.txt"))
        .await);
    assert!(text.contains(&text_of(&missing)), "{text}");
    assert!(text.contains("does not exist"), "{text}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(at(&dir, "n.txt"), at(&dir, "link.txt")).unwrap();
        let link = at(&dir, "link.txt");
        let text = err(fx
            .call("upload", upload_args(&fx.ssh, &link, "/var/www/b.txt"))
            .await);
        assert!(text.contains("symbolic link"), "{text}");
        assert!(text.contains(&text_of(&link)), "{text}");
        // A socket and a device would block or read without end.
        let socket = at(&dir, "s.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        for odd in [socket.as_path(), std::path::Path::new("/dev/null")] {
            let text = err(fx
                .call("upload", upload_args(&fx.ssh, odd, "/var/www/b.txt"))
                .await);
            assert!(text.contains("not a regular file or a folder"), "{text}");
        }
    }
    #[cfg(windows)]
    for name in ["CON", "nul.txt", "COM1"] {
        let device = at(&dir, name);
        let text = err(fx
            .call("upload", upload_args(&fx.ssh, &device, "/var/www/b.txt"))
            .await);
        assert!(text.contains("not a regular file or a folder"), "{text}");
    }
    assert_eq!(fx.host.connects(&fx.ssh.id), 0);
    // On the server: a link, and nothing.
    fx.ssh_fs.add_symlink("/var/www/l", "/var/www/a.txt");
    let landing = at(&dir, "l.txt");
    let text = err(fx
        .call("download", download_args(&fx.ssh, "/var/www/l", &landing))
        .await);
    assert!(text.contains("/var/www/l is a symbolic link"), "{text}");
    let text = err(fx
        .call(
            "download",
            download_args(&fx.ssh, "/var/www/nope", &landing),
        )
        .await);
    assert!(text.starts_with("Could not read /var/www/nope: "), "{text}");
    assert!(fx.host.enqueued().is_empty());
}

/// The fake ends the root: the call answers its summary without waiting the minute.
#[tokio::test(start_paused = true)]
async fn finished_transfer_answers_its_summary() {
    let fx = fixture();
    let dir = local_dir();
    let started = Instant::now();
    let task = spawn_call(
        &fx,
        "download",
        download_args(&fx.ssh, "/var/www/a.txt", &at(&dir, "a.txt")),
    );
    until_enqueued(&fx, 1).await;
    finish_file(&fx, TransferId(1), 5).await;
    let reply = head(joined(task).await);
    assert_eq!(
        reply,
        json!({"transfer": 1, "state": "done", "files_done": 1, "bytes_done": 5,
            "skipped": 0, "failed_count": 0, "failed": []})
    );
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test(start_paused = true)]
async fn skipped_single_file_says_skipped() {
    let fx = fixture();
    let dir = local_dir();
    let task = spawn_call(
        &fx,
        "download",
        download_args(&fx.ssh, "/var/www/a.txt", &at(&dir, "a.txt")),
    );
    until_enqueued(&fx, 1).await;
    fx.host
        .set_state(TransferId(1), TransferState::Skipped("exists".to_owned()));
    let reply = head(joined(task).await);
    assert_eq!(reply["state"], "skipped", "{reply}");
    assert_eq!(reply["skipped"], 1, "{reply}");
    assert_eq!(reply["files_done"], 0, "{reply}");
    assert_eq!(reply["failed_count"], 0, "{reply}");
}

/// RF4: no end within a minute: the call answers the id and "running" (pending until
/// then), and `transfer_status` answers the summary later.
#[tokio::test(start_paused = true)]
async fn slow_transfer_answers_running() {
    let fx = fixture();
    let dir = local_dir();
    let source = at(&dir, "big.bin");
    std::fs::write(&source, b"big").unwrap();
    let task = spawn_call(
        &fx,
        "upload",
        upload_args(&fx.ssh, &source, "/var/www/big.bin"),
    );
    until_enqueued(&fx, 1).await;
    let enqueued = Instant::now();
    tokio::time::advance(Duration::from_secs(58)).await;
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert!(!task.is_finished(), "pending before the minute");
    // While it waits, the call counts as running on its site.
    assert!(fx.service.activity().running.contains_key(&fx.ssh.id));
    let reply = head(joined(task).await);
    assert!(enqueued.elapsed() >= Duration::from_secs(59));
    assert_eq!(reply["transfer"], 1, "{reply}");
    assert_eq!(reply["state"], "running", "{reply}");
    assert!(
        reply["note"].as_str().unwrap().contains("transfer_status"),
        "{reply}"
    );
    assert!(fx.service.activity().running.is_empty());

    finish_file(&fx, TransferId(1), 3).await;
    let status = head(fx.call("transfer_status", json!({"transfer": 1})).await);
    assert_eq!(status["state"], "done", "{status}");
    assert_eq!(status["files_done"], 1, "{status}");
    assert_eq!(status["bytes_done"], 3, "{status}");
}

/// A user's transfer, a child, an unknown id, an id past `i64` and an agent's transfer
/// on a site no longer open to agents: one text, one log line.
#[tokio::test]
async fn transfer_status_is_for_agent_transfers_only() {
    let (logs, _guard) = capture_logs();
    let fx = fixture();
    let item = |id: i64, parent: Option<i64>, site: &Site, origin: Origin| TransferItem {
        id: TransferId(id),
        parent: parent.map(TransferId),
        origin,
        ..transfer(&site.id, TransferState::Done)
    };
    fx.host.set_transfers(vec![
        item(10, None, &fx.ssh, Origin::User),
        item(11, None, &fx.ssh, Origin::Agent),
        item(12, Some(11), &fx.ssh, Origin::Agent),
        item(13, None, &fx.ro, Origin::Agent),
        item(14, None, &fx.closed, Origin::Agent),
    ]);
    let status = |id: u64| fx.call("transfer_status", json!({"transfer": id}));
    assert_eq!(head(status(11).await)["state"], "done");
    assert_eq!(head(status(13).await)["state"], "done");
    fx.set_level(&fx.ro, AgentAccess::None);
    let before = logs.agent_lines().len();
    for id in [10, 12, 13, 14, 99, 1 << 63] {
        assert_eq!(err(status(id).await), NO_SUCH_TRANSFER, "{id}");
    }
    let lines = logs.agent_lines();
    assert_eq!(lines.len(), before + 6, "{lines:?}");
    for line in &lines[before..] {
        assert!(
            line.ends_with("refused transfer_status: no such agent transfer"),
            "{line}"
        );
    }
    assert!(
        lines[0].ends_with("checked transfer 11 on \"Acme\" (done)"),
        "{lines:?}"
    );
}

/// A root waiting on a decision ends the wait: "paused", now.
#[tokio::test(start_paused = true)]
async fn agent_never_waits_on_a_question() {
    let fx = fixture();
    let dir = local_dir();
    let task = spawn_call(
        &fx,
        "download",
        download_args(&fx.ssh, "/var/www/a.txt", &at(&dir, "a.txt")),
    );
    until_enqueued(&fx, 1).await;
    let asked = Instant::now();
    fx.host.set_state(
        TransferId(1),
        TransferState::AwaitingDecision(hoprs_transfer::ConflictInfo {
            src_size: 5,
            dst_size: 3,
            src_modified: None,
            dst_modified: None,
        }),
    );
    let reply = head(joined(task).await);
    assert!(asked.elapsed() < Duration::from_secs(1));
    assert_eq!(reply["state"], "paused", "{reply}");
    assert!(
        reply["note"].as_str().unwrap().contains("hoprs window"),
        "{reply}"
    );
    let status = head(fx.call("transfer_status", json!({"transfer": 1})).await);
    assert_eq!(status["state"], "paused", "{status}");
}

/// One line per transfer call: the action, both paths, the site and the outcome; a
/// cancelled call is logged and its transfer stays in the queue.
#[tokio::test(start_paused = true)]
async fn transfer_calls_are_logged() {
    let (logs, _guard) = capture_logs();
    let fx = fixture();
    let dir = protected_tree(&fx);
    let landing = at(&dir, "work/a.txt");
    let task = spawn_call(
        &fx,
        "download",
        download_args(&fx.ssh, "/var/www/a.txt", &landing),
    );
    until_enqueued(&fx, 1).await;
    finish_file(&fx, TransferId(1), 5).await;
    ok(joined(task).await);
    let source = at(&dir, "work/n.txt");
    ok(fx
        .call("upload", upload_args(&fx.ssh, &source, "/var/www/n.txt"))
        .await);
    let key = at(&dir, "keys/deploy.pem");
    err(fx
        .call("upload", upload_args(&fx.ssh, &key, "/var/www/k"))
        .await);
    let lines = logs.agent_lines();
    assert_eq!(lines.len(), 3, "{lines:?}");
    let want = [
        format!(
            "downloaded /var/www/a.txt to {} on \"Acme\" (transfer 1: done)",
            text_of(&landing)
        ),
        format!(
            "uploaded {} to /var/www/n.txt on \"Acme\" (transfer 2: running)",
            text_of(&source)
        ),
        format!(
            "uploaded {} to /var/www/k on \"Acme\" (failed: protected local path)",
            text_of(&key)
        ),
    ];
    for (line, want) in lines.iter().zip(want) {
        assert!(line.ends_with(&want), "{line}\n{want}");
    }

    let task = spawn_call(
        &fx,
        "upload",
        upload_args(&fx.ssh, &source, "/var/www/m.txt"),
    );
    until_enqueued(&fx, 3).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let lines = logs.agent_lines();
    assert!(
        lines[3].ends_with(&format!(
            "uploaded {} to /var/www/m.txt on \"Acme\" (cancelled)",
            text_of(&source)
        )),
        "{lines:?}"
    );
    assert!(fx.service.activity().running.is_empty());
    // The transfer is the queue's: still there, still queued.
    let items = fx.host.transfers().await;
    let item = items.iter().find(|item| item.id == TransferId(3)).unwrap();
    assert_eq!(item.state, TransferState::Queued);
}
