//! The settings tab's page in the center: transfers (workers per site, conflict policy,
//! keepalive), the terminal (font size, scrollback, theme) and the encrypted backup.

use std::fmt;
use std::path::Path;

use hoprs_core::{AgentAccess, Site};

use hoprs_transfer::ConflictPolicy;
use hoprs_vault::{BackupImportReport, ExportScope, ImportMode};
use iced::widget::{
    button, checkbox, column, container, pick_list, row, rule, scrollable, space, text, text_input,
};
use iced::{Alignment, Element, Length, Padding};

use crate::app::settings::{
    Backup, BackupMsg, BackupPanel, BackupState, SettingsMsg, SettingsSection, SettingsTab,
};
use crate::app::update::ManualCheck;
use crate::app::{App, ExportMsg, Message, PickTarget};
use crate::settings::{
    FONT_SIZE_MAX, FONT_SIZE_MIN, LanguageChoice, TerminalTheme, WORKERS_MAX, WORKERS_MIN,
};
use crate::ui::icons::{self, Icon};
use crate::ui::site_form::Password;
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use crate::ui::widgets::{self, busy, confirm_with, failure, field};

fn msg(m: SettingsMsg) -> Message {
    Message::Settings(m)
}

fn backup_msg(m: BackupMsg) -> Message {
    msg(SettingsMsg::Backup(m))
}

/// `pick_list` needs `Display`: the UI label of a policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PolicyChoice(ConflictPolicy);

impl fmt::Display for PolicyChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(policy_label(self.0))
    }
}

pub fn policy_label(policy: ConflictPolicy) -> &'static str {
    match policy {
        ConflictPolicy::Ask => strings::policy_ask(),
        ConflictPolicy::Overwrite => strings::policy_overwrite(),
        ConflictPolicy::Skip => strings::policy_skip(),
        ConflictPolicy::OverwriteIfNewer => strings::policy_overwrite_if_newer(),
        ConflictPolicy::Resume => strings::policy_resume(),
    }
}

/// Scrollback sizes the page offers (lines).
const SCROLLBACK_CHOICES: [usize; 7] = [1_000, 2_000, 5_000, 10_000, 20_000, 50_000, 100_000];

/// `pick_list` needs `Display`: a scrollback size, in lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Lines(usize);

impl fmt::Display for Lines {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Keepalive intervals the page offers (seconds).
const KEEPALIVE_CHOICES: [u64; 7] = [10, 15, 30, 60, 120, 180, 300];

/// `pick_list` needs `Display`: a keepalive interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Secs(u64);

impl fmt::Display for Secs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&strings::seconds(self.0))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LanguageOption(LanguageChoice);

impl fmt::Display for LanguageOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.label())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ThemeChoice(TerminalTheme);

impl fmt::Display for ThemeChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.label())
    }
}

const POLICIES: [PolicyChoice; 5] = [
    PolicyChoice(ConflictPolicy::Ask),
    PolicyChoice(ConflictPolicy::Overwrite),
    PolicyChoice(ConflictPolicy::OverwriteIfNewer),
    PolicyChoice(ConflictPolicy::Resume),
    PolicyChoice(ConflictPolicy::Skip),
];

fn muted(label: &str) -> Element<'_, Message> {
    text(label)
        .size(theme::TEXT_SM)
        .color(palette().muted)
        .into()
}

fn heading(label: &str) -> Element<'_, Message> {
    text(label)
        .size(theme::TEXT_XS)
        .font(theme::INTER_SEMIBOLD)
        .into()
}

fn hrule<'a>() -> Element<'a, Message> {
    rule::horizontal(theme::HAIRLINE)
        .style(theme::rule_1px)
        .into()
}

fn primary(label: &str, on_press: Option<Message>) -> Element<'_, Message> {
    button(text(label).font(theme::INTER_SEMIBOLD))
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
        .style(theme::button_primary)
        .on_press_maybe(on_press)
        .into()
}

fn outlined(label: &str, on_press: Option<Message>) -> Element<'_, Message> {
    let enabled = on_press.is_some();
    widgets::outlined(
        button(text(label))
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_SM]))
            .on_press_maybe(on_press),
        enabled,
    )
}

