//! Sessions over time and the app's own lifecycle: the connection state each session's
//! feed reports (status bar, reconnect banner), the tabs a workspace saves when it
//! closes, and the orderly shutdown when the window closes (Review Focus 5).
//!
//! The *open* workspace is the one "Open workspace" opened last: opening another saves
//! the first one's tabs and makes the new one open (the first one's tabs stay). It is
//! the one whose tabs the window close saves. "Close tabs" on it saves its tabs, then
//! closes every tab of its sites, also a tab opened from "All sites" for one of them.
//!
//! Closing the window: unsent remote edits are asked about first, before anything else
//! happens (the window stays open meanwhile, see [`super::edit`]). Then the app asks
//! for one [`Effect::Shutdown`], which saves the open
//! workspace's tabs, shuts the transfer engine down (its running items keep their stored
//! state and reopen paused on the next start), then closes the sessions, all within
//! 5 s (the save gets 2 s of them). Only its [`Message::ShutdownDone`] closes the window,
//! whatever the outcome. A failed or slow save is logged and the shutdown goes on; a
//! backend that hangs or panics is abandoned. Meanwhile the dialogs are answered Cancel
//! and the user's input is ignored; a second close request closes at once.

use hoprs_core::{
    ConnectionState, Error, ErrorKind, ReconnectPolicy, SavedTab, TabKind, WorkspaceId,
};

use super::{App, Effect, Message, TabState};
use crate::app::files::{Listing, Pane, local_fs_path};
use crate::app::tabs::TabId;
use crate::backend::{AuthMethod, BrokerEvent, PromptAnswer, SessionId, Side, WorkspaceTabs};

/// What the status bar says about a tab's connection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConnStatus<'a> {
    Connecting,
    Connected,
    /// Attempt `attempt` of `of`.
    Reconnecting {
        attempt: u32,
        of: u32,
    },
    /// A prompt holds the connection until the user answers.
    Waiting,
    Disconnected(&'a Error),
}

impl super::SiteConn {
    /// The connection as the status bar shows it: the connect's outcome, then what the
    /// session's feed reports.
    pub fn conn_status(&self) -> ConnStatus<'_> {
        match &self.state {
            TabState::Connecting => ConnStatus::Connecting,
            TabState::Failed(failure) => ConnStatus::Disconnected(&failure.error),
            TabState::Connected(_) => match &self.conn {
                None | Some(ConnectionState::Connected) => ConnStatus::Connected,
                Some(ConnectionState::Connecting) => ConnStatus::Connecting,
                Some(ConnectionState::Reconnecting { attempt }) => ConnStatus::Reconnecting {
                    attempt: *attempt,
                    // Sessions use the default policy (see `backend::mapping`).
                    of: ReconnectPolicy::default().max_attempts,
                },
                Some(ConnectionState::WaitingForUser) => ConnStatus::Waiting,
                Some(ConnectionState::Disconnected(error)) => ConnStatus::Disconnected(error),
            },
        }
    }

    /// Why a connected session is lost for good (it gave up reconnecting, or was refused):
    /// the tab shows it in a banner with "Reconnect" (or, if the host key changed, the
    /// blocking explanation with "Close and reconnect").
    pub fn lost_connection(&self) -> Option<&Error> {
        match (&self.state, &self.conn) {
            (TabState::Connected(_), Some(ConnectionState::Disconnected(error))) => Some(error),
            _ => None,
        }
    }
}

impl super::Tab {
    /// The tab as a workspace saves it: a terminal tab as such (a restored one opens a new
    /// shell), a files tab at its two folders. A side whose listing failed keeps what
    /// `previous` saved for it: a folder that could not be read is not worth restoring.
    fn saved(&self, previous: Option<&SavedTab>) -> SavedTab {
        let Some(files) = self.files() else {
            return SavedTab {
                kind: TabKind::Terminal,
                remote_dir: None,
                local_dir: None,
            };
        };
        let failed = |pane: &Pane| matches!(pane.listing, Listing::Failed(_));
        SavedTab {
            kind: TabKind::Files,
            remote_dir: if failed(&files.remote) {
                previous.and_then(|t| t.remote_dir.clone())
            } else {
                Some(files.remote.path.as_str().to_owned())
            },
            local_dir: if failed(&files.local) {
                previous.and_then(|t| t.local_dir.clone())
            } else {
                Some(local_fs_path(&files.local.path))
            },
        }
    }
}

