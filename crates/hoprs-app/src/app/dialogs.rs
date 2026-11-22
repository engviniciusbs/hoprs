//! The security and credential dialogs (host key, certificate, password, passphrase,
//! keyboard-interactive): the only modals of the app.
//!
//! - **One at a time, in arrival order.** The prompt broker sends a request per dialog;
//!   the first is shown, the others wait in a FIFO queue. A withdrawn request (its
//!   session closed, its connection attempt ended) disappears, shown or queued, without
//!   an answer.
//! - **Closing is refusing.** Esc, Cancel and the close of a dialog answer
//!   [`PromptAnswer::Cancel`].
//! - **The safe button is the default.** iced 0.14 buttons take no keyboard focus, so
//!   Enter in a host key or certificate dialog means Cancel (the one drawn as the default
//!   button); trusting needs a click on "Trust…". Enter in a credential dialog submits it.
//! - **Typed secrets leave the UI on submit.** The fields are taken out of the state into
//!   the [`Effect::AnswerPrompt`]'s `SecretString`s; nothing typed stays in `App`. The
//!   fields' `Debug` never shows them.
//! - **"Save in the system keyring"** goes with the answer (`save: true`): the backend
//!   stores the secret only once the session connects with it. The app reloads the
//!   profiles after that connect, so the site shows its saved password.
//!
//! A *changed* host key is not a dialog: the connection fails with it, and the tab shows a
//! blocking explanation with no way to accept the new key, only "Remove the old entry…"
//! (see [`KnownHostsFix`]).

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt;

use hoprs_core::{Error, SiteId};
use hoprs_ssh::HostKeyChange;
use secrecy::SecretString;

use super::{App, Effect, FileKey, Message, SiteConn, Tab, TabId, TabState};
use crate::agent::AgentEvent;
use crate::backend::broker::{PromptKind, PromptRequest};
use crate::backend::{BrokerEvent, KnownHostEntry, PromptAnswer, SessionId};

/// The dialog shown and the ones waiting.
#[derive(Debug, Default)]
pub struct Dialogs {
    pub current: Option<Dialog>,
    queue: VecDeque<PromptRequest>,
    /// Sessions whose typed secret was answered with "save" (and their site): their next
    /// successful connect stored it, so the profiles are reloaded then.
    saving: HashMap<SessionId, SiteId>,
    /// Sites whose tab was closed while connecting, until that connect's result lands:
    /// a dialog their connect still raises is dropped (and its session closed) unseen.
    abandoned: HashSet<SiteId>,
}

impl Dialogs {
    pub fn is_open(&self) -> bool {
        self.current.is_some()
    }

    /// A secret typed for `session` waits for its connect to be saved.
    #[cfg(test)]
    pub fn is_saving(&self, session: SessionId) -> bool {
        self.saving.contains_key(&session)
    }

    /// The requests waiting behind the current dialog, in order.
    pub fn queued(&self) -> impl Iterator<Item = &PromptRequest> {
        self.queue.iter()
    }
}

/// The open dialog: its request and what the user typed so far.
#[derive(Debug)]
pub struct Dialog {
    pub request: PromptRequest,
    /// One per input: one for a password or passphrase, one per keyboard-interactive
    /// prompt, none for a decision.
    pub fields: Vec<Typed>,
    /// "Save in the system keyring" (password and passphrase only).
    pub save: bool,
    /// The field with the keyboard focus (as far as the app knows: the one focused when
    /// shown, typed in last, or reached with Tab).
    pub focus: usize,
}

impl Dialog {
    fn new(request: PromptRequest) -> Self {
        let inputs = match &request.kind {
            PromptKind::HostKey { .. } | PromptKind::Certificate { .. } => 0,
            PromptKind::Password { .. } | PromptKind::Passphrase { .. } => 1,
            PromptKind::KeyboardInteractive { prompts, .. } => prompts.len(),
        };
        Self {
            request,
            fields: vec![Typed::default(); inputs],
            save: false,
            focus: 0,
        }
    }

