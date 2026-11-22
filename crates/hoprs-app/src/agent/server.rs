//! The app's side of the agent channel: listens while the master switch is on, greets
//! each relay, feeds its calls to the [`Service`] and writes the replies back.
//!
//! **The switch.** The server follows the switch's latest value, not the order of its
//! messages: on, it opens the channel ([`AgentEvent::Channel`]`(None)`, or `Some` with
//! why it could not, and it tries again when the switch is turned on again); off, it
//! says `{"bye":"off"}` to every relay, closes them and the channel (the socket file
//! goes), and reports no relays. [`AgentServer::shutdown`] does the same with
//! `{"bye":"quit"}`. Without a place for the channel (no instance lock) it says so once
//! in the log and never listens. Calls in flight when a relay goes, or when the channel
//! closes, are dropped, which cancels them: the service logs each as cancelled and
//! releases what it held.
//!
//! **A relay.** Its first line must be a [`Hello`] of at most [`proto::HELLO_MAX`] bytes
//! within [`HELLO_WAIT`]: anything else, or nothing, closes the connection without an
//! answer; another channel number or
//! another version is answered with a mismatch, then closed. At most [`MAX_RELAYS`] are
//! connected at once (one more is closed at its hello). Then each call runs in its own
//! task through [`Service::call`], the only way to the tools, at most [`MAX_IN_FLIGHT`]
//! at a time per relay (the next ones wait, in order, up to [`WAITING_MAX`]; one more is
//! answered at once with an error saying so, and reading goes on); its reply is one line
//! under its id. A cancel drops that call and nothing is written for it. A line that is
//! not a message, a call id already in use, or the end of the input closes the
//! connection. Cancels and the end of the input are always read.
//!
//! **Back-pressure.** Each relay has its own reader and writer: a relay that stops
//! reading its replies holds up only itself (its calls wait for their replies to be
//! written before their turn is free), never another relay or the service. Memory stays
//! bounded: at most [`MAX_IN_FLIGHT`] running and [`WAITING_MAX`] waiting calls, and a
//! relay with [`WAITING_MAX`] refusals not yet written is closed. A goodbye is given
//! [`BYE_WAIT`], then the connection is closed anyway.
//!
//! **Switch off.** Once the channel is closed the service's reaper runs at once: with
//! the switch off it closes the sessions only an agent used.
//!
//! Nothing here logs a message's content. [`Service::reap`] runs every [`REAP_EVERY`]
//! for the whole run, the switch on or off.

use std::collections::{HashMap, VecDeque};
use std::io::ErrorKind as IoKind;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use hoprs_core::{Error, ErrorKind};
use serde_json::Value;
use tokio::io::{AsyncWriteExt, BufReader, ReadHalf, WriteHalf};
use tokio::sync::{mpsc, watch};
use tokio::task::{AbortHandle, JoinHandle, JoinSet};
use tokio::time::{Instant, MissedTickBehavior, timeout};

use super::channel::{Endpoint, Listener, Stream};
use super::proto::{self, ByeReason, HelloReply, Reply, ToApp, ToRelay};
use super::service::Service;
use super::{AgentEvent, HELLO_WAIT, MAX_IN_FLIGHT, MAX_RELAYS, REAP_EVERY};

/// How long a goodbye, or the answer to a mismatched hello, may take to be written.
const BYE_WAIT: Duration = Duration::from_secs(1);
/// Calls read from a relay and waiting for a turn; one more is refused (see
/// [`TOO_MANY`]). Also the most refusals a relay may leave unread before it is closed.
const WAITING_MAX: usize = MAX_IN_FLIGHT;
/// Connections still in their hello at once; more are closed at once.
const HANDSHAKES_MAX: usize = MAX_RELAYS;
/// The pause after the channel failed to take a connection (out of descriptors, say).
const ACCEPT_PAUSE: Duration = Duration::from_millis(100);
/// The reply to a call whose task failed (a bug in hoprs): the agent is not left waiting.
const FAILED: &str = "hoprs failed while answering this call. Call the tool again; if it fails again, tell the user.";
/// The reply to a call that arrives while [`WAITING_MAX`] calls wait already.
const TOO_MANY: &str = "Too many calls are waiting in hoprs on this connection. Wait for some to finish, then call the tool again.";
/// The reply to a call whose answer does not fit in one line of the channel.
const TOO_LONG: &str = "The answer was too long for hoprs's channel. Ask for less (a smaller range, a lower limit), or use download.";

