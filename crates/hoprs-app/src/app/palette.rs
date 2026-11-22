//! The Ctrl+K command palette (Plan 6 Task 9): an overlay at the top of the window, with
//! a search field and up to [`MAX_RESULTS`] rows.
//!
//! **What it lists.** Only what makes sense now ([`App::palette_entries`]):
//! - the active tab's own actions (the focused pane's F7 / F2 / Del / F4 / Ctrl+Enter, the
//!   terminal's copy, paste and search), then Reconnect, the new tabs on its session and
//!   Close tab;
//! - "Go to": every open tab ("site · kind"), the settings tab included (with "Close
//!   settings", first while it is shown);
//! - the global actions (New site, Open workspace…, the queue and log toggles, Import from
//!   FileZilla, Settings);
//! - every saved site (connect, or focus its tab) and workspace (open).
//!
//! Each row runs the message the action's own button or key sends: the palette never
//! repeats an action's logic. A row with a key shows it.
//!
//! **Matching.** [`fuzzy_score`]: the query's letters in order, case-insensitive, spaces
//! ignored; word starts and contiguous runs score more. Results go best first, equal scores
//! alphabetically. An empty query lists a few of each kind so "go to" needs no typing:
//! up to 5 of the tab's own actions, up to 3 other tabs, "Open workspace…", Settings, up to
//! 4 sites, then the other global actions ([`empty_query_order`]). Rows are ranked only
//! when something needs them (the view, ↑/↓, a run), not on every message.
//!
//! **Input.** While it is open, the palette has the keyboard: ↑/↓ move, Enter runs, Esc
//! closes, every other key is dropped, and what the window behind could still send (a
//! focused field, a pane, the terminal) is dropped as under a dialog
//! ([`App::blocked_by_overlay`](super::App)). A security dialog always wins: the palette
//! does not open under one, and closes when one arrives.

use hoprs_core::{SiteId, TabKind, WorkspaceId};
use hoprs_vault::ExportScope;

use super::{
    App, CompareMsg, Effect, ExportMsg, FailureAction, FileKey, LogMsg, Message, Tab, TabId,
    TabState,
};
use crate::app::terminal::TerminalMsg;
use crate::backend::Side;
use crate::ui::strings;

/// The most results the palette lists.
pub const MAX_RESULTS: usize = 12;