fn plain(label: &str, on_press: Option<Message>) -> Element<'_, Message> {
    button(text(label))
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
        .style(theme::button_text)
        .on_press_maybe(on_press)
        .into()
}

fn input<'a>(
    value: &'a str,
    secure: bool,
    on_input: impl Fn(String) -> Message + 'a,
) -> iced::widget::TextInput<'a, Message> {
    let field = text_input("", value)
        .on_input(on_input)
        .secure(secure)
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
        .style(theme::search);
    if secure {
        field.size(theme::TEXT_SM)
    } else {
        // Paths are data: mono.
        field.font(theme::MONO).size(theme::TEXT_MONO)
    }
}

fn nav_row<'a>(section: SettingsSection, selected: bool) -> Element<'a, Message> {
    let label = section_title(section);
    let glyph = icons::icon(section_icon(section), icons::SMALL).color(if selected {
        palette().ink
    } else {
        palette().muted
    });
    let bar = container(space())
        .width(2)
        .height(Length::Fill)
        .style(move |t: &iced::Theme| {
            if selected {
                theme::selection_bar(true)(t)
            } else {
                container::Style::default()
            }
        });
    let line = row![
        glyph,
        text(label).size(theme::TEXT_SM).font(if selected {
            theme::INTER_SEMIBOLD
        } else {
            theme::INTER
        }),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center);
    row![
        bar,
        button(line)
            .width(Length::Fill)
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
            .style(if selected {
                theme::row_selected
            } else {
                theme::row
            })
            .on_press(msg(SettingsMsg::Section(section))),
    ]
    .height(30)
    .into()
}

/// The section's icon in the navigation.
pub fn section_icon(section: SettingsSection) -> Icon {
    match section {
        SettingsSection::General => Icon::SlidersHorizontal,
        SettingsSection::Transfers => Icon::ArrowsDownUp,
        SettingsSection::Compare => Icon::GitDiff,
        SettingsSection::Editing => Icon::PencilSimple,
        SettingsSection::Terminal => Icon::TerminalWindow,
        SettingsSection::Agents => Icon::PlugsConnected,
        SettingsSection::Backup => Icon::ShieldCheck,
    }
}

fn section_title(section: SettingsSection) -> &'static str {
    match section {
        SettingsSection::General => strings::settings_general(),
        SettingsSection::Transfers => strings::settings_transfers(),
        SettingsSection::Compare => strings::settings_compare(),
        SettingsSection::Editing => strings::settings_editing(),
        SettingsSection::Terminal => strings::settings_terminal(),
        SettingsSection::Agents => strings::agents_section(),
        SettingsSection::Backup => strings::backup(),
    }
}

fn section_description(section: SettingsSection) -> &'static str {
    match section {
        SettingsSection::General => strings::settings_general_description(),
        SettingsSection::Transfers => strings::settings_transfers_description(),
        SettingsSection::Compare => strings::settings_compare_description(),
        SettingsSection::Editing => strings::settings_editing_description(),
        SettingsSection::Terminal => strings::settings_terminal_description(),
        SettingsSection::Agents => strings::settings_agents_description(),
        SettingsSection::Backup => strings::settings_backup_description(),
    }
}

/// The version line of General.
fn version_line() -> String {
    format!("hoprs {}", strings::VERSION)
}

fn general(app: &App) -> Element<'_, Message> {
    let settings = &app.settings;
    let languages: Vec<LanguageOption> = LanguageChoice::ALL
        .into_iter()
        .map(LanguageOption)
        .collect();
    let language = pick_list(
        languages,
        Some(LanguageOption(settings.language)),
        |choice| msg(SettingsMsg::Language(choice.0)),
    )
    .text_size(theme::TEXT_SM)
    .width(240)
    .style(theme::pick_list)
    .menu_style(theme::menu);

    let manual = &app.updater.manual;
    let mut check = row![outlined(
        strings::settings_check_now(),
        (*manual != ManualCheck::Checking).then(|| msg(SettingsMsg::CheckNow)),
    )]
    .spacing(theme::SPACE_SM)
    .align_y(Alignment::Center);
    if let Some(status) = update_status(manual) {
        let color = match manual {
            ManualCheck::Failed(_) => palette().error,
            _ => palette().muted,
        };
        check = check.push(text(status).size(theme::TEXT_SM).color(color));
    }

    column![
        field(strings::settings_language(), language.into(), None),
        field(
            strings::settings_update_check(),
            checkbox(settings.update_check)
                .on_toggle(|on| msg(SettingsMsg::UpdateCheck(on)))
                .style(theme::checkbox)
                .into(),
            None
        ),
        muted(strings::settings_update_check_hint()),
        check,
        muted_owned(version_line()),
    ]
    .spacing(theme::SPACE_SM)
    .into()
}

