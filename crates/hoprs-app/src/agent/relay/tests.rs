//! The relay over in-memory streams: an MCP client on one side (lines written and read
//! as text), and on the other a scripted app behind a [`Connect`] made of `duplex`
//! halves, or the real server over a [`FakeHost`] on a channel in a temporary directory.
//! Every awaited I/O step is a [`step`], so a stall names where it stopped.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use futures_util::FutureExt;
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf,
};
use tokio::sync::mpsc;

use super::*;
use crate::agent::testing::step;
use crate::agent::{TURNED_OFF, UNREACHABLE, tools, version_mismatch};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const BUFFER: usize = 1 << 20;

// ---- The MCP client's side ----

/// The client end of a running [`serve`]: what it writes is the relay's stdin, what it
/// reads the relay's stdout. Every line read must be one JSON-RPC message.
struct Mcp {
    input: Option<DuplexStream>,
    output: BufReader<DuplexStream>,
    /// Lines read while looking for another id.
    early: HashMap<String, Value>,
    served: Option<tokio::task::JoinHandle<io::Result<()>>>,
}

fn start(app: impl Connect) -> Mcp {
    let (input, relay_in) = tokio::io::duplex(BUFFER);
    let (relay_out, output) = tokio::io::duplex(BUFFER);
    let served = tokio::spawn(serve((relay_in, relay_out), app));
    Mcp {
        input: Some(input),
        output: BufReader::new(output),
        early: HashMap::new(),
        served: Some(served),
    }
}

/// A line of the relay's output: one JSON object with `"jsonrpc": "2.0"`.
fn protocol_line(line: &str) -> Value {
    let value: Value = serde_json::from_str(line)
        .unwrap_or_else(|e| panic!("the output carried a line that is not JSON ({e}): {line:?}"));
    assert!(value.is_object(), "not an object: {line:?}");
    assert_eq!(value["jsonrpc"], "2.0", "not JSON-RPC: {line:?}");
    value
}

fn id_key(id: &Value) -> String {
    id.to_string()
}

impl Mcp {
    async fn send_line(&mut self, line: &str) {
        let input = self.input.as_mut().expect("the input is open");
        step("client: writing a line", async {
            input.write_all(line.as_bytes()).await.unwrap();
            input.write_all(b"\n").await.unwrap();
            input.flush().await.unwrap();
        })
        .await;
    }

    async fn send(&mut self, message: Value) {
        self.send_line(&message.to_string()).await;
    }

    async fn request(&mut self, id: u64, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
    }

    async fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .await;
    }

    /// The next line of output, `None` at its end.
    async fn next_line(&mut self) -> Option<Value> {
        let mut line = String::new();
        let read = step("client: reading a line", self.output.read_line(&mut line))
            .await
            .unwrap();
        if read == 0 {
            return None;
        }
        assert!(line.ends_with('\n'), "a line without its newline: {line:?}");
        Some(protocol_line(line.trim_end_matches(['\n', '\r'])))
    }

    /// The response to request `id` (result or error); other responses read meanwhile
    /// are kept for later.
    async fn response(&mut self, id: u64) -> Value {
        let key = id_key(&json!(id));
        if let Some(found) = self.early.remove(&key) {
            return found;
        }
        loop {
            let line = self.next_line().await.expect("a response before the end");
            let this = id_key(&line["id"]);
            if this == key {
                return line;
            }
            self.early.insert(this, line);
        }
    }

    async fn call(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.request(id, method, params).await;
        self.response(id).await
    }

    async fn call_tool(&mut self, id: u64, name: &str, arguments: Value) -> Value {
        self.call(
            id,
            "tools/call",
            json!({"name": name, "arguments": arguments}),
        )
        .await
    }

    /// The legacy opening: `initialize`, its answer, `notifications/initialized`.
    async fn open_legacy(&mut self) -> Value {
        let answer = self
            .call(
                0,
                "initialize",
                json!({
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "test", "version": "0"},
                }),
            )
            .await;
        self.notify("notifications/initialized", json!({})).await;
        answer
    }

    /// Closes the input; the relay's run must end within `within`.
    async fn close(&mut self, within: Duration) -> io::Result<()> {
        drop(self.input.take());
        let served = self.served.take().expect("still served");
        match tokio::time::timeout(within, served).await {
            Ok(done) => done.expect("serve did not panic"),
            Err(_) => panic!("serve still running {within:?} after its input closed"),
        }
    }
}

