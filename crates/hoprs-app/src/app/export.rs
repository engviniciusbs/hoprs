//! The export screen: one site, one workspace or everything, sealed into one encrypted
//! file. A centre form ([`Form::Export`]) whose state is [`App::export`], because the
//! result must land after the view is gone.
//!
//! Leaving the form (anything that shows something else in the centre) empties the
//! passwords and the questions, and drops the flow unless an export runs; a result that
//! arrives with the form gone drops the flow. The typed password leaves the form inside
//! the effect (a [`SecretString`]) and both fields are emptied.

use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use hoprs_core::Error;
use hoprs_vault::{ExportReport, ExportScope};
use secrecy::SecretString;

use super::settings::{BackupState, path_of};
use super::{App, Effect, Form};
use crate::ui::site_form::Password;
use crate::ui::strings;

/// The longest slug in a default file name, in characters.
const SLUG_MAX: usize = 40;

/// A name made safe for a file name: lowercase, every run of characters that are not
/// Unicode alphanumerics becomes one `-`, no `-` at either end, at most 40 characters;
/// `export` when nothing is left.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(c);
        } else {
            pending_dash = true;
        }
    }
    let mut out: String = out.chars().take(SLUG_MAX).collect();
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() { "export".into() } else { out }
}

/// `hoprs-<slug>-<YYYY-MM-DD>.hoprs`, or `hoprs-backup-<date>.hoprs` without a name.
pub fn default_file_name(name: Option<&str>, today: NaiveDate) -> String {
    let what = name.map_or_else(|| "backup".to_owned(), slug);
    format!("hoprs-{what}-{}.hoprs", today.format("%Y-%m-%d"))
}

/// What the export screen says it exports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportTitle {
    Site { name: String },
    Workspace { name: String, sites: usize },
    All { sites: usize, workspaces: usize },
}

impl ExportTitle {
    /// The site or workspace name; `None` for everything.
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Site { name } | Self::Workspace { name, .. } => Some(name),
            Self::All { .. } => None,
        }
    }
}

impl App {
    /// The title for `scope`; `None` when the site or workspace is not in the profiles.
    pub fn export_title(&self, scope: &ExportScope) -> Option<ExportTitle> {
        match scope {
            ExportScope::Site(id) => self.site(id).map(|site| ExportTitle::Site {
                name: site.name.clone(),
            }),
            ExportScope::Workspace(id) => {
                self.workspace(id).map(|workspace| ExportTitle::Workspace {
                    name: workspace.name.clone(),
                    sites: workspace
                        .sites
                        .iter()
                        .filter(|entry| self.site(&entry.site).is_some())
                        .count(),
                })
            }
            ExportScope::All => Some(ExportTitle::All {
                sites: self.sites.len(),
                workspaces: self.workspaces.len(),
            }),
        }
    }
}

/// The export screen's state.
#[derive(Debug)]
pub struct ExportFlow {
    pub scope: ExportScope,
    pub title: ExportTitle,
    /// The default file name (what the save dialog proposes).
    pub file_name: String,
    /// The file, as typed or chosen (shown in mono).
    pub path: String,
    /// The path came from the save dialog (which already asked about overwriting).
    pub path_picked: bool,
    /// Whether the typed file exists is being checked.
    pub checking: bool,
    /// The typed file exists: "Overwrite?" waits for its answer.
    pub confirm_overwrite: bool,
    pub password: Password,
    pub confirm: Password,
    /// What is wrong with the fields.
    pub field_error: Option<&'static str>,
    pub state: BackupState<ExportReport>,
}

impl ExportFlow {
    /// A new form for `scope`: the File field starts as `dir/<default name>`, empty
    /// without a folder.
    pub fn open(
        scope: ExportScope,
        title: ExportTitle,
        dir: Option<&Path>,
        today: NaiveDate,
    ) -> ExportFlow {
        let file_name = default_file_name(title.name(), today);
        let path = dir
            .map(|dir| dir.join(&file_name).to_string_lossy().into_owned())
            .unwrap_or_default();
        ExportFlow {
            scope,
            title,
            file_name,
            path,
            path_picked: false,
            checking: false,
            confirm_overwrite: false,
            password: Password::default(),
            confirm: Password::default(),
            field_error: None,
            state: BackupState::Idle,
        }
    }

