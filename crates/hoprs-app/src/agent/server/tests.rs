//! The server over a [`FakeHost`], its channel in a temporary directory per test, and
//! clients that speak `proto` through `channel::connect`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use hoprs_core::{AgentAccess, Protocol, Site};
use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, BufReader, ReadHalf, WriteHalf};
use tokio::sync::{mpsc, watch};

use super::*;
use crate::agent::channel::{self, Endpoint};
use crate::agent::testing::{FakeHost, FsOp, STEP, step};
use crate::agent::{AgentEvent, HELLO_WAIT, Limits, MAX_RELAYS, REAP_EVERY};

/// How long a test waits for something to happen (a channel to open, an event, a
/// condition); one I/O step has [`crate::agent::testing::STEP`].
const WAIT: Duration = Duration::from_secs(10);
const VERSION: &str = env!("CARGO_PKG_VERSION");
const FILE: &str = "/var/www/a.txt";

struct Fx {
    host: Arc<FakeHost>,
    site: Site,
    endpoint: Endpoint,
    switch: watch::Sender<bool>,
    events: mpsc::UnboundedReceiver<AgentEvent>,
    server: AgentServer,
    _dir: tempfile::TempDir,
}

fn fixture(on: bool) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let endpoint = channel::endpoint_in(dir.path());
    fixture_at(dir, Some(endpoint), on)
}

fn fixture_at(dir: tempfile::TempDir, endpoint: Option<Endpoint>, on: bool) -> Fx {
    let host = FakeHost::new();
    let mut site = Site::new("Acme", "acme.example", "u", Protocol::Ssh);
    site.agent_access = AgentAccess::Read;
    let fs = host.add_site(site.clone());
    fs.add_file(FILE, b"hello", SystemTime::UNIX_EPOCH);
    let (tx, events) = mpsc::unbounded_channel();
    let service = Service::new(host.clone(), Limits::default(), tx.clone());
    let (switch, watching) = watch::channel(on);
    let server = AgentServer::start(service, endpoint.clone(), watching, tx);
    Fx {
        host,
        site,
        endpoint: endpoint.unwrap_or_else(|| channel::endpoint_in(dir.path())),
        switch,
        events,
        server,
        _dir: dir,
    }
}

/// One relay's end of the channel. Every write and read is a [`step`] named after the
/// client and the line's number, so a stall says where it stopped.
struct Client {
    name: String,
    sent: u64,
    received: u64,
    reader: BufReader<ReadHalf<channel::Stream>>,
    writer: WriteHalf<channel::Stream>,
}

/// Numbers the clients of the tests, for their names.
static CLIENTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);

impl Client {
    /// Connects, waiting for the server to open the channel.
    async fn connect(endpoint: &Endpoint) -> Client {
        let n = CLIENTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Client::named(endpoint, &format!("client {n}")).await
    }