/// A tool result's texts and its `isError`.
fn texts(answer: &Value) -> (Vec<String>, bool) {
    let result = &answer["result"];
    assert!(result.is_object(), "not a result: {answer}");
    let texts = result["content"]
        .as_array()
        .expect("content")
        .iter()
        .map(|block| {
            assert_eq!(block["type"], "text", "{block}");
            block["text"].as_str().unwrap().to_owned()
        })
        .collect();
    (texts, result["isError"] == true)
}

/// The `_meta` a modern request carries (protocol fact 4).
fn modern_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {"name": "test", "version": "0"},
    })
}

// ---- The scripted app ----

/// A [`Connect`] whose every connection is handed to the test as an [`App`] end.
#[derive(Clone)]
struct FakeApp {
    ends: mpsc::UnboundedSender<DuplexStream>,
    refuse: Arc<AtomicBool>,
}

/// The test's side of [`FakeApp`].
struct AppSide {
    ends: mpsc::UnboundedReceiver<DuplexStream>,
    refuse: Arc<AtomicBool>,
}

fn fake_app() -> (FakeApp, AppSide) {
    let (tx, rx) = mpsc::unbounded_channel();
    let refuse = Arc::new(AtomicBool::new(false));
    (
        FakeApp {
            ends: tx,
            refuse: refuse.clone(),
        },
        AppSide { ends: rx, refuse },
    )
}

impl Connect for FakeApp {
    fn connect(&self) -> BoxFuture<'_, io::Result<(BoxAsyncRead, BoxAsyncWrite)>> {
        async move {
            if self.refuse.load(Ordering::SeqCst) {
                return Err(io::Error::new(io::ErrorKind::NotFound, "no channel"));
            }
            let (relay, app) = tokio::io::duplex(BUFFER);
            let _ = self.ends.send(app);
            let (read, write) = tokio::io::split(relay);
            Ok((
                Box::new(read) as BoxAsyncRead,
                Box::new(write) as BoxAsyncWrite,
            ))
        }
        .boxed()
    }
}

impl AppSide {
    fn refuse(&self, refuse: bool) {
        self.refuse.store(refuse, Ordering::SeqCst);
    }

    /// The next connection the relay made.
    async fn accept(&mut self) -> App {
        let stream = step("app: a connection", self.ends.recv())
            .await
            .expect("the relay is alive");
        let (read, writer) = tokio::io::split(stream);
        App {
            reader: BufReader::new(read),
            writer,
        }
    }

    /// Whether the relay connected since the last look.
    fn connected(&mut self) -> bool {
        self.ends.try_recv().is_ok()
    }

    /// The next connection, greeted with an ok.
    async fn greeted(&mut self) -> App {
        let mut app = self.accept().await;
        app.greet().await;
        app
    }
}

/// One connection of the scripted app.
struct App {
    reader: BufReader<ReadHalf<DuplexStream>>,
    writer: WriteHalf<DuplexStream>,
}

impl App {
    /// The relay's next message, `None` at the end of the connection.
    async fn recv(&mut self) -> Option<Value> {
        let line = step("app: reading a line", proto::read_line(&mut self.reader))
            .await
            .expect("a readable line");
        line.map(|line| serde_json::from_str(&line).expect("a JSON line"))
    }

    async fn send(&mut self, message: Value) {
        let mut line = message.to_string();
        line.push('\n');
        step("app: writing a line", async {
            self.writer.write_all(line.as_bytes()).await.unwrap();
            self.writer.flush().await.unwrap();
        })
        .await;
    }

    /// Reads the relay's hello and answers ok.
    async fn greet(&mut self) {
        let hello = self.recv().await;
        assert_eq!(
            hello,
            Some(json!({"hoprs_agent": 1, "version": VERSION})),
            "the relay's hello"
        );
        self.send(json!({"hello": "ok", "version": VERSION})).await;
    }