/// Check now's inline result; `None` before it was clicked.
pub fn update_status(manual: &ManualCheck) -> Option<String> {
    match manual {
        ManualCheck::Idle => None,
        ManualCheck::Checking => Some(strings::settings_update_checking().to_owned()),
        ManualCheck::UpToDate => Some(strings::settings_up_to_date().to_owned()),
        ManualCheck::Available(version) => Some(strings::update_available(&version.to_string())),
        ManualCheck::Failed(error) => Some(strings::error(error)),
    }
}

fn muted_owned<'a>(label: String) -> Element<'a, Message> {
    text(label)
        .size(theme::TEXT_SM)
        .color(palette().muted)
        .into()
}

fn transfers(app: &App) -> Element<'_, Message> {
    let settings = &app.settings;
    let workers = settings.workers_per_site;
    let stepper = row![
        outlined(
            strings::decrease(),
            (workers > WORKERS_MIN).then(|| msg(SettingsMsg::Workers(workers - 1)))
        ),
        container(
            text(workers.to_string())
                .font(theme::MONO)
                .size(theme::TEXT_MONO)
        )
        .width(36)
        .align_x(Alignment::Center),
        outlined(
            strings::increase(),
            (workers < WORKERS_MAX).then(|| msg(SettingsMsg::Workers(workers + 1)))
        ),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center);

    let policy = pick_list(
        POLICIES,
        Some(PolicyChoice(settings.default_policy)),
        |choice| msg(SettingsMsg::Policy(choice.0)),
    )
    .text_size(theme::TEXT_SM)
    .width(240)
    .style(theme::pick_list)
    .menu_style(theme::menu);

    // A value from the file that is not on the list is offered too.
    let mut intervals: Vec<Secs> = KEEPALIVE_CHOICES.into_iter().map(Secs).collect();
    if !KEEPALIVE_CHOICES.contains(&settings.keepalive_secs) {
        intervals.push(Secs(settings.keepalive_secs));
        intervals.sort();
    }
    let keepalive = pick_list(intervals, Some(Secs(settings.keepalive_secs)), |secs| {
        msg(SettingsMsg::Keepalive(secs.0))
    })
    .text_size(theme::TEXT_SM)
    .width(120)
    .style(theme::pick_list)
    .menu_style(theme::menu);

    column![
        field(strings::settings_workers(), stepper.into(), None),
        muted(strings::settings_workers_hint()),
        field(strings::settings_policy(), policy.into(), None),
        muted(strings::settings_policy_hint()),
        field(strings::settings_keepalive(), keepalive.into(), None),
        muted(strings::settings_keepalive_hint()),
    ]
    .spacing(theme::SPACE_SM)
    .into()
}

fn terminal(app: &App) -> Element<'_, Message> {
    let settings = &app.settings;
    let font_size = settings.terminal_font_size;
    let font_stepper = row![
        outlined(
            strings::decrease(),
            (font_size > FONT_SIZE_MIN).then(|| msg(SettingsMsg::TerminalFontSize(font_size - 1)))
        ),
        container(
            text(font_size.to_string())
                .font(theme::MONO)
                .size(theme::TEXT_MONO)
        )
        .width(36)
        .align_x(Alignment::Center),
        outlined(
            strings::increase(),
            (font_size < FONT_SIZE_MAX).then(|| msg(SettingsMsg::TerminalFontSize(font_size + 1)))
        ),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center);
    let mut lines: Vec<Lines> = SCROLLBACK_CHOICES.iter().copied().map(Lines).collect();
    if !SCROLLBACK_CHOICES.contains(&settings.terminal_scrollback) {
        // A value typed into the file: shown as it is.
        lines.push(Lines(settings.terminal_scrollback));
        lines.sort();
    }
    let scrollback = pick_list(lines, Some(Lines(settings.terminal_scrollback)), |lines| {
        msg(SettingsMsg::TerminalScrollback(lines.0))
    })
    .text_size(theme::TEXT_SM)
    .width(240)
    .style(theme::pick_list)
    .menu_style(theme::menu);
    let themes: Vec<ThemeChoice> = TerminalTheme::ALL.into_iter().map(ThemeChoice).collect();
    let term_theme = pick_list(
        themes,
        Some(ThemeChoice(settings.terminal_theme)),
        |choice| msg(SettingsMsg::TerminalTheme(choice.0)),
    )
    .text_size(theme::TEXT_SM)
    .width(240)
    .style(theme::pick_list)
    .menu_style(theme::menu);
    let terminal = column![
        field(strings::settings_font_size(), font_stepper.into(), None),
        field(strings::settings_scrollback(), scrollback.into(), None),
        muted(strings::settings_scrollback_hint()),
        field(strings::settings_theme(), term_theme.into(), None),
    ]
    .spacing(theme::SPACE_SM);

    terminal.into()
}

