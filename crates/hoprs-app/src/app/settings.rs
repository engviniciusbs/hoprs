//! The settings page, shown as a tab in the strip (no modal): workers per site, the
//! default conflict policy, the keepalive, the terminal, and the encrypted backup (export
//! and restore).
//!
//! The settings tab is a slot of its own ([`App::settings_tab`]), not a session [`Tab`](super::Tab)
//! (it has no site): at most one, never saved with a workspace. While it is selected no
//! session tab is (`active_tab` is `None`), so every "active tab" path shows no
//! connection; closing it returns to the tab it came from.
//!
//! A change applies at once (the policy to the engine, the workers and the keepalive to
//! the next connections) and is saved to `settings.toml`, unless that file was unreadable at start:
//! then nothing is saved over it.
//!
//! Backup passwords live in the page only while typed: they leave it inside the effect
//! (a [`SecretString`]) and the fields are emptied.

use std::path::PathBuf;

use hoprs_core::Error;
use hoprs_transfer::ConflictPolicy;
use hoprs_vault::{BackupImportReport, ExportReport, ExportScope, ImportMode};
use secrecy::SecretString;

use super::{App, DividerMsg, Effect, Panel, PickTarget, Resize, TabId};
use crate::settings::{
    LanguageChoice, SettingsLoad, TerminalTheme, clamp_font_size, clamp_keepalive,
    clamp_scrollback, clamp_workers, round_panel_height, valid_editor,
};
use crate::ui::site_form::Password;
use crate::ui::strings;

#[derive(Debug, Clone)]
pub enum SettingsMsg {
    Workers(usize),
    Policy(ConflictPolicy),
    /// The terminal's font size (clamped to `10..=24`); applies at once.
    TerminalFontSize(u16),
    /// Scrollback lines (clamped to `1 000..=100 000`), for the terminals opened next.
    TerminalScrollback(usize),
    TerminalTheme(TerminalTheme),
    /// The interface language; applies at once.
    Language(LanguageChoice),
    /// Seconds between keepalives (clamped to `10..=300`), for the next connections.
    Keepalive(u64),
    /// The editor command as typed: applied (and saved) when not empty.
    EditorCommand(String),
    /// Folder compare re-runs by itself; saved at once.
    CompareAuto(bool),
    /// The compare's ignore list as typed (comma-separated): saved as parsed.
    CompareIgnore(String),
    /// "Check for updates automatically"; saved at once.
    UpdateCheck(bool),
    /// "Check now": asks for the latest release, the result shown inline.
    CheckNow,
    Backup(BackupMsg),
    /// A section of the left navigation.
    Section(SettingsSection),
}

/// A section of the settings tab (its left navigation).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum SettingsSection {
    #[default]
    General,
    Transfers,
    Compare,
    Editing,
    Terminal,
    Backup,
}

impl SettingsSection {
    /// The navigation's order.
    pub const ALL: [SettingsSection; 6] = [
        SettingsSection::General,
        SettingsSection::Transfers,
        SettingsSection::Compare,
        SettingsSection::Editing,
        SettingsSection::Terminal,
        SettingsSection::Backup,
    ];
}

/// The settings tab in the strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsTab {
    pub section: SettingsSection,
    /// Selected: the center shows it, and no session tab is active.
    pub active: bool,
    /// The session tab that was selected when it was opened (or last selected), where
    /// closing it returns.
    pub return_to: Option<TabId>,
}

#[derive(Debug, Clone)]
pub enum BackupMsg {
    OpenExport,
    OpenRestore,
    /// Closes the export or restore panel.
    Cancel,
    /// "Choose…" for the export or the restore file.
    Choose(PickTarget),
    ExportPath(String),
    /// A typed password: its `Debug` never shows it.
    ExportPassword(Password),
    ExportConfirm(Password),
    Export,
    /// "Overwrite" on the inline question about an existing export file.
    ConfirmOverwrite,
    CancelOverwrite,
    RestorePath(String),
    RestorePassword(Password),
    RestoreMode(ImportMode),
    Restore,
    ConfirmReplace,
    CancelReplace,
}

/// Which backup panel is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupPanel {
    Closed,
    Export,
    Restore,
}

/// An export or a restore: idle, running (indeterminate progress), or its outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum BackupState<R> {
    Idle,
    Running,
    Done(R),
    Failed(Error),
}

impl<R> BackupState<R> {
    pub fn running(&self) -> bool {
        matches!(self, Self::Running)
    }
}