impl App {
    /// The sessions whose state the app follows: those of the connected tabs, each once
    /// (one feed per session, however many tabs share it). A session whose last tab
    /// closes or fails drops out, which ends its feed.
    pub fn watched_sessions(&self) -> Vec<SessionId> {
        let mut sessions: Vec<SessionId> = Vec::new();
        for tab in &self.tabs {
            if let Some(session) = self.conn(&tab.site).session()
                && !sessions.contains(&session)
            {
                sessions.push(session);
            }
        }
        sessions
    }

    /// How many of the workspace's sites have a connected tab, of all its sites.
    pub fn workspace_connected(&self, id: &WorkspaceId) -> Option<(usize, usize)> {
        let workspace = self.workspace(id)?;
        let connected = workspace
            .sites
            .iter()
            .filter(|entry| {
                self.tab_of(&entry.site)
                    .is_some_and(|_| self.conn(&entry.site).conn_status() == ConnStatus::Connected)
            })
            .count();
        Some((connected, workspace.sites.len()))
    }

    /// A session's feed: its state and, once logged in, how, for its site's connection
    /// (all its tabs). Only a connected site takes it (a failed one follows its connect's
    /// outcome). Back to `Connected` after a loss, the site's transfers run again and a
    /// listing that failed meanwhile is retried.
    pub(super) fn session_state(
        &mut self,
        session: SessionId,
        state: ConnectionState,
        auth: Option<AuthMethod>,
    ) -> Vec<Effect> {
        let Some(site) = self.site_of_session(session) else {
            return Vec::new();
        };
        let conn = self.conn_mut(&site);
        let back = state == ConnectionState::Connected
            && conn
                .conn
                .as_ref()
                .is_some_and(|c| *c != ConnectionState::Connected);
        let refused = match &state {
            ConnectionState::Disconnected(error) if error.kind != ErrorKind::Network => {
                Some(error.clone())
            }
            _ => None,
        };
        let dropped = match &state {
            ConnectionState::Disconnected(error) => Some(error.clone()),
            _ => None,
        };
        let lost = dropped.is_some();
        let down = state != ConnectionState::Connected;
        if auth.is_some() {
            conn.auth = auth;
        }
        conn.conn = Some(state);
        if lost {
            self.note_failed_endpoint(&site);
        }
        if down {
            self.site_connection_down(&site);
            // A drag in one of the site's tabs ends: its target (or source) went away.
            if let Some(drag) = self.drag
                && self
                    .tab_index(drag.tab)
                    .is_some_and(|i| self.tabs[i].site == site)
            {
                self.drag = None;
            }
        }
        // Refused for good: the actions waiting for a reconnection fail now. Given up for a
        // network reason while they waited for the transport's own round: the app
        // reconnects for them.
        let mut effects = match &refused {
            Some(error) => self.fail_held(&site, error),
            None if lost => self.reconnect_for_held(&site).into_iter().collect(),
            None => Vec::new(),
        };
        // A comparison on the dropped session stops with the reason.
        if let Some(error) = &dropped {
            for index in self.tabs_of(&site) {
                effects.extend(self.compare_session_gone(index, error));
            }
        }
        if !back {
            return effects;
        }
        self.conn_mut(&site).reconnecting = false;
        self.queue.site_restored(&site);
        // The actions that waited for the connection run (once).
        effects.extend(self.release_held(&site));
        // The shells died with the old connection: the terminals open new ones.
        effects.extend(self.open_site_shells(&site, true));
        for index in self.tabs_of(&site) {
            if self.tabs[index]
                .pane(Side::Remote)
                .is_some_and(|pane| matches!(pane.listing, Listing::Failed(_)))
            {
                effects.extend(self.request_listing(index, Side::Remote));
            }
        }
        effects
    }

    /// "Reconnect" in the banner of any tab of the session: the backend's reconnect
    /// (which also settles the secrets typed with "Save in the system keyring"), one at
    /// a time for the session.
    pub(super) fn reconnect_session(&mut self, tab: TabId) -> Vec<Effect> {
        let Some(index) = self.tab_index(tab) else {
            return Vec::new();
        };
        let site = self.tabs[index].site.clone();
        let conn = self.conn_mut(&site);
        let Some(session) = conn.session() else {
            return Vec::new();
        };
        // A changed host key fails every reconnect of this session: the banner offers
        // "Close and reconnect" instead.
        if conn.reconnecting || conn.lost_connection().is_none() || conn.host_key_change().is_some()
        {
            return Vec::new();
        }
        conn.reconnecting = true;
        vec![Effect::Reconnect { session }]
    }