    /// The next call: its id, tool and arguments.
    async fn call(&mut self) -> (u64, String, Value) {
        let message = self.recv().await.expect("a call");
        let id = message["call"]
            .as_u64()
            .unwrap_or_else(|| panic!("{message}"));
        (
            id,
            message["tool"].as_str().unwrap().to_owned(),
            message["args"].clone(),
        )
    }

    async fn ok(&mut self, id: u64, blocks: &[&str]) {
        self.send(json!({"reply": id, "ok": true, "blocks": blocks}))
            .await;
    }

    async fn fail(&mut self, id: u64, error: &str) {
        self.send(json!({"reply": id, "ok": false, "error": error}))
            .await;
    }

    /// Waits for the relay to close this connection (what it sends meanwhile is read).
    async fn ends(&mut self) {
        while self.recv().await.is_some() {}
    }
}

// ---- The opening exchanges ----

#[tokio::test]
async fn legacy_handshake_then_tools() {
    let (app, mut side) = fake_app();
    let mut mcp = start(app);
    let answer = mcp.open_legacy().await;
    let result = &answer["result"];
    assert_eq!(result["protocolVersion"], "2025-11-25", "{answer}");
    assert!(result["capabilities"]["tools"].is_object(), "{answer}");
    assert_eq!(result["serverInfo"]["name"], "hoprs");
    assert_eq!(result["serverInfo"]["version"], VERSION);
    assert_eq!(result["instructions"], crate::agent::INSTRUCTIONS);
    // Legacy results carry no modern fields.
    assert!(result.get("resultType").is_none(), "{answer}");

    let list = mcp.call(1, "tools/list", json!({})).await;
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    let table: Vec<&str> = tools::all().iter().map(|t| t.name).collect();
    assert_eq!(names, table);
    assert_eq!(names.len(), 13);
    assert!(list["result"].get("resultType").is_none(), "{list}");

    let ping = mcp.call(2, "ping", json!({})).await;
    assert_eq!(ping["result"], json!({}), "{ping}");
    // Nothing of that reached for the app.
    assert!(!side.connected());
    mcp.close(Duration::from_secs(1)).await.unwrap();
}

#[tokio::test]
async fn modern_requests_without_a_handshake() {
    let (app, mut side) = fake_app();
    let mut mcp = start(app);
    let discover = mcp
        .call(1, "server/discover", json!({"_meta": modern_meta()}))
        .await;
    let result = &discover["result"];
    assert!(
        result["supportedVersions"]
            .as_array()
            .unwrap()
            .contains(&json!("2026-07-28")),
        "{discover}"
    );
    assert!(
        result["supportedVersions"]
            .as_array()
            .unwrap()
            .contains(&json!("2025-11-25")),
        "{discover}"
    );
    assert!(result["capabilities"]["tools"].is_object(), "{discover}");
    assert_eq!(result["resultType"], "complete", "{discover}");
    assert!(result["ttlMs"].is_u64(), "{discover}");
    assert!(result["cacheScope"].is_string(), "{discover}");
    assert_eq!(result["instructions"], crate::agent::INSTRUCTIONS);

    let list = mcp
        .call(2, "tools/list", json!({"_meta": modern_meta()}))
        .await;
    let result = &list["result"];
    assert_eq!(result["resultType"], "complete", "{list}");
    assert!(result["ttlMs"].is_u64(), "{list}");
    // One list for every client: cacheable by anyone.
    assert_eq!(result["cacheScope"], "public", "{list}");
    assert_eq!(result["tools"].as_array().unwrap().len(), 13);

    let mut old = modern_meta();
    old["io.modelcontextprotocol/protocolVersion"] = json!("1900-01-01");
    let refused = mcp.call(3, "tools/list", json!({"_meta": old})).await;
    assert_eq!(refused["error"]["code"], -32022, "{refused}");
    assert!(
        refused["error"]["data"]["supported"]
            .as_array()
            .unwrap()
            .contains(&json!("2026-07-28")),
        "{refused}"
    );

    let bare = mcp.call(4, "tools/list", json!({})).await;
    assert_eq!(bare["error"]["code"], -32602, "{bare}");
    assert!(!side.connected());
    mcp.close(Duration::from_secs(1)).await.unwrap();
}

