//! The app's settings: `settings.toml`, beside the profiles.
//!
//! A missing file means the defaults. A file that cannot be read or parsed also gives
//! the defaults, with the error to show inline; it is never overwritten (the app does not
//! save settings while it is unreadable). A value out of range is clamped, not an error;
//! an unknown terminal theme falls back to the default. Saving keeps the keys this
//! version does not know (a newer hoprs may have written them).

use std::path::Path;

use hoprs_core::{Error, ErrorKind, Result};
use hoprs_transfer::ConflictPolicy;
use serde::{Deserialize, Serialize};

use crate::i18n::Language;
use crate::terminal::TermTheme;
use crate::ui::layout::QUEUE_MAX_HEIGHT;

pub const FILE_NAME: &str = "settings.toml";
pub const WORKERS_MIN: usize = 1;
pub const WORKERS_MAX: usize = 16;
pub const WORKERS_DEFAULT: usize = 4;
/// Terminal font size, in px (spec §8).
pub const FONT_SIZE_MIN: u16 = 10;
pub const FONT_SIZE_MAX: u16 = 24;
pub const FONT_SIZE_DEFAULT: u16 = 13;
/// Terminal scrollback, in lines (spec §8).
pub const SCROLLBACK_MIN: usize = 1_000;
pub const SCROLLBACK_MAX: usize = 100_000;
pub const SCROLLBACK_DEFAULT: usize = 10_000;
/// Keepalive of SSH and FTP sessions, in seconds (spec §4.3).
pub const KEEPALIVE_MIN: u64 = 10;
pub const KEEPALIVE_MAX: u64 = 300;
pub const KEEPALIVE_DEFAULT: u64 = 30;
/// The editor remote edits open files with: `system` is the OS's default editor; any other
/// value is a command, the path added as its last argument.
pub const EDITOR_DEFAULT: &str = SYSTEM_EDITOR;
/// The editor setting that means the OS's default editor ([`is_system_editor`]).
pub const SYSTEM_EDITOR: &str = "system";
/// Largest `settings.toml` read; a real one is under 200 bytes.
const MAX_BYTES: u64 = 64 * 1024;

/// The terminal's colours. Only one for now: the setting exists so that adding themes
/// later does not change the file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TerminalTheme {
    #[default]
    HoprsDark,
}

impl TerminalTheme {
    pub const ALL: [TerminalTheme; 1] = [TerminalTheme::HoprsDark];

    /// The name in `settings.toml`.
    fn key(self) -> &'static str {
        match self {
            Self::HoprsDark => "hoprs-dark",
        }
    }

    /// The name the settings page shows.
    pub fn label(self) -> &'static str {
        match self {
            Self::HoprsDark => crate::ui::strings::theme_hoprs_dark(),
        }
    }

    /// The colours.
    pub fn colors(self) -> TermTheme {
        match self {
            Self::HoprsDark => TermTheme::hoprs_dark(),
        }
    }

    /// The theme named `key`; an unknown name (a newer hoprs) is the default.
    fn from_key(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|theme| theme.key() == key)
            .unwrap_or_default()
    }
}

/// The interface language setting: follow the OS, or a fixed language.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LanguageChoice {
    #[default]
    System,
    Fixed(Language),
}

impl LanguageChoice {
    /// The choices the settings page offers, in order.
    pub const ALL: [LanguageChoice; 4] = [
        LanguageChoice::System,
        LanguageChoice::Fixed(Language::En),
        LanguageChoice::Fixed(Language::PtBr),
        LanguageChoice::Fixed(Language::Tr),
    ];

    /// The value in `settings.toml`.
    fn key(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Fixed(lang) => lang.tag(),
        }
    }

    /// The name the settings page shows (a language is named in itself).
    pub fn label(self) -> &'static str {
        match self {
            Self::System => crate::ui::strings::language_system(),
            Self::Fixed(Language::En) => crate::ui::strings::language_english(),
            Self::Fixed(Language::PtBr) => crate::ui::strings::language_pt_br(),
            Self::Fixed(Language::Tr) => crate::ui::strings::language_tr(),
        }
    }

    /// The choice named `key`; an unknown value (a newer hoprs) is `System`.
    fn from_key(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|choice| choice.key() == key)
            .unwrap_or_default()
    }
}