/// The shell command that registers hoprs in Claude Code: the executable's path is quoted
/// for the shell (on Unix `"`, `\`, `$` and `` ` `` get a backslash; on Windows the path
/// goes in as is, no Windows path holds a `"`). Without a path, `hoprs`.
pub fn agent_command(exe: Option<&Path>) -> String {
    let path = exe.map_or_else(|| "hoprs".to_owned(), |p| p.to_string_lossy().into_owned());
    let quoted = if cfg!(windows) {
        path
    } else {
        let mut out = String::with_capacity(path.len());
        for c in path.chars() {
            if matches!(c, '"' | '\\' | '$' | '`') {
                out.push('\\');
            }
            out.push(c);
        }
        out
    };
    format!("claude mcp add hoprs -- \"{quoted}\" mcp")
}

/// The same registration as the JSON other MCP clients take, written by `serde_json` so
/// any path (a Windows one with backslashes, a quote) stays valid.
pub fn agent_json(exe: Option<&Path>) -> String {
    let path = exe.map_or_else(|| "hoprs".to_owned(), |p| p.to_string_lossy().into_owned());
    serde_json::json!({ "command": path, "args": ["mcp"] }).to_string()
}

/// How many sites are at `Read` and at `All`.
pub fn agent_levels(sites: &[Site]) -> (usize, usize) {
    let count = |level| sites.iter().filter(|s| s.agent_access == level).count();
    (count(AgentAccess::Read), count(AgentAccess::All))
}

/// A read-only, selectable line (a text input that ignores edits) with its Copy button.
fn copyable<'a>(value: &str, copy: SettingsMsg) -> Element<'a, Message> {
    row![
        text_input("", value)
            .on_input(|_| msg(SettingsMsg::AgentTextEdited))
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
            .style(theme::search)
            .font(theme::MONO)
            .size(theme::TEXT_MONO)
            .width(Length::Fill),
        outlined(strings::update_copy(), Some(msg(copy))),
    ]
    .spacing(theme::SPACE_SM)
    .align_y(Alignment::Center)
    .into()
}

/// Why the channel for agents could not open, as the line under the checkbox.
fn agents_error(app: &App) -> Option<String> {
    app.agents.error.as_ref().map(strings::error)
}

fn agents(app: &App) -> Element<'_, Message> {
    let (read, all) = agent_levels(&app.sites);
    let command = agent_command(app.agent_exe.as_deref());
    let json = agent_json(app.agent_exe.as_deref());
    let channel_error =
        agents_error(app).map(|error| text(error).size(theme::TEXT_SM).color(palette().error));
    column![
        checkbox(app.settings.agents_enabled)
            .label(strings::agents_allow())
            .on_toggle(|on| msg(SettingsMsg::Agents(on)))
            .style(theme::checkbox),
        channel_error,
        muted(strings::agents_allow_hint()),
        field(
            strings::agents_claude_code(),
            copyable(&command, SettingsMsg::CopyAgentCommand),
            None
        ),
        field(
            strings::agents_other_clients(),
            copyable(&json, SettingsMsg::CopyAgentJson),
            None
        ),
        muted_owned(strings::agents_levels(read, all)),
    ]
    .spacing(theme::SPACE_SM)
    .into()
}