/// The agent channel's server (see the module docs). Dropped without
/// [`shutdown`](Self::shutdown), it still closes the channel, without waiting.
pub struct AgentServer {
    quit: watch::Sender<bool>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl AgentServer {
    /// Starts the server's task and returns at once: nothing is bound on the caller's
    /// thread. Must run inside the tokio runtime.
    pub fn start(
        service: Arc<Service>,
        endpoint: Option<Endpoint>,
        switch: watch::Receiver<bool>,
        events: mpsc::UnboundedSender<AgentEvent>,
    ) -> AgentServer {
        let (quit, quitting) = watch::channel(false);
        let task = tokio::spawn(run(service, endpoint, switch, events, quitting));
        AgentServer {
            quit,
            task: Mutex::new(Some(task)),
        }
    }

    /// hoprs is quitting: `{"bye":"quit"}` to every relay, the connections and the
    /// channel closed, the reaper stopped. Returns when that is done.
    pub async fn shutdown(&self) {
        self.quit.send_replace(true);
        let task = self
            .task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }
}

impl Drop for AgentServer {
    fn drop(&mut self) {
        self.quit.send_replace(true);
    }
}

/// What the relays' tasks share.
struct Shared {
    service: Arc<Service>,
    events: mpsc::UnboundedSender<AgentEvent>,
    /// Relays past their hello.
    relays: Mutex<usize>,
    /// Connections still in their hello.
    handshakes: AtomicUsize,
}

impl Shared {
    fn relays(&self) -> MutexGuard<'_, usize> {
        self.relays.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn send(&self, event: AgentEvent) {
        let _ = self.events.send(event);
    }
}

/// A relay's place among the [`MAX_RELAYS`]: counted and reported while it lives.
struct Seat(Arc<Shared>);

impl Seat {
    fn take(shared: &Arc<Shared>) -> Option<Seat> {
        let mut relays = shared.relays();
        if *relays >= MAX_RELAYS {
            return None;
        }
        *relays += 1;
        // Sent under the lock, so the counts arrive in order.
        shared.send(AgentEvent::Relays(*relays));
        Some(Seat(shared.clone()))
    }
}

impl Drop for Seat {
    fn drop(&mut self) {
        let mut relays = self.0.relays();
        *relays = relays.saturating_sub(1);
        self.0.send(AgentEvent::Relays(*relays));
    }
}

/// A connection in its hello, counted until it ends or is past it.
struct Handshake(Arc<Shared>);

impl Drop for Handshake {
    fn drop(&mut self) {
        self.0.handshakes.fetch_sub(1, Ordering::SeqCst);
    }
}

async fn run(
    service: Arc<Service>,
    endpoint: Option<Endpoint>,
    mut switch: watch::Receiver<bool>,
    events: mpsc::UnboundedSender<AgentEvent>,
    mut quitting: watch::Receiver<bool>,
) {
    let reaper = tokio::spawn(reap(service.clone()));
    let shared = Arc::new(Shared {
        service,
        events,
        relays: Mutex::new(0),
        handshakes: AtomicUsize::new(0),
    });
    match endpoint {
        Some(endpoint) => follow(&shared, &endpoint, &mut switch, &mut quitting).await,
        None => {
            tracing::warn!(
                "no place for the agent channel this run (no instance lock): agents cannot connect"
            );
            quit(&mut quitting).await;
        }
    }
    reaper.abort();
    let _ = reaper.await;
}

/// The reaper: every [`REAP_EVERY`], for the whole run.
async fn reap(service: Arc<Service>) {
    let mut every = tokio::time::interval_at(Instant::now() + REAP_EVERY, REAP_EVERY);
    every.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        every.tick().await;
        service.reap().await;
    }
}

/// Until hoprs quits (or the server is dropped).
async fn quit(quitting: &mut watch::Receiver<bool>) {
    let _ = quitting.wait_for(|quit| *quit).await;
}