#[tokio::test]
async fn served_list_is_the_table() {
    // Unreachable for one relay, reachable for the other, legacy and modern: one list.
    let (app, mut side) = fake_app();
    side.refuse(true);
    let mut unreachable = start(app);
    unreachable.open_legacy().await;
    let legacy = unreachable.call(1, "tools/list", json!({})).await;
    let (app, mut other_side) = fake_app();
    let mut reachable = start(app);
    let modern = reachable
        .call(1, "tools/list", json!({"_meta": modern_meta()}))
        .await;
    let served = legacy["result"]["tools"].as_array().unwrap().clone();
    assert_eq!(&served, modern["result"]["tools"].as_array().unwrap());
    assert_eq!(served.len(), tools::all().len());
    for (tool, spec) in served.iter().zip(tools::all()) {
        assert_eq!(tool["name"], spec.name);
        assert_eq!(tool["title"], spec.title, "{}", spec.name);
        assert_eq!(tool["description"], spec.description, "{}", spec.name);
        assert_eq!(tool["inputSchema"], (spec.schema)(), "{}", spec.name);
        let hints = &tool["annotations"];
        assert_eq!(hints["readOnlyHint"], spec.read_only, "{}", spec.name);
        assert_eq!(hints["destructiveHint"], spec.destructive, "{}", spec.name);
        assert_eq!(hints["idempotentHint"], spec.idempotent, "{}", spec.name);
        assert_eq!(hints["openWorldHint"], spec.open_world, "{}", spec.name);
        assert!(tool.get("outputSchema").is_none(), "{}", spec.name);
    }
    assert!(!side.connected() && !other_side.connected());
    unreachable.close(Duration::from_secs(1)).await.unwrap();
    reachable.close(Duration::from_secs(1)).await.unwrap();
}

#[tokio::test]
async fn tool_entries_carry_hints_and_schemas() {
    let (app, _side) = fake_app();
    let mut mcp = start(app);
    mcp.open_legacy().await;
    let list = mcp.call(1, "tools/list", json!({})).await;
    let tools: HashMap<&str, &Value> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["name"].as_str().unwrap(), t))
        .collect();
    let read = tools["read_file"];
    assert_eq!(read["annotations"]["readOnlyHint"], true);
    assert_eq!(read["inputSchema"]["type"], "object");
    assert_eq!(read["title"], "Read a text file");
    let delete = tools["delete"];
    assert_eq!(delete["annotations"]["destructiveHint"], true);
    assert_eq!(delete["annotations"]["readOnlyHint"], false);
    assert_eq!(
        tools["make_directory"]["annotations"]["destructiveHint"],
        false
    );
    mcp.close(Duration::from_secs(1)).await.unwrap();
}

// ---- Calls ----

#[tokio::test]
async fn a_call_goes_to_the_app_and_back() {
    let (app, mut side) = fake_app();
    let mut mcp = start(app);
    mcp.open_legacy().await;
    let arguments = json!({"site": "s1", "path": "/var/www"});
    mcp.request(
        1,
        "tools/call",
        json!({"name": "stat", "arguments": arguments}),
    )
    .await;
    let mut conn = side.greeted().await;
    let (id, tool, args) = conn.call().await;
    assert_eq!((tool.as_str(), &args), ("stat", &arguments));
    conn.ok(id, &["{\"path\":\"/var/www\"}", "second"]).await;
    let (blocks, failed) = texts(&mcp.response(1).await);
    assert_eq!(blocks, ["{\"path\":\"/var/www\"}", "second"]);
    assert!(!failed);

    // The same connection; a failure is a tool error with its text.
    mcp.request(
        2,
        "tools/call",
        json!({"name": "read_file", "arguments": {"site": "s1", "path": "/x"}}),
    )
    .await;
    let (id, tool, _) = conn.call().await;
    assert_eq!(tool, "read_file");
    conn.fail(id, "read /x: No such file").await;
    let (blocks, failed) = texts(&mcp.response(2).await);
    assert_eq!(blocks, ["read /x: No such file"]);
    assert!(failed);
    assert!(!side.connected(), "one connection for both calls");
    mcp.close(Duration::from_secs(1)).await.unwrap();
}

