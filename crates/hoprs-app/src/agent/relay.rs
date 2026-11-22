//! `hoprs mcp`, the relay: an MCP server on stdio that an MCP client (Claude Code, any
//! other) starts as a subprocess, and that forwards each tool call over the private
//! channel ([`super::channel`], [`super::proto`]) to the running hoprs.
//!
//! **The protocol is the SDK's.** `rmcp` speaks both eras on one stdio process: the
//! legacy one (`initialize`, `notifications/initialized`, `ping`; 2025-11-25 and
//! earlier) and the modern one (2026-07-28: no handshake, each request's version and
//! capabilities in its `_meta`, `server/discover`), and fills each era's result fields.
//! The relay supports every version the SDK knows. It only answers what the protocol
//! leaves to a server: who it is ([`INSTRUCTIONS`]), the tool list and the calls.
//!
//! **The tool list is the table** ([`tools::all`]), in its order, the same for every
//! client and whether or not hoprs runs: it never asks the app. Permissions are the
//! app's, checked on every call.
//!
//! **Calls.** A call of a tool of the table goes to the app as `{"call":id,…}` with the
//! arguments as the client gave them; the app's answer becomes the result (one text
//! block per block, or `isError` with the app's text). Ids are the relay's, unique for
//! its whole life (so never reused on a connection), and calls overlap. A call the client
//! cancels is cancelled at the app (`{"cancel":id}`) and gets no response. An unknown
//! tool is the protocol's error (`-32602`); everything else the agent can act on is a
//! tool error: hoprs not reachable ([`UNREACHABLE`]), agent access turned off
//! ([`TURNED_OFF`]), another version in the window ([`version_mismatch`]).
//!
//! **The link to the app.** One connection, kept while the relay lives and opened by the
//! first call that finds none (the relay never starts hoprs). A mismatch drops it, and
//! the next call tries again. The app's `{"bye":"off"}` answers the calls in flight, and
//! the next calls that cannot connect, with [`TURNED_OFF`]; `{"bye":"quit"}` or a
//! connection lost answers the calls in flight with [`UNREACHABLE`], and the next call
//! connects again. The relay reads the app's lines all the time.
//!
//! **Leaving.** The relay ends when its input ends (end-of-file or a read error): calls
//! still in flight get [`INPUT_END_GRACE`], then are cancelled at the app; the link is
//! closed and [`serve`] returns.
//!
//! **Stdout carries protocol messages only.** The SDK writes them; nothing in this
//! module, or under `agent/`, prints (a test reads the sources). Nothing here logs a
//! tool's arguments or results.

use std::collections::HashMap;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use hoprs_remote::{BoxAsyncRead, BoxAsyncWrite};
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
    Implementation, ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities,
    ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::{RequestContext, ServerInitializeError};
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, ReadBuf};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::channel::{self, Endpoint};
use super::proto::{self, ByeReason, Hello, HelloReply, Reply, ToApp, ToRelay};
use super::tools::{self, ToolSpec};
use super::{HELLO_WAIT, INSTRUCTIONS, TURNED_OFF, UNREACHABLE, version_mismatch};

/// This program's version: the relay's in its hello, the server's in `serverInfo`.
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The longest a call waits for the app's channel to connect and answer the hello.
const OPEN_WAIT: Duration = HELLO_WAIT;
/// What calls in flight when the input ends get to finish, before they are cancelled.
/// Short: Claude Code on Windows ends the process tree 300 ms after closing stdin.
pub const INPUT_END_GRACE: Duration = Duration::from_millis(250);
/// The longest the link takes, as it closes, to hand the app what it still has queued
/// (the cancels).
const CLOSE_WAIT: Duration = Duration::from_secs(1);
/// How long a modern client may keep the tool list (an hour; it never changes while
/// this program runs).
const LIST_TTL_MS: u64 = 60 * 60 * 1000;
/// A call still in flight when the client closed the relay's input.
const STOPPING: &str = "hoprs mcp is stopping: the MCP client closed its input before this call ended, and the call was cancelled.";
/// A call whose arguments do not fit in one line of the channel.
const CALL_TOO_LONG: &str = "The call was too long for hoprs's channel (over 8 MiB). Send less in one call: write_file takes at most 1 MiB, and upload sends a whole file.";