/// Until the channel closes, and why.
async fn stopped(stop: &mut watch::Receiver<Option<ByeReason>>) -> ByeReason {
    let reason = stop.wait_for(Option::is_some).await;
    reason
        .ok()
        .and_then(|reason| *reason)
        .unwrap_or(ByeReason::Quit)
}

/// Whether the switch is on now (a backend that went away turned it off).
fn is_on(switch: &mut watch::Receiver<bool>) -> bool {
    switch.has_changed().is_ok() && *switch.borrow_and_update()
}

/// Opens and closes the channel as the switch says, until hoprs quits.
async fn follow(
    shared: &Arc<Shared>,
    endpoint: &Endpoint,
    switch: &mut watch::Receiver<bool>,
    quitting: &mut watch::Receiver<bool>,
) {
    loop {
        if !is_on(switch) && !turned(switch, quitting).await {
            return;
        }
        if !is_on(switch) {
            continue;
        }
        let listener = match Listener::bind(endpoint) {
            Ok(listener) => listener,
            Err(err) => {
                tracing::warn!(error = %err, "could not open the channel for agents");
                shared.send(AgentEvent::Channel(Some(
                    Error::coded(
                        ErrorKind::Io,
                        "agent-channel-failed",
                        "Could not open the channel for agents",
                    )
                    .with_detail(err.to_string()),
                )));
                // Again when the switch is turned on again.
                if !turned(switch, quitting).await {
                    return;
                }
                continue;
            }
        };
        shared.send(AgentEvent::Channel(None));
        tracing::info!("the channel for agents is open");
        let reason = listen(shared, listener, switch, quitting).await;
        tracing::info!("the channel for agents is closed");
        if reason == ByeReason::Quit {
            return;
        }
        // Agents were turned off: their sessions go now, not after the idle time.
        shared.service.reap().await;
    }
}

/// Waits until the switch is flipped (`true`) or hoprs quits (`false`).
async fn turned(switch: &mut watch::Receiver<bool>, quitting: &mut watch::Receiver<bool>) -> bool {
    if *quitting.borrow() {
        return false;
    }
    let flipped = tokio::select! {
        changed = switch.changed() => changed.is_ok(),
        () = quit(quitting) => return false,
    };
    if !flipped {
        // The backend went away: off for good.
        quit(quitting).await;
    }
    flipped
}

/// Takes relays until the switch goes off or hoprs quits; then says goodbye to each,
/// closes the channel and returns why.
async fn listen(
    shared: &Arc<Shared>,
    mut listener: Listener,
    switch: &mut watch::Receiver<bool>,
    quitting: &mut watch::Receiver<bool>,
) -> ByeReason {
    let (stop, _) = watch::channel(None::<ByeReason>);
    let mut relays = JoinSet::new();
    let reason = loop {
        tokio::select! {
            () = quit(quitting) => break ByeReason::Quit,
            _ = switch.changed() => {
                if !is_on(switch) {
                    break ByeReason::Off;
                }
            }
            accepted = listener.accept() => match accepted {
                Ok(stream) => {
                    if shared.handshakes.load(Ordering::SeqCst) >= HANDSHAKES_MAX {
                        continue;
                    }
                    shared.handshakes.fetch_add(1, Ordering::SeqCst);
                    let handshake = Handshake(shared.clone());
                    relays.spawn(relay(shared.clone(), stream, handshake, stop.subscribe()));
                }
                Err(err) => {
                    tracing::warn!(error = %err, "the channel for agents could not take a connection");
                    tokio::time::sleep(ACCEPT_PAUSE).await;
                }
            },
            Some(_) = relays.join_next(), if !relays.is_empty() => {}
        }
    };
    // No new relay from here; the socket file goes with the listener.
    drop(listener);
    stop.send_replace(Some(reason));
    while relays.join_next().await.is_some() {}
    // Each relay's seat already counted it out; this only guards the count.
    let mut count = shared.relays();
    if *count != 0 {
        *count = 0;
        shared.send(AgentEvent::Relays(0));
    }
    drop(count);
    reason
}

