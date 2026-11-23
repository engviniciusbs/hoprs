//! Interactive PTY shell channel.
//!
//! [`SshSession::open_shell`] gives a [`ShellChannel`]; [`ShellChannel::into_output`] splits
//! it into a cloneable [`ShellHandle`] (write, resize, close) and an owned `'static` output
//! stream, which can feed an iced `Subscription`.
//!
//! The output pump never blocks on the consumer: a slow reader cannot stall the connection
//! (see `MAX_PENDING_OUTPUT`). Closing never waits for the server either: [`ShellHandle::close`]
//! only signals the pump, which ends the output and sends the channel close in the
//! background. Each shell holds one slot of the connection's channel budget
//! ([`crate::channels`]) until it ends. Terminal bytes are never logged.

use std::collections::VecDeque;
use std::fmt;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::Stream;
use hoprs_core::{Error, ErrorKind, Result};
use russh::client::Msg;
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf};
use tokio::sync::{Notify, mpsc};
use tokio::time::Instant;

use crate::channels::ChannelSlot;
use crate::session::{SshSession, map_russh_error};

/// Capacity of the output queue handed to the consumer.
const OUTPUT_QUEUE: usize = 256;

/// Most output that may wait for a slow consumer. russh delivers channel data from the single
/// session task, so the pump must never block on the consumer (a stalled terminal tab would
/// freeze SFTP, keepalives and every other channel). While the queue is full the pump keeps
/// reading and coalesces `Data`; past this cap the channel is closed and the excess dropped.
/// A terminal that is not drained for that long is torn down rather than stalling the connection.
const MAX_PENDING_OUTPUT: usize = 16 * 1024 * 1024;

/// Largest `Data` item the pump builds by coalescing. With the 256-slot queue this bounds the
/// output held for a consumer at 256 x 64 KiB = 16 MiB queued, plus at most `MAX_PENDING_OUTPUT`
/// waiting behind a full queue.
const MAX_COALESCED_CHUNK: usize = 64 * 1024;

/// How long after an `Eof` the server may take to send `Close` before we close the channel.
const EOF_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the server may take to answer the PTY and shell requests.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Terminal request for [`SshSession::open_shell`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtyRequest {
    pub term: String,
    pub cols: u32,
    pub rows: u32,
    /// Environment variables sent before the shell starts. A server that refuses one (no
    /// matching `AcceptEnv`, the OpenSSH default) is ignored: the shell opens without it.
    pub env: Vec<(String, String)>,
}

impl Default for PtyRequest {
    fn default() -> Self {
        Self {
            term: "xterm-256color".to_owned(),
            cols: 80,
            rows: 24,
            env: vec![("COLORTERM".to_owned(), "truecolor".to_owned())],
        }
    }
}

/// What a shell produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellOutput {
    Data(Vec<u8>),
    Exit(Option<u32>),
    /// The consumer let more than `MAX_PENDING_OUTPUT` pile up: the excess was dropped and
    /// the shell is being closed. Comes right before `Closed`, so the consumer can say why
    /// the shell ended instead of losing output silently.
    Overflowed,
    Closed,
}

/// The sending side of a shell channel (a trait so the pump can be tested without a server).
#[async_trait]
pub(crate) trait ShellWriter: Send + Sync + 'static {
    async fn data(&self, data: Vec<u8>) -> std::result::Result<(), russh::Error>;
    async fn window_change(&self, cols: u32, rows: u32) -> std::result::Result<(), russh::Error>;
    async fn close(&self) -> std::result::Result<(), russh::Error>;
    /// Best-effort `signal KILL` request (no reply wanted). Only exec channels use it.
    async fn kill(&self) -> std::result::Result<(), russh::Error> {
        Ok(())
    }
}

#[async_trait]
impl ShellWriter for ChannelWriteHalf<Msg> {
    async fn data(&self, data: Vec<u8>) -> std::result::Result<(), russh::Error> {
        self.data_bytes(data).await
    }

    async fn window_change(&self, cols: u32, rows: u32) -> std::result::Result<(), russh::Error> {
        ChannelWriteHalf::window_change(self, cols, rows, 0, 0).await
    }

    async fn close(&self) -> std::result::Result<(), russh::Error> {
        ChannelWriteHalf::close(self).await
    }