/// How the relay reaches the app: a connection of the channel, as its two halves.
pub trait Connect: Send + Sync + 'static {
    fn connect(&self) -> BoxFuture<'_, io::Result<(BoxAsyncRead, BoxAsyncWrite)>>;
}

/// The real way to the app: the first of the channel's candidates that connects (see
/// [`channel::candidates`]; [`channel::connect`] checks that the other end is this user's).
pub struct Dial {
    endpoints: Vec<Endpoint>,
}

impl Dial {
    /// The channels of this user's hoprs, in the order [`channel::candidates`] gives.
    pub fn standard() -> Dial {
        Dial {
            endpoints: channel::candidates(),
        }
    }

    /// These channels, in order.
    #[cfg(test)]
    pub fn at(endpoints: Vec<Endpoint>) -> Dial {
        Dial { endpoints }
    }
}

impl Connect for Dial {
    fn connect(&self) -> BoxFuture<'_, io::Result<(BoxAsyncRead, BoxAsyncWrite)>> {
        async move {
            let mut last = io::Error::new(io::ErrorKind::NotFound, "no channel to hoprs");
            for endpoint in &self.endpoints {
                match channel::connect(endpoint).await {
                    Ok(stream) => {
                        let (read, write) = tokio::io::split(stream);
                        return Ok((
                            Box::new(read) as BoxAsyncRead,
                            Box::new(write) as BoxAsyncWrite,
                        ));
                    }
                    Err(err) => last = err,
                }
            }
            Err(last)
        }
        .boxed()
    }
}

/// Serves MCP on `streams` (the client's input, then where the relay writes) until the
/// input ends, reaching the app through `connect`. `Ok` when the input ended (also
/// before any opening); an error when the streams could not be used or the client's
/// opening could not be served.
pub async fn serve<R, W>(streams: (R, W), connect: impl Connect) -> io::Result<()>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let (read, write) = streams;
    let ended = CancellationToken::new();
    let link = Arc::new(Link::new(Box::new(connect), ended.clone()));
    let relay = Relay { link: link.clone() };
    let read = UntilEnd {
        inner: read,
        ended: ended.clone(),
    };
    let served = match relay.serve((read, write)).await {
        Ok(running) => match running.waiting().await {
            Ok(_) => Ok(()),
            Err(_) => Err(io::Error::other("the MCP service failed")),
        },
        // The input ended before (or during) the opening.
        Err(ServerInitializeError::ConnectionClosed(_)) => Ok(()),
        Err(err) => Err(opening_failed(&err)),
    };
    link.close().await;
    served
}

/// Why the opening failed, in a fixed text: the SDK's own error can quote a message.
fn opening_failed(err: &ServerInitializeError) -> io::Error {
    let what = match err {
        ServerInitializeError::ExpectedInitializeRequest(_) => {
            "the MCP client's first message was not a request"
        }
        ServerInitializeError::InitializeFailed(_) => "the MCP client's opening was refused",
        ServerInitializeError::TransportError { .. } => {
            "a message to the MCP client could not be written"
        }
        _ => "the MCP client's opening could not be served",
    };
    io::Error::other(what)
}

/// The MCP server's handler: what the protocol leaves to the server.
pub struct Relay {
    link: Arc<Link>,
}

impl ServerHandler for Relay {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("hoprs", VERSION))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let mut result = ListToolsResult::with_all_items(tools::all().iter().map(tool).collect());
        // The same list for every client: cacheable by anyone (the SDK's default is
        // private and fresh for 0 ms). Legacy results carry no cache fields.
        if is_modern(&context) {
            result.ttl_ms = Some(LIST_TTL_MS);
            result.cache_scope = Some(CacheScope::Public);
        }
        Ok(result)
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        tools::find(name).map(tool)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let Some(spec) = tools::find(&request.name) else {
            return Err(ErrorData::invalid_params(
                format!("Unknown tool: {}", request.name),
                None,
            ));
        };
        // The arguments go as given: the app checks them and says what is wrong.
        let args = Value::Object(request.arguments.unwrap_or_default());
        let result = match self.link.call(spec.name, args, &context.ct).await {
            Answer::Blocks(blocks) => {
                CallToolResult::success(blocks.into_iter().map(ContentBlock::text).collect())
            }
            Answer::Error(text) => CallToolResult::error(vec![ContentBlock::text(text)]),
            // The SDK sends nothing for a cancelled request.
            Answer::Cancelled => return Err(ErrorData::internal_error("cancelled", None)),
        };
        Ok(result.into())
    }
}