    pub fn running(&self) -> bool {
        self.state.running()
    }

    /// Empties the passwords and drops the questions that would send them.
    fn forget_passwords(&mut self) {
        self.password = Password::default();
        self.confirm = Password::default();
        self.checking = false;
        self.confirm_overwrite = false;
    }
}

#[derive(Debug, Clone)]
pub enum ExportMsg {
    /// Shows the form for `scope` (a running export is shown instead, unchanged).
    Open(ExportScope),
    Path(String),
    /// "Choose…": the system save dialog.
    Choose,
    /// The save dialog closed: the path chosen, `None` if cancelled.
    Picked(Option<PathBuf>),
    /// A typed password: its `Debug` never shows it.
    Password(Password),
    Confirm(Password),
    Submit,
    /// Whether the typed file exists.
    TargetChecked {
        path: PathBuf,
        exists: bool,
    },
    /// "Overwrite" on the inline question about an existing file.
    ConfirmOverwrite,
    CancelOverwrite,
    /// The export finished.
    Done(Result<ExportReport, Error>),
    Close,
}

impl ExportMsg {
    /// Sent by the user's click or key (dropped behind a dialog or the palette, and
    /// while the window closes), not by work finishing.
    pub fn is_user(&self) -> bool {
        !matches!(
            self,
            ExportMsg::Picked(_) | ExportMsg::TargetChecked { .. } | ExportMsg::Done(_)
        )
    }
}

impl App {
    /// An export runs.
    pub(super) fn export_running(&self) -> bool {
        self.export.as_ref().is_some_and(ExportFlow::running)
    }

    /// Whether Export can be pressed: a form, nothing running in it, no check or question
    /// pending, and no restore running (both lock the vault; one at a time).
    pub fn export_ready(&self) -> bool {
        self.export
            .as_ref()
            .is_some_and(|flow| !flow.running() && !flow.checking && !flow.confirm_overwrite)
            && !self.backup.restore.running()
    }

    fn export_shown(&self) -> bool {
        matches!(self.form, Some(Form::Export))
    }

    pub(super) fn export_msg(&mut self, msg: ExportMsg) -> Vec<Effect> {
        match msg {
            ExportMsg::Open(scope) => return self.open_export(scope),
            ExportMsg::Done(result) => {
                self.export_done(result);
                return Vec::new();
            }
            ExportMsg::TargetChecked { path, exists } => {
                return self.export_target_checked(path, exists);
            }
            _ => {}
        }
        // The rest is the form's own input (or the save dialog it opened).
        if !self.export_shown() {
            return Vec::new();
        }
        let Some(flow) = self.export.as_mut() else {
            return Vec::new();
        };
        match msg {
            ExportMsg::Open(_) | ExportMsg::Done(_) | ExportMsg::TargetChecked { .. } => {}
            ExportMsg::Path(path) => {
                flow.path = path;
                flow.path_picked = false;
                flow.checking = false;
                flow.confirm_overwrite = false;
            }
            ExportMsg::Choose => {
                return vec![Effect::PickExportFile {
                    name: flow.file_name.clone(),
                }];
            }
            ExportMsg::Picked(Some(path)) => {
                flow.path = path.to_string_lossy().into_owned();
                // The save dialog asked about an existing file already.
                flow.path_picked = true;
                flow.checking = false;
                flow.confirm_overwrite = false;
            }
            ExportMsg::Picked(None) => {}
            ExportMsg::Password(password) => flow.password = password,
            ExportMsg::Confirm(password) => flow.confirm = password,
            ExportMsg::Submit => return self.submit_export(),
            ExportMsg::ConfirmOverwrite => {
                if flow.confirm_overwrite {
                    flow.confirm_overwrite = false;
                    return self.start_export();
                }
            }
            ExportMsg::CancelOverwrite => flow.confirm_overwrite = false,
            ExportMsg::Close => self.form = None,
        }
        Vec::new()
    }

    /// Shows the form for `scope`: nothing for a site or workspace that is not in the
    /// profiles; a running export is shown as it is; else a new form.
    fn open_export(&mut self, scope: ExportScope) -> Vec<Effect> {
        let Some(title) = self.export_title(&scope) else {
            return Vec::new();
        };
        if !self.export_running() {
            self.export = Some(ExportFlow::open(
                scope,
                title,
                self.export_dir.as_deref(),
                chrono::Local::now().date_naive(),
            ));
        }
        self.form = Some(Form::Export);
        Vec::new()
    }