    async fn kill(&self) -> std::result::Result<(), russh::Error> {
        self.signal(russh::Sig::KILL).await
    }
}

/// The receiving side of a shell channel.
#[async_trait]
pub(crate) trait ShellReader: Send + 'static {
    async fn wait(&mut self) -> Option<ChannelMsg>;
}

#[async_trait]
impl ShellReader for ChannelReadHalf {
    async fn wait(&mut self) -> Option<ChannelMsg> {
        ChannelReadHalf::wait(self).await
    }
}

/// An open shell on a PTY, before [`into_output`](Self::into_output). Dropping it closes the
/// remote channel.
#[derive(Debug)]
pub struct ShellChannel {
    handle: ShellHandle,
    rx: mpsc::Receiver<ShellOutput>,
}

/// Controls a shell: write, resize, close. Cheap to clone; the shell closes when the last
/// clone is dropped (its output then ends with `Closed`).
#[derive(Clone)]
pub struct ShellHandle {
    inner: Arc<HandleInner>,
}

struct HandleInner {
    writer: Arc<dyn ShellWriter>,
    closed: Arc<AtomicBool>,
    overflowed: Arc<AtomicBool>,
    stop: Arc<Notify>,
}

impl Drop for HandleInner {
    fn drop(&mut self) {
        // The pump closes the channel; the permit is kept if it is not waiting yet.
        self.stop.notify_one();
    }
}

impl fmt::Debug for ShellHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShellHandle")
            .field("closed", &self.inner.closed.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

fn shell_closed() -> Error {
    Error::coded(ErrorKind::Network, "ssh-shell-closed", "Shell closed")
}

impl ShellChannel {
    /// Starts the pump over an opened channel. `slot` is released when the shell ends.
    pub(crate) fn start(
        reader: Box<dyn ShellReader>,
        writer: Arc<dyn ShellWriter>,
        slot: Option<ChannelSlot>,
    ) -> ShellChannel {
        let (tx, rx) = mpsc::channel(OUTPUT_QUEUE);
        let closed = Arc::new(AtomicBool::new(false));
        let overflowed = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(Notify::new());
        tokio::spawn(pump(
            Pump {
                reader: Some(reader),
                writer: writer.clone(),
                closed: closed.clone(),
                overflowed: overflowed.clone(),
                slot,
            },
            tx,
            stop.clone(),
        ));
        ShellChannel {
            handle: ShellHandle {
                inner: Arc::new(HandleInner {
                    writer,
                    closed,
                    overflowed,
                    stop,
                }),
            },
            rx,
        }
    }

    /// Splits the shell into its handle and its output: stdout and stderr merged as `Data`,
    /// then `Exit` when the server reports one (or [`ShellOutput::Overflowed`] when the
    /// consumer fell too far behind), then exactly one [`ShellOutput::Closed`] (channel
    /// closed, connection lost, closed by us, or torn down after overflowing), after which
    /// the stream ends. Reconnecting a shell is up to the caller.
    ///
    /// Dropping the stream closes the shell too.
    pub fn into_output(
        self,
    ) -> (
        ShellHandle,
        impl Stream<Item = ShellOutput> + Send + Unpin + 'static,
    ) {
        (self.handle, OutputStream(self.rx))
    }
}

/// The output of a shell; see [`ShellChannel::into_output`].
struct OutputStream(mpsc::Receiver<ShellOutput>);

impl Stream for OutputStream {
    type Item = ShellOutput;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<ShellOutput>> {
        self.0.poll_recv(cx)
    }
}

impl ShellHandle {
    fn check_open(&self) -> Result<()> {
        if self.inner.closed.load(Ordering::SeqCst) {
            Err(shell_closed())
        } else {
            Ok(())
        }
    }

    /// Sends `data` to the shell's stdin (the terminal input). Fails with `Network`, "Shell
    /// closed" once the shell ended.
    pub async fn write(&self, data: &[u8]) -> Result<()> {
        self.check_open()?;
        self.inner
            .writer
            .data(data.to_vec())
            .await
            .map_err(map_send)
    }

    /// Tells the server the terminal is now `cols` x `rows` characters (window-change).
    pub async fn resize(&self, cols: u32, rows: u32) -> Result<()> {
        self.check_open()?;
        self.inner
            .writer
            .window_change(cols, rows)
            .await
            .map_err(map_send)
    }

    /// Closes the shell without waiting for the server: the output ends with `Closed` and
    /// the channel close is sent in the background. Closing a closed shell does nothing.
    pub fn close(&self) {
        if !self.inner.closed.swap(true, Ordering::SeqCst) {
            self.inner.stop.notify_one();
        }
    }

    /// Whether the pump gave up on a consumer that let `MAX_PENDING_OUTPUT` pile up.
    /// For tests.
    #[doc(hidden)]
    pub fn overflowed(&self) -> bool {
        self.inner.overflowed.load(Ordering::SeqCst)
    }
}

fn map_send(e: russh::Error) -> Error {
    match e {
        russh::Error::SendError => shell_closed(),
        other => map_russh_error(other),
    }
}

/// Asks the server to close the channel without waiting for the request to be queued: the
/// handle-to-session channel can be full while the session task is busy delivering data.
fn close_detached(writer: &Arc<dyn ShellWriter>) {
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        let writer = writer.clone();
        runtime.spawn(async move {
            let _ = writer.close().await;
        });
    }
}

