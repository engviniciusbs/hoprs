//! The prompt broker: turns the connection crates' [`Prompter`] / [`FtpPrompter`] calls into
//! dialogs for the UI and the UI's answers back into return values.
//!
//! - Every call becomes a [`PromptRequest`] `{ id, session, site, key: "host:port", kind }`, sent
//!   as a [`BrokerEvent::Request`] on the channel the UI subscribes to
//!   ([`PromptBroker::take_events`]). The answer comes back through
//!   [`PromptBroker::answer`].
//! - **Serialized per `key`:** while a host key or certificate dialog for a `key` is open,
//!   every other host key or certificate request for that `key` waits. When the dialog is
//!   answered, waiters that ask about the same key or certificate (same fingerprint) get
//!   the same answer without a dialog of their own; the others then ask in turn. Password,
//!   passphrase and keyboard-interactive prompts are never shared (each session asks).
//! - **Cancelled is refused:** a dialog the user closes answers [`PromptAnswer::Cancel`],
//!   which is `false` / `None` (and is shared with the waiters like any answer). Closing a
//!   session ([`SessionPrompter::cancel`]) refuses its open and future prompts; its open
//!   dialogs are withdrawn ([`BrokerEvent::Withdrawn`]). A dialog withdrawn that way is not
//!   an answer: the requests waiting on it ask again, in a dialog of their own.
//! - **Saving a typed secret:** a password or passphrase answered with `save: true` is
//!   kept by the session's prompter until the session manager sees the session connect (see
//!   [`SessionPrompter::take_pending_secrets`]), fail or close. Only a secret that proved
//!   right is handed out for the keyring: the connection crates report it through
//!   [`Prompter::secret_accepted`] (the server accepted the password, the passphrase
//!   decrypted its key) and [`FtpPrompter::password_accepted`] (the FTP login worked). A
//!   password the server refused before keyboard-interactive or a key succeeded, or a wrong
//!   passphrase skipped for another method, is never reported, so never saved. A later
//!   answer of the same kind replaces the pending one.
//! - **Asked by an agent:** the prompter of a session an agent started
//!   ([`SessionPrompter::by_agent`]) flags each of its requests ([`PromptRequest::by_agent`]):
//!   the dialog says so. The agent never answers or skips one; it only waits, and
//!   [`PromptBroker::has_open`] tells whether a session is waiting on the user.
//! - **Pins:** `store_pin` writes the accepted certificate to the session's site, if the
//!   site still points at the session's `host:port`
//!   ([`Vault::set_ftps_pin`](hoprs_vault::Vault::set_ftps_pin)); `pin_superseded` (a trusted
//!   chain beat a stale pin, ruling R5 of Plan 3) clears it.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use hoprs_core::SiteId;
use hoprs_remote::{CertProblem, FtpPrompter};
use hoprs_ssh::{AcceptedSecret, KbdPrompt, Prompter};
use hoprs_vault::SecretKind;
use secrecy::SecretString;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

use super::sessions::SessionId;
use super::vault::VaultHandle;

/// Identifies one dialog.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PromptId(pub u64);

/// A dialog the UI must show.
#[derive(Clone, Debug, PartialEq)]
pub struct PromptRequest {
    pub id: PromptId,
    pub session: SessionId,
    /// The site whose connection asks.
    pub site: SiteId,
    /// `host:port` of the connection asking.
    pub key: String,
    pub kind: PromptKind,
    /// The session was started by an agent's call (the dialog says so).
    pub by_agent: bool,
}

/// What a dialog asks.
#[derive(Clone, Debug, PartialEq)]
pub enum PromptKind {
    /// Trust an unknown SSH host key? `other_key_types`: key types already known for the
    /// host (empty for a host never seen).
    HostKey {
        host: String,
        port: u16,
        key_type: String,
        fingerprint: String,
        other_key_types: Vec<String>,
    },
    /// Accept (and pin) an FTPS certificate?
    Certificate {
        host: String,
        port: u16,
        fingerprint: String,
        problem: CertProblem,
    },
    Password {
        user: String,
        host: String,
    },
    Passphrase {
        path: PathBuf,
    },
    KeyboardInteractive {
        name: String,
        instructions: String,
        prompts: Vec<KbdField>,
    },
}

/// One keyboard-interactive field: its label, and whether the answer may be shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KbdField {
    pub text: String,
    pub echo: bool,
}

/// The UI's answer to a dialog. An answer of the wrong shape for the dialog counts as
/// [`Cancel`](Self::Cancel).
#[derive(Clone)]
pub enum PromptAnswer {
    /// Host key or certificate: trust it or not.
    Decision(bool),
    /// Password or passphrase. `save`: store it in the system keyring once the session
    /// connects with it (never before, never after a failure).
    Secret { secret: SecretString, save: bool },
    /// Keyboard-interactive: one response per prompt.
    Responses(Vec<SecretString>),
    /// The dialog was closed: a refusal.
    Cancel,
}

/// Never prints secrets.
impl fmt::Debug for PromptAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decision(accept) => f.debug_tuple("Decision").field(accept).finish(),
            Self::Secret { save, .. } => write!(f, "Secret(<redacted>, save: {save})"),
            Self::Responses(r) => write!(f, "Responses(<{} redacted>)", r.len()),
            Self::Cancel => f.write_str("Cancel"),
        }
    }
}