/// One sidebar section's state: folded to its heading, or hidden altogether.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct SectionState {
    pub collapsed: bool,
    pub hidden: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Transfer workers per site (`1..=16`), for the connections made from now on.
    pub workers_per_site: usize,
    /// What a transfer does when its destination exists.
    pub default_policy: ConflictPolicy,
    /// The terminal's font size in px (`10..=24`).
    pub terminal_font_size: u16,
    /// Lines of history each terminal keeps (`1 000..=100 000`), for the terminals opened
    /// from now on.
    pub terminal_scrollback: usize,
    pub terminal_theme: TerminalTheme,
    /// The interface language: the OS's, or a fixed one.
    pub language: LanguageChoice,
    /// Seconds between keepalives on an idle SSH or FTP connection (`10..=300`), for the
    /// connections made from now on.
    pub keepalive_secs: u64,
    /// The editor of remote edits: `system` (the OS's default editor), or a command split
    /// with shell quoting rules (Windows command-line rules on Windows; no shell runs), the
    /// file's path added last. Never empty ([`valid_editor`]).
    pub editor_command: String,
    /// Whether a folder compare re-runs by itself when the two folders change.
    pub compare_auto: bool,
    /// Names a folder compare skips: `*` matches any run of characters, matched on the
    /// entry name.
    pub compare_ignore: Vec<String>,
    /// The sidebar's Workspaces section.
    pub workspaces: SectionState,
    /// The sidebar's All sites section.
    pub sites: SectionState,
    /// The compare panel's height in px; `None` is its default share of the tab. Clamped
    /// where it is drawn, never rewritten for a small window.
    pub compare_height: Option<f32>,
    /// The queue's maximum height in px (it still shrinks to its rows).
    pub queue_height: f32,
    /// Whether the app checks for a newer release by itself.
    pub update_check: bool,
    /// The newest version an update card was shown for; `None` until one was.
    pub update_seen: Option<semver::Version>,
    /// Whether the Google Drive destination was connected on this computer (the token
    /// itself is in the keyring); written only when `true`.
    pub google_drive: bool,
    /// Whether coding agents may connect to hoprs at all (the master switch of Settings
    /// › Agents (MCP)); off by default, written only when `true`.
    pub agents_enabled: bool,
}

/// Keys [`file_of`] leaves out when they hold their default.
const OPTIONAL_KEYS: [&str; 4] = [
    "compare_height",
    "update_seen",
    "google_drive",
    "agents_enabled",
];

/// The largest height a saved panel size may have.
const PANEL_HEIGHT_MAX: f32 = 10_000.0;

/// A dragged panel height as it is kept and saved: whole pixels, at most
/// [`PANEL_HEIGHT_MAX`] (what [`panel_height`] reads back).
pub fn round_panel_height(height: f32) -> f32 {
    height.round().min(PANEL_HEIGHT_MAX)
}

/// A saved panel height: finite, positive, at most [`PANEL_HEIGHT_MAX`].
fn panel_height(value: Option<toml::Value>) -> Option<f32> {
    let height = match value? {
        toml::Value::Float(f) => f as f32,
        toml::Value::Integer(i) => i as f32,
        _ => return None,
    }
    .round();
    (height.is_finite() && height > 0.0 && height <= PANEL_HEIGHT_MAX).then_some(height)
}

/// The names a folder compare skips until the user changes the list.
pub const COMPARE_IGNORE_DEFAULT: [&str; 3] = [".git", "node_modules", ".DS_Store"];

/// The default ignore list, owned.
pub fn compare_ignore_default() -> Vec<String> {
    COMPARE_IGNORE_DEFAULT
        .iter()
        .map(|s| (*s).to_owned())
        .collect()
}

/// The ignore list typed as comma-separated names: trimmed, empty items dropped.
pub fn parse_compare_ignore(typed: &str) -> Vec<String> {
    typed
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            workers_per_site: WORKERS_DEFAULT,
            default_policy: ConflictPolicy::Ask,
            terminal_font_size: FONT_SIZE_DEFAULT,
            terminal_scrollback: SCROLLBACK_DEFAULT,
            terminal_theme: TerminalTheme::HoprsDark,
            language: LanguageChoice::System,
            keepalive_secs: KEEPALIVE_DEFAULT,
            editor_command: EDITOR_DEFAULT.to_owned(),
            compare_auto: true,
            compare_ignore: compare_ignore_default(),
            workspaces: SectionState::default(),
            sites: SectionState::default(),
            compare_height: None,
            queue_height: QUEUE_MAX_HEIGHT,
            update_check: true,
            update_seen: None,
            google_drive: false,
            agents_enabled: false,
        }
    }
}

/// Whether the editor setting `command` means the OS's default editor: `system`, trimmed,
/// in any case.
pub fn is_system_editor(command: &str) -> bool {
    command.trim().eq_ignore_ascii_case(SYSTEM_EDITOR)
}

/// Whether `command` can be the editor command: something besides blanks.
pub fn valid_editor(command: &str) -> bool {
    !command.trim().is_empty()
}

/// `size` within `10..=24`.
pub fn clamp_font_size(size: u16) -> u16 {
    size.clamp(FONT_SIZE_MIN, FONT_SIZE_MAX)
}