fn editing(app: &App) -> Element<'_, Message> {
    let settings = &app.settings;
    let editor_value = app
        .editor_input
        .as_deref()
        .unwrap_or(&settings.editor_command);
    let mut editor_field = row![
        input(editor_value, false, |command| {
            msg(SettingsMsg::EditorCommand(command))
        })
        .width(320)
    ]
    .spacing(theme::SPACE_SM)
    .align_y(Alignment::Center);
    // The setting stays a string; `system` reads as what it does.
    if crate::settings::is_system_editor(editor_value) {
        editor_field = editor_field.push(muted(strings::editor_system_default()));
    }
    let mut editing = column![
        field(strings::editor_command(), editor_field.into(), None),
        muted(strings::editor_command_hint()),
    ]
    .spacing(theme::SPACE_SM);
    if app.editor_input.is_some() {
        editing = editing.push(
            text(strings::editor_command_empty())
                .size(theme::TEXT_SM)
                .color(palette().error),
        );
    }

    editing.into()
}

fn comparing(app: &App) -> Element<'_, Message> {
    let settings = &app.settings;
    let ignore_value = app
        .compare_ignore_input
        .clone()
        .unwrap_or_else(|| settings.compare_ignore.join(", "));
    let ignore_default = crate::settings::compare_ignore_default().join(", ");
    let comparing = column![
        field(
            strings::settings_compare_auto(),
            checkbox(settings.compare_auto)
                .on_toggle(|on| msg(SettingsMsg::CompareAuto(on)))
                .style(theme::checkbox)
                .into(),
            None
        ),
        field(
            strings::settings_compare_ignore(),
            text_input(&ignore_default, &ignore_value)
                .on_input(|typed| msg(SettingsMsg::CompareIgnore(typed)))
                .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                .style(theme::search)
                .font(theme::MONO)
                .size(theme::TEXT_MONO)
                .width(320)
                .into(),
            None
        ),
        muted(strings::settings_compare_ignore_hint()),
    ]
    .spacing(theme::SPACE_SM);

    comparing.into()
}

pub fn view<'a>(app: &'a App, tab: &SettingsTab) -> Element<'a, Message> {
    let nav = column(
        SettingsSection::ALL
            .into_iter()
            .map(|section| nav_row(section, section == tab.section)),
    )
    .spacing(theme::SPACE_3XS);
    let nav = container(nav)
        .width(200)
        .height(Length::Fill)
        .padding(theme::SPACE_XS);

    let mut content = column![
        text(section_title(tab.section))
            .size(theme::TEXT_MD)
            .font(theme::INTER_SEMIBOLD),
        muted(section_description(tab.section)),
    ]
    .spacing(theme::SPACE_XS);
    if app.settings_load_error.is_some() {
        content = content.push(
            column![
                text(strings::settings_unreadable())
                    .size(theme::TEXT_SM)
                    .color(palette().error),
                failure(app.settings_load_error.as_ref()),
            ]
            .spacing(theme::SPACE_2XS),
        );
    }
    if let Some(error) = &app.settings_save_error {
        content = content.push(
            column![
                text(strings::settings_save_failed())
                    .size(theme::TEXT_SM)
                    .color(palette().error),
                failure(Some(error)),
            ]
            .spacing(theme::SPACE_2XS),
        );
    }
    content = content.push(hrule()).push(match tab.section {
        SettingsSection::General => general(app),
        SettingsSection::Transfers => transfers(app),
        SettingsSection::Compare => comparing(app),
        SettingsSection::Editing => editing(app),
        SettingsSection::Terminal => terminal(app),
        SettingsSection::Agents => agents(app),
        SettingsSection::Backup => backup_view(app),
    });

    row![
        nav,
        rule::vertical(theme::HAIRLINE).style(theme::rule_1px),
        scrollable(
            container(content.spacing(theme::SPACE_MD).max_width(640))
                .width(Length::Fill)
                .padding(theme::SPACE_LG),
        )
        .width(Length::Fill)
        .height(Length::Fill),
    ]
    .into()
}

/// The Backup line for a Google Drive connection: from the marker in `settings.toml`
/// (no keyring read), only in a build that offers Google Drive.
pub fn drive_line(app: &App) -> Option<&'static str> {
    (app.drive_available && app.settings.google_drive).then(strings::drive_settings_line)
}