fn pty_refused() -> Error {
    Error::coded(
        ErrorKind::Protocol,
        "ssh-pty-refused",
        "Server refused the PTY/shell request",
    )
}

/// Waits for the reply to a channel request (`Success`/`Failure`), skipping other messages.
/// `refused` is the error for a `Failure`.
pub(crate) async fn expect_reply(
    reader: &mut ChannelReadHalf,
    what: &str,
    refused: Error,
) -> Result<()> {
    let wait = async {
        loop {
            match reader.wait().await {
                Some(ChannelMsg::Success) => return Ok(()),
                Some(ChannelMsg::Failure) => return Err(refused),
                Some(ChannelMsg::Close | ChannelMsg::Eof) | None => {
                    return Err(Error::coded(
                        ErrorKind::Network,
                        "ssh-closed-during-request",
                        format!("Connection closed during the {what} request"),
                    )
                    .arg("what", what));
                }
                Some(_) => {}
            }
        }
    };
    tokio::time::timeout(REQUEST_TIMEOUT, wait)
        .await
        .unwrap_or_else(|_| {
            Err(Error::coded(
                ErrorKind::Network,
                "ssh-shell-request-timed-out",
                "Shell request timed out",
            ))
        })
}

/// Output waiting for the consumer, in order. Consecutive `Data` are coalesced.
#[derive(Default)]
struct Pending {
    items: VecDeque<ShellOutput>,
    bytes: usize,
}

impl Pending {
    /// Queues `data`; returns false (dropping it) when the cap would be exceeded.
    fn push_data(&mut self, data: Vec<u8>) -> bool {
        if self.bytes + data.len() > MAX_PENDING_OUTPUT {
            return false;
        }
        self.bytes += data.len();
        match self.items.back_mut() {
            Some(ShellOutput::Data(last)) if last.len() + data.len() <= MAX_COALESCED_CHUNK => {
                last.extend_from_slice(&data);
            }
            _ => self.items.push_back(ShellOutput::Data(data)),
        }
        true
    }

    fn pop(&mut self) -> Option<ShellOutput> {
        let item = self.items.pop_front()?;
        if let ShellOutput::Data(d) = &item {
            self.bytes -= d.len();
        }
        Some(item)
    }
}

/// What the pump owns.
struct Pump {
    /// `None` once the shell ended (nothing more is read).
    reader: Option<Box<dyn ShellReader>>,
    writer: Arc<dyn ShellWriter>,
    closed: Arc<AtomicBool>,
    overflowed: Arc<AtomicBool>,
    /// The shell's place in the channel budget, given back when it ends.
    slot: Option<ChannelSlot>,
}

impl Pump {
    /// Marks the shell closed, queues `Closed` after everything else, stops reading (so
    /// russh is never left waiting on this channel) and frees the budget slot. With
    /// `close_channel`, also asks the server to close the channel, in the background.
    fn finish(&mut self, pending: &mut Pending, close_channel: bool) {
        self.closed.store(true, Ordering::SeqCst);
        self.reader = None;
        self.slot = None;
        pending.items.push_back(ShellOutput::Closed);
        if close_channel {
            close_detached(&self.writer);
        }
    }
}