#[cfg(test)]
thread_local! {
    /// How many times this thread ranked the palette's rows ([`App::palette_results`]).
    static RESULTS_COMPUTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many times this thread ranked the palette's rows.
#[cfg(test)]
pub fn results_computed() -> usize {
    RESULTS_COMPUTED.with(std::cell::Cell::get)
}

/// Every matched letter.
const BASE: u32 = 1;
/// A letter that starts a word (the first, one after a separator, an inner capital).
const WORD_START: u32 = 8;
/// A letter right after the previous matched one.
const CONTIGUOUS: u32 = 6;

/// How well `query` matches `candidate`, or `None` if it does not: every letter of the
/// query (spaces ignored) must appear in order, case-insensitive. The best alignment
/// counts: each letter scores [`BASE`], plus [`WORD_START`] at the start of a word, plus
/// [`CONTIGUOUS`] right after the previous one. An empty query matches with 0.
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<u32> {
    let query: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(fold)
        .collect();
    if query.is_empty() {
        return Some(0);
    }
    let chars: Vec<char> = candidate.chars().collect();
    let folded: Vec<char> = chars.iter().copied().map(fold).collect();
    if query.len() > folded.len() {
        return None;
    }
    // best[j]: the best score of the query so far with its last letter at `j`.
    let mut best: Vec<Option<u32>> = vec![None; folded.len()];
    for (i, &letter) in query.iter().enumerate() {
        let mut next = vec![None; folded.len()];
        // The best of `best[..j - 1]`: the previous letter somewhere before, not adjacent.
        let mut before: Option<u32> = None;
        for j in 0..folded.len() {
            if j >= 2 {
                before = before.max(best[j - 2]);
            }
            if folded[j] != letter {
                continue;
            }
            let own = BASE + if word_start(&chars, j) { WORD_START } else { 0 };
            if i == 0 {
                next[j] = Some(own);
                continue;
            }
            let adjacent = j
                .checked_sub(1)
                .and_then(|k| best[k])
                .map(|s| s + CONTIGUOUS);
            next[j] = adjacent.max(before).map(|s| s + own);
        }
        best = next;
    }
    best.into_iter().flatten().max()
}

/// One letter, case-folded (its first lowercase form).
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Whether `chars[j]` starts a word: the first letter, one after a non-alphanumeric
/// character (space, `-`, `_`, `.`, `/`, `@`, `·`), or a capital after a lowercase letter.
fn word_start(chars: &[char], j: usize) -> bool {
    let Some(&previous) = j.checked_sub(1).and_then(|k| chars.get(k)) else {
        return true;
    };
    !previous.is_alphanumeric() || (previous.is_lowercase() && chars[j].is_uppercase())
}

/// The `items` that `score` matches, best first; equal scores alphabetically by `label`
/// (case-insensitive), then in their order.
fn ranked<T>(
    items: Vec<T>,
    score: impl Fn(&T) -> Option<u32>,
    label: impl Fn(&T) -> String,
) -> Vec<T> {
    let mut scored: Vec<(u32, String, T)> = items
        .into_iter()
        .filter_map(|item| score(&item).map(|s| (s, label(&item), item)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, item)| item).collect()
}

/// The most of the active tab's own actions an empty query lists.
const EMPTY_CONTEXT: usize = 5;
/// The most other tabs an empty query lists.
const EMPTY_TABS: usize = 3;
/// The most sites an empty query lists before the other global actions.
const EMPTY_SITES: usize = 4;

/// The rows without a query, so that every kind is reachable without typing: up to
/// [`EMPTY_CONTEXT`] of the active tab's own actions, up to [`EMPTY_TABS`] other tabs
/// (not `active`, nor the settings tab while it is shown), "Open workspace…" and Settings, up to [`EMPTY_SITES`] sites, then the
/// other global actions and everything left, in [`App::palette_entries`]' order.
fn empty_query_order(
    entries: Vec<Entry>,
    active: Option<TabId>,
    settings_shown: bool,
) -> Vec<Entry> {
    let mut context = Vec::new();
    let mut tabs = Vec::new();
    let mut go_to = Vec::new();
    let mut sites = Vec::new();
    let mut rest = Vec::new();
    for entry in entries {
        match &entry.item {
            PaletteItem::Tab(id) if Some(*id) == active => {}
            PaletteItem::SettingsTab if settings_shown => {}
            // Not the shown tab's action: listed, not offered first.
            PaletteItem::CloseSettings if !settings_shown => rest.push(entry),
            PaletteItem::Tab(_) | PaletteItem::SettingsTab if tabs.len() < EMPTY_TABS => {
                tabs.push(entry);
            }
            PaletteItem::OpenWorkspace | PaletteItem::Settings | PaletteItem::Update => {
                go_to.push(entry);
            }
            PaletteItem::Site(_) if sites.len() < EMPTY_SITES => sites.push(entry),
            PaletteItem::FileKey(..)
            | PaletteItem::Copy(_)
            | PaletteItem::Paste(_)
            | PaletteItem::Find(_)
            | PaletteItem::Compare(_)
            | PaletteItem::Reconnect(_)
            | PaletteItem::NewTerminalTab(_)
            | PaletteItem::NewFilesTab(_)
            | PaletteItem::CloseTab(_)
            | PaletteItem::CloseSettings
                if context.len() < EMPTY_CONTEXT =>
            {
                context.push(entry);
            }
            _ => rest.push(entry),
        }
    }
    // "Open workspace…" before Settings.
    go_to.sort_by_key(|e| e.item != PaletteItem::OpenWorkspace);
    let mut out = context;
    out.extend(tabs);
    out.extend(go_to);
    out.extend(sites);
    out.extend(rest);
    out
}

/// The candidates matching `query` in the palette's order, as indices.
#[cfg(test)]
pub fn rank(query: &str, candidates: &[&str]) -> Vec<usize> {
    ranked(
        (0..candidates.len()).collect(),
        |&i| fuzzy_score(query, candidates[i]),
        |&i| candidates[i].to_lowercase(),
    )
}

/// What a palette row does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteItem {
    /// Go to an open tab.
    Tab(TabId),
    /// Connect a saved site, or focus its tab.
    Site(SiteId),
    /// Open a workspace.
    Workspace(WorkspaceId),
    NewSite,
    ImportFilezilla,
    /// "Export everything…": the export screen for the whole vault.
    ExportAll,
    Settings,
    ToggleQueue,
    ToggleLog,
    NewTerminalTab(TabId),
    NewFilesTab(TabId),
    /// The tab's lost (or failed) connection.
    Reconnect(TabId),
    CloseTab(TabId),
    /// Go to the settings tab (listed while it is open).
    SettingsTab,
    /// Close the settings tab.
    CloseSettings,
    /// "Open workspace…": narrows the palette to the workspaces.
    OpenWorkspace,
    /// "Close all in <workspace>": the open workspace's tabs are saved, then closed.
    CloseWorkspace(WorkspaceId),
    /// A key of the focused pane (`Side`) of the active files tab.
    FileKey(FileKey, Side),
    Copy(TabId),
    Paste(TabId),
    Find(TabId),
    /// Compares the files tab's two folders.
    Compare(TabId),
    /// "Update hoprs to X.Y.Z": the card's Update (listed while a newer version is known).
    Update,
}