/// What the UI receives from the broker.
#[derive(Clone, Debug, PartialEq)]
pub enum BrokerEvent {
    /// Show this dialog.
    Request(PromptRequest),
    /// Close this dialog without an answer: nobody waits on it any more (its session was
    /// closed, or its connection attempt ended).
    Withdrawn(PromptId),
}

/// Hands out per-session prompters and routes the UI's answers. Cheap to clone.
#[derive(Clone)]
pub struct PromptBroker {
    inner: Arc<Inner>,
}

struct Inner {
    vault: VaultHandle,
    next_id: AtomicU64,
    events: mpsc::UnboundedSender<BrokerEvent>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<BrokerEvent>>>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// Open dialogs.
    pending: HashMap<PromptId, oneshot::Sender<PromptAnswer>>,
    /// Open host key / certificate dialogs, by `key`.
    decisions: HashMap<String, InFlight>,
    /// Per session, its dialogs open in the UI plus its waits on another session's
    /// dialog (see [`PromptBroker::has_open`]). A session with none has no entry.
    open: HashMap<SessionId, usize>,
}

/// An open host key or certificate dialog that others may wait on.
struct InFlight {
    /// What it asks about: requests with the same subject reuse its answer.
    subject: String,
    /// Becomes `Some(answer)` once answered; closed without a value if withdrawn.
    answer: watch::Receiver<Option<bool>>,
}

impl PromptBroker {
    pub fn new(vault: VaultHandle) -> Self {
        let (events, receiver) = mpsc::unbounded_channel();
        Self {
            inner: Arc::new(Inner {
                vault,
                next_id: AtomicU64::new(1),
                events,
                receiver: Mutex::new(Some(receiver)),
                state: Mutex::new(State::default()),
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The channel of dialogs for the UI. Only the first call gets it.
    pub fn take_events(&self) -> Option<mpsc::UnboundedReceiver<BrokerEvent>> {
        self.inner
            .receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// The prompter of one session of `site`, whose connection goes to `host:port`.
    pub fn session(
        &self,
        session: SessionId,
        site: SiteId,
        host: &str,
        port: u16,
    ) -> SessionPrompter {
        SessionPrompter {
            broker: self.clone(),
            session,
            site,
            key: endpoint_key(host, port),
            cancel: CancellationToken::new(),
            pending: Arc::default(),
            by_agent: Arc::default(),
        }
    }

    /// Whether a dialog of `session` is open in the UI, or `session` waits on another
    /// session's host key or certificate dialog: the session is waiting on the user.
    pub fn has_open(&self, session: SessionId) -> bool {
        self.state().open.contains_key(&session)
    }

    /// Delivers the UI's answer. Returns false if the dialog is no longer open.
    pub fn answer(&self, id: PromptId, answer: PromptAnswer) -> bool {
        let Some(reply) = self.state().pending.remove(&id) else {
            return false;
        };
        reply.send(answer).is_ok()
    }

    /// Shows one dialog and waits for its answer. `None` when it was withdrawn instead:
    /// the session was cancelled, the UI is gone, or the caller stopped waiting.
    async fn ask(
        &self,
        from: &SessionPrompter,
        key: String,
        kind: PromptKind,
    ) -> Option<PromptAnswer> {
        if from.cancel.is_cancelled() {
            return None;
        }
        let id = PromptId(self.inner.next_id.fetch_add(1, Ordering::Relaxed));
        let (reply, answer) = oneshot::channel();
        self.state().pending.insert(id, reply);
        // Withdraws the dialog however this future ends without an answer (dropped too).
        let _open = OpenDialog { broker: self, id };
        let _waiting = Waiting::new(self, from.session);
        let request = PromptRequest {
            id,
            session: from.session,
            site: from.site.clone(),
            key,
            kind,
            by_agent: from.is_by_agent(),
        };
        if self
            .inner
            .events
            .send(BrokerEvent::Request(request))
            .is_err()
        {
            return None;
        }
        tokio::select! {
            answer = answer => answer.ok(),
            () = from.cancel.cancelled() => None,
        }
    }

    /// A host key or certificate decision, serialized per `key` (see the module docs).
    /// `subject` names what is decided: waiters with the same subject reuse the answer.
    async fn decide(
        &self,
        from: &SessionPrompter,
        key: String,
        subject: String,
        kind: PromptKind,
    ) -> bool {
        loop {
            if from.cancel.is_cancelled() {
                return false;
            }
            let turn = {
                let mut state = self.state();
                match state.decisions.get(&key) {
                    Some(open) => Turn::Wait {
                        same: open.subject == subject,
                        answer: open.answer.clone(),
                    },
                    None => {
                        let (answer, rx) = watch::channel(None);
                        let open = InFlight {
                            subject: subject.clone(),
                            answer: rx,
                        };
                        state.decisions.insert(key.clone(), open);
                        Turn::Lead(answer)
                    }
                }
            };
            match turn {
                Turn::Lead(answer) => {
                    // This request's dialog decides for the waiters too.
                    let leader = Leader {
                        broker: self,
                        key: &key,
                        answer: Some(answer),
                    };
                    return match self.ask(from, key.clone(), kind).await {
                        Some(answer) => {
                            let accept = matches!(answer, PromptAnswer::Decision(true));
                            leader.finish(accept);
                            accept
                        }
                        // Withdrawn: not an answer. Dropping `leader` wakes the waiters,
                        // and they ask again.
                        None => false,
                    };
                }
                Turn::Wait { same, mut answer } => {
                    let _waiting = Waiting::new(self, from.session);
                    let decided = tokio::select! {
                        decided = answer.wait_for(Option::is_some) => {
                            decided.ok().and_then(|value| *value)
                        }
                        () = from.cancel.cancelled() => return false,
                    };
                    if let Some(accept) = decided
                        && same
                    {
                        return accept;
                    }
                    // Answered about another key or certificate, or withdrawn: look again.
                }
            }
        }
    }
}

fn endpoint_key(host: &str, port: u16) -> String {
    format!("{host}:{port}")
}

/// Whether a decision request leads (shows the dialog) or waits for the open one.
enum Turn {
    Lead(watch::Sender<Option<bool>>),
    Wait {
        same: bool,
        answer: watch::Receiver<Option<bool>>,
    },
}

/// An open dialog: withdrawn when dropped unanswered.
struct OpenDialog<'a> {
    broker: &'a PromptBroker,
    id: PromptId,
}

impl Drop for OpenDialog<'_> {
    fn drop(&mut self) {
        let unanswered = self.broker.state().pending.remove(&self.id).is_some();
        if unanswered {
            let _ = self
                .broker
                .inner
                .events
                .send(BrokerEvent::Withdrawn(self.id));
        }
    }
}

/// Counts one dialog or wait of a session in [`State::open`] while it lives.
struct Waiting<'a> {
    broker: &'a PromptBroker,
    session: SessionId,
}

impl<'a> Waiting<'a> {
    fn new(broker: &'a PromptBroker, session: SessionId) -> Waiting<'a> {
        *broker.state().open.entry(session).or_default() += 1;
        Waiting { broker, session }
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        let mut state = self.broker.state();
        if let Some(count) = state.open.get_mut(&self.session) {
            *count -= 1;
            if *count == 0 {
                state.open.remove(&self.session);
            }
        }
    }
}

/// The leading request of a `key`: owns its [`InFlight`] entry until it is answered.
/// Dropped unanswered, it removes the entry and closes the channel, which wakes the
/// waiters with no value.
struct Leader<'a> {
    broker: &'a PromptBroker,
    key: &'a str,
    answer: Option<watch::Sender<Option<bool>>>,
}

impl Leader<'_> {
    /// Ends the turn and hands `accept` to the waiters. Both happen under the state lock:
    /// no request can see the entry gone before the waiters have the answer.
    fn finish(mut self, accept: bool) {
        let mut state = self.broker.state();
        state.decisions.remove(self.key);
        if let Some(answer) = self.answer.take() {
            answer.send_replace(Some(accept));
        }
    }
}

impl Drop for Leader<'_> {
    fn drop(&mut self) {
        if self.answer.is_some() {
            // The sender is dropped right after this, closing the waiters' channel.
            self.broker.state().decisions.remove(self.key);
        }
    }
}