/// `lines` within `1 000..=100 000`.
pub fn clamp_scrollback(lines: usize) -> usize {
    lines.clamp(SCROLLBACK_MIN, SCROLLBACK_MAX)
}

/// `secs` within `10..=300`.
pub fn clamp_keepalive(secs: u64) -> u64 {
    secs.clamp(KEEPALIVE_MIN, KEEPALIVE_MAX)
}

/// `n` within `1..=16`.
pub fn clamp_workers(n: usize) -> usize {
    n.clamp(WORKERS_MIN, WORKERS_MAX)
}

/// What [`load`] found: the settings to use and, if the file was unusable, why (the
/// settings are then the defaults).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsLoad {
    pub settings: Settings,
    pub error: Option<Error>,
}

/// The file's layout. Unknown keys are ignored here (a newer hoprs may add some) and
/// kept by [`save`].
#[derive(Debug, Default, Deserialize, Serialize)]
struct File {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workers_per_site: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_policy: Option<Policy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_font_size: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_scrollback: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    keepalive_secs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    editor_command: Option<String>,
    // Any TOML value: a wrong type reads as the default instead of failing the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compare_auto: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compare_ignore: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspaces_collapsed: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspaces_hidden: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sites_collapsed: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sites_hidden: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compare_height: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    queue_height: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    update_check: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    update_seen: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    google_drive: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agents_enabled: Option<toml::Value>,
}