/// One relay, from its hello to its end.
async fn relay(
    shared: Arc<Shared>,
    stream: Stream,
    handshake: Handshake,
    mut stop: watch::Receiver<Option<ByeReason>>,
) {
    let (read, mut write) = tokio::io::split(stream);
    let mut reader = BufReader::new(read);
    let hello = tokio::select! {
        hello = timeout(HELLO_WAIT, proto::read_hello(&mut reader)) => hello,
        _ = stopped(&mut stop) => return,
    };
    // Silence, a line that is not a hello, or the end: closed without an answer.
    let Ok(Ok(Some(hello))) = hello else {
        return;
    };
    let answer = proto::hello_reply(&hello);
    if matches!(answer, HelloReply::Mismatch { .. }) {
        let _ = timeout(BYE_WAIT, async {
            proto::write_line(&mut write, &answer).await?;
            write.shutdown().await
        })
        .await;
        return;
    }
    // One relay too many: closed at its hello.
    let Some(_seat) = Seat::take(&shared) else {
        return;
    };
    drop(handshake);
    let greeted = timeout(HELLO_WAIT, proto::write_line(&mut write, &answer)).await;
    if !matches!(greeted, Ok(Ok(()))) {
        return;
    }
    Connection::new(shared.service.clone(), write)
        .serve(reader, stop)
        .await;
}

/// What the writer of a relay writes.
enum Outgoing {
    /// `turn`: the answer of a call that had a turn (else a refusal).
    Answer {
        id: u64,
        reply: Reply,
        turn: bool,
    },
    Bye(ByeReason),
}

/// Why a relay's connection ends.
enum End {
    /// The relay went, sent what is not a message, or stopped taking our lines.
    Gone,
    /// The channel is closing.
    Stop(ByeReason),
}