#[derive(Debug)]
pub struct Backup {
    pub panel: BackupPanel,
    pub export_path: String,
    pub export_password: Password,
    pub export_confirm: Password,
    pub restore_path: String,
    pub restore_password: Password,
    pub restore_mode: ImportMode,
    /// "Replace" asked once and waits for its confirmation.
    pub confirm_replace: bool,
    /// The export path came from the save dialog (which already asked about overwriting).
    pub export_path_picked: bool,
    /// Whether the typed export file exists is being checked.
    pub export_checking: bool,
    /// The typed export file exists: "Overwrite?" waits for its answer.
    pub confirm_overwrite: bool,
    /// What is wrong with the fields of the open panel.
    pub field_error: Option<&'static str>,
    pub export: BackupState<ExportReport>,
    pub restore: BackupState<BackupImportReport>,
}

impl Default for Backup {
    fn default() -> Self {
        Self {
            panel: BackupPanel::Closed,
            export_path: String::new(),
            export_password: Password::default(),
            export_confirm: Password::default(),
            restore_path: String::new(),
            restore_password: Password::default(),
            restore_mode: ImportMode::Merge,
            confirm_replace: false,
            export_path_picked: false,
            export_checking: false,
            confirm_overwrite: false,
            field_error: None,
            export: BackupState::Idle,
            restore: BackupState::Idle,
        }
    }
}

impl Backup {
    /// An export or a restore runs (both lock the vault; one at a time).
    pub fn running(&self) -> bool {
        self.export.running() || self.restore.running()
    }

    /// Empties every password field, and drops the questions that would send one.
    pub(super) fn forget_passwords(&mut self) {
        self.export_password = Password::default();
        self.export_confirm = Password::default();
        self.restore_password = Password::default();
        self.confirm_replace = false;
        self.confirm_overwrite = false;
        self.export_checking = false;
    }
}

/// The path typed in a field, or the reason it can't be used.
fn path_of(field: &str) -> Result<PathBuf, &'static str> {
    let trimmed = field.trim();
    if trimmed.is_empty() {
        Err(strings::backup_path_required())
    } else {
        Ok(PathBuf::from(trimmed))
    }
}

/// A sidebar section.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SidebarSection {
    Workspaces,
    Sites,
}

/// Something in the sidebar's sections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarMsg {
    /// Folds or unfolds a section to its heading.
    ToggleCollapsed(SidebarSection),
    /// Hides a section (⋯ menu) or shows it again.
    SetHidden {
        section: SidebarSection,
        hidden: bool,
    },
}

impl App {
    pub(super) fn sidebar_msg(&mut self, msg: SidebarMsg) -> Vec<Effect> {
        match msg {
            SidebarMsg::ToggleCollapsed(section) => {
                let state = self.section_mut(section);
                state.collapsed = !state.collapsed;
            }
            SidebarMsg::SetHidden { section, hidden } => {
                self.section_mut(section).hidden = hidden;
            }
        }
        self.save_settings_effect().into_iter().collect()
    }

    fn section_mut(&mut self, section: SidebarSection) -> &mut crate::settings::SectionState {
        match section {
            SidebarSection::Workspaces => &mut self.settings.workspaces,
            SidebarSection::Sites => &mut self.settings.sites,
        }
    }

    /// The state of a sidebar section.
    pub fn section(&self, section: SidebarSection) -> crate::settings::SectionState {
        match section {
            SidebarSection::Workspaces => self.settings.workspaces,
            SidebarSection::Sites => self.settings.sites,
        }
    }

    /// Whether the center shows the settings tab (selected, no form over it).
    pub fn settings_shown(&self) -> bool {
        self.form.is_none() && self.settings_tab.as_ref().is_some_and(|tab| tab.active)
    }

    /// Opens the settings tab, or selects it if it is open (there is at most one), and
    /// scrolls the strip to it. Like selecting a tab, it drops the form shown.
    pub(super) fn open_settings(&mut self) -> Vec<Effect> {
        self.form = None;
        let from = self.shown_session_tab();
        let tab = self.settings_tab.get_or_insert(SettingsTab {
            section: SettingsSection::default(),
            active: false,
            return_to: None,
        });
        if from.is_some() {
            tab.return_to = from;
        }
        tab.active = true;
        self.active_tab = None;
        // It is the strip's last tab: bring it into view.
        vec![Effect::ScrollTabs { x: 1.0 }]
    }