    /// Export: the fields checked in order (file, password, the same password twice);
    /// a chosen file starts at once, a typed one is checked for an existing file first.
    fn submit_export(&mut self) -> Vec<Effect> {
        if !self.export_ready() {
            return Vec::new();
        }
        let Some(flow) = self.export.as_mut() else {
            return Vec::new();
        };
        let path = match path_of(&flow.path) {
            Ok(path) => path,
            Err(why) => {
                flow.field_error = Some(why);
                return Vec::new();
            }
        };
        if flow.password.0.is_empty() {
            flow.field_error = Some(strings::backup_password_required());
            return Vec::new();
        }
        if flow.password != flow.confirm {
            flow.field_error = Some(strings::backup_passwords_differ());
            return Vec::new();
        }
        flow.field_error = None;
        if flow.path_picked {
            return self.start_export();
        }
        // A typed path: an existing file is overwritten only once confirmed.
        flow.checking = true;
        vec![Effect::CheckExportTarget(path)]
    }

    /// Whether the typed file exists came back.
    fn export_target_checked(&mut self, path: PathBuf, exists: bool) -> Vec<Effect> {
        let Some(flow) = self.export.as_mut() else {
            return Vec::new();
        };
        // A path edited meanwhile (or the form left) drops this answer.
        if !flow.checking || path_of(&flow.path).ok() != Some(path) {
            return Vec::new();
        }
        flow.checking = false;
        if exists {
            flow.confirm_overwrite = true;
            return Vec::new();
        }
        self.start_export()
    }

    /// Sends the export with the typed password, which leaves the form.
    fn start_export(&mut self) -> Vec<Effect> {
        let restoring = self.backup.restore.running();
        let Some(flow) = self.export.as_mut() else {
            return Vec::new();
        };
        let Ok(path) = path_of(&flow.path) else {
            return Vec::new();
        };
        if restoring
            || flow.running()
            || flow.password.0.is_empty()
            || flow.password != flow.confirm
        {
            return Vec::new();
        }
        // From here on only the effect holds the password.
        let password = SecretString::from(std::mem::take(&mut flow.password.0));
        flow.confirm = Password::default();
        flow.state = BackupState::Running;
        vec![Effect::ExportBackup {
            scope: flow.scope.clone(),
            path,
            password,
        }]
    }

    /// The export finished: its outcome shows in the form, or the flow goes if the form
    /// was left meanwhile.
    fn export_done(&mut self, result: Result<ExportReport, Error>) {
        if !self.export_running() {
            return;
        }
        if !self.export_shown() {
            self.export = None;
            return;
        }
        if let Some(flow) = self.export.as_mut() {
            flow.state = match result {
                Ok(report) => BackupState::Done(report),
                Err(error) => BackupState::Failed(error),
            };
        }
    }

    /// The centre shows something else: a running export keeps its flow without the
    /// passwords and questions; any other goes.
    pub(super) fn export_left(&mut self) {
        if self.export_shown() {
            return;
        }
        match self.export.as_mut() {
            Some(flow) if flow.running() => flow.forget_passwords(),
            Some(_) => self.export = None,
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn slug_rules() {
        assert_eq!(slug("Cliente Acme"), "cliente-acme");
        assert_eq!(slug("São Paulo / prod"), "são-paulo-prod");
        assert_eq!(slug("a/b\\c:d"), "a-b-c-d");
        for empty in ["  --  ", "///", "🙂", ""] {
            assert_eq!(slug(empty), "export", "{empty:?}");
        }
        assert_eq!(slug(&"a".repeat(200)), "a".repeat(40));
        let cut = slug(&format!("{} x", "a".repeat(39)));
        assert_eq!(cut, "a".repeat(39));
        assert!(!cut.ends_with('-'));
    }

    #[test]
    fn default_file_names() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 22).unwrap();
        assert_eq!(
            default_file_name(Some("Cliente Acme"), day),
            "hoprs-cliente-acme-2026-11-22.hoprs"
        );
        assert_eq!(
            default_file_name(None, day),
            "hoprs-backup-2026-11-22.hoprs"
        );
    }
}