/// The calls of one relay after its hello.
struct Connection {
    service: Arc<Service>,
    /// The calls started and not answered, by id: a cancel aborts the task; an answer
    /// is sent only by the task that still finds its id here (so a cancel that came
    /// first means nothing is sent).
    running: Arc<Mutex<HashMap<u64, AbortHandle>>>,
    /// Calls started whose answer is not written yet (a cancelled one stops counting at
    /// once).
    busy: usize,
    /// Calls read and waiting for a turn, in order.
    waiting: VecDeque<(u64, String, Value)>,
    /// Refusals ([`TOO_MANY`]) not written yet.
    refused: usize,
    tasks: JoinSet<()>,
    out: mpsc::UnboundedSender<Outgoing>,
    /// One per answer written: whether it had a turn (now free) or was a refusal.
    written: mpsc::UnboundedReceiver<bool>,
    writer: JoinHandle<()>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Connection {
    fn new(service: Arc<Service>, write: WriteHalf<Stream>) -> Connection {
        let (out, outgoing) = mpsc::unbounded_channel();
        let (done, written) = mpsc::unbounded_channel();
        Connection {
            service,
            running: Arc::default(),
            busy: 0,
            waiting: VecDeque::new(),
            refused: 0,
            tasks: JoinSet::new(),
            out,
            written,
            writer: tokio::spawn(write_lines(write, outgoing, done)),
        }
    }

    async fn serve(
        mut self,
        reader: BufReader<ReadHalf<Stream>>,
        mut stop: watch::Receiver<Option<ByeReason>>,
    ) {
        let (to_us, mut incoming) = mpsc::channel(1);
        let reading = tokio::spawn(read_lines(reader, to_us));
        let end = loop {
            tokio::select! {
                biased;
                reason = stopped(&mut stop) => break End::Stop(reason),
                _ = &mut self.writer => break End::Gone,
                Some(turn) = self.written.recv() => {
                    if turn {
                        self.busy = self.busy.saturating_sub(1);
                    } else {
                        self.refused = self.refused.saturating_sub(1);
                    }
                }
                Some(joined) = self.tasks.join_next_with_id(), if !self.tasks.is_empty() => {
                    if let Err(err) = joined {
                        self.failed(err.id());
                    }
                }
                message = incoming.recv() => match message {
                    None => break End::Gone,
                    Some(ToApp::Cancel { cancel }) => self.cancel(cancel),
                    Some(ToApp::Call { call, tool, args }) => {
                        if self.knows(call) {
                            // Ids are unique per connection: the relay is confused.
                            break End::Gone;
                        }
                        if self.waiting.len() < WAITING_MAX {
                            self.waiting.push_back((call, tool, args));
                        } else if self.refused < WAITING_MAX {
                            // This call only; it holds nothing.
                            self.refused += 1;
                            let _ = self.out.send(Outgoing::Answer {
                                id: call,
                                reply: Reply::Err(TOO_MANY.to_owned()),
                                turn: false,
                            });
                        } else {
                            // Flooding calls and not reading the answers.
                            break End::Gone;
                        }
                    }
                },
            }
            self.start_waiting();
        };
        reading.abort();
        lock(&self.running).clear();
        self.waiting.clear();
        self.tasks.abort_all();
        while self.tasks.join_next().await.is_some() {}
        match end {
            End::Stop(reason) => {
                let _ = self.out.send(Outgoing::Bye(reason));
                if timeout(BYE_WAIT, &mut self.writer).await.is_err() {
                    self.writer.abort();
                }
            }
            End::Gone => self.writer.abort(),
        }
    }

    /// Whether `id` is a call of this relay not answered yet.
    fn knows(&self, id: u64) -> bool {
        lock(&self.running).contains_key(&id) || self.waiting.iter().any(|(w, ..)| *w == id)
    }

    /// Drops call `id`, waiting or running; an unknown id (answered already) is nothing.
    fn cancel(&mut self, id: u64) {
        if let Some(task) = lock(&self.running).remove(&id) {
            task.abort();
            self.busy = self.busy.saturating_sub(1);
        } else {
            self.waiting.retain(|(w, ..)| *w != id);
        }
    }

    /// A call's task ended without answering (it panicked; a cancelled one is no longer
    /// listed): its answer says so, and its turn is freed when that is written.
    fn failed(&mut self, task: tokio::task::Id) {
        let mut running = lock(&self.running);
        let id = running
            .iter()
            .find(|(_, handle)| handle.id() == task)
            .map(|(id, _)| *id);
        if let Some(id) = id {
            running.remove(&id);
            tracing::warn!("a call of an agent failed inside hoprs");
            let _ = self.out.send(Outgoing::Answer {
                id,
                reply: Reply::Err(FAILED.to_owned()),
                turn: true,
            });
        }
    }

    /// Starts waiting calls while there are turns.
    fn start_waiting(&mut self) {
        while self.busy < MAX_IN_FLIGHT {
            let Some((id, tool, args)) = self.waiting.pop_front() else {
                return;
            };
            let service = self.service.clone();
            let running = self.running.clone();
            let out = self.out.clone();
            // Under the lock until the id is in: the task cannot answer before.
            let mut calls = lock(&self.running);
            let task = self.tasks.spawn(async move {
                let reply = service.call(&tool, args).await;
                if lock(&running).remove(&id).is_some() {
                    let _ = out.send(Outgoing::Answer {
                        id,
                        reply,
                        turn: true,
                    });
                }
            });
            calls.insert(id, task);
            drop(calls);
            self.busy += 1;
        }
    }
}

/// A relay's reader: its messages, until the end or a line that is not one. Waits while
/// the connection does not take them.
async fn read_lines(mut reader: BufReader<ReadHalf<Stream>>, to: mpsc::Sender<ToApp>) {
    while let Ok(Some(message)) = proto::read_message::<_, ToApp>(&mut reader).await {
        if to.send(message).await.is_err() {
            return;
        }
    }
}

/// A relay's writer: each answer as one line, then the goodbye; ends at the first
/// failed write.
async fn write_lines(
    mut write: WriteHalf<Stream>,
    mut outgoing: mpsc::UnboundedReceiver<Outgoing>,
    written: mpsc::UnboundedSender<bool>,
) {
    while let Some(message) = outgoing.recv().await {
        match message {
            Outgoing::Answer { id, reply, turn } => {
                let line = ToRelay::answer(id, reply);
                let sent = match proto::write_line(&mut write, &line).await {
                    Err(err) if err.kind() == IoKind::InvalidInput => {
                        let line = ToRelay::answer(id, Reply::Err(TOO_LONG.to_owned()));
                        proto::write_line(&mut write, &line).await
                    }
                    sent => sent,
                };
                if sent.is_err() {
                    return;
                }
                let _ = written.send(turn);
            }
            Outgoing::Bye(bye) => {
                let _ = proto::write_line(&mut write, &ToRelay::Bye { bye }).await;
                let _ = write.shutdown().await;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests;