/// The last Google Drive failure, in a build that offers Google Drive: shown whether or
/// not the marker is still there (a failed Disconnect clears it).
pub fn drive_error(app: &App) -> Option<&hoprs_core::Error> {
    app.drive_available.then_some(app.drive_error.as_ref())?
}

fn backup_view(app: &App) -> Element<'_, Message> {
    let backup = &app.backup;
    // One at a time: an export and a restore both lock the vault.
    let running = backup.restore.running() || app.export.as_ref().is_some_and(|f| f.running());
    let buttons = row![
        outlined(
            strings::export_backup(),
            (!running).then_some(Message::Export(ExportMsg::Open(ExportScope::All)))
        ),
        outlined(
            strings::restore_backup(),
            (!running).then(|| backup_msg(BackupMsg::OpenRestore))
        ),
    ]
    .spacing(theme::SPACE_XS);
    let mut col = column![muted(strings::backup_hint()), buttons].spacing(theme::SPACE_SM);
    if let Some(line) = drive_line(app) {
        col = col.push(
            row![
                text(line).size(theme::TEXT_SM),
                plain(
                    strings::drive_disconnect(),
                    (!running).then_some(Message::Export(ExportMsg::Disconnect)),
                ),
            ]
            .spacing(theme::SPACE_XS)
            .align_y(Alignment::Center),
        );
    }
    if let Some(error) = drive_error(app) {
        col = col.push(failure(Some(error)));
    }

    // The running restore and its outcome, above the panel (in view without scrolling).
    if backup.restore.running() {
        col = col.push(busy(strings::restoring(), app.busy_phase));
    }
    if backup.panel == BackupPanel::Restore {
        match &backup.restore {
            BackupState::Done(report) => col = col.push(restore_report(report)),
            BackupState::Failed(error) => {
                col = col.push(outcome_error(strings::restore_failed(), error));
            }
            BackupState::Idle | BackupState::Running => {}
        }
    }
    match backup.panel {
        BackupPanel::Closed => {}
        BackupPanel::Restore => col = col.push(restore_panel(backup, running)),
    }
    col.into()
}

fn panel<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .width(Length::Fill)
        .padding(theme::SPACE_SM)
        .style(theme::outline_box)
        .into()
}

fn path_row<'a>(
    value: &'a str,
    target: PickTarget,
    on_input: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message> {
    row![
        input(value, false, on_input),
        outlined(
            strings::choose_file(),
            Some(backup_msg(BackupMsg::Choose(target)))
        ),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center)
    .into()
}

fn field_error(backup: &Backup) -> Element<'_, Message> {
    match backup.field_error {
        Some(error) => text(error)
            .size(theme::TEXT_XS)
            .color(palette().error)
            .into(),
        None => space().into(),
    }
}

/// The restore form; Restore is off while `running` (a restore or an export).
fn restore_panel(backup: &Backup, running: bool) -> Element<'_, Message> {
    let mode = |label, mode: ImportMode| {
        button(text(label))
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
            .style(theme::segment(backup.restore_mode == mode))
            .on_press(backup_msg(BackupMsg::RestoreMode(mode)))
    };
    let mode_hint = match backup.restore_mode {
        ImportMode::Merge => strings::restore_merge_hint(),
        ImportMode::Replace => strings::restore_replace_hint(),
    };
    let actions: Element<'_, Message> = if backup.confirm_replace {
        confirm_with(
            strings::restore_replace_confirm(),
            strings::replace(),
            backup_msg(BackupMsg::ConfirmReplace),
            backup_msg(BackupMsg::CancelReplace),
        )
    } else {
        row![
            primary(
                strings::restore(),
                (!running).then(|| backup_msg(BackupMsg::Restore))
            ),
            plain(strings::cancel(), Some(backup_msg(BackupMsg::Cancel))),
        ]
        .spacing(theme::SPACE_XS)
        .align_y(Alignment::Center)
        .into()
    };
    panel(
        column![
            heading(strings::restore_backup()),
            field(
                strings::backup_file(),
                path_row(&backup.restore_path, PickTarget::RestoreBackup, |v| {
                    backup_msg(BackupMsg::RestorePath(v))
                }),
                None,
            ),
            field(
                strings::backup_password(),
                input(&backup.restore_password.0, true, |v| {
                    backup_msg(BackupMsg::RestorePassword(Password(v)))
                })
                .on_submit(backup_msg(BackupMsg::Restore))
                .into(),
                None,
            ),
            field(
                strings::restore_mode(),
                row![
                    mode(strings::restore_merge(), ImportMode::Merge),
                    mode(strings::restore_replace(), ImportMode::Replace),
                ]
                .spacing(theme::SPACE_XS)
                .into(),
                None,
            ),
            muted(mode_hint),
            field_error(backup),
            actions,
        ]
        .spacing(theme::SPACE_SM),
    )
}