/// The group of a row, shown as a muted tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Action,
    Tab,
    Site,
    Workspace,
}

/// One row: what it does and how it reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub item: PaletteItem,
    pub label: String,
    /// Shown after the label in mono: a site's `user@host`, the tab Close tab closes.
    pub detail: Option<String>,
    pub group: Group,
    /// The key that does the same outside the palette.
    pub shortcut: Option<&'static str>,
    /// The label is data (a site, tab or workspace name): JetBrains Mono.
    pub mono: bool,
}

impl Entry {
    fn action(item: PaletteItem, label: &str, shortcut: Option<&'static str>) -> Self {
        Self {
            item,
            label: label.to_owned(),
            detail: None,
            group: Group::Action,
            shortcut,
            mono: false,
        }
    }

    /// How well `query` matches: the label, or for a site its `user@host` too.
    fn score(&self, query: &str) -> Option<u32> {
        let label = fuzzy_score(query, &self.label);
        let detail = match (self.group, &self.detail) {
            (Group::Site, Some(detail)) => fuzzy_score(query, detail),
            _ => None,
        };
        label.max(detail)
    }
}

/// What the palette lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    #[default]
    All,
    /// After "Open workspace…": the workspaces only.
    Workspaces,
}

/// The open palette.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Palette {
    /// As typed.
    pub query: String,
    /// The highlighted row of the results.
    pub selected: usize,
    pub scope: Scope,
}

#[derive(Debug, Clone)]
pub enum PaletteMsg {
    /// Ctrl+K (Ctrl+Shift+K in a focused terminal) or the top bar's button: opens it, or
    /// closes it when it is open.
    Open,
    /// Esc, or a click outside it.
    Close,
    Query(String),
    /// Enter in the field: runs the highlighted row.
    Run,
    /// A click on row `n`: runs it.
    Pick(usize),
    /// Narrows the palette to `Scope` ("Open workspace…").
    Scope(Scope),
}

/// The tab as the palette names it: "site · kind".
pub fn tab_label(app: &App, tab: &Tab) -> String {
    let site = app
        .site(&tab.site)
        .map_or(strings::site_missing(), |s| s.name.as_str());
    let kind = match tab.kind() {
        TabKind::Terminal => strings::palette_kind_terminal(),
        TabKind::Files => strings::palette_kind_files(),
    };
    format!("{site} · {kind}")
}

impl App {
    /// A dialog or the palette covers the window: what is behind takes no input.
    pub fn overlay_open(&self) -> bool {
        self.dialogs.is_open() || self.palette.is_some()
    }

    /// Whether the terminal of `tab` takes the keys: it has the focus, its shell is open
    /// and nothing covers it.
    pub fn terminal_has_keyboard(&self, tab: &Tab) -> bool {
        tab.terminal()
            .is_some_and(|t| t.focused && t.open_shell().is_some())
            && !self.overlay_open()
    }