/// Whether this request is served in the modern era (2026-07-28 or later).
fn is_modern(context: &RequestContext<RoleServer>) -> bool {
    context
        .protocol_version()
        .is_some_and(|v| v.as_str() >= ProtocolVersion::V_2026_07_28.as_str())
}

/// The protocol's entry for a tool of the table.
fn tool(spec: &ToolSpec) -> Tool {
    let schema = match (spec.schema)() {
        Value::Object(schema) => schema,
        // Every schema of the table is an object (`tools` tests it).
        _ => serde_json::Map::new(),
    };
    Tool::new(spec.name, spec.description, Arc::new(schema))
        .with_title(spec.title)
        .with_annotations(
            ToolAnnotations::new()
                .read_only(spec.read_only)
                .destructive(spec.destructive)
                .idempotent(spec.idempotent)
                .open_world(spec.open_world),
        )
}

/// How a call ended, for the client.
enum Answer {
    Blocks(Vec<String>),
    Error(String),
    /// The client cancelled it: nothing is sent.
    Cancelled,
}

/// How a call in flight on a connection ended.
enum Outcome {
    Reply(Reply),
    /// Its arguments did not fit in one line of the channel.
    TooLong,
    /// The connection ended first.
    Ended(Fate),
}

/// How a connection ended.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fate {
    /// `{"bye":"off"}`: the user turned agent access off.
    Off,
    /// `{"bye":"quit"}`, the connection lost or closed by the relay.
    Lost,
}

impl Outcome {
    fn answer(self) -> Answer {
        match self {
            Outcome::Reply(Reply::Ok(blocks)) => Answer::Blocks(blocks),
            Outcome::Reply(Reply::Err(text)) => Answer::Error(text),
            Outcome::TooLong => Answer::Error(CALL_TOO_LONG.to_owned()),
            Outcome::Ended(fate) => fate.answer(),
        }
    }
}