    /// A trust decision (host key, certificate) rather than a credential.
    pub fn is_decision(&self) -> bool {
        matches!(
            self.request.kind,
            PromptKind::HostKey { .. } | PromptKind::Certificate { .. }
        )
    }

    /// Whether the dialog offers "Save in the system keyring".
    pub fn can_save(&self) -> bool {
        matches!(
            self.request.kind,
            PromptKind::Password { .. } | PromptKind::Passphrase { .. }
        )
    }
}

/// A dialog field as typed. Its `Debug` never shows it.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Typed(pub String);

impl fmt::Debug for Typed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Typed(***)")
    }
}

/// Something in the open dialog.
#[derive(Debug, Clone)]
pub enum DialogMsg {
    /// Field `index` changed.
    Input(usize, Typed),
    /// "Save in the system keyring" toggled.
    Save(bool),
    /// Send the typed credentials.
    Submit,
    /// "Trust and connect" / "Trust this certificate".
    Trust,
    /// Cancel, Esc or the dialog's close: a refusal.
    Cancel,
}

/// "Remove the old entry…" of a tab whose host key changed.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum KnownHostsFix {
    /// The explanation, with the action.
    #[default]
    Idle,
    /// Reading the `known_hosts` lines that would be removed.
    Checking,
    /// Asking to remove these lines (empty: none is left).
    Confirm(Vec<KnownHostEntry>),
    Removing,
    /// Reading or removing failed.
    Failed(Error),
}

#[derive(Debug, Clone)]
pub enum KnownHostsMsg {
    /// "Remove the old entry…": look up what would go.
    Start,
    /// Remove the lines shown.
    Confirm,
    /// Keep them.
    Cancel,
}

/// What a failed tab offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureAction {
    /// Connect again (the sidebar's connect path).
    Reconnect,
    EditSite,
    /// The host key changed: remove the old `known_hosts` entry (never accept the key).
    RemoveOldEntry,
    /// The key changed from the one this session verified: close it and start a new one.
    CloseAndReconnect,
}

impl SiteConn {
    /// The host key changed: why this connection failed, if so.
    ///
    /// A first connect fails with it (`known_hosts` has another key). A session that
    /// already connected pins its key, so a reconnect that meets another one leaves the
    /// site connected with its feed reporting `Disconnected` with the change.
    pub fn host_key_change(&self) -> Option<HostKeyChange> {
        match (&self.state, &self.conn) {
            (TabState::Failed(failure), _) => hoprs_ssh::host_key_change(&failure.error),
            (TabState::Connected(_), Some(hoprs_core::ConnectionState::Disconnected(error))) => {
                hoprs_ssh::host_key_change(error)
            }
            (TabState::Connecting | TabState::Connected(_), _) => None,
        }
    }
}

impl App {
    /// What the agent side reports: the numbers the status bar and the sidebar read, the
    /// channel's error, and the end of a connect the user answered with "save". Nothing
    /// else changes: no tab, no focus, no dialog.
    pub(super) fn agent_event(&mut self, event: AgentEvent) -> Vec<Effect> {
        match event {
            AgentEvent::Relays(relays) => self.agents.relays = relays,
            AgentEvent::Sites(sites) => self.agents.sites = sites.into_iter().collect(),
            AgentEvent::Connected(_, session) => return self.settle_saving(session, true),
            AgentEvent::Channel(error) => self.agents.error = error,
        }
        Vec::new()
    }