#[tokio::test]
async fn calls_overlap() {
    let (app, mut side) = fake_app();
    let mut mcp = start(app);
    mcp.open_legacy().await;
    mcp.call_tool_later(1, "stat", json!({"site": "a", "path": "/one"}))
        .await;
    let mut conn = side.greeted().await;
    let first = conn.call().await;
    mcp.call_tool_later(2, "stat", json!({"site": "a", "path": "/two"}))
        .await;
    let second = conn.call().await;
    assert_ne!(first.0, second.0, "ids are unique on a connection");
    assert_eq!(first.2["path"], "/one");
    assert_eq!(second.2["path"], "/two");
    // A reply for an id the relay never used is ignored.
    conn.ok(first.0.max(second.0) + 1000, &["stray"]).await;
    conn.ok(second.0, &["two"]).await;
    assert_eq!(texts(&mcp.response(2).await).0, ["two"]);
    conn.ok(first.0, &["one"]).await;
    assert_eq!(texts(&mcp.response(1).await).0, ["one"]);
    // A reply for an id answered already is ignored too.
    conn.ok(first.0, &["again"]).await;
    let ping = mcp.call(3, "ping", json!({})).await;
    assert_eq!(ping["result"], json!({}));
    mcp.close(Duration::from_secs(1)).await.unwrap();
}

impl Mcp {
    async fn call_tool_later(&mut self, id: u64, name: &str, arguments: Value) {
        self.request(
            id,
            "tools/call",
            json!({"name": name, "arguments": arguments}),
        )
        .await;
    }
}

#[tokio::test]
async fn unknown_tool_is_a_protocol_error() {
    let (app, mut side) = fake_app();
    let mut mcp = start(app);
    mcp.open_legacy().await;
    let answer = mcp.call_tool(1, "nope", json!({})).await;
    assert_eq!(answer["error"]["code"], -32602, "{answer}");
    assert_eq!(answer["error"]["message"], "Unknown tool: nope");
    assert!(answer.get("result").is_none());
    assert!(!side.connected(), "the app saw nothing");
    mcp.close(Duration::from_secs(1)).await.unwrap();
}

#[tokio::test]
async fn bad_arguments_are_a_tool_error() {
    let (app, mut side) = fake_app();
    let mut mcp = start(app);
    mcp.open_legacy().await;
    let wrong = json!({"site": 7, "path": ["/"], "extra": true});
    mcp.call_tool_later(1, "stat", wrong.clone()).await;
    let mut conn = side.greeted().await;
    let (id, tool, args) = conn.call().await;
    assert_eq!((tool.as_str(), &args), ("stat", &wrong), "passed as given");
    conn.fail(id, "site must be a string").await;
    let (blocks, failed) = texts(&mcp.response(1).await);
    assert_eq!(blocks, ["site must be a string"]);
    assert!(failed);
    mcp.close(Duration::from_secs(1)).await.unwrap();
}

#[tokio::test]
async fn missing_arguments_reach_the_app_as_an_object() {
    let (app, mut side) = fake_app();
    let mut mcp = start(app);
    mcp.open_legacy().await;
    mcp.request(1, "tools/call", json!({"name": "list_sites"}))
        .await;
    let mut conn = side.greeted().await;
    let (id, tool, args) = conn.call().await;
    assert_eq!((tool.as_str(), &args), ("list_sites", &json!({})));
    conn.ok(id, &["{\"sites\":[]}"]).await;
    assert_eq!(texts(&mcp.response(1).await).0, ["{\"sites\":[]}"]);
    mcp.close(Duration::from_secs(1)).await.unwrap();
}