/// A typed secret waiting to be saved.
struct Pending {
    secret: SecretString,
    /// The connection crate reported it right.
    accepted: bool,
}

/// The [`Prompter`] and [`FtpPrompter`] of one session. Cheap to clone.
#[derive(Clone)]
pub struct SessionPrompter {
    broker: PromptBroker,
    session: SessionId,
    site: SiteId,
    /// `host:port` of the session's connection.
    key: String,
    cancel: CancellationToken,
    /// Typed secrets to save once the session connects (see the module docs), with
    /// whether they proved right.
    pending: Arc<Mutex<HashMap<SecretKind, Pending>>>,
    /// The session was started by an agent (shared by the clones: the user's connect of
    /// the same site takes it over and clears it).
    by_agent: Arc<AtomicBool>,
}

impl SessionPrompter {
    /// This session was started by an agent's call: its dialogs say so.
    pub fn by_agent(self) -> Self {
        self.by_agent.store(true, Ordering::SeqCst);
        self
    }

    /// The user connected this session's site from a tab: its dialogs are the user's
    /// from now on.
    pub fn taken_over(&self) {
        self.by_agent.store(false, Ordering::SeqCst);
    }

    fn is_by_agent(&self) -> bool {
        self.by_agent.load(Ordering::SeqCst)
    }

    /// The session is closing: refuses its open prompts (their dialogs are withdrawn) and
    /// every later one.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    fn pending(&self) -> MutexGuard<'_, HashMap<SecretKind, Pending>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The session just connected: the typed secrets the user asked to save that proved
    /// right, taken out. The others (refused, or never tried) are dropped.
    pub fn take_pending_secrets(&self) -> Vec<(SecretKind, SecretString)> {
        self.pending()
            .drain()
            .filter(|(_, pending)| pending.accepted)
            .map(|(kind, pending)| (kind, pending.secret))
            .collect()
    }

    /// The pending secret of `kind` proved right.
    fn accept(&self, kind: SecretKind) {
        if let Some(pending) = self.pending().get_mut(&kind) {
            pending.accepted = true;
        }
    }

    /// Forgets the typed secrets waiting to be saved: the connection failed or closed.
    pub fn discard_pending_secrets(&self) {
        self.pending().clear();
    }

    /// Asks for a password or passphrase, which `slot` names for saving.
    async fn secret(&self, kind: PromptKind, slot: SecretKind) -> Option<SecretString> {
        match self.broker.ask(self, self.key.clone(), kind).await {
            Some(PromptAnswer::Secret { secret, save }) => {
                let mut pending = self.pending();
                if save {
                    let secret = secret.clone();
                    let accepted = false;
                    pending.insert(slot, Pending { secret, accepted });
                } else {
                    // A new answer without "save": an earlier one was not the right one.
                    pending.remove(&slot);
                }
                Some(secret)
            }
            _ => None,
        }
    }