    /// A request or withdrawal from the broker.
    pub(super) fn prompt_event(&mut self, event: BrokerEvent) -> Vec<Effect> {
        match event {
            BrokerEvent::Request(request) => {
                // A request of an agent's session is not a tab's: closing the last tab of
                // the site while it connected does not abandon it (the user may still
                // answer or cancel it).
                if !request.by_agent && self.dialogs.abandoned.contains(&request.site) {
                    // Its tab is gone: closing the session withdraws the dialog.
                    self.forget_session(request.session);
                    return vec![Effect::CloseSession(request.session)];
                }
                if self.dialogs.current.is_some() {
                    self.dialogs.queue.push_back(request);
                    return Vec::new();
                }
                self.show(request)
            }
            BrokerEvent::Withdrawn(id) => {
                if self.dialogs.current.as_ref().map(|d| d.request.id) == Some(id) {
                    self.dialogs.current = None;
                    return self.show_next();
                }
                self.dialogs.queue.retain(|r| r.id != id);
                Vec::new()
            }
        }
    }

    /// The window is closing: every dialog, shown or waiting, is answered Cancel (as
    /// closing it would) and none is shown any more.
    pub(super) fn cancel_all_prompts(&mut self) -> Vec<Effect> {
        let shown = self.dialogs.current.take().map(|d| d.request.id);
        shown
            .into_iter()
            .chain(self.dialogs.queue.drain(..).map(|r| r.id))
            .map(|id| Effect::AnswerPrompt {
                id,
                answer: PromptAnswer::Cancel,
            })
            .collect()
    }

    /// Shows `request`, focusing its first field if it has one.
    fn show(&mut self, request: PromptRequest) -> Vec<Effect> {
        let dialog = Dialog::new(request);
        let fields = !dialog.fields.is_empty();
        self.dialogs.current = Some(dialog);
        // The keyboard leaves whatever had it behind the dialog (a rename editor, a form
        // field, the sidebar filter): `opaque` only stops the mouse.
        if fields {
            vec![Effect::FocusDialogField(0)]
        } else {
            vec![Effect::Unfocus]
        }
    }

    fn show_next(&mut self) -> Vec<Effect> {
        match self.dialogs.queue.pop_front() {
            Some(request) => self.show(request),
            None => Vec::new(),
        }
    }

    /// Closes the open dialog with `answer` and shows the next one.
    fn close_dialog(&mut self, answer: impl FnOnce(&mut Dialog) -> PromptAnswer) -> Vec<Effect> {
        let Some(mut dialog) = self.dialogs.current.take() else {
            return Vec::new();
        };
        let answer = answer(&mut dialog);
        let mut effects = vec![Effect::AnswerPrompt {
            id: dialog.request.id,
            answer,
        }];
        effects.extend(self.show_next());
        effects
    }

    /// Something in the open dialog.
    pub(super) fn dialog_msg(&mut self, msg: DialogMsg) -> Vec<Effect> {
        let Some(dialog) = &mut self.dialogs.current else {
            return Vec::new();
        };
        match msg {
            DialogMsg::Input(index, typed) => {
                if let Some(field) = dialog.fields.get_mut(index) {
                    *field = typed;
                    dialog.focus = index;
                }
                Vec::new()
            }
            DialogMsg::Save(save) => {
                dialog.save = save && dialog.can_save();
                Vec::new()
            }
            DialogMsg::Submit if !dialog.is_decision() => {
                if dialog.save && dialog.can_save() {
                    let request = &dialog.request;
                    self.dialogs
                        .saving
                        .insert(request.session, request.site.clone());
                }
                self.close_dialog(answer_of)
            }
            DialogMsg::Trust if dialog.is_decision() => {
                self.close_dialog(|_| PromptAnswer::Decision(true))
            }
            DialogMsg::Cancel => self.close_dialog(|_| PromptAnswer::Cancel),
            // Trust on a credential, Submit on a decision: not this dialog's.
            DialogMsg::Submit | DialogMsg::Trust => Vec::new(),
        }
    }