    /// The settings tab's ×: it goes; if it was selected, the tab it came from is
    /// selected again if still open, else the last tab.
    pub(super) fn close_settings(&mut self) -> Vec<Effect> {
        let Some(tab) = self.settings_tab.take() else {
            return Vec::new();
        };
        if tab.active {
            let back = tab
                .return_to
                .and_then(|id| self.tab_index(id))
                .or_else(|| self.tabs.len().checked_sub(1));
            match back {
                Some(index) => self.select_session_tab(index),
                None => self.active_tab = None,
            }
        }
        Vec::new()
    }

    /// Shows a session tab. A terminal shown again takes the keyboard (its search bar
    /// keeps it).
    pub(super) fn select_session_tab(&mut self, index: usize) {
        self.active_tab = Some(index);
        if let Some(terminal) = self.tabs[index].terminal_mut()
            && terminal.search.is_none()
        {
            terminal.focused = true;
        }
    }

    /// The selected session tab's id, if any.
    fn shown_session_tab(&self) -> Option<TabId> {
        self.active_tab
            .and_then(|i| self.tabs.get(i))
            .map(|tab| tab.id)
    }

    /// The settings the backend read at start; the first update check is armed.
    pub(super) fn settings_loaded(&mut self, loaded: SettingsLoad) -> Vec<Effect> {
        self.settings = loaded.settings;
        self.settings_load_error = loaded.error;
        self.schedule_update_check(crate::update::START_DELAY)
    }

    pub(super) fn settings_msg(&mut self, msg: SettingsMsg) -> Vec<Effect> {
        match msg {
            SettingsMsg::Workers(workers) => {
                let workers = clamp_workers(workers);
                if workers == self.settings.workers_per_site {
                    return Vec::new();
                }
                self.settings.workers_per_site = workers;
                let mut effects = vec![Effect::SetWorkers(workers)];
                effects.extend(self.save_settings_effect());
                effects
            }
            SettingsMsg::Policy(policy) => {
                if policy == self.settings.default_policy {
                    return Vec::new();
                }
                self.settings.default_policy = policy;
                // The engine runs under this policy from now on, whatever an answer "to
                // all" set before.
                self.queue.set_session_policy(None);
                let mut effects = vec![Effect::SetPolicy(policy)];
                effects.extend(self.save_settings_effect());
                effects
            }
            SettingsMsg::TerminalFontSize(size) => {
                let size = clamp_font_size(size);
                if size == self.settings.terminal_font_size {
                    return Vec::new();
                }
                self.settings.terminal_font_size = size;
                self.save_settings_effect().into_iter().collect()
            }
            SettingsMsg::TerminalScrollback(lines) => {
                let lines = clamp_scrollback(lines);
                if lines == self.settings.terminal_scrollback {
                    return Vec::new();
                }
                self.settings.terminal_scrollback = lines;
                self.save_settings_effect().into_iter().collect()
            }
            SettingsMsg::TerminalTheme(theme) => {
                if theme == self.settings.terminal_theme {
                    return Vec::new();
                }
                self.settings.terminal_theme = theme;
                for tab in &mut self.tabs {
                    if let Some(terminal) = tab.terminal_mut() {
                        terminal.model.set_theme(theme.colors());
                    }
                }
                self.save_settings_effect().into_iter().collect()
            }
            SettingsMsg::Section(section) => {
                if let Some(tab) = &mut self.settings_tab {
                    tab.section = section;
                }
                Vec::new()
            }
            SettingsMsg::Language(choice) => {
                if choice == self.settings.language {
                    return Vec::new();
                }
                self.settings.language = choice;
                crate::i18n::set_language(crate::i18n::resolve(choice));
                self.save_settings_effect().into_iter().collect()
            }
            SettingsMsg::Keepalive(secs) => {
                let secs = clamp_keepalive(secs);
                if secs == self.settings.keepalive_secs {
                    return Vec::new();
                }
                self.settings.keepalive_secs = secs;
                let mut effects = vec![Effect::SetKeepalive(secs)];
                effects.extend(self.save_settings_effect());
                effects
            }
            SettingsMsg::EditorCommand(command) => {
                // Blank is not a command: it stays as typed (with the hint), and the
                // command in force is the last valid one.
                if !valid_editor(&command) {
                    self.editor_input = Some(command);
                    return Vec::new();
                }
                self.editor_input = None;
                if command == self.settings.editor_command {
                    return Vec::new();
                }
                self.settings.editor_command = command;
                self.save_settings_effect().into_iter().collect()
            }
            SettingsMsg::CompareAuto(on) => {
                if on == self.settings.compare_auto {
                    return Vec::new();
                }
                self.settings.compare_auto = on;
                if !on {
                    // Nothing waits for a debounce any more.
                    for tab in &mut self.tabs {
                        if let Some(state) = tab.compare_mut()
                            && state.phase == super::compare::Phase::Debouncing
                        {
                            state.generation += 1;
                            state.phase = super::compare::Phase::Idle;
                        }
                    }
                }
                self.save_settings_effect().into_iter().collect()
            }
            SettingsMsg::CompareIgnore(typed) => {
                let names = crate::settings::parse_compare_ignore(&typed);
                // The field keeps what was typed ("a, " must not snap back to "a").
                self.compare_ignore_input = Some(typed);
                if names == self.settings.compare_ignore {
                    return Vec::new();
                }
                self.settings.compare_ignore = names;
                self.save_settings_effect().into_iter().collect()
            }
            SettingsMsg::UpdateCheck(on) => {
                if on == self.settings.update_check {
                    return Vec::new();
                }
                self.settings.update_check = on;
                // On: the first check soon; off: the pending timer does nothing.
                let mut effects = self.schedule_update_check(crate::update::START_DELAY);
                effects.extend(self.save_settings_effect());
                effects
            }
            SettingsMsg::CheckNow => self.update_msg(super::UpdateMsg::CheckNow),
            SettingsMsg::Backup(msg) => self.backup_msg(msg),
        }
    }