impl Fate {
    fn answer(self) -> Answer {
        Answer::Error(
            match self {
                Fate::Off => TURNED_OFF,
                Fate::Lost => UNREACHABLE,
            }
            .to_owned(),
        )
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The relay's link to the app (see the module docs).
struct Link {
    connect: Box<dyn Connect>,
    /// The connection, while there is one; a dead one is replaced by the next call.
    current: Mutex<Option<Arc<Conn>>>,
    /// One opening at a time: calls that find no connection wait for the same one.
    opening: tokio::sync::Mutex<()>,
    /// The last connection ended with `{"bye":"off"}` and none opened since.
    off: Arc<AtomicBool>,
    next_id: AtomicU64,
    /// The client's input ended.
    input_ended: CancellationToken,
}

/// What an opening found.
enum Opened {
    Ready(Arc<Conn>),
    /// The window runs this version.
    Mismatch(String),
}

impl Link {
    fn new(connect: Box<dyn Connect>, input_ended: CancellationToken) -> Link {
        Link {
            connect,
            current: Mutex::new(None),
            opening: tokio::sync::Mutex::new(()),
            off: Arc::new(AtomicBool::new(false)),
            next_id: AtomicU64::new(1),
            input_ended,
        }
    }

    /// Calls `tool` at the app and waits for its answer.
    async fn call(&self, tool: &str, args: Value, ct: &CancellationToken) -> Answer {
        let conn = tokio::select! {
            conn = self.conn() => conn,
            _ = ct.cancelled() => return Answer::Cancelled,
            _ = self.stopping() => return Answer::Error(STOPPING.to_owned()),
        };
        let conn = match conn {
            Ok(conn) => conn,
            Err(text) => return Answer::Error(text),
        };
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let answered = match conn.register(id) {
            Ok(answered) => answered,
            Err(fate) => return fate.answer(),
        };
        let call = ToApp::Call {
            call: id,
            tool: tool.to_owned(),
            args,
        };
        if conn.out.send(call).is_err() {
            conn.forget(id);
            return Fate::Lost.answer();
        }
        tokio::select! {
            outcome = answered => match outcome {
                Ok(outcome) => outcome.answer(),
                Err(_) => Fate::Lost.answer(),
            },
            _ = ct.cancelled() => {
                conn.cancel(id);
                Answer::Cancelled
            }
            _ = self.stopping() => {
                conn.cancel(id);
                Answer::Error(STOPPING.to_owned())
            }
        }
    }

    /// Ends [`INPUT_END_GRACE`] after the input ended.
    async fn stopping(&self) {
        self.input_ended.cancelled().await;
        tokio::time::sleep(INPUT_END_GRACE).await;
    }

    /// The live connection, opening one if there is none; else the text for the agent.
    async fn conn(&self) -> Result<Arc<Conn>, String> {
        if let Some(conn) = self.live() {
            return Ok(conn);
        }
        let _turn = self.opening.lock().await;
        if let Some(conn) = self.live() {
            return Ok(conn);
        }
        match tokio::time::timeout(OPEN_WAIT, self.open()).await {
            Ok(Ok(Opened::Ready(conn))) => {
                self.off.store(false, Ordering::SeqCst);
                *lock(&self.current) = Some(conn.clone());
                tracing::info!("connected to hoprs");
                Ok(conn)
            }
            Ok(Ok(Opened::Mismatch(app))) => {
                tracing::warn!(app = %app, relay = VERSION, "hoprs runs another version");
                Err(version_mismatch(&app, VERSION))
            }
            Ok(Err(_)) | Err(_) => Err(if self.off.load(Ordering::SeqCst) {
                TURNED_OFF
            } else {
                UNREACHABLE
            }
            .to_owned()),
        }
    }

    fn live(&self) -> Option<Arc<Conn>> {
        lock(&self.current)
            .as_ref()
            .filter(|conn| !conn.dead.is_cancelled())
            .cloned()
    }

    /// Connects and says hello.
    async fn open(&self) -> io::Result<Opened> {
        let (read, mut write) = self.connect.connect().await?;
        let mut reader = BufReader::new(read);
        let hello = Hello {
            hoprs_agent: proto::PROTOCOL,
            version: VERSION.to_owned(),
        };
        proto::write_line(&mut write, &hello).await?;
        match proto::read_message::<_, HelloReply>(&mut reader).await? {
            Some(HelloReply::Ok { .. }) => {
                Ok(Opened::Ready(Conn::start(reader, write, self.off.clone())))
            }
            // The app closes after a mismatch; so does the relay (both halves dropped).
            Some(HelloReply::Mismatch { version }) => Ok(Opened::Mismatch(version)),
            None => Err(io::ErrorKind::UnexpectedEof.into()),
        }
    }

    /// Closes the connection, if any: what is queued for the app is handed on (within
    /// [`CLOSE_WAIT`]), then the app sees the end. Returns when it is closed.
    async fn close(&self) {
        let conn = lock(&self.current).take();
        if let Some(conn) = conn {
            conn.dead.cancel();
            let task = lock(&conn.task).take();
            if let Some(task) = task {
                let _ = task.await;
            }
        }
    }
}

/// The calls waiting on a connection, or how it ended.
enum Pending {
    Open(HashMap<u64, oneshot::Sender<Outcome>>),
    Closed(Fate),
}

/// One connection to the app, after its hello.
struct Conn {
    /// To the connection's writer.
    out: mpsc::UnboundedSender<ToApp>,
    pending: Mutex<Pending>,
    /// Cancelled once the connection is over (or the relay closes it).
    dead: CancellationToken,
    /// The connection's task (its reader; it owns the writer).
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Conn {
    fn start(
        reader: BufReader<BoxAsyncRead>,
        write: BoxAsyncWrite,
        off: Arc<AtomicBool>,
    ) -> Arc<Conn> {
        let (out, outgoing) = mpsc::unbounded_channel();
        let conn = Arc::new(Conn {
            out,
            pending: Mutex::new(Pending::Open(HashMap::new())),
            dead: CancellationToken::new(),
            task: Mutex::new(None),
        });
        let task = tokio::spawn(drive(conn.clone(), reader, write, outgoing, off));
        *lock(&conn.task) = Some(task);
        conn
    }

    /// Waits for call `id`'s outcome; the connection's fate if it is over.
    fn register(&self, id: u64) -> Result<oneshot::Receiver<Outcome>, Fate> {
        match &mut *lock(&self.pending) {
            Pending::Open(calls) => {
                let (tx, rx) = oneshot::channel();
                calls.insert(id, tx);
                Ok(rx)
            }
            Pending::Closed(fate) => Err(*fate),
        }
    }

    fn deliver(&self, id: u64, outcome: Outcome) {
        if let Pending::Open(calls) = &mut *lock(&self.pending)
            && let Some(waiting) = calls.remove(&id)
        {
            let _ = waiting.send(outcome);
        }
    }

    fn forget(&self, id: u64) {
        if let Pending::Open(calls) = &mut *lock(&self.pending) {
            calls.remove(&id);
        }
    }

    /// Stops waiting for call `id` and cancels it at the app.
    fn cancel(&self, id: u64) {
        self.forget(id);
        let _ = self.out.send(ToApp::Cancel { cancel: id });
    }

    /// The connection is over: every call waiting on it ends with `fate`.
    fn end(&self, fate: Fate) {
        let before = std::mem::replace(&mut *lock(&self.pending), Pending::Closed(fate));
        if let Pending::Open(calls) = before {
            for waiting in calls.into_values() {
                let _ = waiting.send(Outcome::Ended(fate));
            }
        }
    }
}

/// A connection's life: reads the app's lines all the time (answers by id; an unknown id
/// is ignored), until a goodbye, the end, a line that is not a message, the writer's
/// failure or the relay closing it.
async fn drive(
    conn: Arc<Conn>,
    mut reader: BufReader<BoxAsyncRead>,
    write: BoxAsyncWrite,
    outgoing: mpsc::UnboundedReceiver<ToApp>,
    off: Arc<AtomicBool>,
) {
    let mut writer = tokio::spawn(write_calls(write, outgoing, conn.clone()));
    let mut writer_done = false;
    let fate = loop {
        tokio::select! {
            message = proto::read_message::<_, ToRelay>(&mut reader) => match message {
                Ok(Some(ToRelay::Bye { bye: ByeReason::Off })) => break Fate::Off,
                Ok(Some(ToRelay::Bye { bye: ByeReason::Quit })) => break Fate::Lost,
                Ok(Some(answer)) => {
                    if let Some((id, reply)) = answer.into_reply() {
                        conn.deliver(id, Outcome::Reply(reply));
                    }
                }
                Ok(None) | Err(_) => break Fate::Lost,
            },
            _ = &mut writer => {
                writer_done = true;
                break Fate::Lost;
            }
            _ = conn.dead.cancelled() => break Fate::Lost,
        }
    };
    // In this order: a call that finds the connection dead opens another, and that one
    // decides the flag after this.
    off.store(fate == Fate::Off, Ordering::SeqCst);
    conn.end(fate);
    conn.dead.cancel();
    tracing::info!(off = fate == Fate::Off, "the connection to hoprs ended");
    if !writer_done && tokio::time::timeout(CLOSE_WAIT, &mut writer).await.is_err() {
        writer.abort();
    }
}

/// A connection's writer: each call or cancel as one line; once the connection is dead,
/// what is still queued, then the end of the stream.
async fn write_calls(
    mut write: BoxAsyncWrite,
    mut outgoing: mpsc::UnboundedReceiver<ToApp>,
    conn: Arc<Conn>,
) {
    loop {
        let message = tokio::select! {
            biased;
            Some(message) = outgoing.recv() => message,
            _ = conn.dead.cancelled() => break,
        };
        match proto::write_line(&mut write, &message).await {
            Ok(()) => {}
            // Too long for a line: nothing was written, and only this call fails.
            Err(err) if err.kind() == io::ErrorKind::InvalidInput => {
                if let ToApp::Call { call, .. } = message {
                    conn.deliver(call, Outcome::TooLong);
                }
            }
            Err(_) => return,
        }
    }
    let _ = write.shutdown().await;
}

/// The client's input, which tells when it ends (end-of-file or a read error).
struct UntilEnd<R> {
    inner: R,
    ended: CancellationToken,
}

impl<R: AsyncRead + Unpin> AsyncRead for UntilEnd<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let wanted = buf.remaining() > 0;
        let polled = Pin::new(&mut self.inner).poll_read(cx, buf);
        match &polled {
            Poll::Ready(Ok(())) if wanted && buf.filled().len() == before => self.ended.cancel(),
            Poll::Ready(Err(_)) => self.ended.cancel(),
            _ => {}
        }
        polled
    }
}

#[cfg(test)]
mod tests;