    /// A key while a dialog is open: it belongs to the dialog, never to the panes. Esc
    /// refuses; Enter is the default button (Cancel for a trust decision, since iced
    /// buttons take no focus; submit for credentials); Tab and Shift+Tab cycle through the
    /// dialog's own fields only (a decision has none: nothing to move to).
    pub(super) fn dialog_key(&mut self, key: FileKey, shift: bool) -> Vec<Effect> {
        let Some(dialog) = &mut self.dialogs.current else {
            return Vec::new();
        };
        match key {
            FileKey::Escape => self.dialog_msg(DialogMsg::Cancel),
            FileKey::Enter if dialog.is_decision() => self.dialog_msg(DialogMsg::Cancel),
            FileKey::Enter => self.dialog_msg(DialogMsg::Submit),
            FileKey::SwitchPane if !dialog.fields.is_empty() => {
                let n = dialog.fields.len();
                dialog.focus = if shift {
                    (dialog.focus + n - 1) % n
                } else {
                    (dialog.focus + 1) % n
                };
                vec![Effect::FocusDialogField(dialog.focus)]
            }
            _ => Vec::new(),
        }
    }

    /// While a dialog or the palette is open, the window behind it takes no input: the
    /// messages its widgets and keys send are dropped (results of work in flight still
    /// land). The mouse is stopped by the backdrop; this covers what the keyboard could
    /// still reach. Under a dialog, the palette does not open either.
    pub(super) fn blocked_by_overlay(&self, msg: &Message) -> bool {
        if self.dialogs.is_open() {
            return matches!(msg, Message::Palette(_)) || behind_overlay(msg);
        }
        self.palette.is_some() && behind_overlay(msg)
    }

    /// A session's connect finished: a secret answered with "save" was stored if it
    /// connected, so the profiles are reloaded.
    pub(super) fn settle_saving(&mut self, session: SessionId, connected: bool) -> Vec<Effect> {
        if self.dialogs.saving.remove(&session).is_some() && connected {
            vec![self.load_profiles()]
        } else {
            Vec::new()
        }
    }

    /// What the failed tab offers: never a way to accept a changed host key.
    /// - The key differs from `known_hosts`: "Remove the old entry…".
    /// - The key differs from the one this session verified when it first connected
    ///   (`line: None`): editing `known_hosts` cannot help the session, which keeps that
    ///   key; "Close and reconnect" starts a new session, which checks `known_hosts` again.
    pub fn failure_actions(&self, tab: &Tab) -> Vec<FailureAction> {
        let mut actions = Vec::new();
        if self.site(&tab.site).is_none() {
            return actions;
        }
        actions.push(match self.conn(&tab.site).host_key_change() {
            Some(HostKeyChange { line: Some(_) }) => FailureAction::RemoveOldEntry,
            Some(HostKeyChange { line: None }) => FailureAction::CloseAndReconnect,
            None => FailureAction::Reconnect,
        });
        actions.push(FailureAction::EditSite);
        actions
    }

    /// The site's connection failed or was lost: it remembers the site's `host:port` as
    /// it is now, the endpoint "Remove the old entry…" works on even if the site is
    /// edited later.
    pub(super) fn note_failed_endpoint(&mut self, site: &SiteId) {
        let endpoint = self.site(site).map(|s| (s.host.clone(), s.port));
        self.conn_mut(site).failed_endpoint = endpoint;
    }

    /// The `host:port` the site failed with, if its host key changed in `known_hosts`.
    fn changed_endpoint(&self, site: &SiteId) -> Option<(String, u16)> {
        let conn = self.conn(site);
        conn.host_key_change()?.line?;
        conn.failed_endpoint.clone()
    }

    /// "Close and reconnect" (or a removed `known_hosts` entry) on a tab: closes the
    /// failed or lost session of its site and connects the site with a new one, for all
    /// its tabs.
    pub(super) fn restart_session(&mut self, tab: TabId) -> Vec<Effect> {
        match self.tab_index(tab) {
            Some(index) => self.restart_site(self.tabs[index].site.clone()),
            None => Vec::new(),
        }
    }