    /// Saves the settings, unless the file was unreadable: it is never overwritten. One
    /// save at a time: a change made while one runs is saved when it ends, so the file
    /// always ends up with the latest settings (two writes never race).
    pub(super) fn save_settings_effect(&mut self) -> Option<Effect> {
        if self.settings_load_error.is_some() {
            return None;
        }
        if self.settings_saving {
            self.settings_dirty = true;
            return None;
        }
        self.settings_saving = true;
        Some(Effect::SaveSettings(self.settings.clone()))
    }

    pub(super) fn settings_saved(&mut self, result: Result<(), Error>) -> Vec<Effect> {
        self.settings_saving = false;
        self.settings_save_error = result.err();
        if std::mem::take(&mut self.settings_dirty) {
            return self.save_settings_effect().into_iter().collect();
        }
        Vec::new()
    }

    fn backup_msg(&mut self, msg: BackupMsg) -> Vec<Effect> {
        let backup = &mut self.backup;
        match msg {
            BackupMsg::OpenExport | BackupMsg::OpenRestore => {
                let panel = if matches!(msg, BackupMsg::OpenExport) {
                    BackupPanel::Export
                } else {
                    BackupPanel::Restore
                };
                if backup.panel != panel {
                    // A password typed for one panel never goes to the other.
                    backup.forget_passwords();
                }
                backup.panel = panel;
                backup.field_error = None;
            }
            BackupMsg::Cancel => {
                backup.panel = BackupPanel::Closed;
                backup.field_error = None;
                backup.forget_passwords();
            }
            BackupMsg::Choose(target) => {
                if matches!(target, PickTarget::ExportBackup | PickTarget::RestoreBackup) {
                    return vec![Effect::PickFile(target)];
                }
            }
            BackupMsg::ExportPath(path) => {
                backup.export_path = path;
                backup.export_path_picked = false;
                backup.export_checking = false;
                backup.confirm_overwrite = false;
            }
            BackupMsg::ExportPassword(password) => backup.export_password = password,
            BackupMsg::ExportConfirm(password) => backup.export_confirm = password,
            BackupMsg::Export => {
                if backup.running() || backup.export_checking || backup.confirm_overwrite {
                    return Vec::new();
                }
                let path = match path_of(&backup.export_path) {
                    Ok(path) => path,
                    Err(why) => {
                        backup.field_error = Some(why);
                        return Vec::new();
                    }
                };
                if backup.export_password.0.is_empty() {
                    backup.field_error = Some(strings::backup_password_required());
                    return Vec::new();
                }
                if backup.export_password != backup.export_confirm {
                    backup.field_error = Some(strings::backup_passwords_differ());
                    return Vec::new();
                }
                backup.field_error = None;
                if backup.export_path_picked {
                    return self.start_export();
                }
                // A typed path: an existing file is overwritten only once confirmed.
                backup.export_checking = true;
                return vec![Effect::CheckExportTarget(path)];
            }
            BackupMsg::ConfirmOverwrite => {
                if backup.confirm_overwrite {
                    backup.confirm_overwrite = false;
                    return self.start_export();
                }
            }
            BackupMsg::CancelOverwrite => backup.confirm_overwrite = false,
            BackupMsg::RestorePath(path) => {
                backup.restore_path = path;
                backup.confirm_replace = false;
            }
            BackupMsg::RestorePassword(password) => backup.restore_password = password,
            BackupMsg::RestoreMode(mode) => {
                backup.restore_mode = mode;
                backup.confirm_replace = false;
            }
            BackupMsg::Restore => {
                if backup.running() {
                    return Vec::new();
                }
                if let Err(why) = path_of(&backup.restore_path) {
                    backup.field_error = Some(why);
                    return Vec::new();
                }
                if backup.restore_password.0.is_empty() {
                    backup.field_error = Some(strings::backup_password_required());
                    return Vec::new();
                }
                backup.field_error = None;
                if backup.restore_mode == ImportMode::Replace {
                    // Replace drops everything not in the backup: asked once more.
                    backup.confirm_replace = true;
                    return Vec::new();
                }
                return self.start_restore();
            }
            BackupMsg::ConfirmReplace => {
                if backup.confirm_replace && backup.restore_mode == ImportMode::Replace {
                    backup.confirm_replace = false;
                    return self.start_restore();
                }
            }
            BackupMsg::CancelReplace => backup.confirm_replace = false,
        }
        Vec::new()
    }