    async fn named(endpoint: &Endpoint, name: &str) -> Client {
        let start = std::time::Instant::now();
        let stream = loop {
            match step(format!("{name}: connecting"), channel::connect(endpoint)).await {
                Ok(stream) => break stream,
                Err(err) => {
                    assert!(
                        start.elapsed() < WAIT,
                        "{name}: the channel never opened: {err}"
                    );
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        };
        let (read, writer) = tokio::io::split(stream);
        Client {
            name: name.to_owned(),
            sent: 0,
            received: 0,
            reader: BufReader::new(read),
            writer,
        }
    }

    /// Connects and says a hello that the server takes.
    async fn greeted(endpoint: &Endpoint) -> Client {
        let mut client = Client::connect(endpoint).await;
        client.hello().await;
        client
    }

    async fn hello(&mut self) {
        self.send(json!({"hoprs_agent": 1, "version": VERSION}))
            .await;
        let answer = self.recv().await;
        assert_eq!(
            answer,
            Some(json!({"hello": "ok", "version": VERSION})),
            "{}: the hello's answer",
            self.name
        );
    }

    async fn send(&mut self, message: Value) {
        self.send_raw(format!("{message}\n").as_bytes()).await;
    }

    /// Writes `bytes`; a write error (the server closed) is fine, a stall is not.
    async fn send_raw(&mut self, bytes: &[u8]) {
        self.sent += 1;
        let what = format!("{}: writing line {}", self.name, self.sent);
        let _ = step(what, self.writer.write_all(bytes)).await;
    }

    /// The next line as JSON; `None` at the end (or an error: the server closed).
    async fn recv(&mut self) -> Option<Value> {
        self.received += 1;
        let what = format!("{}: reading line {}", self.name, self.received);
        let line = step(what, proto::read_line(&mut self.reader)).await;
        line.ok()
            .flatten()
            .map(|line| serde_json::from_str(&line).unwrap())
    }

    /// The connection ends without another line.
    async fn assert_closed(&mut self) {
        let last = self.recv().await;
        assert_eq!(last, None, "{}: expected the end", self.name);
    }

    async fn call(&mut self, id: u64, tool: &str, args: Value) {
        self.send(json!({"call": id, "tool": tool, "args": args}))
            .await;
    }
}

impl Fx {
    /// Waits for an event that `want` accepts; panics after [`WAIT`], naming the caller's
    /// line and the events seen meanwhile.
    #[track_caller]
    fn event<'a>(
        &'a mut self,
        want: impl Fn(&AgentEvent) -> bool + 'a,
    ) -> impl std::future::Future<Output = AgentEvent> + 'a {
        let at = std::panic::Location::caller();
        async move {
            let start = std::time::Instant::now();
            let mut seen = Vec::new();
            loop {
                let left = WAIT.saturating_sub(start.elapsed());
                match tokio::time::timeout(left, self.events.recv()).await {
                    Ok(Some(event)) if want(&event) => return event,
                    Ok(Some(event)) => seen.push(event),
                    _ => panic!("no matching agent event in time (waited at {at}); saw {seen:?}"),
                }
            }
        }
    }

    fn events_now(&mut self) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            out.push(event);
        }
        out
    }

    fn read_args(&self) -> Value {
        json!({"site": self.site.id.0, "path": FILE})
    }
}

fn reply_id(reply: &Value) -> u64 {
    reply["reply"].as_u64().unwrap_or_else(|| panic!("{reply}"))
}

fn first_block(reply: &Value) -> Value {
    assert_eq!(reply["ok"], true, "{reply}");
    serde_json::from_str(reply["blocks"][0].as_str().unwrap()).unwrap()
}