/// Forwards channel messages to the consumer without ever blocking on it: it selects between
/// reading from russh and delivering queued output.
/// `stop` is signalled by [`ShellHandle::close`] and by dropping the last handle.
async fn pump(mut p: Pump, tx: mpsc::Sender<ShellOutput>, stop: Arc<Notify>) {
    let mut pending = Pending::default();
    let mut eof_at: Option<Instant> = None;
    loop {
        let reading = p.reader.is_some();
        tokio::select! {
            permit = tx.reserve(), if !pending.items.is_empty() => {
                let Ok(permit) = permit else {
                    // The consumer dropped the output.
                    if reading {
                        p.finish(&mut pending, true);
                    }
                    break;
                };
                let item = pending.pop().expect("queue is not empty");
                let last = item == ShellOutput::Closed;
                permit.send(item);
                if last {
                    break;
                }
            }
            msg = async { p.reader.as_mut().expect("reading").wait().await }, if reading => {
                match msg {
                    Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                        if !pending.push_data(data.to_vec()) {
                            p.overflowed.store(true, Ordering::SeqCst);
                            pending.items.push_back(ShellOutput::Overflowed);
                            // Stop reading first: closing may have to wait for the session
                            // task, which may in turn be waiting for this channel to take
                            // its data.
                            p.finish(&mut pending, true);
                        }
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        pending.items.push_back(ShellOutput::Exit(Some(exit_status)));
                    }
                    Some(ChannelMsg::ExitSignal { .. }) => {
                        pending.items.push_back(ShellOutput::Exit(None));
                    }
                    Some(ChannelMsg::Eof) => {
                        eof_at.get_or_insert_with(|| Instant::now() + EOF_CLOSE_TIMEOUT);
                    }
                    Some(ChannelMsg::Close) | None => p.finish(&mut pending, false),
                    Some(_) => {}
                }
            }
            () = async { tokio::time::sleep_until(eof_at.expect("armed")).await }, if reading && eof_at.is_some() => {
                p.finish(&mut pending, true);
            }
            () = stop.notified(), if reading => {
                // Closed by us (or every handle dropped): do not wait for the server.
                p.finish(&mut pending, true);
            }
            () = tx.closed(), if reading && pending.items.is_empty() => {
                // The consumer dropped the output: nobody will read this shell again.
                p.finish(&mut pending, true);
            }
        }
    }
    p.closed.store(true, Ordering::SeqCst);
}