    /// Whether the typed export file exists came back.
    pub(super) fn export_target_checked(&mut self, path: PathBuf, exists: bool) -> Vec<Effect> {
        let backup = &mut self.backup;
        // A path edited meanwhile (or the panel left) drops this answer.
        if !backup.export_checking || path_of(&backup.export_path).ok() != Some(path) {
            return Vec::new();
        }
        backup.export_checking = false;
        if exists {
            backup.confirm_overwrite = true;
            return Vec::new();
        }
        self.start_export()
    }

    /// Sends the export with the typed password, which leaves the page.
    fn start_export(&mut self) -> Vec<Effect> {
        let backup = &mut self.backup;
        let Ok(path) = path_of(&backup.export_path) else {
            return Vec::new();
        };
        if backup.running()
            || backup.export_password.0.is_empty()
            || backup.export_password != backup.export_confirm
        {
            return Vec::new();
        }
        // From here on only the effect holds the password.
        let password = SecretString::from(std::mem::take(&mut backup.export_password.0));
        backup.export_confirm = Password::default();
        backup.export = BackupState::Running;
        vec![Effect::ExportBackup {
            scope: ExportScope::All,
            path,
            password,
        }]
    }

    /// Sends the restore with the typed password, which leaves the page.
    fn start_restore(&mut self) -> Vec<Effect> {
        let backup = &mut self.backup;
        let Ok(path) = path_of(&backup.restore_path) else {
            return Vec::new();
        };
        if backup.running() || backup.restore_password.0.is_empty() {
            return Vec::new();
        }
        let password = SecretString::from(std::mem::take(&mut backup.restore_password.0));
        backup.restore = BackupState::Running;
        vec![Effect::RestoreBackup {
            path,
            password,
            mode: backup.restore_mode,
        }]
    }

    pub(super) fn backup_exported(&mut self, result: Result<ExportReport, Error>) {
        if self.backup.export.running() {
            self.backup.export = match result {
                Ok(report) => BackupState::Done(report),
                Err(error) => BackupState::Failed(error),
            };
        }
    }

    /// A restore finished; what it changed reaches the sidebar.
    pub(super) fn backup_restored(
        &mut self,
        result: Result<BackupImportReport, Error>,
    ) -> Vec<Effect> {
        if !self.backup.restore.running() {
            return Vec::new();
        }
        match result {
            Ok(report) => {
                self.backup.restore = BackupState::Done(report);
                vec![self.load_profiles()]
            }
            Err(error) => {
                self.backup.restore = BackupState::Failed(error);
                Vec::new()
            }
        }
    }