    /// Every row the palette may list now, in its order without a query.
    pub fn palette_entries(&self) -> Vec<Entry> {
        let scope = self.palette.as_ref().map(|p| p.scope).unwrap_or_default();
        let mut out = Vec::new();
        if scope == Scope::All {
            if let Some(index) = self.active_tab.filter(|&i| i < self.tabs.len()) {
                self.tab_actions(index, &mut out);
            }
            let close_settings = Entry::action(
                PaletteItem::CloseSettings,
                strings::palette_close_settings(),
                None,
            );
            // The shown settings tab's own action, as Close tab is a tab's.
            if self.settings_shown() {
                out.push(close_settings.clone());
            }
            out.extend(self.tabs.iter().map(|tab| Entry {
                item: PaletteItem::Tab(tab.id),
                label: tab_label(self, tab),
                detail: None,
                group: Group::Tab,
                shortcut: None,
                mono: true,
            }));
            if self.settings_tab.is_some() {
                out.push(Entry {
                    group: Group::Tab,
                    ..Entry::action(PaletteItem::SettingsTab, strings::settings(), None)
                });
                if !self.settings_shown() {
                    out.push(close_settings);
                }
            }
            out.push(Entry::action(
                PaletteItem::NewSite,
                strings::new_site(),
                None,
            ));
            if !self.workspaces.is_empty() {
                out.push(Entry::action(
                    PaletteItem::OpenWorkspace,
                    strings::palette_open_workspace(),
                    None,
                ));
            }
            if let Some(ws) = self
                .open_workspace
                .as_ref()
                .and_then(|id| self.workspaces.iter().find(|w| &w.id == id))
            {
                out.push(Entry {
                    label: strings::palette_close_all(&ws.name),
                    ..Entry::action(PaletteItem::CloseWorkspace(ws.id.clone()), "", None)
                });
            }
            for (item, label) in [
                (PaletteItem::ToggleQueue, strings::palette_toggle_queue()),
                (PaletteItem::ToggleLog, strings::palette_toggle_log()),
                (PaletteItem::ImportFilezilla, strings::import_filezilla()),
                (PaletteItem::ExportAll, strings::export_everything()),
            ] {
                out.push(Entry::action(item, label, None));
            }
            out.push(Entry::action(
                PaletteItem::Settings,
                strings::settings(),
                Some(strings::key_settings()),
            ));
            if let Some(version) = self.update_offer() {
                out.push(Entry {
                    label: strings::update_palette(&version.to_string()),
                    ..Entry::action(PaletteItem::Update, "", None)
                });
            }
            out.extend(self.sites.iter().map(|site| Entry {
                item: PaletteItem::Site(site.id.clone()),
                label: site.name.clone(),
                detail: Some(if site.user.is_empty() {
                    site.host.clone()
                } else {
                    format!("{}@{}", site.user, site.host)
                }),
                group: Group::Site,
                shortcut: None,
                mono: true,
            }));
        }
        out.extend(self.workspaces.iter().map(|workspace| Entry {
            item: PaletteItem::Workspace(workspace.id.clone()),
            label: workspace.name.clone(),
            detail: None,
            group: Group::Workspace,
            shortcut: None,
            mono: true,
        }));
        out
    }

    /// The rows shown: those matching the query, best first, at most [`MAX_RESULTS`].
    /// Without a query, a few of each kind ([`empty_query_order`]).
    pub fn palette_results(&self) -> Vec<Entry> {
        #[cfg(test)]
        RESULTS_COMPUTED.with(|n| n.set(n.get() + 1));
        let entries = self.palette_entries();
        let query = self.palette.as_ref().map_or("", |p| p.query.as_str());
        let mut results = if query.trim().is_empty() {
            let active = self
                .active_tab
                .and_then(|i| self.tabs.get(i))
                .map(|tab| tab.id);
            empty_query_order(entries, active, self.settings_shown())
        } else {
            ranked(entries, |e| e.score(query), |e| e.label.to_lowercase())
        };
        results.truncate(MAX_RESULTS);
        results
    }

    /// The active tab's own rows: its pane's or terminal's actions (not while a form
    /// covers it), Reconnect, the new tabs on its session, Close tab.
    fn tab_actions(&self, index: usize, out: &mut Vec<Entry>) {
        let tab = &self.tabs[index];
        if self.form.is_none() {
            if tab.is_files() {
                self.pane_actions(index, out);
                if self.tab_session(index).is_some() {
                    out.push(Entry::action(
                        PaletteItem::Compare(tab.id),
                        strings::palette_compare_folders(),
                        None,
                    ));
                }
            } else {
                self.terminal_actions(index, out);
            }
        }
        if self.reconnect_message(tab).is_some() {
            out.push(Entry::action(
                PaletteItem::Reconnect(tab.id),
                strings::reconnect(),
                None,
            ));
        }
        if self.can_open_session_tab(index) {
            out.push(Entry::action(
                PaletteItem::NewTerminalTab(tab.id),
                strings::new_terminal_tab(),
                None,
            ));
            out.push(Entry::action(
                PaletteItem::NewFilesTab(tab.id),
                strings::new_files_tab(),
                None,
            ));
        }
        out.push(Entry {
            detail: Some(tab_label(self, tab)),
            ..Entry::action(
                PaletteItem::CloseTab(tab.id),
                strings::palette_close_tab(),
                None,
            )
        });
    }