    /// The reconnect finished, for the site still on `session` (the tab that asked may be
    /// gone, its siblings not): connected, the remote folders are listed again; refused,
    /// the banners show the new reason. A result for a session no tab has any more is
    /// dropped.
    pub(super) fn reconnected(
        &mut self,
        session: SessionId,
        result: Result<(), Error>,
    ) -> Vec<Effect> {
        let Some(site) = self.site_of_session(session) else {
            return Vec::new();
        };
        let mut effects = self.settle_saving(session, result.is_ok());
        let conn = self.conn_mut(&site);
        // The feed may have said `Connected` first: the shells reopen once.
        let back = conn
            .conn
            .as_ref()
            .is_some_and(|c| *c != ConnectionState::Connected);
        conn.reconnecting = false;
        conn.conn = Some(match &result {
            Ok(()) => ConnectionState::Connected,
            Err(error) => ConnectionState::Disconnected(error.clone()),
        });
        match result {
            Ok(()) => {
                self.queue.site_restored(&site);
                // The actions that waited for the connection run (once), and the other
                // remote folders are listed again.
                effects.extend(self.release_held(&site));
                for index in self.tabs_of(&site) {
                    let replayed = self.tabs[index]
                        .pane(Side::Remote)
                        .and_then(|pane| pane.request)
                        .is_some_and(|request| self.replayed.remove(&request));
                    if !replayed {
                        effects.extend(self.request_listing(index, Side::Remote));
                    }
                }
                if back {
                    effects.extend(self.open_site_shells(&site, true));
                }
                self.forget_stale_replays();
            }
            Err(error) => {
                self.note_failed_endpoint(&site);
                effects.extend(self.fail_held(&site, &error));
            }
        }
        effects
    }

    /// The tabs of the workspace's sites as it saves them: each site with tabs gets them
    /// all, terminals included, in strip order (its n-th files tab falls back on the n-th
    /// files tab it saved before, for a folder that failed to list). Sites without a tab
    /// are not named, so they keep what they had. `None` when there is nothing to save.
    fn workspace_tabs(&self, id: &WorkspaceId) -> Option<WorkspaceTabs> {
        let workspace = self.workspace(id)?;
        let tabs: WorkspaceTabs = workspace
            .sites
            .iter()
            .filter_map(|entry| {
                let open = self.tabs_of(&entry.site);
                if open.is_empty() {
                    return None;
                }
                let mut previous = entry.tabs.iter().filter(|t| t.kind == TabKind::Files);
                let saved = open
                    .into_iter()
                    .map(|index| {
                        let tab = &self.tabs[index];
                        let before = if tab.is_files() {
                            previous.next()
                        } else {
                            None
                        };
                        tab.saved(before)
                    })
                    .collect();
                Some((entry.site.clone(), saved))
            })
            .collect();
        (!tabs.is_empty()).then_some(tabs)
    }

    pub(super) fn save_tabs_effect(&self, id: &WorkspaceId) -> Option<Effect> {
        let tabs = self.workspace_tabs(id)?;
        Some(Effect::SaveTabs {
            workspace: id.clone(),
            tabs,
        })
    }

    /// "Close workspace": saves its tabs, then closes them all and their sessions. Tabs
    /// of other sites stay.
    pub(super) fn close_workspace(&mut self, id: &WorkspaceId) -> Vec<Effect> {
        if self.open_workspace.as_ref() != Some(id) {
            return Vec::new();
        }
        // Unsent remote edits on its sites: the question first (see `edit`).
        if let Some(effects) = self.ask_before_closing_workspace(id) {
            return effects;
        }
        self.close_workspace_now(id)
    }

    /// "Close workspace" once its edits are settled.
    pub(super) fn close_workspace_now(&mut self, id: &WorkspaceId) -> Vec<Effect> {
        if self.open_workspace.as_ref() != Some(id) {
            return Vec::new();
        }
        let mut effects: Vec<Effect> = self.save_tabs_effect(id).into_iter().collect();
        self.open_workspace = None;
        let sites: Vec<_> = self
            .workspace(id)
            .map(|w| w.sites.iter().map(|e| e.site.clone()).collect())
            .unwrap_or_default();
        for site in sites {
            effects.extend(self.close_site_tabs(&site));
        }
        effects
    }