    /// Closes the site's failed or lost session and connects it with a new one (the
    /// effect closes first, then connects: a reused session would keep its old state).
    fn restart_site(&mut self, site: SiteId) -> Vec<Effect> {
        if self.tab_of(&site).is_none() {
            return Vec::new();
        }
        let conn = self.conn_mut(&site);
        let lost = conn.lost_connection().is_some();
        let close = match &conn.state {
            TabState::Failed(failure) => failure.session,
            TabState::Connected(connected) if lost => Some(connected.session),
            TabState::Connected(_) | TabState::Connecting => return Vec::new(),
        };
        *conn = SiteConn::connecting();
        if let Some(session) = close {
            self.forget_session(session);
        }
        vec![Effect::RestartSite { close, site }]
    }

    /// A session is gone: nothing waits to be saved for it.
    pub(super) fn forget_session(&mut self, session: SessionId) {
        self.dialogs.saving.remove(&session);
    }

    /// The last tab of `site` closed while it connected (the app doesn't know that
    /// connect's session yet; a site's tabs all share it, so the site names it). Its
    /// dialogs, shown or waiting, are dropped unanswered, and the sessions they and the
    /// secrets typed with "save" came from are closed: the broker withdraws their dialogs
    /// and the session manager forgets the secrets before the connect can store them.
    /// Not answered Cancel: a host key refusal would be shared with other sessions
    /// waiting on the same `host:port`, while a withdrawal makes them ask again. Until
    /// the connect's result lands, a new dialog of the site is dropped the same way.
    /// A request of an agent's session (`by_agent`) is neither dropped nor closed here.
    pub(super) fn abandon_site_prompts(&mut self, site: &SiteId) -> Vec<Effect> {
        self.dialogs.abandoned.insert(site.clone());
        let mut sessions = BTreeSet::new();
        self.dialogs.queue.retain(|r| {
            let mine = &r.site == site && !r.by_agent;
            if mine {
                sessions.insert(r.session);
            }
            !mine
        });
        self.dialogs.saving.retain(|session, of| {
            let mine = of == site;
            if mine {
                sessions.insert(*session);
            }
            !mine
        });
        let mut effects = Vec::new();
        if let Some(dialog) = self
            .dialogs
            .current
            .take_if(|d| &d.request.site == site && !d.request.by_agent)
        {
            sessions.insert(dialog.request.session);
            effects.extend(self.show_next());
        }
        sessions
            .into_iter()
            .map(Effect::CloseSession)
            .chain(effects)
            .collect()
    }

    /// The site connects again (a new tab), or the abandoned connect's result landed:
    /// its dialogs are shown again.
    pub(super) fn readopt_site(&mut self, site: &SiteId) {
        self.dialogs.abandoned.remove(site);
    }