/// Waits until `done` holds; panics after [`WAIT`].
async fn until(what: &str, done: impl Fn() -> bool) {
    let start = std::time::Instant::now();
    while !done() {
        assert!(start.elapsed() < WAIT, "{what} never happened");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn hello_then_calls() {
    let mut fx = fixture(true);
    let mut client = Client::greeted(&fx.endpoint).await;
    client.call(1, "list_sites", json!({})).await;
    let reply = client.recv().await.unwrap();
    assert_eq!(reply_id(&reply), 1);
    let sites = first_block(&reply);
    assert_eq!(sites["sites"][0]["id"], fx.site.id.0, "{sites}");
    fx.event(|e| *e == AgentEvent::Channel(None)).await;
    fx.event(|e| *e == AgentEvent::Relays(1)).await;
}

#[tokio::test]
async fn calls_overlap_and_match_their_ids() {
    let fx = fixture(true);
    let hold = fx.host.hold(&fx.site.id, FsOp::Read);
    let mut client = Client::greeted(&fx.endpoint).await;
    client.call(1, "read_file", fx.read_args()).await;
    step("a held call reaching the host", hold.entered()).await;
    client.call(2, "list_sites", json!({})).await;
    let quick = client.recv().await.unwrap();
    assert_eq!(reply_id(&quick), 2, "the quick call is answered first");
    hold.release();
    let slow = client.recv().await.unwrap();
    assert_eq!(reply_id(&slow), 1);
    assert_eq!(slow["ok"], true, "{slow}");
    assert_eq!(slow["blocks"][1], "hello");
}

#[tokio::test]
async fn cancel_stops_a_call_and_sends_nothing() {
    let fx = fixture(true);
    let hold = fx.host.hold(&fx.site.id, FsOp::Read);
    let mut client = Client::greeted(&fx.endpoint).await;
    client.call(1, "read_file", fx.read_args()).await;
    step("a held call reaching the host", hold.entered()).await;
    client.send(json!({"cancel": 1})).await;
    until("the held call was dropped", || hold.dropped()).await;
    hold.release();
    // The next lines answer the next calls only: nothing ever comes for 1.
    for id in [2, 3] {
        client.call(id, "list_sites", json!({})).await;
        assert_eq!(reply_id(&client.recv().await.unwrap()), id);
    }
    // Its id is free again.
    client.call(1, "list_sites", json!({})).await;
    assert_eq!(reply_id(&client.recv().await.unwrap()), 1);
}

#[tokio::test]
async fn a_ninth_call_waits_its_turn() {
    let fx = fixture(true);
    let hold = fx.host.hold(&fx.site.id, FsOp::Read);
    let mut client = Client::greeted(&fx.endpoint).await;
    for id in 1..=MAX_IN_FLIGHT as u64 {
        client.call(id, "read_file", fx.read_args()).await;
    }
    until("every held call started", || {
        fx.host.calls(&fx.site.id, FsOp::Read) == MAX_IN_FLIGHT
    })
    .await;
    // The ninth waits: even a quick call is not answered while eight run.
    client.call(100, "list_sites", json!({})).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(fx.host.calls(&fx.site.id, FsOp::Read), MAX_IN_FLIGHT);
    // A cancel is still read, and frees a turn for it.
    client.send(json!({"cancel": 1})).await;
    assert_eq!(reply_id(&client.recv().await.unwrap()), 100);
    hold.release();
    let mut ids: Vec<u64> = Vec::new();
    for _ in 2..=MAX_IN_FLIGHT {
        ids.push(reply_id(&client.recv().await.unwrap()));
    }
    ids.sort_unstable();
    assert_eq!(ids, (2..=MAX_IN_FLIGHT as u64).collect::<Vec<_>>());
}

#[tokio::test]
async fn a_seventeenth_call_is_refused_and_cancels_are_still_read() {
    let mut fx = fixture(true);
    let hold = fx.host.hold(&fx.site.id, FsOp::Read);
    let mut client = Client::greeted(&fx.endpoint).await;
    fx.event(|e| *e == AgentEvent::Relays(1)).await;
    // Eight run, eight wait.
    let queued = (MAX_IN_FLIGHT + WAITING_MAX) as u64;
    for id in 1..=queued {
        client.call(id, "read_file", fx.read_args()).await;
    }
    until("eight calls run", || {
        fx.host.calls(&fx.site.id, FsOp::Read) == MAX_IN_FLIGHT
    })
    .await;
    // One more is answered at once, under its own id, and holds nothing.
    client.call(99, "read_file", fx.read_args()).await;
    let refused = client.recv().await.unwrap();
    assert_eq!(reply_id(&refused), 99);
    assert_eq!(refused["error"], TOO_MANY, "{refused}");
    // A cancel is still read: it frees a turn, and the first waiting call starts.
    client.send(json!({"cancel": 1})).await;
    until("a waiting call started", || {
        fx.host.calls(&fx.site.id, FsOp::Read) == MAX_IN_FLIGHT + 1
    })
    .await;
    // And so is the end: every call goes with the relay, the waiting ones never start.
    drop(client);
    fx.event(|e| *e == AgentEvent::Relays(0)).await;
    assert!(hold.dropped());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fx.host.calls(&fx.site.id, FsOp::Read), MAX_IN_FLIGHT + 1);
}

#[tokio::test]
async fn a_relay_that_floods_without_reading_is_closed() {
    let fx = fixture(true);
    let _hold = fx.host.hold(&fx.site.id, FsOp::Read);
    let mut client = Client::named(&fx.endpoint, "flood").await;
    client.hello().await;
    let Client {
        mut reader,
        mut writer,
        ..
    } = client;
    // Sixteen calls held, then refusals it never reads: far more than any socket or
    // pipe buffer holds, so they pile up unwritten and the relay is cut. It writes as a
    // real relay would, from its own task, and a failed write is the expected end.
    const CALLS: u64 = 20_000;
    let args = fx.read_args();
    let writing = tokio::spawn(async move {
        for id in 1..=CALLS {
            let line = format!(
                "{}\n",
                json!({"call": id, "tool": "read_file", "args": args})
            );
            // Shorter than the outer step, so a stall reports which call.
            match tokio::time::timeout(STEP / 3, writer.write_all(line.as_bytes())).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => return Ok(id - 1),
                Err(_) => return Err(format!("flood: stalled writing call {id} of {CALLS}")),
            }
        }
        Ok(CALLS)
    });
    let written = step("flood: the writer's end", writing)
        .await
        .unwrap()
        .unwrap_or_else(|stalled| panic!("{stalled}"));
    // Whatever it can still read, then the end.
    let mut answers = 0u64;
    loop {
        let what = format!("flood: reading answers after {answers} (wrote {written} calls)");
        match step(what, proto::read_line(&mut reader)).await {
            Ok(Some(_)) => answers += 1,
            Ok(None) | Err(_) => break,
        }
        assert!(answers <= CALLS, "more answers than calls");
    }
    assert!(answers < CALLS - 16, "cut before every refusal: {answers}");
    // The server keeps serving others.
    let mut other = Client::named(&fx.endpoint, "another relay").await;
    other.hello().await;
    other.call(1, "list_sites", json!({})).await;
    assert_eq!(reply_id(&other.recv().await.unwrap()), 1);
}