    /// Saved: the profiles are read again, so the next "Open workspace" restores these
    /// folders. A failure is logged only (the tabs are not worth an error in the UI).
    pub(super) fn tabs_saved(&mut self, result: Result<(), Error>) -> Vec<Effect> {
        match result {
            Ok(()) => vec![self.load_profiles()],
            Err(error) => {
                tracing::warn!(%error, "cannot save the workspace's tabs");
                Vec::new()
            }
        }
    }

    /// The window asks to close: one orderly shutdown (see the module docs). The dialogs
    /// open or waiting are answered Cancel first, so no prompt holds a connect while the
    /// backend stops. A second request while closing closes at once (force close): the
    /// user insists, or the shutdown never reported back.
    pub(super) fn close_requested(&mut self, window: iced::window::Id) -> Vec<Effect> {
        if self.closing {
            tracing::warn!("close requested again while shutting down; closing now");
            return vec![Effect::ExitWindow(window)];
        }
        // Unsent remote edits: the question comes before the save (Review Focus 4).
        if let Some(effects) = self.ask_before_quitting(window) {
            return effects;
        }
        self.begin_close(window)
    }

    /// The close flow proper (see the module docs): from here on the window closes.
    pub(super) fn begin_close(&mut self, window: iced::window::Id) -> Vec<Effect> {
        self.closing = true;
        self.center_ask = None;
        if matches!(self.form, Some(super::Form::QuitEdits)) {
            self.form = None;
        }
        let mut effects = self.cancel_all_prompts();
        let save = self
            .open_workspace
            .clone()
            .and_then(|id| self.workspace_tabs(&id).map(|tabs| (id, tabs)));
        effects.push(Effect::Shutdown { save, window });
        effects
    }

    /// While closing, what a message does instead of its usual effect, if anything: the
    /// user's input is ignored (nothing new may start once the engine stops: no connect,
    /// transfer, file operation or form), and a new dialog is answered Cancel unseen.
    /// Results of work already running (listings, connects, engine events...) and the
    /// shutdown's own messages go through as usual.
    pub(super) fn while_closing(&self, msg: &Message) -> Option<Vec<Effect>> {
        match msg {
            Message::Prompt(BrokerEvent::Request(request)) => Some(vec![Effect::AnswerPrompt {
                id: request.id,
                answer: PromptAnswer::Cancel,
            }]),
            Message::Edit(msg) if msg.is_user() => Some(Vec::new()),
            Message::Export(msg) if msg.is_user() => Some(Vec::new()),
            // Nothing new starts, an automatic check included.
            Message::Update(msg)
                if msg.is_user() || matches!(msg, super::UpdateMsg::CheckDue(_)) =>
            {
                Some(Vec::new())
            }
            // Where the lists are and how far they scrolled (not input): stored as usual,
            // else a list's bounds reporter would publish them again every event.
            Message::Pane {
                msg: super::PaneMsg::Scrolled { .. } | super::PaneMsg::Placed(_),
                ..
            }
            | Message::Queue(super::QueueMsg::Scrolled { .. }) => None,
            Message::ToggleQueue
            | Message::FilterSites(_)
            | Message::ReloadProfiles
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
            | Message::KeyPressed { .. }
            | Message::Key { .. }
            | Message::Modifiers(_)
            | Message::Queue(_)
            | Message::FileDropped(_)
            | Message::FilesHovered(_)
            | Message::DragEnd
            | Message::DragCancel
            | Message::Dialog(_)
            | Message::KnownHosts { .. }
            | Message::RestartSession(_)
            | Message::ReconnectSession(_)
            | Message::CloseWorkspace(_)
            | Message::OpenImport
            | Message::Import(_)
            | Message::OpenSettings
            | Message::SelectSettings
            | Message::CloseSettings
            | Message::Settings(_)
            | Message::Palette(_)
            | Message::PointerPressed(_)
            | Message::PointerMoved(_)
            | Message::Divider(_)
            | Message::Menu(_)
            | Message::Sidebar(_)
            | Message::Terminal { .. } => Some(Vec::new()),
            _ => None,
        }
    }
}