    /// The focused pane's keys, when the pane would act on them now.
    fn pane_actions(&self, index: usize, out: &mut Vec<Entry>) {
        let tab = &self.tabs[index];
        let Some(side) = tab.focus() else {
            return;
        };
        let Some(pane) = tab.pane(side) else {
            return;
        };
        if pane.action.is_some() || !self.can_act(index, side) {
            return;
        }
        let key = |key, label, shortcut| {
            Entry::action(PaletteItem::FileKey(key, side), label, Some(shortcut))
        };
        out.push(key(
            FileKey::NewFolder,
            strings::new_folder(),
            strings::key_new_folder(),
        ));
        if !pane.selected_entries().is_empty() {
            out.push(key(
                FileKey::Rename,
                strings::rename(),
                strings::key_rename(),
            ));
            out.push(key(
                FileKey::Delete,
                strings::delete(),
                strings::key_delete(),
            ));
        }
        if matches!(self.edit_target(index, side), Some(Ok(_))) {
            out.push(key(
                FileKey::Edit,
                strings::palette_edit(),
                strings::key_edit(),
            ));
        }
        if !self.transfer_specs(index, side).is_empty() {
            let label = match side {
                Side::Local => strings::palette_upload(),
                Side::Remote => strings::palette_download(),
            };
            out.push(key(FileKey::Transfer, label, strings::key_transfer()));
        }
        out.push(key(
            FileKey::Refresh,
            strings::menu_refresh(),
            strings::key_refresh(),
        ));
    }

    /// The terminal's copy (with a selection), paste (with a shell) and search.
    fn terminal_actions(&self, index: usize, out: &mut Vec<Entry>) {
        let tab = &self.tabs[index];
        let Some(terminal) = tab.terminal() else {
            return;
        };
        if !matches!(self.tab_conn(index).state, TabState::Connected(_)) {
            return;
        }
        if !terminal.model.selected_text().is_empty() {
            out.push(Entry::action(
                PaletteItem::Copy(tab.id),
                strings::palette_copy(),
                Some(strings::palette_key_copy()),
            ));
        }
        if terminal.open_shell().is_some() {
            out.push(Entry::action(
                PaletteItem::Paste(tab.id),
                strings::palette_paste(),
                Some(strings::palette_key_paste()),
            ));
        }
        out.push(Entry::action(
            PaletteItem::Find(tab.id),
            strings::palette_find(),
            Some(strings::palette_key_find()),
        ));
    }

    /// What "Reconnect" sends for `tab`, if it can reconnect: the lost banner's
    /// Reconnect, else the failed tab's.
    fn reconnect_message(&self, tab: &Tab) -> Option<Message> {
        let conn = self.conn(&tab.site);
        if conn.session().is_some()
            && !conn.reconnecting
            && conn.lost_connection().is_some()
            && conn.host_key_change().is_none()
        {
            return Some(Message::ReconnectSession(tab.id));
        }
        (matches!(conn.state, TabState::Failed(_))
            && self
                .failure_actions(tab)
                .contains(&FailureAction::Reconnect))
        .then(|| Message::ConnectSite(tab.site.clone()))
    }