impl SshSession {
    /// Opens a channel (one slot of the channel budget, see [`crate::channels`]), requests a
    /// PTY (`req`, default terminal modes), sends `req.env` (refusals ignored) and starts a
    /// shell, then forwards the shell's output. While the session is not connected this
    /// fails at once with `Network`, "Not connected"; past the channel budget, or when the
    /// server refuses the channel, with `Protocol`, "Too many channels on this connection
    /// (limit N)".
    pub async fn open_shell(&self, req: PtyRequest) -> Result<ShellChannel> {
        let (channel, slot) = self.open_pty_channel().await?.into_parts();
        let (mut reader, writer) = channel.split();
        writer
            .request_pty(true, &req.term, req.cols, req.rows, 0, 0, &[])
            .await
            .map_err(map_russh_error)?;
        expect_reply(&mut reader, "pty", pty_refused()).await?;
        for (name, value) in &req.env {
            // No reply asked: a refusal (the usual `AcceptEnv` default) is silently ignored.
            writer
                .set_env(false, name.as_str(), value.as_str())
                .await
                .map_err(map_russh_error)?;
        }
        writer.request_shell(true).await.map_err(map_russh_error)?;
        expect_reply(&mut reader, "shell", pty_refused()).await?;
        Ok(ShellChannel::start(
            Box::new(reader),
            Arc::new(writer),
            Some(slot),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_coalesces_data_and_keeps_order() {
        let mut p = Pending::default();
        assert!(p.push_data(b"ab".to_vec()));
        assert!(p.push_data(b"cd".to_vec()));
        p.items.push_back(ShellOutput::Exit(Some(1)));
        assert!(p.push_data(b"ef".to_vec()));
        assert_eq!(p.bytes, 6);
        assert_eq!(p.pop(), Some(ShellOutput::Data(b"abcd".to_vec())));
        assert_eq!(p.pop(), Some(ShellOutput::Exit(Some(1))));
        assert_eq!(p.pop(), Some(ShellOutput::Data(b"ef".to_vec())));
        assert_eq!(p.bytes, 0);
        assert_eq!(p.pop(), None);
    }

    #[test]
    fn pending_chunks_stay_bounded() {
        let mut p = Pending::default();
        let piece = vec![7u8; 32 * 1024];
        for _ in 0..320 {
            assert!(p.push_data(piece.clone()));
        }
        let mut total = 0;
        while let Some(item) = p.pop() {
            let ShellOutput::Data(d) = item else {
                panic!("unexpected {item:?}")
            };
            assert!(d.len() <= MAX_COALESCED_CHUNK);
            total += d.len();
        }
        assert_eq!(total, 320 * 32 * 1024);
    }

    /// A server that never answers: reads never complete and neither does `close`.
    struct SilentServer {
        close_called: Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl ShellWriter for SilentServer {
        async fn data(&self, _data: Vec<u8>) -> std::result::Result<(), russh::Error> {
            std::future::pending().await
        }

        async fn window_change(
            &self,
            _cols: u32,
            _rows: u32,
        ) -> std::result::Result<(), russh::Error> {
            std::future::pending().await
        }

        async fn close(&self) -> std::result::Result<(), russh::Error> {
            self.close_called.store(true, Ordering::SeqCst);
            std::future::pending().await
        }
    }

    struct SilentReader;

    #[async_trait::async_trait]
    impl ShellReader for SilentReader {
        async fn wait(&mut self) -> Option<ChannelMsg> {
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn close_does_not_wait_for_the_server() {
        use futures_util::StreamExt;

        let budget = crate::channels::ChannelBudget::new(1);
        budget.reset(1);
        let slot = budget.acquire(1).unwrap();
        let close_called = Arc::new(AtomicBool::new(false));
        let writer = Arc::new(SilentServer {
            close_called: close_called.clone(),
        });
        let shell = ShellChannel::start(Box::new(SilentReader), writer, Some(slot));
        let (handle, mut output) = shell.into_output();
        let copy = handle.clone();

        // Returns at once although the server never answers the close.
        handle.close();
        let e = copy.write(b"x").await.unwrap_err();
        assert_eq!(e.message, "Shell closed");
        let closed = tokio::time::timeout(Duration::from_secs(1), output.next())
            .await
            .expect("Closed must not wait for the server");
        assert_eq!(closed, Some(ShellOutput::Closed));
        assert_eq!(output.next().await, None);
        // The close request still went out (in the background), and the slot is free.
        let deadline = Instant::now() + Duration::from_secs(1);
        while !close_called.load(Ordering::SeqCst) {
            assert!(
                Instant::now() < deadline,
                "the channel close was never sent"
            );
            tokio::task::yield_now().await;
        }
        assert_eq!(budget.open(), 0);
        // Closing again is harmless.
        copy.close();
    }

    /// A server that sends 1 MiB chunks for ever.
    struct Flood;

    #[async_trait::async_trait]
    impl ShellReader for Flood {
        async fn wait(&mut self) -> Option<ChannelMsg> {
            tokio::task::yield_now().await;
            Some(ChannelMsg::Data {
                data: vec![b'y'; 1024 * 1024].into(),
            })
        }
    }

    #[tokio::test]
    async fn overflow_is_reported_before_closed() {
        use futures_util::StreamExt;

        let writer = Arc::new(SilentServer {
            close_called: Arc::new(AtomicBool::new(false)),
        });
        let shell = ShellChannel::start(Box::new(Flood), writer, None);
        let (handle, mut output) = shell.into_output();
        // Nobody reads until the pump gives up on this consumer.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !handle.overflowed() {
            assert!(Instant::now() < deadline, "the pump never overflowed");
            tokio::task::yield_now().await;
        }
        let mut seen = Vec::new();
        while let Some(item) = output.next().await {
            if !matches!(item, ShellOutput::Data(_)) {
                seen.push(item);
            }
        }
        assert_eq!(seen, [ShellOutput::Overflowed, ShellOutput::Closed]);
    }

    #[test]
    fn pending_refuses_data_past_the_cap() {
        let mut p = Pending::default();
        assert!(p.push_data(vec![0; MAX_PENDING_OUTPUT]));
        assert!(!p.push_data(vec![0]));
        assert_eq!(p.bytes, MAX_PENDING_OUTPUT);
    }
}