    /// "Remove the old entry…" and its confirmation, from any tab of the site (the flow
    /// is the site's: its results land even if that tab closed meanwhile).
    pub(super) fn known_hosts_msg(&mut self, tab: TabId, msg: KnownHostsMsg) -> Vec<Effect> {
        let Some(index) = self.tab_index(tab) else {
            return Vec::new();
        };
        let site = self.tabs[index].site.clone();
        let Some((host, port)) = self.changed_endpoint(&site) else {
            return Vec::new();
        };
        let fix = &mut self.conn_mut(&site).known_hosts;
        match (msg, &*fix) {
            (KnownHostsMsg::Start, KnownHostsFix::Idle | KnownHostsFix::Failed(_)) => {
                *fix = KnownHostsFix::Checking;
                vec![Effect::KnownHostEntries { site, host, port }]
            }
            (KnownHostsMsg::Confirm, KnownHostsFix::Confirm(entries)) => {
                let expected = entries.clone();
                *fix = KnownHostsFix::Removing;
                vec![Effect::RemoveKnownHost {
                    site,
                    host,
                    port,
                    expected,
                }]
            }
            (KnownHostsMsg::Cancel, KnownHostsFix::Confirm(_)) => {
                *fix = KnownHostsFix::Idle;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn known_host_entries(
        &mut self,
        site: SiteId,
        result: Result<Vec<KnownHostEntry>, Error>,
    ) -> Vec<Effect> {
        if let Some(conn) = self.conns.get_mut(&site)
            && conn.known_hosts == KnownHostsFix::Checking
        {
            conn.known_hosts = match result {
                Ok(entries) => KnownHostsFix::Confirm(entries),
                Err(error) => KnownHostsFix::Failed(error),
            };
        }
        Vec::new()
    }

    /// Removed: the site connects again, and that connect asks about the new key.
    pub(super) fn known_host_removed(
        &mut self,
        site: SiteId,
        result: Result<usize, Error>,
    ) -> Vec<Effect> {
        let Some(conn) = self.conns.get_mut(&site) else {
            return Vec::new();
        };
        if conn.known_hosts != KnownHostsFix::Removing {
            return Vec::new();
        }
        match result {
            // A new session checks `known_hosts` again (the failed one is closed).
            Ok(_) => self.restart_site(site),
            Err(error) => {
                conn.known_hosts = KnownHostsFix::Failed(error);
                Vec::new()
            }
        }
    }
}

/// The answer a dialog's fields make, taking them out of the dialog.
fn take_secret(field: &mut Typed) -> SecretString {
    SecretString::from(std::mem::take(&mut field.0))
}

/// The answer of a credential dialog: its fields, taken out of it (nothing typed stays
/// in the state).
fn answer_of(dialog: &mut Dialog) -> PromptAnswer {
    match &dialog.request.kind {
        PromptKind::KeyboardInteractive { .. } => {
            PromptAnswer::Responses(dialog.fields.iter_mut().map(take_secret).collect())
        }
        _ => PromptAnswer::Secret {
            secret: dialog
                .fields
                .first_mut()
                .map(take_secret)
                .unwrap_or_default(),
            save: dialog.save,
        },
    }
}

/// Whether `msg` is input from the window behind a dialog or the palette.
fn behind_overlay(msg: &Message) -> bool {
    match msg {
        Message::Edit(msg) => msg.is_user(),
        Message::Update(msg) => msg.is_user(),
        Message::Export(msg) => msg.is_user(),
        // Scroll offsets the lists report (also after a programmatic scroll).
        Message::Pane {
            msg: super::PaneMsg::Scrolled { .. } | super::PaneMsg::Placed(_),
            ..
        }
        | Message::Queue(super::QueueMsg::Scrolled { .. }) => false,
        Message::ToggleQueue
        | Message::FilterSites(_)
        | Message::ToggleWorkspace(_)
        | Message::OpenWorkspace(_)
        | Message::ConnectSite(_)
        | Message::SelectTab(_)
        | Message::NewSite
        | Message::EditSite(_)
        | Message::NewWorkspace
        | Message::EditWorkspace(_)
        | Message::SiteForm(_)
        | Message::WorkspaceForm(_)
        | Message::OpenProfileFolder
        | Message::CloseTab(_)
        | Message::CloseOtherTabs(_)
        | Message::Compare(super::CompareMsg::ContextMenu { .. })
        | Message::NewTab { .. }
        | Message::Pane { .. }
        | Message::Key { .. }
        | Message::Queue(_)
        | Message::FileDropped(_)
        | Message::KnownHosts { .. }
        | Message::RestartSession(_)
        | Message::OpenImport
        | Message::Import(_)
        | Message::OpenSettings
        | Message::SelectSettings
        | Message::CloseSettings
        | Message::Settings(_)
        | Message::PointerPressed(_)
        | Message::PointerMoved(_)
        | Message::Divider(_)
        | Message::Menu(_)
        | Message::Sidebar(_) => true,
        // The terminal's keyboard, mouse and search: never under a dialog or the palette
        // (its timers and replies still land).
        Message::Terminal { msg, .. } => !matches!(
            msg,
            super::TerminalMsg::ResizeDue(_)
                | super::TerminalMsg::SearchDue(_)
                | super::TerminalMsg::InputSent(_)
                | super::TerminalMsg::Pasted(_)
                | super::TerminalMsg::Resized { .. }
        ),
        _ => false,
    }
}