    /// The message a row sends: the same as the action's own button or key.
    pub(super) fn item_message(&self, item: &PaletteItem) -> Option<Message> {
        Some(match item {
            PaletteItem::Tab(id) => Message::SelectTab(self.tab_index(*id)?),
            PaletteItem::Site(id) => Message::ConnectSite(id.clone()),
            PaletteItem::Workspace(id) => Message::OpenWorkspace(id.clone()),
            PaletteItem::NewSite => Message::NewSite,
            PaletteItem::ImportFilezilla => Message::OpenImport,
            PaletteItem::ExportAll => Message::Export(ExportMsg::Open(ExportScope::All)),
            PaletteItem::Settings => Message::OpenSettings,
            PaletteItem::SettingsTab => Message::SelectSettings,
            PaletteItem::CloseSettings => Message::CloseSettings,
            PaletteItem::ToggleQueue => Message::ToggleQueue,
            PaletteItem::ToggleLog => Message::Log(LogMsg::Toggle),
            PaletteItem::NewTerminalTab(tab) => Message::NewTab {
                tab: *tab,
                kind: TabKind::Terminal,
            },
            PaletteItem::NewFilesTab(tab) => Message::NewTab {
                tab: *tab,
                kind: TabKind::Files,
            },
            PaletteItem::Reconnect(id) => {
                self.reconnect_message(&self.tabs[self.tab_index(*id)?])?
            }
            PaletteItem::CloseTab(id) => Message::CloseTab(*id),
            PaletteItem::CloseWorkspace(id) => Message::CloseWorkspace(id.clone()),
            PaletteItem::OpenWorkspace => Message::Palette(PaletteMsg::Scope(Scope::Workspaces)),
            PaletteItem::FileKey(key, _) => Message::Key {
                key: *key,
                shift: false,
            },
            PaletteItem::Copy(tab) => Message::Terminal {
                tab: *tab,
                msg: TerminalMsg::Copy,
            },
            PaletteItem::Paste(tab) => Message::Terminal {
                tab: *tab,
                msg: TerminalMsg::Paste,
            },
            PaletteItem::Compare(tab) => Message::Compare(CompareMsg::Start { tab: *tab }),
            PaletteItem::Find(tab) => Message::Terminal {
                tab: *tab,
                msg: TerminalMsg::OpenSearch,
            },
            PaletteItem::Update => Message::Update(super::UpdateMsg::Start),
        })
    }

    /// Run requests (Enter, the field's submit, a click on a row) become the row's own
    /// message, which `update` then handles as if its button or key had sent it (the
    /// palette closes first, so nothing blocks it). Anything else passes unchanged.
    pub(super) fn palette_command(&mut self, msg: Message) -> Message {
        let Some(palette) = &self.palette else {
            return msg;
        };
        // Only a run request ranks the rows (not every message while it is open).
        let row = match &msg {
            Message::Palette(PaletteMsg::Run)
            | Message::KeyPressed {
                key: FileKey::Enter,
                ..
            } => None,
            Message::Palette(PaletteMsg::Pick(row)) => Some(*row),
            _ => return msg,
        };
        let selected = palette.selected;
        let results = self.palette_results();
        // The highlighted row (the list may have shrunk under it).
        let row = row.unwrap_or_else(|| selected.min(results.len().saturating_sub(1)));
        let Some(command) = results
            .get(row)
            .and_then(|entry| self.item_message(&entry.item))
        else {
            return msg;
        };
        // "Open workspace…" keeps it open, narrowed.
        if !matches!(command, Message::Palette(_)) {
            self.palette = None;
        }
        command
    }

    pub(super) fn palette_msg(&mut self, msg: PaletteMsg) -> Vec<Effect> {
        match msg {
            PaletteMsg::Open => {
                if self.palette.take().is_none() && !self.dialogs.is_open() && !self.closing {
                    self.palette = Some(Palette::default());
                    return vec![Effect::FocusPalette];
                }
            }
            PaletteMsg::Close => self.palette = None,
            PaletteMsg::Query(query) => {
                if let Some(palette) = &mut self.palette {
                    palette.query = query;
                    palette.selected = 0;
                }
            }
            PaletteMsg::Scope(scope) => {
                if let Some(palette) = &mut self.palette {
                    *palette = Palette {
                        scope,
                        ..Palette::default()
                    };
                }
            }
            // Nothing (left) to run: `palette_command` took every row that runs.
            PaletteMsg::Run | PaletteMsg::Pick(_) => {}
        }
        Vec::new()
    }

    /// A key while the palette is open: ↑/↓ move through the results (wrapping), Esc
    /// closes; every other key is dropped (Enter with a row to run never gets here).
    pub(super) fn palette_key(&mut self, key: FileKey) -> Vec<Effect> {
        if self.palette.is_none() {
            return Vec::new();
        }
        match key {
            FileKey::Escape => self.palette = None,
            FileKey::Down | FileKey::Up => {
                let count = self.palette_results().len();
                if let Some(palette) = self.palette.as_mut().filter(|_| count > 0) {
                    palette.selected = if key == FileKey::Down {
                        (palette.selected + 1) % count
                    } else {
                        (palette.selected + count - 1) % count
                    };
                }
            }
            _ => {}
        }
        Vec::new()
    }
}