/// An agent that fans out many calls while a large answer is still going out, and reads
/// at a normal pace: it keeps its connection; only the calls beyond the queue fail, each
/// with its own error, and every call gets one reply.
#[tokio::test]
async fn a_fan_out_behind_a_large_answer_keeps_its_connection() {
    let fx = fixture(true);
    let big = "x".repeat(crate::agent::FILE_MAX);
    let fs = fx.host.add_site(fx.site.clone());
    fs.add_file(FILE, big.as_bytes(), SystemTime::UNIX_EPOCH);
    let mut other = Site::new("Other", "other.example", "u", Protocol::Ssh);
    other.agent_access = AgentAccess::Read;
    let other_fs = fx.host.add_site(other.clone());
    other_fs.add_file(FILE, b"small", SystemTime::UNIX_EPOCH);
    let hold = fx.host.hold(&other.id, FsOp::Read);
    let mut client = Client::greeted(&fx.endpoint).await;

    // A 1 MiB answer: about 1.4 MB of line, more than the socket holds unread.
    client.call(1, "read_file", fx.read_args()).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    // Forty calls meanwhile: some run, eight wait, the others are refused behind it.
    // How many run depends on when the large answer frees its turn, which is when the
    // system has taken it: on Unix it does not fit in the socket's buffer, so it keeps
    // its turn and seven run; on Windows mio copies the whole line for a pending write
    // and reports it written at once (`Write for &NamedPipe`), so its turn is free and
    // eight run. Either way the refusals still queue behind that answer.
    let held = json!({"site": other.id.0, "path": FILE});
    const FAN: u64 = 40;
    for id in 2..=FAN + 1 {
        client.call(id, "read_file", held.clone()).await;
    }
    until("seven held calls run", || {
        fx.host.calls(&other.id, FsOp::Read) >= MAX_IN_FLIGHT - 1
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let running = fx.host.calls(&other.id, FsOp::Read);
    assert!(
        [MAX_IN_FLIGHT - 1, MAX_IN_FLIGHT].contains(&running),
        "{running} calls run"
    );

    let large = client.recv().await.expect("the large answer");
    assert_eq!(reply_id(&large), 1);
    assert_eq!(
        large["blocks"][1].as_str().unwrap().len(),
        crate::agent::FILE_MAX
    );
    let refusals = FAN as usize - running - WAITING_MAX;
    let mut answered = Vec::new();
    for _ in 0..refusals {
        let reply = client.recv().await.expect("a refusal, not the end");
        assert_eq!(reply["error"], TOO_MANY, "{reply}");
        answered.push(reply_id(&reply));
    }
    hold.release();
    for _ in 0..(running + WAITING_MAX) {
        let reply = client.recv().await.expect("an answer, not the end");
        assert_eq!(reply["ok"], true, "{reply}");
        answered.push(reply_id(&reply));
    }
    answered.sort_unstable();
    assert_eq!(
        answered,
        (2..=FAN + 1).collect::<Vec<_>>(),
        "one reply per call"
    );
    // Still connected.
    client.call(100, "list_sites", json!({})).await;
    assert_eq!(reply_id(&client.recv().await.unwrap()), 100);
}

#[tokio::test]
async fn a_call_whose_task_panics_is_answered() {
    let dir = tempfile::tempdir().unwrap();
    let endpoint = channel::endpoint_in(dir.path());
    let (tx, _events) = mpsc::unbounded_channel();
    let service = Service::new(Arc::new(PanicHost), Limits::default(), tx.clone());
    let (_switch, watching) = watch::channel(true);
    let server = AgentServer::start(service, Some(endpoint.clone()), watching, tx);
    let mut client = Client::greeted(&endpoint).await;
    // Each call is answered, and its turn comes back: more than eight in a row.
    for id in 1..=(MAX_IN_FLIGHT as u64 + 2) {
        client.call(id, "list_sites", json!({})).await;
        let reply = client.recv().await.unwrap();
        assert_eq!(reply_id(&reply), id);
        assert_eq!(reply["ok"], false, "{reply}");
        assert_eq!(reply["error"], FAILED, "{reply}");
    }
    step("server stop", server.shutdown()).await;
}

#[tokio::test]
async fn an_answer_too_long_for_a_line_is_replaced() {
    let fx = fixture(true);
    // A site whose name alone is a full line: list_sites cannot be one line.
    let mut huge = fx.site.clone();
    huge.id = hoprs_core::SiteId::new();
    huge.name = "n".repeat(proto::MAX_LINE);
    fx.host.add_site(huge);
    let mut client = Client::greeted(&fx.endpoint).await;
    client.call(1, "list_sites", json!({})).await;
    let reply = client.recv().await.unwrap();
    assert_eq!(reply_id(&reply), 1);
    assert_eq!(reply["error"], TOO_LONG, "{reply}");
    // The connection goes on.
    client.call(2, "stat", fx.read_args()).await;
    assert_eq!(reply_id(&client.recv().await.unwrap()), 2);
}

#[tokio::test]
async fn hellos_in_progress_are_capped() {
    let fx = fixture(true);
    let mut silent = Vec::new();
    for _ in 0..HANDSHAKES_MAX {
        silent.push(Client::connect(&fx.endpoint).await);
    }
    // One more is closed at once, long before the hello's time is up.
    let start = std::time::Instant::now();
    let mut extra = Client::connect(&fx.endpoint).await;
    extra.assert_closed().await;
    assert!(start.elapsed() < HELLO_WAIT / 2, "{:?}", start.elapsed());
    // Once they are gone, a relay is greeted again.
    drop(silent);
    let start = std::time::Instant::now();
    loop {
        let mut client = Client::connect(&fx.endpoint).await;
        client
            .send(json!({"hoprs_agent": 1, "version": VERSION}))
            .await;
        if client.recv().await.is_some() {
            break;
        }
        assert!(start.elapsed() < WAIT, "never greeted again");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn the_hello_is_small() {
    let fx = fixture(true);
    let hello = json!({"hoprs_agent": 1, "version": VERSION}).to_string();
    // Up to the limit: greeted.
    let mut client = Client::connect(&fx.endpoint).await;
    client
        .send_raw(format!("{hello:<width$}\n", width = proto::HELLO_MAX).as_bytes())
        .await;
    assert_eq!(
        client.recv().await,
        Some(json!({"hello": "ok", "version": VERSION}))
    );
    // One byte more: closed without an answer.
    let mut client = Client::connect(&fx.endpoint).await;
    client
        .send_raw(format!("{hello:<width$}\n", width = proto::HELLO_MAX + 1).as_bytes())
        .await;
    client.assert_closed().await;
}

#[tokio::test]
async fn mismatch_is_answered_and_closed() {
    let mut fx = fixture(true);
    for hello in [
        json!({"hoprs_agent": 1, "version": format!("{VERSION}-other")}),
        json!({"hoprs_agent": 2, "version": VERSION}),
    ] {
        let mut client = Client::connect(&fx.endpoint).await;
        client.send(hello).await;
        assert_eq!(
            client.recv().await,
            Some(json!({"hello": "mismatch", "version": VERSION}))
        );
        client.assert_closed().await;
    }
    assert!(
        !fx.events_now()
            .iter()
            .any(|e| matches!(e, AgentEvent::Relays(n) if *n > 0))
    );
}

/// In real time (five seconds): paused time over a real socket or pipe could advance
/// while the I/O is still on its way.
#[tokio::test]
async fn silent_client_is_dropped() {
    let mut fx = fixture(true);
    let start = std::time::Instant::now();
    let mut client = Client::connect(&fx.endpoint).await;
    client.assert_closed().await;
    // The server's clock started at its accept, after `start`: a margin for the
    // connect, none the other way.
    let waited = start.elapsed();
    assert!(
        waited + Duration::from_millis(500) >= HELLO_WAIT,
        "{waited:?}"
    );
    assert!(waited < WAIT, "{waited:?}");
    assert!(!fx.events_now().contains(&AgentEvent::Relays(1)));
}

#[tokio::test]
async fn junk_and_oversized_lines_close_the_connection() {
    let mut oversized = vec![b'a'; proto::MAX_LINE + 1];
    oversized.push(b'\n');
    let junk: [&[u8]; 4] = [b"not json\n", b"[1,2]\n", b"{\"what\":1}\n", &oversized];
    let fx = fixture(true);
    for line in junk {
        // As the hello: closed without an answer.
        let mut client = Client::connect(&fx.endpoint).await;
        client.name = format!("{}, junk as its hello", client.name);
        client.send_raw(line).await;
        client.assert_closed().await;

        // After the hello: closed too, no answer.
        let mut client = Client::greeted(&fx.endpoint).await;
        client.name = format!("{}, junk after its hello", client.name);
        client.send_raw(line).await;
        client.assert_closed().await;
    }
    // The server still serves a new relay.
    let mut client = Client::greeted(&fx.endpoint).await;
    client.call(1, "list_sites", json!({})).await;
    assert_eq!(reply_id(&client.recv().await.unwrap()), 1);
}

#[tokio::test]
async fn duplicate_ids_close_and_unknown_cancels_are_ignored() {
    let fx = fixture(true);
    let hold = fx.host.hold(&fx.site.id, FsOp::Read);
    let mut client = Client::greeted(&fx.endpoint).await;
    // A cancel of nothing is nothing.
    client.send(json!({"cancel": 42})).await;
    client.call(1, "list_sites", json!({})).await;
    assert_eq!(reply_id(&client.recv().await.unwrap()), 1);
    // A second call under an id still running closes the connection; the held call
    // goes with it.
    client.call(2, "read_file", fx.read_args()).await;
    step("a held call reaching the host", hold.entered()).await;
    client.call(2, "list_sites", json!({})).await;
    client.assert_closed().await;
    until("the held call was dropped", || hold.dropped()).await;
}

#[tokio::test]
async fn calls_end_with_their_relay() {
    let mut fx = fixture(true);
    let hold = fx.host.hold(&fx.site.id, FsOp::Read);
    let mut client = Client::greeted(&fx.endpoint).await;
    fx.event(|e| *e == AgentEvent::Relays(1)).await;
    client.call(1, "read_file", fx.read_args()).await;
    step("a held call reaching the host", hold.entered()).await;
    drop(client);
    until("the held call was dropped", || hold.dropped()).await;
    fx.event(|e| *e == AgentEvent::Relays(0)).await;
}

#[tokio::test]
async fn seventeenth_relay_is_refused() {
    let mut fx = fixture(true);
    let mut relays = Vec::new();
    for _ in 0..MAX_RELAYS {
        relays.push(Client::greeted(&fx.endpoint).await);
    }
    fx.event(|e| *e == AgentEvent::Relays(MAX_RELAYS)).await;
    let mut extra = Client::connect(&fx.endpoint).await;
    extra
        .send(json!({"hoprs_agent": 1, "version": VERSION}))
        .await;
    extra.assert_closed().await;
    // The others still work, and a place freed is taken again.
    relays[0].call(1, "list_sites", json!({})).await;
    assert_eq!(reply_id(&relays[0].recv().await.unwrap()), 1);
    relays.pop();
    fx.event(|e| *e == AgentEvent::Relays(MAX_RELAYS - 1)).await;
    relays.push(Client::greeted(&fx.endpoint).await);
}

#[tokio::test]
async fn switch_off_drops_relays_and_removes_the_socket() {
    let mut fx = fixture(true);
    let hold = fx.host.hold(&fx.site.id, FsOp::Read);
    let mut client = Client::greeted(&fx.endpoint).await;
    let mut idle = Client::greeted(&fx.endpoint).await;
    fx.event(|e| *e == AgentEvent::Relays(2)).await;
    client.call(1, "read_file", fx.read_args()).await;
    step("a held call reaching the host", hold.entered()).await;

    // An agent-only session, used a moment ago: it goes with the switch.
    fx.host.mark_agent_only(&fx.site.id);
    fx.host.set_enabled(false);
    fx.switch.send_replace(false);
    for relay in [&mut client, &mut idle] {
        assert_eq!(relay.recv().await, Some(json!({"bye": "off"})));
        relay.assert_closed().await;
    }
    fx.event(|e| *e == AgentEvent::Relays(0)).await;
    assert!(hold.dropped(), "the call in flight was dropped");
    fx.event(|e| *e == AgentEvent::Sites(Vec::new())).await;
    assert_eq!(fx.host.closed(), [fx.site.id.clone()], "closed at once");
    fx.host.set_enabled(true);
    #[cfg(unix)]
    {
        let Endpoint::Socket(path) = &fx.endpoint;
        assert!(
            std::fs::symlink_metadata(path).is_err(),
            "the socket file is gone"
        );
    }
    assert!(
        step(
            "connecting to a closed channel",
            channel::connect(&fx.endpoint)
        )
        .await
        .is_err()
    );

    fx.switch.send_replace(true);
    let mut again = Client::greeted(&fx.endpoint).await;
    again.call(1, "list_sites", json!({})).await;
    assert_eq!(reply_id(&again.recv().await.unwrap()), 1);
}

#[tokio::test]
async fn switch_follows_its_latest_value() {
    let mut fx = fixture(false);
    // Flipped several times before the server looks: only the last value counts.
    for on in [true, false, true, false, true] {
        fx.switch.send_replace(on);
    }
    let _client = Client::greeted(&fx.endpoint).await;
    for on in [false, true, false] {
        fx.switch.send_replace(on);
    }
    fx.event(|e| *e == AgentEvent::Relays(0)).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        step(
            "connecting to a closed channel",
            channel::connect(&fx.endpoint)
        )
        .await
        .is_err(),
        "off at last"
    );
}

#[tokio::test]
async fn shutdown_says_quit() {
    let mut fx = fixture(true);
    let mut client = Client::greeted(&fx.endpoint).await;
    fx.event(|e| *e == AgentEvent::Relays(1)).await;
    step("server stop", fx.server.shutdown()).await;
    assert_eq!(client.recv().await, Some(json!({"bye": "quit"})));
    client.assert_closed().await;
    #[cfg(unix)]
    {
        let Endpoint::Socket(path) = &fx.endpoint;
        assert!(
            std::fs::symlink_metadata(path).is_err(),
            "the socket file is gone"
        );
    }
    // A second shutdown is nothing.
    step("server stop", fx.server.shutdown()).await;
}

#[tokio::test]
async fn a_relay_that_does_not_read_holds_up_nobody() {
    let fx = fixture(true);
    let mut stuck = Client::greeted(&fx.endpoint).await;
    // Answers of a full file, asked for many times and never read.
    let big = "x".repeat(crate::agent::FILE_MAX);
    let fs = fx.host.add_site(fx.site.clone());
    fs.add_file(FILE, big.as_bytes(), SystemTime::UNIX_EPOCH);
    for id in 0..64 {
        stuck.call(id, "read_file", fx.read_args()).await;
    }
    // Another relay is served meanwhile.
    let mut other = Client::greeted(&fx.endpoint).await;
    other.call(1, "list_sites", json!({})).await;
    assert_eq!(reply_id(&other.recv().await.unwrap()), 1);
    // And the goodbye does not wait for the stuck one.
    step("server stop", fx.server.shutdown()).await;
    drop(stuck);
}

#[cfg(unix)]
#[tokio::test]
async fn bind_failure_is_reported_and_retried() {
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, b"").unwrap();
    let endpoint = channel::endpoint_in(&blocker);
    let mut fx = fixture_at(dir, Some(endpoint.clone()), true);
    let AgentEvent::Channel(Some(error)) = fx.event(|e| matches!(e, AgentEvent::Channel(_))).await
    else {
        panic!("a failure first");
    };
    assert_eq!(error.code, Some("agent-channel-failed"));
    assert!(error.detail.is_some());
    // Fixed, and the switch cycled: open.
    std::fs::remove_file(&blocker).unwrap();
    fx.switch.send_replace(false);
    fx.switch.send_replace(true);
    fx.event(|e| *e == AgentEvent::Channel(None)).await;
    let _client = Client::greeted(&endpoint).await;
}

/// What is logged on this thread while the guard lives (the tests run on one thread).
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

#[tokio::test]
async fn no_endpoint_never_listens() {
    let logs = LogBuf::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::INFO)
        .finish();
    let _default = tracing::subscriber::set_default(subscriber);

    let dir = tempfile::tempdir().unwrap();
    let mut fx = fixture_at(dir, None, true);
    for on in [false, true, false, true] {
        fx.switch.send_replace(on);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        !fx.events_now()
            .iter()
            .any(|e| matches!(e, AgentEvent::Channel(_))),
        "never opened, never failed"
    );
    let text = String::from_utf8_lossy(&logs.0.lock().unwrap()).into_owned();
    let warned: Vec<&str> = text
        .lines()
        .filter(|line| line.contains("no place for the agent channel"))
        .collect();
    assert_eq!(warned.len(), 1, "{text}");
    assert!(warned[0].contains("WARN"), "{text}");
    // Nothing was made where a channel of this run's directory would be.
    assert!(
        step(
            "connecting to a closed channel",
            channel::connect(&fx.endpoint)
        )
        .await
        .is_err()
    );
    step("server stop", fx.server.shutdown()).await;
}

#[tokio::test(start_paused = true)]
async fn reaper_runs_with_the_switch_off() {
    let mut fx = fixture(false);
    fx.host.mark_agent_only(&fx.site.id);
    tokio::time::sleep(REAP_EVERY - Duration::from_secs(1)).await;
    assert!(fx.host.closed().is_empty(), "not before its time");
    tokio::time::sleep(Duration::from_secs(2)).await;
    until("the idle session was reaped", || {
        fx.host.closed() == [fx.site.id.clone()]
    })
    .await;
    fx.event(|e| matches!(e, AgentEvent::Sites(_))).await;
    assert!(
        !fx.events_now()
            .iter()
            .any(|e| matches!(e, AgentEvent::Channel(_)))
    );
}

/// A host whose every call is a bug: the service's task panics at its first step.
struct PanicHost;

#[async_trait::async_trait]
impl crate::agent::host::AgentHost for PanicHost {
    fn enabled(&self) -> bool {
        panic!("a bug in hoprs (test)")
    }
    async fn sites(&self) -> hoprs_core::Result<Vec<Site>> {
        unreachable!()
    }
    fn connected(&self, _: &hoprs_core::SiteId) -> Option<crate::backend::SessionId> {
        unreachable!()
    }
    async fn connect(
        &self,
        _: &hoprs_core::SiteId,
    ) -> std::result::Result<crate::backend::SessionId, crate::agent::host::ConnectFailure> {
        unreachable!()
    }
    fn prompt_open(&self, _: &hoprs_core::SiteId) -> bool {
        unreachable!()
    }
    async fn close_agent_session(&self, _: &hoprs_core::SiteId) -> bool {
        unreachable!()
    }
    fn agent_sites(&self) -> Vec<hoprs_core::SiteId> {
        unreachable!()
    }
    fn remote(
        &self,
        _: crate::backend::SessionId,
    ) -> hoprs_core::Result<Arc<dyn hoprs_remote::RemoteFs>> {
        unreachable!()
    }
    async fn exec(
        &self,
        _: crate::backend::SessionId,
        _: String,
        _: hoprs_ssh::ExecLimits,
    ) -> hoprs_core::Result<hoprs_ssh::ExecOutput> {
        unreachable!()
    }
    async fn enqueue(
        &self,
        _: hoprs_transfer::TransferSpec,
        _: hoprs_transfer::ConflictPolicy,
    ) -> hoprs_core::Result<hoprs_transfer::TransferId> {
        unreachable!()
    }
    async fn transfers(&self) -> Vec<hoprs_transfer::TransferItem> {
        unreachable!()
    }
    fn transfer_events(&self) -> tokio::sync::broadcast::Receiver<hoprs_transfer::Event> {
        unreachable!()
    }
    async fn protected(&self) -> crate::agent::paths::Protected {
        unreachable!()
    }
}