    async fn password_prompt(&self, user: &str, host: &str) -> Option<SecretString> {
        let kind = PromptKind::Password {
            user: user.to_owned(),
            host: host.to_owned(),
        };
        self.secret(kind, SecretKind::Password).await
    }
}

#[async_trait]
impl Prompter for SessionPrompter {
    async fn confirm_host_key(
        &self,
        host: &str,
        port: u16,
        key_type: &str,
        fingerprint: &str,
        other_key_types: &[String],
    ) -> bool {
        let kind = PromptKind::HostKey {
            host: host.to_owned(),
            port,
            key_type: key_type.to_owned(),
            fingerprint: fingerprint.to_owned(),
            other_key_types: other_key_types.to_vec(),
        };
        let subject = format!("host key {key_type} {fingerprint}");
        self.broker
            .decide(self, endpoint_key(host, port), subject, kind)
            .await
    }

    async fn password(&self, user: &str, host: &str) -> Option<SecretString> {
        self.password_prompt(user, host).await
    }

    async fn key_passphrase(&self, path: &Path) -> Option<SecretString> {
        let kind = PromptKind::Passphrase {
            path: path.to_path_buf(),
        };
        self.secret(kind, SecretKind::KeyPassphrase).await
    }

    async fn keyboard_interactive(
        &self,
        name: &str,
        instructions: &str,
        prompts: &[KbdPrompt],
    ) -> Option<Vec<SecretString>> {
        // Servers may send a round with nothing to ask (only instructions, or nothing).
        if prompts.is_empty() {
            return Some(Vec::new());
        }
        let kind = PromptKind::KeyboardInteractive {
            name: name.to_owned(),
            instructions: instructions.to_owned(),
            prompts: prompts
                .iter()
                .map(|p| KbdField {
                    text: p.text.clone(),
                    echo: p.echo,
                })
                .collect(),
        };
        match self.broker.ask(self, self.key.clone(), kind).await {
            Some(PromptAnswer::Responses(responses)) if responses.len() == prompts.len() => {
                Some(responses)
            }
            _ => None,
        }
    }

    fn secret_accepted(&self, secret: AcceptedSecret) {
        self.accept(match secret {
            AcceptedSecret::Password => SecretKind::Password,
            AcceptedSecret::Passphrase => SecretKind::KeyPassphrase,
        });
    }
}

#[async_trait]
impl FtpPrompter for SessionPrompter {
    async fn password(&self, user: &str, host: &str) -> Option<SecretString> {
        self.password_prompt(user, host).await
    }

    async fn confirm_certificate(
        &self,
        host: &str,
        port: u16,
        fingerprint: &str,
        problem: &CertProblem,
    ) -> bool {
        let kind = PromptKind::Certificate {
            host: host.to_owned(),
            port,
            fingerprint: fingerprint.to_owned(),
            problem: problem.clone(),
        };
        let subject = format!("certificate {fingerprint}");
        self.broker
            .decide(self, endpoint_key(host, port), subject, kind)
            .await
    }

    fn password_accepted(&self) {
        self.accept(SecretKind::Password);
    }

    /// Pins the certificate on the site, unless the site was edited to another `host:port`
    /// since this session started (the pin would belong to another server).
    async fn store_pin(&self, _host: &str, _port: u16, fingerprint: &str) {
        let vault = &self.broker.inner.vault;
        let saved = vault
            .set_ftps_pin_at(self.site.clone(), self.key.clone(), fingerprint.to_owned())
            .await;
        match saved {
            Ok(true) => {}
            Ok(false) => {
                tracing::debug!(site = %self.site, "the site moved to another endpoint; the accepted certificate is not pinned");
            }
            Err(err) => {
                // The session keeps the pin in memory; the next run asks again.
                tracing::warn!(site = %self.site, error = %err, "cannot save the accepted certificate");
            }
        }
    }