/// A bool key; anything else (or missing) is `false`.
fn flag(value: Option<toml::Value>) -> bool {
    value.and_then(|value| value.as_bool()).unwrap_or(false)
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Policy {
    Ask,
    Overwrite,
    Skip,
    OverwriteIfNewer,
    Resume,
}

impl From<ConflictPolicy> for Policy {
    fn from(policy: ConflictPolicy) -> Self {
        match policy {
            ConflictPolicy::Ask => Self::Ask,
            ConflictPolicy::Overwrite => Self::Overwrite,
            ConflictPolicy::Skip => Self::Skip,
            ConflictPolicy::OverwriteIfNewer => Self::OverwriteIfNewer,
            ConflictPolicy::Resume => Self::Resume,
        }
    }
}

impl From<Policy> for ConflictPolicy {
    fn from(policy: Policy) -> Self {
        match policy {
            Policy::Ask => Self::Ask,
            Policy::Overwrite => Self::Overwrite,
            Policy::Skip => Self::Skip,
            Policy::OverwriteIfNewer => Self::OverwriteIfNewer,
            Policy::Resume => Self::Resume,
        }
    }
}

/// Parses `settings.toml` content; missing keys take their defaults.
pub fn parse(content: &str) -> Result<Settings> {
    let file: File = toml::from_str(content).map_err(|err| {
        Error::coded(
            ErrorKind::Protocol,
            "settings-cannot-read",
            "Can't read settings.toml",
        )
        .with_detail(err.message())
    })?;
    let defaults = Settings::default();
    Ok(Settings {
        workers_per_site: file
            .workers_per_site
            .map_or(defaults.workers_per_site, |n| {
                clamp_workers(usize::try_from(n.max(0)).unwrap_or(WORKERS_MAX))
            }),
        default_policy: file
            .default_policy
            .map_or(defaults.default_policy, ConflictPolicy::from),
        terminal_font_size: file
            .terminal_font_size
            .map_or(defaults.terminal_font_size, |n| {
                clamp_font_size(u16::try_from(n.max(0)).unwrap_or(FONT_SIZE_MAX))
            }),
        terminal_scrollback: file
            .terminal_scrollback
            .map_or(defaults.terminal_scrollback, |n| {
                clamp_scrollback(usize::try_from(n.max(0)).unwrap_or(SCROLLBACK_MAX))
            }),
        terminal_theme: file
            .terminal_theme
            .as_deref()
            .map_or(defaults.terminal_theme, TerminalTheme::from_key),
        language: file
            .language
            .as_deref()
            .map_or(defaults.language, LanguageChoice::from_key),
        keepalive_secs: file.keepalive_secs.map_or(defaults.keepalive_secs, |n| {
            clamp_keepalive(u64::try_from(n.max(0)).unwrap_or(KEEPALIVE_MAX))
        }),
        // An empty command is not a command: the default.
        editor_command: file
            .editor_command
            .filter(|command| valid_editor(command))
            .unwrap_or(defaults.editor_command),
        compare_auto: file
            .compare_auto
            .and_then(|value| value.as_bool())
            .unwrap_or(defaults.compare_auto),
        compare_ignore: file
            .compare_ignore
            .and_then(|value| match value {
                toml::Value::Array(items) => items
                    .into_iter()
                    .map(|item| match item {
                        toml::Value::String(name) => Some(name),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>(),
                _ => None,
            })
            .map_or(defaults.compare_ignore, |names| {
                names
                    .into_iter()
                    .map(|name| name.trim().to_owned())
                    .filter(|name| !name.is_empty())
                    .collect()
            }),
        workspaces: SectionState {
            collapsed: flag(file.workspaces_collapsed),
            hidden: flag(file.workspaces_hidden),
        },
        sites: SectionState {
            collapsed: flag(file.sites_collapsed),
            hidden: flag(file.sites_hidden),
        },
        compare_height: panel_height(file.compare_height),
        queue_height: panel_height(file.queue_height).unwrap_or(defaults.queue_height),
        update_check: file
            .update_check
            .and_then(|value| value.as_bool())
            .unwrap_or(defaults.update_check),
        update_seen: file
            .update_seen
            .and_then(|value| value.as_str().and_then(|text| text.parse().ok())),
        google_drive: flag(file.google_drive),
        agents_enabled: flag(file.agents_enabled),
    })
}

/// The file's keys for `settings` (clamped).
fn file_of(settings: &Settings) -> File {
    File {
        workers_per_site: i64::try_from(clamp_workers(settings.workers_per_site)).ok(),
        default_policy: Some(settings.default_policy.into()),
        terminal_font_size: Some(clamp_font_size(settings.terminal_font_size).into()),
        terminal_scrollback: i64::try_from(clamp_scrollback(settings.terminal_scrollback)).ok(),
        terminal_theme: Some(settings.terminal_theme.key().to_owned()),
        language: Some(settings.language.key().to_owned()),
        keepalive_secs: i64::try_from(clamp_keepalive(settings.keepalive_secs)).ok(),
        editor_command: Some(if valid_editor(&settings.editor_command) {
            settings.editor_command.clone()
        } else {
            EDITOR_DEFAULT.to_owned()
        }),
        compare_auto: Some(toml::Value::Boolean(settings.compare_auto)),
        compare_ignore: Some(toml::Value::Array(
            settings
                .compare_ignore
                .iter()
                .cloned()
                .map(toml::Value::String)
                .collect(),
        )),
        workspaces_collapsed: Some(toml::Value::Boolean(settings.workspaces.collapsed)),
        workspaces_hidden: Some(toml::Value::Boolean(settings.workspaces.hidden)),
        sites_collapsed: Some(toml::Value::Boolean(settings.sites.collapsed)),
        sites_hidden: Some(toml::Value::Boolean(settings.sites.hidden)),
        compare_height: settings
            .compare_height
            .and_then(|h| panel_height(Some(toml::Value::Float(f64::from(h)))))
            .map(|h| toml::Value::Float(f64::from(h))),
        queue_height: Some(toml::Value::Float(f64::from(
            panel_height(Some(toml::Value::Float(f64::from(settings.queue_height))))
                .unwrap_or(QUEUE_MAX_HEIGHT),
        ))),
        update_check: Some(toml::Value::Boolean(settings.update_check)),
        update_seen: settings
            .update_seen
            .as_ref()
            .map(|version| toml::Value::String(version.to_string())),
        google_drive: settings.google_drive.then_some(toml::Value::Boolean(true)),
        agents_enabled: settings
            .agents_enabled
            .then_some(toml::Value::Boolean(true)),
    }
}

/// The file content for `settings`.
pub fn render(settings: &Settings) -> String {
    toml::to_string(&file_of(settings)).expect("plain keys always serialize")
}

/// `existing` (a file's text) with the keys of `settings` set: every other key, table
/// and value stays as it was. Fails if `existing` is not TOML.
fn render_over(existing: &str, settings: &Settings) -> Result<String> {
    let mut table: toml::Table = toml::from_str(existing).map_err(|err| {
        Error::coded(
            ErrorKind::Protocol,
            "settings-cannot-read",
            "Can't read settings.toml",
        )
        .with_detail(err.message())
    })?;
    let mut ours = toml::Table::try_from(file_of(settings)).expect("plain keys always serialize");
    // A theme this version does not know reads as the default: unless the user picked
    // another one, the name stays as written (a newer hoprs may know it).
    if let Some(existing) = table.get("terminal_theme").and_then(toml::Value::as_str)
        && TerminalTheme::from_key(existing) == settings.terminal_theme
    {
        ours.remove("terminal_theme");
    }
    // Same for a language this version does not know: it reads as `system`.
    if let Some(existing) = table.get("language").and_then(toml::Value::as_str)
        && LanguageChoice::from_key(existing) == settings.language
    {
        ours.remove("language");
    }
    // An optional key that is now at its default is written by nobody: drop the old one.
    for key in OPTIONAL_KEYS {
        if !ours.contains_key(key) {
            table.remove(key);
        }
    }
    table.extend(ours);
    Ok(toml::to_string(&table).expect("a parsed table always serializes"))
}

/// Reads `path`. Blocking (small file): the backend calls it once when it opens.
pub fn load(path: &Path) -> SettingsLoad {
    match read(path) {
        Ok(Some(content)) => match parse(&content) {
            Ok(settings) => SettingsLoad {
                settings,
                error: None,
            },
            Err(error) => SettingsLoad {
                settings: Settings::default(),
                error: Some(error),
            },
        },
        Ok(None) => SettingsLoad::default(),
        Err(error) => SettingsLoad {
            settings: Settings::default(),
            error: Some(error),
        },
    }
}

/// The file's text, `None` if it does not exist.
fn read(path: &Path) -> Result<Option<String>> {
    use std::io::Read;

    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(cannot_read(err.to_string())),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| cannot_read(err.to_string()))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(cannot_read(format!("larger than {MAX_BYTES} bytes")));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| cannot_read("not valid UTF-8".to_owned()))
}

fn cannot_read(detail: String) -> Error {
    Error::coded(
        ErrorKind::Io,
        "settings-cannot-read",
        "Can't read settings.toml",
    )
    .with_detail(detail)
}

/// Writes `settings` to `path` atomically (the vault's crash-safe write), keeping the
/// keys of the file there this version does not know. A file that can't be read or
/// parsed is left alone (an error). Blocking.
pub fn save(path: &Path, settings: &Settings) -> Result<()> {
    let content = match read(path)? {
        Some(existing) => render_over(&existing, settings)?,
        None => render(settings),
    };
    hoprs_vault::write_atomic(path, content.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_settings_default_roundtrip_and_invalid() {
        let defaults = parse("").unwrap();
        assert!(defaults.compare_auto);
        assert_eq!(
            defaults.compare_ignore,
            vec![".git", "node_modules", ".DS_Store"]
        );

        let custom = Settings {
            compare_auto: false,
            compare_ignore: vec!["a".into(), "*.log".into()],
            ..Settings::default()
        };
        assert_eq!(parse(&render(&custom)).unwrap(), custom);
        // An empty list is a valid choice: nothing ignored.
        let none = Settings {
            compare_ignore: Vec::new(),
            ..Settings::default()
        };
        assert_eq!(parse(&render(&none)).unwrap(), none);

        // Wrong types read as the defaults, the rest of the file still applies.
        let bad =
            parse("compare_auto = \"yes\"\ncompare_ignore = 3\nworkers_per_site = 2").unwrap();
        assert!(bad.compare_auto);
        assert_eq!(bad.compare_ignore, defaults.compare_ignore);
        assert_eq!(bad.workers_per_site, 2);
        let bad = parse("compare_ignore = [\"a\", 1]").unwrap();
        assert_eq!(bad.compare_ignore, defaults.compare_ignore);
    }

    #[test]
    fn sidebar_sections_roundtrip_and_invalid() {
        let all = Settings {
            workspaces: SectionState {
                collapsed: true,
                hidden: true,
            },
            sites: SectionState {
                collapsed: true,
                hidden: true,
            },
            ..Settings::default()
        };
        assert_eq!(parse(&render(&all)).unwrap(), all);
        let missing = parse("").unwrap();
        assert_eq!(missing.workspaces, SectionState::default());
        assert_eq!(missing.sites, SectionState::default());

        let bad =
            parse("workspaces_hidden = 3\nsites_collapsed = true\nworkers_per_site = 2").unwrap();
        assert!(!bad.workspaces.hidden);
        assert!(bad.sites.collapsed);
        assert_eq!(bad.workers_per_site, 2);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "future_key = 1\nsites_hidden = true\n").unwrap();
        let loaded = load(&path).settings;
        assert!(loaded.sites.hidden);
        save(&path, &loaded).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("future_key = 1"), "{text}");
        assert!(text.contains("sites_hidden = true"), "{text}");
    }

    #[test]
    fn update_keys_roundtrip_and_invalid() {
        let defaults = Settings::default();
        assert!(defaults.update_check);
        assert_eq!(defaults.update_seen, None);
        assert!(!render(&defaults).contains("update_seen"));

        let set = Settings {
            update_check: false,
            update_seen: Some(semver::Version::new(1, 4, 0)),
            ..Settings::default()
        };
        assert_eq!(parse(&render(&set)).unwrap(), set);

        assert!(parse("update_check = \"no\"").unwrap().update_check);
        assert_eq!(parse("update_seen = 3").unwrap().update_seen, None);
        assert_eq!(parse("update_seen = \"banana\"").unwrap().update_seen, None);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "future_key = 1\nupdate_seen = \"1.4.0\"\n").unwrap();
        let loaded = load(&path).settings;
        assert_eq!(loaded.update_seen, Some(semver::Version::new(1, 4, 0)));
        save(&path, &loaded).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("future_key = 1"), "{text}");
        assert!(text.contains("update_seen = \"1.4.0\""), "{text}");
        // Back to unset: the key goes away.
        save(&path, &Settings::default()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("update_seen"), "{text}");
    }

    #[test]
    fn panel_heights_roundtrip_and_invalid() {
        let tall = Settings {
            compare_height: Some(241.0),
            queue_height: 300.0,
            ..Settings::default()
        };
        assert_eq!(parse(&render(&tall)).unwrap(), tall);
        let defaults = parse("").unwrap();
        assert_eq!(defaults.compare_height, None);
        assert_eq!(defaults.queue_height, QUEUE_MAX_HEIGHT);
        let integers = parse("compare_height = 240\nqueue_height = 300").unwrap();
        assert_eq!(integers.compare_height, Some(240.0));
        assert_eq!(integers.queue_height, 300.0);
        for bad in ["-5", "0", "nan", "inf", "1e9", "\"tall\""] {
            let s = parse(&format!("compare_height = {bad}\nqueue_height = {bad}")).unwrap();
            assert_eq!(s.compare_height, None, "{bad}");
            assert_eq!(s.queue_height, QUEUE_MAX_HEIGHT, "{bad}");
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "future_key = 1\ncompare_height = 241.0\n").unwrap();
        let loaded = load(&path).settings;
        assert_eq!(loaded.compare_height, Some(241.0));
        save(&path, &loaded).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("future_key = 1"), "{text}");
        assert!(text.contains("compare_height = 241.0"), "{text}");
        assert!(text.contains("queue_height = 210.0"), "{text}");
    }

    #[test]
    fn resetting_compare_height_removes_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "future_key = 1\n").unwrap();
        let mut s = load(&path).settings;
        s.compare_height = Some(240.3);
        save(&path, &s).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("compare_height = 240.0"), "{text}");
        s.compare_height = None;
        save(&path, &s).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("compare_height"), "{text}");
        assert!(text.contains("future_key = 1"), "{text}");
        assert_eq!(load(&path).settings.compare_height, None);
    }

    #[test]
    fn compare_settings_keep_unknown_keys_on_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "future_key = 1\ncompare_auto = \"yes\"\n").unwrap();
        let loaded = load(&path).settings;
        save(
            &path,
            &Settings {
                compare_auto: false,
                ..loaded
            },
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("future_key = 1"), "{text}");
        assert!(text.contains("compare_auto = false"), "{text}");
        assert!(text.contains("compare_ignore"), "{text}");
    }

    #[test]
    fn compare_ignore_typed_is_trimmed_and_cleaned() {
        assert_eq!(parse_compare_ignore(" a, ,*.log "), vec!["a", "*.log"]);
        assert!(parse_compare_ignore("  ").is_empty());
    }

    /// A fresh install opens remote edits with the OS's default editor.
    #[test]
    fn editor_default_is_system() {
        assert_eq!(Settings::default().editor_command, "system");
    }

    #[test]
    fn system_editor_is_recognised() {
        assert!(is_system_editor(" System "));
        assert!(is_system_editor("system"));
        assert!(!is_system_editor("xdg-open"));
        assert!(!is_system_editor("system-editor"));
    }

    #[test]
    fn settings_roundtrip_and_invalid_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles").join(FILE_NAME);

        // No file: the defaults, no error, nothing created.
        assert_eq!(load(&path), SettingsLoad::default());
        assert_eq!(
            Settings::default(),
            Settings {
                workers_per_site: 4,
                default_policy: ConflictPolicy::Ask,
                ..Settings::default()
            }
        );
        assert!(!path.exists());

        // Every policy survives a save and a load.
        for policy in [
            ConflictPolicy::Ask,
            ConflictPolicy::Overwrite,
            ConflictPolicy::Skip,
            ConflictPolicy::OverwriteIfNewer,
            ConflictPolicy::Resume,
        ] {
            let settings = Settings {
                workers_per_site: 7,
                default_policy: policy,
                ..Settings::default()
            };
            save(&path, &settings).unwrap();
            assert_eq!(
                load(&path),
                SettingsLoad {
                    settings,
                    error: None,
                }
            );
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("workers_per_site = 7"), "{text}");
        assert!(text.contains("default_policy = \"resume\""), "{text}");

        // Missing keys take their defaults; unknown keys are ignored.
        std::fs::write(&path, "default_policy = \"skip\"\nfuture_key = 1\n").unwrap();
        assert_eq!(
            load(&path).settings,
            Settings {
                workers_per_site: 4,
                default_policy: ConflictPolicy::Skip,
                ..Settings::default()
            }
        );

        // Invalid files: the defaults plus an error, and the file is left as it was.
        for bad in [
            "workers_per_site = \"many\"\n",
            "default_policy = \"sometimes\"\n",
            "this is not toml",
        ] {
            std::fs::write(&path, bad).unwrap();
            let loaded = load(&path);
            assert_eq!(loaded.settings, Settings::default(), "{bad}");
            let error = loaded.error.expect("an invalid file is reported");
            assert_eq!(error.message, "Can't read settings.toml");
            assert!(error.detail.is_some());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), bad);
        }
        std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        assert!(load(&path).error.is_some());
        assert_eq!(std::fs::read(&path).unwrap(), [0xff, 0xfe, 0x00]);
    }

    #[test]
    fn settings_terminal_fields_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let defaults = Settings::default();
        assert_eq!(defaults.terminal_font_size, 13);
        assert_eq!(defaults.terminal_scrollback, 10_000);
        assert_eq!(defaults.terminal_theme, TerminalTheme::HoprsDark);

        // A file from before the terminal: the terminal defaults.
        std::fs::write(&path, "workers_per_site = 2\n").unwrap();
        let loaded = load(&path).settings;
        assert_eq!(loaded.workers_per_site, 2);
        assert_eq!(loaded.terminal_font_size, 13);
        assert_eq!(loaded.terminal_scrollback, 10_000);

        // Saved and read back, next to keys this version does not know (kept as they were).
        std::fs::write(
            &path,
            "future_key = 1\nworkers_per_site = 2\n\n[future_table]\nx = \"y\"\n",
        )
        .unwrap();
        let settings = Settings {
            terminal_font_size: 18,
            terminal_scrollback: 50_000,
            ..loaded
        };
        save(&path, &settings).unwrap();
        assert_eq!(
            load(&path),
            SettingsLoad {
                settings,
                error: None
            }
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("terminal_font_size = 18"), "{text}");
        assert!(text.contains("terminal_scrollback = 50000"), "{text}");
        assert!(text.contains("terminal_theme = \"hoprs-dark\""), "{text}");
        assert!(
            text.contains("future_key = 1"),
            "unknown keys survive: {text}"
        );
        assert!(text.contains("[future_table]"), "{text}");

        // Out of range is clamped, on load and on save: 10..=24 and 1 000..=100 000.
        for (file, font, lines) in [
            ("terminal_font_size = 2\nterminal_scrollback = 5", 10, 1_000),
            (
                "terminal_font_size = 99\nterminal_scrollback = 900000000",
                24,
                100_000,
            ),
            (
                "terminal_font_size = -4\nterminal_scrollback = -1",
                10,
                1_000,
            ),
        ] {
            let parsed = parse(file).unwrap();
            assert_eq!(
                (parsed.terminal_font_size, parsed.terminal_scrollback),
                (font, lines),
                "{file}"
            );
        }
        let rendered = render(&Settings {
            terminal_font_size: 40,
            terminal_scrollback: 1,
            ..Settings::default()
        });
        let parsed = parse(&rendered).unwrap();
        assert_eq!(
            (parsed.terminal_font_size, parsed.terminal_scrollback),
            (24, 1_000)
        );
        assert_eq!(clamp_font_size(9), 10);
        assert_eq!(clamp_scrollback(250_000), 100_000);

        // A theme this version does not know falls back to "hoprs dark", without an error,
        // and saving keeps its name.
        let parsed = parse("terminal_theme = \"solarized\"").unwrap();
        assert_eq!(parsed.terminal_theme, TerminalTheme::HoprsDark);
        std::fs::write(&path, "terminal_theme = \"solarized\"\n").unwrap();
        let loaded = load(&path).settings;
        save(
            &path,
            &Settings {
                terminal_font_size: 15,
                ..loaded
            },
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("terminal_theme = \"solarized\""), "{text}");
        assert!(text.contains("terminal_font_size = 15"), "{text}");
        assert_eq!(TerminalTheme::HoprsDark.label(), "hoprs dark");

        // An unreadable file is still never written over: save only merges into a file
        // it can parse, else it would lose the user's keys.
        std::fs::write(&path, "this is not toml").unwrap();
        assert!(save(&path, &Settings::default()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "this is not toml");
    }

    #[test]
    fn workers_clamped() {
        assert_eq!(clamp_workers(0), 1);
        assert_eq!(clamp_workers(1), 1);
        assert_eq!(clamp_workers(9), 9);
        assert_eq!(clamp_workers(16), 16);
        assert_eq!(clamp_workers(17), 16);
        assert_eq!(clamp_workers(usize::MAX), 16);
        for (file, expected) in [
            ("workers_per_site = 0", 1),
            ("workers_per_site = -3", 1),
            ("workers_per_site = 99", 16),
            ("workers_per_site = 9223372036854775807", 16),
            ("workers_per_site = 12", 12),
        ] {
            assert_eq!(parse(file).unwrap().workers_per_site, expected, "{file}");
        }
        // Saving clamps too: the file never holds an out-of-range value.
        let rendered = render(&Settings {
            workers_per_site: 40,
            default_policy: ConflictPolicy::Ask,
            ..Settings::default()
        });
        assert_eq!(parse(&rendered).unwrap().workers_per_site, 16);
    }

    /// The keepalive of SSH and FTP sessions: `10..=300` s, 30 by default, clamped on load
    /// and on save.
    #[test]
    fn keepalive_clamped() {
        assert_eq!(Settings::default().keepalive_secs, 30);
        assert_eq!(clamp_keepalive(0), 10);
        assert_eq!(clamp_keepalive(45), 45);
        assert_eq!(clamp_keepalive(301), 300);
        for (file, expected) in [
            ("", 30),
            ("keepalive_secs = 5", 10),
            ("keepalive_secs = -1", 10),
            ("keepalive_secs = 60", 60),
            ("keepalive_secs = 99999", 300),
        ] {
            assert_eq!(parse(file).unwrap().keepalive_secs, expected, "{file}");
        }
        let rendered = render(&Settings {
            keepalive_secs: 1,
            ..Settings::default()
        });
        assert!(rendered.contains("keepalive_secs = 10"), "{rendered}");
    }

    #[test]
    fn language_setting_loads_and_keeps_unknown() {
        use crate::i18n::Language;

        assert_eq!(Settings::default().language, LanguageChoice::System);
        assert_eq!(
            parse("language = \"pt-BR\"").unwrap().language,
            LanguageChoice::Fixed(Language::PtBr)
        );
        assert_eq!(
            parse("language = \"en\"").unwrap().language,
            LanguageChoice::Fixed(Language::En)
        );
        assert_eq!(
            parse("language = \"system\"").unwrap().language,
            LanguageChoice::System
        );
        assert_eq!(parse("").unwrap().language, LanguageChoice::System);
        assert_eq!(
            parse("language = \"klingon\"").unwrap().language,
            LanguageChoice::System
        );

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        // An unknown value stays in the file while the user changes nothing about it.
        std::fs::write(&path, "language = \"klingon\"\n").unwrap();
        let loaded = load(&path).settings;
        assert_eq!(loaded.language, LanguageChoice::System);
        save(&path, &loaded).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("language = \"klingon\""), "{text}");
        // Picking a language replaces it.
        let picked = Settings {
            language: LanguageChoice::Fixed(Language::PtBr),
            ..loaded
        };
        save(&path, &picked).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("language = \"pt-BR\""), "{text}");
        assert!(!text.contains("klingon"), "{text}");
        assert_eq!(load(&path).settings, picked);
        // A fresh file writes the default as `system`.
        assert!(render(&Settings::default()).contains("language = \"system\""));
    }

    #[test]
    fn google_drive_marker_roundtrip_and_invalid() {
        let defaults = Settings::default();
        assert!(!defaults.google_drive);
        assert!(!render(&defaults).contains("google_drive"));
        let on = Settings {
            google_drive: true,
            ..Settings::default()
        };
        assert!(render(&on).contains("google_drive = true"));
        assert_eq!(parse(&render(&on)).unwrap(), on);
        assert!(!parse("google_drive = \"yes\"").unwrap().google_drive);
        assert!(!parse("google_drive = 1").unwrap().google_drive);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, "future_key = 1\ngoogle_drive = true\n").unwrap();
        let loaded = load(&path).settings;
        assert!(loaded.google_drive);
        save(&path, &loaded).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("google_drive = true"), "{text}");
        save(
            &path,
            &Settings {
                google_drive: false,
                ..loaded
            },
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("google_drive"), "{text}");
        assert!(text.contains("future_key = 1"), "{text}");
    }

    #[test]
    fn agents_switch_roundtrip_and_invalid() {
        let defaults = Settings::default();
        assert!(!defaults.agents_enabled);
        assert!(!render(&defaults).contains("agents_enabled"));
        let on = Settings {
            agents_enabled: true,
            ..Settings::default()
        };
        assert!(render(&on).contains("agents_enabled = true"));
        assert_eq!(parse(&render(&on)).unwrap(), on);
        assert!(!parse("agents_enabled = \"yes\"").unwrap().agents_enabled);
        assert!(!parse("agents_enabled = 1").unwrap().agents_enabled);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(
            &path,
            "future_key = 1\ngoogle_drive = true\nagents_enabled = true\n",
        )
        .unwrap();
        let loaded = load(&path).settings;
        assert!(loaded.agents_enabled && loaded.google_drive);
        save(
            &path,
            &Settings {
                agents_enabled: false,
                ..loaded
            },
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("agents_enabled"), "{text}");
        assert!(text.contains("future_key = 1"), "{text}");
        assert!(text.contains("google_drive = true"), "{text}");
    }
}