// ---- The app away, gone, older or off ----

#[tokio::test]
async fn unreachable_app_is_a_tool_error() {
    let (app, side) = fake_app();
    side.refuse(true);
    let mut mcp = start(app);
    mcp.open_legacy().await;
    let list = mcp.call(1, "tools/list", json!({})).await;
    assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 13);
    for id in [2, 3] {
        let (blocks, failed) = texts(&mcp.call_tool(id, "list_sites", json!({})).await);
        assert_eq!(blocks, [UNREACHABLE]);
        assert!(failed);
    }
    mcp.close(Duration::from_secs(1)).await.unwrap();
}

#[tokio::test]
async fn mismatch_and_bye_have_their_own_texts() {
    let (app, mut side) = fake_app();
    let mut mcp = start(app);
    mcp.open_legacy().await;

    // An older window: the mismatch names both versions, and the connection goes.
    mcp.call_tool_later(1, "list_sites", json!({})).await;
    let mut conn = side.accept().await;
    assert!(conn.recv().await.is_some(), "the hello");
    conn.send(json!({"hello": "mismatch", "version": "1.5.0"}))
        .await;
    let (blocks, failed) = texts(&mcp.response(1).await);
    assert_eq!(blocks, [version_mismatch("1.5.0", VERSION)]);
    assert!(failed);
    conn.ends().await;

    // The next call tries again; the switch goes off while it is held.
    mcp.call_tool_later(2, "list_sites", json!({})).await;
    let mut conn = side.greeted().await;
    let _held = conn.call().await;
    side.refuse(true);
    conn.send(json!({"bye": "off"})).await;
    drop(conn);
    let (blocks, failed) = texts(&mcp.response(2).await);
    assert_eq!(blocks, [TURNED_OFF]);
    assert!(failed);
    // ... and the next one, which cannot connect, says the same.
    let (blocks, failed) = texts(&mcp.call_tool(3, "list_sites", json!({})).await);
    assert_eq!(blocks, [TURNED_OFF]);
    assert!(failed);

    // Back on: works again; hoprs quits during a call: that call cannot reach it, nor
    // the next one.
    side.refuse(false);
    mcp.call_tool_later(4, "list_sites", json!({})).await;
    let mut conn = side.greeted().await;
    let (id, ..) = conn.call().await;
    conn.ok(id, &["fine"]).await;
    assert_eq!(
        texts(&mcp.response(4).await),
        (vec!["fine".to_owned()], false)
    );
    mcp.call_tool_later(5, "list_sites", json!({})).await;
    let _held = conn.call().await;
    side.refuse(true);
    conn.send(json!({"bye": "quit"})).await;
    drop(conn);
    assert_eq!(
        texts(&mcp.response(5).await),
        (vec![UNREACHABLE.to_owned()], true)
    );
    assert_eq!(
        texts(&mcp.call_tool(6, "list_sites", json!({})).await),
        (vec![UNREACHABLE.to_owned()], true)
    );
    mcp.close(Duration::from_secs(1)).await.unwrap();
}

#[tokio::test]
async fn reconnects_when_the_app_returns() {
    let (app, mut side) = fake_app();
    let mut mcp = start(app);
    mcp.open_legacy().await;
    mcp.call_tool_later(1, "list_sites", json!({})).await;
    let mut conn = side.greeted().await;
    let _held = conn.call().await;
    // hoprs crashes: the connection is gone without a goodbye.
    drop(conn);
    assert_eq!(
        texts(&mcp.response(1).await),
        (vec![UNREACHABLE.to_owned()], true)
    );
    // It is back: the next call connects again, hello first.
    mcp.call_tool_later(2, "list_sites", json!({})).await;
    let mut conn = side.accept().await;
    conn.greet().await;
    let (id, tool, _) = conn.call().await;
    assert_eq!(tool, "list_sites");
    conn.ok(id, &["{\"sites\":[]}"]).await;
    assert_eq!(
        texts(&mcp.response(2).await),
        (vec!["{\"sites\":[]}".to_owned()], false)
    );
    mcp.close(Duration::from_secs(1)).await.unwrap();
}