pub(super) fn outcome_error<'a>(
    title: &'a str,
    error: &'a hoprs_core::Error,
) -> Element<'a, Message> {
    column![
        text(title).size(theme::TEXT_SM).color(palette().error),
        failure(Some(error)),
    ]
    .spacing(theme::SPACE_2XS)
    .into()
}

/// A titled list of names (mono), skipped when empty.
pub(super) fn names<'a>(
    col: iced::widget::Column<'a, Message>,
    title: &'a str,
    items: impl IntoIterator<Item = String>,
    error: bool,
) -> iced::widget::Column<'a, Message> {
    let items: Vec<String> = items.into_iter().collect();
    if items.is_empty() {
        return col;
    }
    let count = items.len();
    let color = if error {
        palette().error
    } else {
        palette().ink_2
    };
    let list = column(items.into_iter().map(|item| {
        text(item)
            .font(theme::MONO)
            .size(theme::TEXT_MONO)
            .color(color)
            .into()
    }))
    .spacing(theme::SPACE_3XS);
    col.push(
        column![
            text(strings::section_count(title, count))
                .size(theme::TEXT_XS)
                .font(theme::INTER_SEMIBOLD),
            list
        ]
        .spacing(theme::SPACE_2XS),
    )
}

fn restore_report(report: &BackupImportReport) -> Element<'_, Message> {
    let mut col = column![text(strings::restore_done(
        report.added_sites,
        report.added_workspaces,
        report.secrets_restored
    ))]
    .spacing(theme::SPACE_SM);
    col = names(
        col,
        strings::backup_secret_errors(),
        report.secret_errors.iter().cloned(),
        true,
    );
    col = names(
        col,
        strings::backup_skipped_sites(),
        report.skipped_sites.iter().cloned(),
        false,
    );
    col = names(
        col,
        strings::backup_skipped_workspaces(),
        report.skipped_workspaces.iter().cloned(),
        false,
    );
    col.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Language, set_language};

    #[test]
    fn agents_section_shows_the_channel_error() {
        use hoprs_core::{Error, ErrorKind};

        let mut app = App::new();
        assert_eq!(agents_error(&app), None);
        app.agents.error = Some(Error::coded(
            ErrorKind::Io,
            "agent-channel-failed",
            "Could not open the channel for agents",
        ));
        assert_eq!(
            agents_error(&app).as_deref(),
            Some("Could not open the channel for agents")
        );
    }

    #[test]
    fn every_section_has_title_description_and_icon() {
        for language in Language::ALL {
            set_language(language);
            let titles: Vec<&str> = SettingsSection::ALL
                .into_iter()
                .map(section_title)
                .collect();
            let descriptions: Vec<&str> = SettingsSection::ALL
                .into_iter()
                .map(section_description)
                .collect();
            for list in [&titles, &descriptions] {
                assert!(list.iter().all(|t| !t.is_empty()), "{language:?}");
                let mut sorted = list.to_vec();
                sorted.sort_unstable();
                sorted.dedup();
                assert_eq!(
                    sorted.len(),
                    SettingsSection::ALL.len(),
                    "{language:?}: {list:?}"
                );
            }
        }
        set_language(Language::En);
        let mut icons: Vec<char> = SettingsSection::ALL
            .into_iter()
            .map(|s| section_icon(s).codepoint())
            .collect();
        icons.sort_unstable();
        icons.dedup();
        assert_eq!(icons.len(), SettingsSection::ALL.len());
    }

    #[test]
    fn update_status_text_per_manual_check() {
        set_language(Language::En);
        let error = hoprs_core::Error::coded(
            hoprs_core::ErrorKind::Network,
            "update-check-failed",
            "Could not check for updates",
        );
        assert_eq!(update_status(&ManualCheck::Idle), None);
        assert_eq!(
            update_status(&ManualCheck::Checking).as_deref(),
            Some("Checking…")
        );
        assert_eq!(
            update_status(&ManualCheck::UpToDate).as_deref(),
            Some("hoprs is up to date")
        );
        assert_eq!(
            update_status(&ManualCheck::Available(semver::Version::new(1, 4, 0))).as_deref(),
            Some("hoprs 1.4.0 is available")
        );
        assert_eq!(
            update_status(&ManualCheck::Failed(error.clone())),
            Some(strings::error(&error))
        );
    }

    #[test]
    fn general_shows_the_version() {
        assert!(version_line().contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn drive_line_follows_the_marker_and_the_build() {
        set_language(Language::En);
        let mut app = App::new();
        for (available, marker, line) in [
            (false, false, None),
            (false, true, None),
            (true, false, None),
            (true, true, Some("Google Drive: connected")),
        ] {
            app.drive_available = available;
            app.settings.google_drive = marker;
            assert_eq!(drive_line(&app), line, "{available} {marker}");
        }
    }

    #[test]
    fn drive_error_shows_without_the_marker_but_only_in_a_drive_build() {
        let mut app = App::new();
        app.drive_error = Some(hoprs_core::Error::new(
            hoprs_core::ErrorKind::Network,
            "keyring",
        ));
        app.settings.google_drive = false;
        app.drive_available = true;
        assert_eq!(drive_line(&app), None);
        assert!(drive_error(&app).is_some());
        app.drive_available = false;
        assert!(drive_error(&app).is_none());
        app.drive_available = true;
        app.drive_error = None;
        assert!(drive_error(&app).is_none());
    }

    #[test]
    fn agent_command_quotes_the_path() {
        let exe = |p: &str| agent_command(Some(Path::new(p)));
        assert_eq!(
            exe("/usr/bin/hoprs"),
            "claude mcp add hoprs -- \"/usr/bin/hoprs\" mcp"
        );
        assert_eq!(
            exe("/Applications/hoprs.app/Contents/MacOS/hoprs"),
            "claude mcp add hoprs -- \"/Applications/hoprs.app/Contents/MacOS/hoprs\" mcp"
        );
        assert_eq!(agent_command(None), "claude mcp add hoprs -- \"hoprs\" mcp");
        #[cfg(unix)]
        for path in [
            "/opt/my apps/hoprs",
            "/opt/a\"b $HOME `x` \\y/hoprs",
            "/opt/it's/hoprs",
        ] {
            let command = exe(path);
            let words = shlex::split(&command).expect("a well-formed command");
            assert_eq!(
                words,
                ["claude", "mcp", "add", "hoprs", "--", path, "mcp"],
                "{command}"
            );
        }
        #[cfg(windows)]
        assert_eq!(
            exe(r"C:\Program Files\hoprs\hoprs.exe"),
            "claude mcp add hoprs -- \"C:\\Program Files\\hoprs\\hoprs.exe\" mcp"
        );
    }

    #[test]
    fn agent_json_is_valid_json() {
        for path in [
            Some(r"C:\Program Files\hoprs\hoprs.exe"),
            Some("/usr/bin/hoprs"),
            Some("/opt/a\"b c/hoprs"),
            None,
        ] {
            let json = agent_json(path.map(Path::new));
            let value: serde_json::Value = serde_json::from_str(&json).expect(&json);
            assert_eq!(value["command"], path.unwrap_or("hoprs"), "{json}");
            assert_eq!(value["args"], serde_json::json!(["mcp"]), "{json}");
        }
    }

    #[test]
    fn agent_levels_counts_each_level() {
        use hoprs_core::{AgentAccess, Protocol};
        let at = |access| {
            let mut site = Site::new("s", "h.example", "u", Protocol::Ssh);
            site.agent_access = access;
            site
        };
        let sites = [
            at(AgentAccess::None),
            at(AgentAccess::Read),
            at(AgentAccess::Read),
            at(AgentAccess::All),
        ];
        assert_eq!(agent_levels(&sites), (2, 1));
        assert_eq!(agent_levels(&[]), (0, 0));
    }
}