    async fn pin_superseded(&self, _host: &str, _port: u16, pinned: &str) {
        let vault = &self.broker.inner.vault;
        let cleared = vault
            .clear_stale_ftps_pin(self.site.clone(), pinned.to_owned())
            .await;
        match cleared {
            Ok(true) => tracing::info!(site = %self.site, "cleared a stale certificate pin"),
            Ok(false) => {}
            Err(err) => {
                tracing::warn!(site = %self.site, error = %err, "cannot clear a stale certificate pin");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use hoprs_core::{Protocol, Site};
    use hoprs_remote::CertReason;
    use hoprs_vault::{MemoryStore, Vault};
    use secrecy::ExposeSecret;
    use tokio::sync::mpsc::UnboundedReceiver;
    use tokio::task::JoinHandle;

    use super::*;

    struct Fixture {
        broker: PromptBroker,
        events: UnboundedReceiver<BrokerEvent>,
        vault: VaultHandle,
        site: Site,
        _dir: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = Vault::open(dir.path(), Arc::new(MemoryStore::new())).unwrap();
        // The endpoint the prompters below connect to (`h:21`).
        let site = Site::new("f", "h", "u", Protocol::FtpsExplicit);
        vault.upsert_site(site.clone()).unwrap();
        let vault = VaultHandle::new(vault);
        let broker = PromptBroker::new(vault.clone());
        let events = broker.take_events().unwrap();
        assert!(
            broker.take_events().is_none(),
            "the channel is handed out once"
        );
        Fixture {
            broker,
            events,
            vault,
            site,
            _dir: dir,
        }
    }

    impl Fixture {
        fn prompter(&self, session: u64) -> SessionPrompter {
            self.broker
                .session(SessionId(session), self.site.id.clone(), "h", 21)
        }

        /// The next event, failing after a second.
        async fn next(&mut self) -> BrokerEvent {
            tokio::time::timeout(Duration::from_secs(1), self.events.recv())
                .await
                .expect("no broker event")
                .expect("broker channel closed")
        }

        async fn next_request(&mut self) -> PromptRequest {
            match self.next().await {
                BrokerEvent::Request(r) => r,
                other => panic!("expected a request, got {other:?}"),
            }
        }

        /// Asserts no event arrives for a while.
        async fn quiet(&mut self) {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if let Ok(event) = self.events.try_recv() {
                panic!("unexpected {event:?}");
            }
        }
    }

    fn secret(value: &str, save: bool) -> PromptAnswer {
        PromptAnswer::Secret {
            secret: SecretString::from(value),
            save,
        }
    }

    fn problem() -> CertProblem {
        CertProblem::Untrusted {
            reason: CertReason::UnknownIssuer,
        }
    }

    fn ask_cert(p: SessionPrompter, fingerprint: &str) -> JoinHandle<bool> {
        let fingerprint = fingerprint.to_owned();
        tokio::spawn(async move {
            p.confirm_certificate("h", 21, &fingerprint, &problem())
                .await
        })
    }

    async fn join<T>(handle: JoinHandle<T>) -> T {
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("prompt never returned")
            .unwrap()
    }

    #[tokio::test]
    async fn broker_reuses_answer() {
        let mut f = fixture();
        let asks: Vec<_> = (1..=3)
            .map(|s| ask_cert(f.prompter(s), "SHA256:AA"))
            .collect();

        let request = f.next_request().await;
        assert_eq!(request.key, "h:21");
        assert_eq!(
            request.kind,
            PromptKind::Certificate {
                host: "h".into(),
                port: 21,
                fingerprint: "SHA256:AA".into(),
                problem: problem(),
            }
        );
        f.quiet().await;
        assert!(f.broker.answer(request.id, PromptAnswer::Decision(true)));
        for ask in asks {
            assert!(join(ask).await);
        }
        f.quiet().await;
        // Answering twice does nothing.
        assert!(!f.broker.answer(request.id, PromptAnswer::Decision(false)));

        // A refusal is shared the same way.
        let asks: Vec<_> = (1..=3)
            .map(|s| ask_cert(f.prompter(s), "SHA256:BB"))
            .collect();
        let request = f.next_request().await;
        f.quiet().await;
        f.broker.answer(request.id, PromptAnswer::Decision(false));
        for ask in asks {
            assert!(!join(ask).await);
        }
        f.quiet().await;
    }

    #[tokio::test]
    async fn broker_serializes_same_host() {
        let mut f = fixture();
        let first = ask_cert(f.prompter(1), "SHA256:AA");
        let r1 = f.next_request().await;

        // Another certificate and a host key for the same `host:port` wait their turn.
        let second = ask_cert(f.prompter(2), "SHA256:BB");
        let p3 = f.prompter(3);
        let third = tokio::spawn(async move {
            p3.confirm_host_key("h", 21, "ssh-ed25519", "SHA256:CC", &[])
                .await
        });
        // A different host:port does not wait.
        let p4 = f
            .broker
            .session(SessionId(4), f.site.id.clone(), "other", 21);
        let other = tokio::spawn(async move {
            p4.confirm_certificate("other", 21, "SHA256:AA", &problem())
                .await
        });
        let r_other = f.next_request().await;
        assert_eq!(r_other.key, "other:21");
        f.quiet().await;

        f.broker.answer(r1.id, PromptAnswer::Decision(true));
        assert!(join(first).await);
        // One of the waiters asks now, the other still waits.
        let r2 = f.next_request().await;
        f.quiet().await;
        f.broker.answer(r2.id, PromptAnswer::Decision(false));
        let r3 = f.next_request().await;
        f.broker.answer(r3.id, PromptAnswer::Decision(true));
        let answers = (join(second).await, join(third).await);
        let kinds = [&r2.kind, &r3.kind];
        let cert_first = matches!(kinds[0], PromptKind::Certificate { .. });
        assert_eq!(
            answers,
            if cert_first {
                (false, true)
            } else {
                (true, false)
            }
        );
        assert!(matches!(
            kinds[usize::from(cert_first)],
            PromptKind::HostKey { .. }
        ));

        f.broker.answer(r_other.id, PromptAnswer::Decision(true));
        assert!(join(other).await);
    }

    #[tokio::test]
    async fn password_prompts_are_not_shared() {
        let mut f = fixture();
        let asks: Vec<_> = (1..=2)
            .map(|s| {
                let p = f.prompter(s);
                tokio::spawn(async move { Prompter::password(&p, "u", "h").await })
            })
            .collect();
        let a = f.next_request().await;
        let b = f.next_request().await;
        assert_ne!(a.session, b.session);
        assert_eq!(
            a.kind,
            PromptKind::Password {
                user: "u".into(),
                host: "h".into()
            }
        );
        f.broker.answer(a.id, secret("one", false));
        f.broker.answer(b.id, secret("two", false));
        let mut got: Vec<String> = Vec::new();
        for ask in asks {
            got.push(join(ask).await.unwrap().expose_secret().to_owned());
        }
        got.sort();
        assert_eq!(got, ["one", "two"]);
    }

    #[tokio::test]
    async fn broker_cancel_is_reject() {
        let mut f = fixture();
        let p = f.prompter(1);

        // Closing the dialog refuses, whatever it asked.
        let ask = ask_cert(p.clone(), "SHA256:AA");
        let r = f.next_request().await;
        f.broker.answer(r.id, PromptAnswer::Cancel);
        assert!(!join(ask).await);

        let q = p.clone();
        let ask = tokio::spawn(async move { FtpPrompter::password(&q, "u", "h").await });
        let r = f.next_request().await;
        f.broker.answer(r.id, PromptAnswer::Cancel);
        assert!(join(ask).await.is_none());

        let q = p.clone();
        let ask = tokio::spawn(async move { q.key_passphrase(Path::new("/k")).await });
        let r = f.next_request().await;
        assert_eq!(r.kind, PromptKind::Passphrase { path: "/k".into() });
        f.broker.answer(r.id, PromptAnswer::Cancel);
        assert!(join(ask).await.is_none());

        let q = p.clone();
        let ask = tokio::spawn(async move {
            let prompts = [KbdPrompt {
                text: "Code: ".into(),
                echo: false,
            }];
            q.keyboard_interactive("n", "i", &prompts).await
        });
        let r = f.next_request().await;
        f.broker.answer(r.id, PromptAnswer::Cancel);
        assert!(join(ask).await.is_none());

        // An answer of the wrong shape is a refusal too.
        let ask = ask_cert(p.clone(), "SHA256:AA");
        let r = f.next_request().await;
        f.broker.answer(r.id, secret("x", false));
        assert!(!join(ask).await);

        // Closing the session refuses its open dialog and withdraws it...
        let ask = ask_cert(p.clone(), "SHA256:AA");
        let r = f.next_request().await;
        p.cancel();
        assert!(!join(ask).await);
        assert_eq!(f.next().await, BrokerEvent::Withdrawn(r.id));
        assert!(!f.broker.answer(r.id, PromptAnswer::Decision(true)));
        // ...and every later prompt, without a dialog.
        assert!(Prompter::password(&p, "u", "h").await.is_none());
        assert!(!ask_cert(p.clone(), "SHA256:AA").await.unwrap());
        f.quiet().await;
    }

    #[tokio::test]
    async fn withdrawn_leader_hands_over_to_waiters() {
        let mut f = fixture();
        let leader = f.prompter(1);
        let first = ask_cert(leader.clone(), "SHA256:AA");
        let r1 = f.next_request().await;
        let waiter = ask_cert(f.prompter(2), "SHA256:AA");
        f.quiet().await;

        // The leader's session closes: that is not an answer for the waiter.
        leader.cancel();
        assert!(!join(first).await);
        assert_eq!(f.next().await, BrokerEvent::Withdrawn(r1.id));
        let r2 = f.next_request().await;
        assert_eq!(r2.session, SessionId(2));
        f.broker.answer(r2.id, PromptAnswer::Decision(true));
        assert!(join(waiter).await);

        // A waiter whose own session closes stops waiting.
        let first = ask_cert(f.prompter(1), "SHA256:BB");
        let r = f.next_request().await;
        let p2 = f.prompter(2);
        let waiting = ask_cert(p2.clone(), "SHA256:BB");
        f.quiet().await;
        p2.cancel();
        assert!(!join(waiting).await);
        f.quiet().await;
        f.broker.answer(r.id, PromptAnswer::Decision(true));
        assert!(join(first).await);
    }

    #[tokio::test]
    async fn keyboard_interactive_answers_and_skips_empty_rounds() {
        let mut f = fixture();
        let p = f.prompter(1);
        // A round with no prompts needs no dialog.
        assert_eq!(
            p.keyboard_interactive("", "", &[]).await.map(|r| r.len()),
            Some(0)
        );
        f.quiet().await;

        let ask = tokio::spawn(async move {
            let prompts = [KbdPrompt {
                text: "Code: ".into(),
                echo: false,
            }];
            p.keyboard_interactive("n", "i", &prompts).await
        });
        let r = f.next_request().await;
        assert!(
            matches!(&r.kind, PromptKind::KeyboardInteractive { prompts, .. } if prompts.len() == 1)
        );
        f.broker.answer(
            r.id,
            PromptAnswer::Responses(vec![SecretString::from("123456")]),
        );
        let responses = join(ask).await.unwrap();
        assert_eq!(responses[0].expose_secret(), "123456");
        // Debug of an answer never shows it.
        let debug = format!(
            "{:?}",
            PromptAnswer::Responses(vec![SecretString::from("123456")])
        );
        assert!(!debug.contains("123456"), "{debug}");
    }

    /// A secret answered with "save" waits in the prompter (for the session manager);
    /// one answered without it, of the same kind, drops the earlier one.
    #[tokio::test]
    async fn saved_secret_waits_in_the_prompter() {
        let mut f = fixture();
        let p = f.prompter(1);
        let answer = |value: &'static str, save: bool| {
            let q = p.clone();
            let ask = tokio::spawn(async move { Prompter::password(&q, "u", "h").await });
            (ask, value, save)
        };
        let (ask, value, save) = answer("pw", true);
        let r = f.next_request().await;
        f.broker.answer(r.id, secret(value, save));
        assert_eq!(join(ask).await.unwrap().expose_secret(), "pw");
        let q = p.clone();
        let ask = tokio::spawn(async move { q.key_passphrase(Path::new("/k")).await });
        let r = f.next_request().await;
        f.broker.answer(r.id, secret("phrase", true));
        join(ask).await.unwrap();

        // Only what proved right is handed out (the server accepted the password); the
        // passphrase never decrypted its key, or was never tried, so it is dropped.
        p.secret_accepted(hoprs_ssh::AcceptedSecret::Password);
        let taken: Vec<_> = p
            .take_pending_secrets()
            .into_iter()
            .map(|(kind, s)| (kind.account(), s.expose_secret().to_owned()))
            .collect();
        assert_eq!(taken, [("password", "pw".to_owned())]);
        p.secret_accepted(hoprs_ssh::AcceptedSecret::Passphrase);
        assert!(p.take_pending_secrets().is_empty(), "taken once");

        // A refused secret is never accepted: a new prompt replaces it, and only the
        // new one, once accepted, is handed out. FTP reports its login the same way.
        let (ask, value, save) = answer("refused", true);
        let r = f.next_request().await;
        f.broker.answer(r.id, secret(value, save));
        join(ask).await;
        let (ask, value, save) = answer("right", true);
        let r = f.next_request().await;
        f.broker.answer(r.id, secret(value, save));
        join(ask).await;
        FtpPrompter::password_accepted(&p);
        let taken: Vec<_> = p
            .take_pending_secrets()
            .into_iter()
            .map(|(_, s)| s.expose_secret().to_owned())
            .collect();
        assert_eq!(taken, ["right"]);
        // An acceptance with nothing pending does nothing.
        p.secret_accepted(hoprs_ssh::AcceptedSecret::Password);
        assert!(p.take_pending_secrets().is_empty());

        // Without "save", a new answer of the same kind drops the earlier one.
        let (ask, value, save) = answer("first", true);
        let r = f.next_request().await;
        f.broker.answer(r.id, secret(value, save));
        join(ask).await;
        let (ask, value, save) = answer("second", false);
        let r = f.next_request().await;
        f.broker.answer(r.id, secret(value, save));
        join(ask).await;
        p.secret_accepted(hoprs_ssh::AcceptedSecret::Password);
        assert!(p.take_pending_secrets().is_empty());

        // Discarding forgets them.
        let (ask, value, save) = answer("third", true);
        let r = f.next_request().await;
        f.broker.answer(r.id, secret(value, save));
        join(ask).await;
        p.secret_accepted(hoprs_ssh::AcceptedSecret::Password);
        p.discard_pending_secrets();
        assert!(p.take_pending_secrets().is_empty());
        // Debug of an answer shows the flag, never the secret.
        let debug = format!("{:?}", secret("hunter2", true));
        assert_eq!(debug, "Secret(<redacted>, save: true)");
    }

    /// A session an agent started says so on each of its dialogs; another session's
    /// dialogs do not.
    #[tokio::test]
    async fn agent_prompts_are_flagged() {
        let mut f = fixture();
        let agent = f.prompter(1).by_agent();
        let user = f.prompter(2);

        let q = agent.clone();
        let ask = tokio::spawn(async move { Prompter::password(&q, "u", "h").await });
        let r = f.next_request().await;
        assert!(r.by_agent, "{r:?}");
        assert_eq!(r.session, SessionId(1));
        f.broker.answer(r.id, PromptAnswer::Cancel);
        join(ask).await;

        let ask = ask_cert(agent.clone(), "SHA256:AA");
        let r = f.next_request().await;
        assert!(r.by_agent, "a decision too: {r:?}");
        f.broker.answer(r.id, PromptAnswer::Decision(false));
        join(ask).await;

        let q = user.clone();
        let ask = tokio::spawn(async move { Prompter::password(&q, "u", "h").await });
        let r = f.next_request().await;
        assert!(!r.by_agent, "{r:?}");
        f.broker.answer(r.id, PromptAnswer::Cancel);
        join(ask).await;
    }

    /// `has_open` is true while a dialog of the session is shown or the session waits on
    /// another session's host-key dialog, and false once that ends.
    #[tokio::test]
    async fn has_open_follows_the_dialog() {
        let mut f = fixture();
        let p = f.prompter(1);
        assert!(!f.broker.has_open(SessionId(1)));

        let q = p.clone();
        let ask = tokio::spawn(async move { Prompter::password(&q, "u", "h").await });
        let r = f.next_request().await;
        assert!(f.broker.has_open(SessionId(1)));
        assert!(!f.broker.has_open(SessionId(2)), "another session");
        f.broker.answer(r.id, secret("pw", false));
        join(ask).await;
        assert!(!f.broker.has_open(SessionId(1)));

        // Session 2 waits on session 1's host-key dialog for the same `host:port`.
        let p1 = f.prompter(1);
        let leader = tokio::spawn(async move {
            p1.confirm_host_key("h", 21, "ssh-ed25519", "SHA256:AA", &[])
                .await
        });
        let r = f.next_request().await;
        let p2 = f.prompter(2);
        let waiter = tokio::spawn({
            let p2 = p2.clone();
            async move {
                p2.confirm_host_key("h", 21, "ssh-ed25519", "SHA256:AA", &[])
                    .await
            }
        });
        f.quiet().await;
        assert!(f.broker.has_open(SessionId(1)));
        assert!(
            f.broker.has_open(SessionId(2)),
            "waiting on the other's dialog"
        );
        f.broker.answer(r.id, PromptAnswer::Decision(true));
        assert!(join(leader).await);
        assert!(join(waiter).await);
        assert!(!f.broker.has_open(SessionId(1)));
        assert!(!f.broker.has_open(SessionId(2)));

        // Closing the session ends its dialog.
        let q = p.clone();
        let ask = tokio::spawn(async move { Prompter::password(&q, "u", "h").await });
        let r = f.next_request().await;
        assert!(f.broker.has_open(SessionId(1)));
        p.cancel();
        assert!(join(ask).await.is_none());
        assert_eq!(f.next().await, BrokerEvent::Withdrawn(r.id));
        assert!(!f.broker.has_open(SessionId(1)));
    }

    #[tokio::test]
    async fn store_pin_writes_vault() {
        let f = fixture();
        let p = f.prompter(1);
        p.store_pin("h", 21, "SHA256:AA").await;
        let site = f.vault.site(f.site.id.clone()).await.unwrap().unwrap();
        assert_eq!(site.ftp.pinned_fingerprint.as_deref(), Some("SHA256:AA"));

        // A stale pin other than the current one is left alone; the current one is cleared.
        p.pin_superseded("h", 21, "SHA256:BB").await;
        let site = f.vault.site(f.site.id.clone()).await.unwrap().unwrap();
        assert_eq!(site.ftp.pinned_fingerprint.as_deref(), Some("SHA256:AA"));
        p.pin_superseded("h", 21, "SHA256:AA").await;
        let site = f.vault.site(f.site.id.clone()).await.unwrap().unwrap();
        assert!(site.ftp.pinned_fingerprint.is_none());
    }

    /// A certificate accepted by a session of the site's old endpoint is not pinned on the
    /// site once its host or port was edited: the pin would belong to another server.
    #[tokio::test]
    async fn store_pin_skips_an_edited_site() {
        let f = fixture();
        let p = f.prompter(1);
        let mut moved = f.site.clone();
        moved.host = "elsewhere.example".into();
        f.vault.upsert_site(moved).await.unwrap();
        p.store_pin("h", 21, "SHA256:AA").await;
        let site = f.vault.site(f.site.id.clone()).await.unwrap().unwrap();
        assert_eq!(site.ftp.pinned_fingerprint, None, "host changed");

        let mut moved = f.site.clone();
        moved.port = 2121;
        f.vault.upsert_site(moved).await.unwrap();
        p.store_pin("h", 21, "SHA256:AA").await;
        let site = f.vault.site(f.site.id.clone()).await.unwrap().unwrap();
        assert_eq!(site.ftp.pinned_fingerprint, None, "port changed");

        // Back on the endpoint the session dialled: pinned.
        f.vault.upsert_site(f.site.clone()).await.unwrap();
        p.store_pin("h", 21, "SHA256:AA").await;
        let site = f.vault.site(f.site.id.clone()).await.unwrap().unwrap();
        assert_eq!(site.ftp.pinned_fingerprint.as_deref(), Some("SHA256:AA"));
    }

    /// Plan 6 Task 6: the answers to the credential dialogs (a password kept for the
    /// keyring, a passphrase, keyboard-interactive responses, an FTP password) never
    /// reach the log, at trace, on any thread.
    #[test]
    fn prompt_answers_never_reach_the_log() {
        const PASSWORD: &str = "pw-Broker-Hq61";
        const PASSPHRASE: &str = "phrase-Broker-Lt28";
        const CODE: &str = "code-Broker-Vy05";
        const FTP_PASSWORD: &str = "ftp-Broker-Rn93";
        let _lock = crate::subscriber_test_lock();
        let (buffer, capture) = crate::capture_log(&crate::log_filter(Some("trace")));
        capture.runtime().block_on(async {
            let mut f = fixture();
            let p = f.prompter(1);

            let q = p.clone();
            let ask = tokio::spawn(async move { Prompter::password(&q, "u", "h").await });
            let r = f.next_request().await;
            f.broker.answer(r.id, secret(PASSWORD, true));
            assert_eq!(join(ask).await.unwrap().expose_secret(), PASSWORD);

            let q = p.clone();
            let ask = tokio::spawn(async move { q.key_passphrase(Path::new("/k")).await });
            let r = f.next_request().await;
            f.broker.answer(r.id, secret(PASSPHRASE, true));
            assert_eq!(join(ask).await.unwrap().expose_secret(), PASSPHRASE);

            let q = p.clone();
            let ask = tokio::spawn(async move {
                let prompts = [KbdPrompt {
                    text: "Code: ".into(),
                    echo: false,
                }];
                q.keyboard_interactive("n", "i", &prompts).await
            });
            let r = f.next_request().await;
            f.broker.answer(
                r.id,
                PromptAnswer::Responses(vec![SecretString::from(CODE)]),
            );
            assert_eq!(join(ask).await.unwrap()[0].expose_secret(), CODE);

            // Accepted, then handed to the session manager for the keyring.
            p.secret_accepted(AcceptedSecret::Password);
            p.secret_accepted(AcceptedSecret::Passphrase);
            assert_eq!(p.take_pending_secrets().len(), 2);

            let q = f.prompter(2);
            let ask = tokio::spawn(async move { FtpPrompter::password(&q, "u", "h").await });
            let r = f.next_request().await;
            f.broker.answer(r.id, secret(FTP_PASSWORD, false));
            assert_eq!(join(ask).await.unwrap().expose_secret(), FTP_PASSWORD);
            crate::log_canaries().await;
        });
        drop(capture);
        crate::assert_no_secrets(&buffer, &[PASSWORD, PASSPHRASE, CODE, FTP_PASSWORD]);
    }
}