#[tokio::test]
async fn an_app_that_never_greets_is_unreachable() {
    tokio::time::pause();
    let (app, mut side) = fake_app();
    let mut mcp = start(app);
    mcp.open_legacy().await;
    mcp.call_tool_later(1, "list_sites", json!({})).await;
    let mut conn = side.accept().await;
    assert!(conn.recv().await.is_some(), "the hello");
    // No answer to the hello: the call does not wait for ever.
    let (blocks, failed) = texts(&mcp.response(1).await);
    assert_eq!(blocks, [UNREACHABLE]);
    assert!(failed);
    conn.ends().await;
    mcp.close(Duration::from_secs(1)).await.unwrap();
}

#[tokio::test]
async fn cancel_reaches_the_app() {
    let (app, mut side) = fake_app();
    let mut mcp = start(app);
    mcp.open_legacy().await;
    mcp.call_tool_later(5, "read_file", json!({"site": "a", "path": "/big"}))
        .await;
    let mut conn = side.greeted().await;
    let (id, ..) = conn.call().await;
    mcp.notify(
        "notifications/cancelled",
        json!({"requestId": 5, "reason": "the user pressed Esc"}),
    )
    .await;
    assert_eq!(conn.recv().await, Some(json!({"cancel": id})));
    // A late answer from the app changes nothing.
    conn.ok(id, &["late"]).await;
    mcp.request(6, "ping", json!({})).await;
    loop {
        let line = mcp.next_line().await.expect("the ping's answer");
        assert_ne!(line["id"], 5, "a response for a cancelled request: {line}");
        if line["id"] == 6 {
            break;
        }
    }
    // The connection stays for the next call.
    mcp.call_tool_later(7, "list_sites", json!({})).await;
    let (id, ..) = conn.call().await;
    conn.ok(id, &["next"]).await;
    assert_eq!(texts(&mcp.response(7).await).0, ["next"]);
    mcp.close(Duration::from_secs(1)).await.unwrap();
}

// ---- Leaving ----

#[tokio::test]
async fn ends_with_its_input() {
    let (app, mut side) = fake_app();
    let mut mcp = start(app);
    mcp.open_legacy().await;
    // A call held at the app when the input closes: the relay still leaves promptly.
    mcp.call_tool_later(1, "list_sites", json!({})).await;
    let mut conn = side.greeted().await;
    let (id, ..) = conn.call().await;
    let started = Instant::now();
    mcp.close(Duration::from_secs(1)).await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    // The app is told, and the link to it is closed.
    assert_eq!(conn.recv().await, Some(json!({"cancel": id})));
    assert_eq!(conn.recv().await, None);
    // What was written is protocol lines only, then the end.
    while mcp.next_line().await.is_some() {}
}

#[tokio::test]
async fn ends_with_its_input_before_any_opening() {
    let (app, _side) = fake_app();
    let mut mcp = start(app);
    mcp.close(Duration::from_secs(1)).await.unwrap();
    assert!(mcp.next_line().await.is_none());
}

#[tokio::test]
async fn output_is_json_rpc_lines_only() {
    let (app, side) = fake_app();
    side.refuse(true);
    let mut mcp = start(app);
    mcp.open_legacy().await;
    mcp.request(1, "tools/list", json!({})).await;
    mcp.request(
        2,
        "tools/call",
        json!({"name": "write_file", "arguments": {"content": "SECRET-BODY"}}),
    )
    .await;
    for junk in [
        "this is not json {",
        "{\"jsonrpc\":\"2.0\",\"id\":",
        "[1,2,3]",
        "{}",
        "\u{1b}[31mred",
        "{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":\"x\"}",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/unknown\"}",
    ] {
        mcp.send_line(junk).await;
    }
    mcp.request(3, "ping", json!({})).await;
    // Read everything as bytes to the end: each line one JSON-RPC message.
    assert_eq!(mcp.response(3).await["result"], json!({}));
    mcp.close(Duration::from_secs(1)).await.unwrap();
    let mut rest = Vec::new();
    step("client: the rest", mcp.output.read_to_end(&mut rest))
        .await
        .unwrap();
    let rest = String::from_utf8(rest).expect("UTF-8 output");
    assert!(rest.is_empty() || rest.ends_with('\n'), "{rest:?}");
    for line in rest.lines() {
        protocol_line(line);
    }
    let seen: Vec<String> = mcp.early.keys().cloned().collect();
    assert!(seen.contains(&"1".to_owned()) && seen.contains(&"2".to_owned()));
}