    /// The system file dialog answered (`None`: cancelled).
    pub(super) fn file_picked(&mut self, target: PickTarget, path: Option<PathBuf>) {
        let Some(path) = path else {
            return;
        };
        let path = path.to_string_lossy().into_owned();
        match target {
            PickTarget::Sitemanager => {
                self.import.path = path;
                self.import.path_error = None;
            }
            PickTarget::ExportBackup => {
                self.backup.export_path = path;
                // The save dialog asked about an existing file already.
                self.backup.export_path_picked = true;
                self.backup.export_checking = false;
                self.backup.confirm_overwrite = false;
            }
            PickTarget::RestoreBackup => {
                self.backup.restore_path = path;
                self.backup.confirm_replace = false;
            }
        }
    }

    /// An import or a backup runs: the progress indicator moves.
    pub fn busy(&self) -> bool {
        self.import.running() || self.backup.running()
    }

    /// Whether the pointer's moves are wanted: only while a divider is dragged or a press
    /// on a row may become a drag (the subscription that sends them runs only then).
    pub fn tracks_pointer(&self) -> bool {
        self.resize.is_some() || self.drag.is_some()
    }

    /// A press or double-click on a panel's divider. A press starts a resize at the
    /// press's position ([`App::last_press`], published before the divider's message);
    /// a double-click puts the panel back to its default height and saves it (if that
    /// changed anything).
    pub(super) fn divider_msg(&mut self, msg: DividerMsg) -> Vec<Effect> {
        match msg {
            DividerMsg::Press { panel, height, cap } => {
                self.resize = Some(Resize {
                    panel,
                    start_y: self.last_press.y,
                    start_height: height,
                    drawn: height,
                    cap,
                    saved_at_press: self.panel_setting(panel),
                });
                Vec::new()
            }
            DividerMsg::Reset(panel) => {
                // The double-click's second press started a resize: it ends here.
                self.resize = None;
                let before = self.panel_setting(panel);
                match panel {
                    Panel::Compare => self.settings.compare_height = None,
                    Panel::Queue => {
                        self.settings.queue_height = crate::ui::layout::QUEUE_MAX_HEIGHT;
                    }
                }
                self.save_panel_if_changed(panel, before)
            }
        }
    }

    /// The saved height of `panel` (`None`: the compare panel's default share).
    fn panel_setting(&self, panel: Panel) -> Option<f32> {
        match panel {
            Panel::Compare => self.settings.compare_height,
            Panel::Queue => Some(self.settings.queue_height),
        }
    }

    /// Saves the settings when `panel`'s height is no longer `before` (a plain click on a
    /// divider writes nothing).
    fn save_panel_if_changed(&mut self, panel: Panel, before: Option<f32>) -> Vec<Effect> {
        if self.panel_setting(panel) == before {
            return Vec::new();
        }
        self.save_settings_effect().into_iter().collect()
    }

    /// The pointer moved: while a divider is dragged, its panel follows the pointer, as
    /// much taller as it went up, between the panel's minimum and the window's cap now
    /// (past either, the anchor moves with the pointer so turning back acts at once).
    /// Below the height drawn at the press the setting is that height; above it the
    /// setting only grows: the queue's setting is its maximum, so a queue drawn shorter
    /// than its maximum keeps that maximum until the drag passes it. Saved when the button
    /// is up.
    pub(super) fn pointer_moved(&mut self, at: iced::Point) {
        self.drag_moved(at);
        let Some(resize) = self.resize.as_mut() else {
            return;
        };
        let min = match resize.panel {
            Panel::Compare => crate::ui::compare::MIN_HEIGHT,
            Panel::Queue => crate::ui::queue::MIN_HEIGHT,
        };
        let wanted = resize.start_height + (resize.start_y - at.y);
        let height = wanted.min(resize.cap).max(min);
        if height != wanted {
            resize.start_y = at.y;
            resize.start_height = height;
        }
        let saved = resize.saved_at_press.unwrap_or(resize.drawn);
        let value = if height < resize.drawn {
            Some(round_panel_height(height))
        } else if height <= saved {
            resize.saved_at_press
        } else {
            Some(round_panel_height(height))
        };
        match resize.panel {
            Panel::Compare => self.settings.compare_height = value,
            Panel::Queue => {
                self.settings.queue_height = value.unwrap_or(crate::ui::layout::QUEUE_MAX_HEIGHT);
            }
        }
    }

    /// The button went up or the pointer left the window: a resize ends with the height
    /// it reached, saved if it changed.
    pub(super) fn end_resize(&mut self) -> Vec<Effect> {
        let Some(resize) = self.resize.take() else {
            return Vec::new();
        };
        self.save_panel_if_changed(resize.panel, resize.saved_at_press)
    }
}
