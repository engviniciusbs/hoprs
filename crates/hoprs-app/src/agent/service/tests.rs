use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use hoprs_core::{AgentAccess, Error, ErrorKind, Protocol, RemotePath, Site, SiteId};
use hoprs_remote::MemFs;
use hoprs_transfer::{Direction, Origin, TransferId, TransferItem, TransferSpec, TransferState};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::*;
use crate::agent::testing::{FakeHost, FsOp};
use crate::agent::tools::{self, Need};
use crate::agent::{
    HOST_KEY_CHANGED, SITE_UNAVAILABLE, TURNED_OFF, WAITING_FOR_USER, read_only, ssh_only,
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
    host.add_site(ftp.clone());
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
#[tokio::test]
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
    for op in [FsOp::List, FsOp::Stat, FsOp::Read] {
        assert_eq!(fx.host.calls(&fx.closed.id, op), 0);
    }
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
        .disconnect_user(&fx.ftp.id, Error::new(ErrorKind::Network, "gone"));
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
    fx.host.connect_user(&tab.id);
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