// ---- The real channel and the real server ----

#[tokio::test]
async fn dial_reaches_the_real_server() {
    use crate::agent::channel;
    use crate::agent::server::AgentServer;
    use crate::agent::service::Service;
    use crate::agent::testing::FakeHost;
    use crate::agent::{AgentEvent, Limits};
    use hoprs_core::{AgentAccess, Protocol, Site};

    let dir = tempfile::tempdir().unwrap();
    let endpoint = channel::endpoint_in(dir.path());
    let host = FakeHost::new();
    let mut site = Site::new("Acme", "acme.example", "u", Protocol::Ssh);
    site.agent_access = AgentAccess::Read;
    host.add_site(site.clone());
    let (tx, mut events) = mpsc::unbounded_channel();
    let service = Service::new(host.clone(), Limits::default(), tx.clone());
    let (_switch, watching) = tokio::sync::watch::channel(true);
    let server = AgentServer::start(service, Some(endpoint.clone()), watching, tx);
    // The channel is open once the server says so.
    loop {
        match step("server: the channel opens", events.recv()).await {
            Some(AgentEvent::Channel(None)) => break,
            Some(AgentEvent::Channel(Some(err))) => panic!("no channel: {err}"),
            Some(_) => {}
            None => panic!("the server stopped"),
        }
    }

    let mut mcp = start(Dial::at(vec![endpoint]));
    mcp.open_legacy().await;
    let (blocks, failed) = texts(&mcp.call_tool(1, "list_sites", json!({})).await);
    assert!(!failed, "{blocks:?}");
    let sites: Value = serde_json::from_str(&blocks[0]).unwrap();
    assert_eq!(sites["sites"][0]["name"], "Acme", "{sites}");
    assert_eq!(sites["sites"][0]["id"], site.id.0.as_str());
    let (blocks, failed) = texts(
        &mcp.call_tool(2, "stat", json!({"site": "nope", "path": "/"}))
            .await,
    );
    assert!(failed);
    assert_eq!(blocks, [crate::agent::SITE_UNAVAILABLE]);

    // hoprs quits: the call after that cannot reach it.
    step("server: shutdown", server.shutdown()).await;
    let (blocks, failed) = texts(&mcp.call_tool(3, "list_sites", json!({})).await);
    assert!(failed);
    assert_eq!(blocks, [UNREACHABLE]);
    mcp.close(Duration::from_secs(1)).await.unwrap();
}

#[tokio::test]
async fn dial_without_a_channel_fails() {
    let dir = tempfile::tempdir().unwrap();
    let dial = Dial::at(vec![crate::agent::channel::endpoint_in(dir.path())]);
    assert!(step("dial", dial.connect()).await.is_err());
    assert!(
        step("dial nothing", Dial::at(Vec::new()).connect())
            .await
            .is_err()
    );
}

// ---- The sources ----

#[test]
fn agent_sources_never_print() {
    // Spelled in pieces, so this file does not match itself.
    let banned = [
        concat!("print", "ln!"),
        concat!("print", "!("),
        concat!("db", "g!("),
    ];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/agent");
    let mut files = Vec::new();
    let mut dirs = vec![root];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                files.push(path);
            }
        }
    }
    assert!(files.len() >= 10, "{files:?}");
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            for pattern in banned {
                let found = code.match_indices(pattern).any(|(at, _)| {
                    // `eprintln!` writes to stderr: only a whole macro name counts.
                    code[..at]
                        .chars()
                        .next_back()
                        .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
                });
                assert!(!found, "{}:{}: {line}", file.display(), n + 1);
            }
        }
    }
}
