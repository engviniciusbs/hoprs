//! The export screen: one site, one workspace or everything, sealed into one encrypted
//! file. A centre form ([`Form::Export`]) whose state is [`App::export`], because the
//! result must land after the view is gone.
//!
//! Leaving the form (anything that shows something else in the centre) empties the
//! passwords and the questions, and drops the flow unless an export runs; a result that
//! arrives with the form gone drops the flow. The typed password leaves the form inside
//! the effect (a [`SecretString`]) and both fields are emptied.
//!
//! The second destination is Google Drive: the export is sealed in memory, then uploaded;
//! the same bytes serve "Try again" and "Save to this computer…", and go with the flow.
//! The connection ([`App::drive_link`]) lasts for the session (ruling 11): the keyring is
//! read the first time the destination is picked, never to draw a screen.

use std::fmt;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use hoprs_core::Error;
use hoprs_vault::{ExportReport, ExportScope};
use secrecy::SecretString;

use super::settings::{BackupState, path_of};
use super::{App, Effect, Form};
use crate::drive::{ConnectEvent, DriveAccount, DriveFile, Sealed};
use crate::terminal::Redacted;
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

/// What the app knows of the Google Drive connection (for the session).
#[derive(Clone, PartialEq, Eq)]
pub enum DriveLink {
    /// The keyring was not read (or could not be: it is asked again next time).
    Unknown,
    /// The keyring is being read.
    Reading,
    Disconnected,
    /// Connect runs: the browser is awaited. `url` (once the backend sent it) is the
    /// authorisation URL, for "Copy the link"; it holds the `state`, so the `Debug` of
    /// this type never shows it.
    Waiting {
        url: Option<String>,
    },
    Connected(DriveAccount),
}

/// Never shows the authorisation URL.
impl fmt::Debug for DriveLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => f.write_str("Unknown"),
            Self::Reading => f.write_str("Reading"),
            Self::Disconnected => f.write_str("Disconnected"),
            Self::Waiting { url: None } => f.write_str("Waiting { url: None }"),
            Self::Waiting { url: Some(_) } => f.write_str("Waiting { url: <redacted> }"),
            Self::Connected(account) => f.debug_tuple("Connected").field(account).finish(),
        }
    }
}

/// Where the export goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    File,
    Drive,
}

/// An export to Google Drive, step by step.
#[derive(Clone, Debug)]
pub enum DriveExport {
    Idle,
    Sealing,
    Uploading,
    Uploaded {
        report: ExportReport,
        file: DriveFile,
    },
    /// Sealing, the upload or the local save failed; after an upload or a save the bytes
    /// are still in [`ExportFlow::sealed`].
    Failed(Error),
    /// "Save to this computer…" writes the sealed bytes.
    Saving,
    Saved {
        report: ExportReport,
        path: PathBuf,
    },
}

impl DriveExport {
    fn running(&self) -> bool {
        matches!(self, Self::Sealing | Self::Uploading | Self::Saving)
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
    /// The file destination's export.
    pub state: BackupState<ExportReport>,
    pub destination: Destination,
    /// The file's name in Google Drive (starts as [`ExportFlow::file_name`]).
    pub drive_name: String,
    /// The sealed export and its report, from sealing until it is uploaded or saved
    /// (kept for "Try again" and "Save to this computer…"; its `Debug` is the length).
    pub sealed: Option<(ExportReport, Sealed)>,
    pub drive: DriveExport,
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
            drive_name: file_name.clone(),
            file_name,
            path,
            path_picked: false,
            checking: false,
            confirm_overwrite: false,
            password: Password::default(),
            confirm: Password::default(),
            field_error: None,
            state: BackupState::Idle,
            destination: Destination::File,
            sealed: None,
            drive: DriveExport::Idle,
        }
    }

    /// An export runs, to either destination (sealing, uploading or saving the bytes).
    pub fn running(&self) -> bool {
        self.state.running() || self.drive.running()
    }

    /// The sealed bytes can be uploaded again or saved: only on the Google Drive
    /// destination, while nothing runs (one export job at a time). Switching to File
    /// keeps them for a switch back; a new export, on either destination, drops them.
    pub fn has_bytes(&self) -> bool {
        !self.running()
            && self.destination == Destination::Drive
            && matches!(self.drive, DriveExport::Failed(_))
            && self.sealed.is_some()
    }

    /// The export of the current destination succeeded (file: the report; Google Drive:
    /// uploaded or saved): the primary button is Done, and Submit does nothing. A failed
    /// export is not finished: it can be submitted again.
    pub fn finished(&self) -> bool {
        match self.destination {
            Destination::File => matches!(self.state, BackupState::Done(_)),
            Destination::Drive => matches!(
                self.drive,
                DriveExport::Uploaded { .. } | DriveExport::Saved { .. }
            ),
        }
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
    /// The destination picked.
    Destination(Destination),
    /// The keyring's answer (see [`App::drive_link`]).
    DriveStatus(Result<Option<DriveAccount>, Error>),
    /// "Connect to Google Drive…".
    Connect,
    /// What the connect reports (its `Debug` never shows the URL).
    Connecting(ConnectEvent),
    CancelConnect,
    /// "Copy the link" while the browser is awaited.
    CopyLink,
    /// "Disconnect", from the form or from Settings.
    Disconnect,
    Disconnected(Result<(), Error>),
    DriveName(String),
    /// Sealing in memory finished.
    SealDone(Result<(ExportReport, Sealed), Error>),
    UploadDone(Result<DriveFile, Error>),
    /// Uploads the same bytes again.
    TryAgain,
    /// "Save to this computer…": the save dialog for the same bytes.
    SaveLocal,
    /// That save dialog closed: the path chosen, `None` if cancelled.
    SavePicked(Option<PathBuf>),
    SaveDone {
        path: PathBuf,
        result: Result<(), Error>,
    },
    /// "Open in Drive".
    OpenInDrive,
    /// The browser was (or could not be) opened.
    PageOpened(Result<(), Error>),
}

impl ExportMsg {
    /// Sent by the user's click or key (dropped behind a dialog or the palette, and
    /// while the window closes), not by work finishing.
    pub fn is_user(&self) -> bool {
        !matches!(
            self,
            ExportMsg::Picked(_)
                | ExportMsg::TargetChecked { .. }
                | ExportMsg::Done(_)
                | ExportMsg::DriveStatus(_)
                | ExportMsg::Connecting(_)
                | ExportMsg::Disconnected(_)
                | ExportMsg::SealDone(_)
                | ExportMsg::UploadDone(_)
                | ExportMsg::SavePicked(_)
                | ExportMsg::SaveDone { .. }
                | ExportMsg::PageOpened(_)
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
            // The connection outlives the form: its answers land wherever the user is.
            ExportMsg::DriveStatus(result) => return self.drive_status(result),
            ExportMsg::Connecting(event) => return self.drive_connecting(event),
            ExportMsg::Disconnect => return self.drive_disconnect(),
            ExportMsg::Disconnected(result) => return self.drive_disconnected(result),
            ExportMsg::PageOpened(result) => {
                if let Err(error) = result {
                    self.drive_error = Some(error);
                }
                return Vec::new();
            }
            ExportMsg::SealDone(result) => return self.seal_done(result),
            ExportMsg::UploadDone(result) => return self.upload_done(result),
            ExportMsg::SaveDone { path, result } => {
                self.sealed_saved(path, result);
                return Vec::new();
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
            ExportMsg::Open(_)
            | ExportMsg::Done(_)
            | ExportMsg::TargetChecked { .. }
            | ExportMsg::DriveStatus(_)
            | ExportMsg::Connecting(_)
            | ExportMsg::Disconnect
            | ExportMsg::Disconnected(_)
            | ExportMsg::PageOpened(_)
            | ExportMsg::SealDone(_)
            | ExportMsg::UploadDone(_)
            | ExportMsg::SaveDone { .. } => {}
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
            ExportMsg::Destination(destination) => return self.pick_destination(destination),
            ExportMsg::Connect => return self.drive_connect(),
            ExportMsg::CancelConnect => return self.cancel_connect(),
            ExportMsg::CopyLink => {
                if let DriveLink::Waiting { url: Some(url) } = &self.drive_link {
                    return vec![Effect::WriteClipboard(Redacted(url.clone()))];
                }
            }
            ExportMsg::DriveName(name) => {
                if !flow.running() {
                    flow.drive_name = name;
                }
            }
            ExportMsg::TryAgain => {
                if flow.has_bytes()
                    && matches!(self.drive_link, DriveLink::Connected(_))
                    && let Some((_, sealed)) = &flow.sealed
                {
                    let sealed = sealed.clone();
                    flow.drive = DriveExport::Uploading;
                    return vec![Effect::DriveUpload {
                        name: flow.drive_name.trim().to_owned(),
                        sealed,
                    }];
                }
            }
            ExportMsg::SaveLocal => {
                if flow.has_bytes() {
                    return vec![Effect::PickSealedTarget {
                        name: flow.drive_name.trim().to_owned(),
                    }];
                }
            }
            ExportMsg::SavePicked(None) => {}
            ExportMsg::SavePicked(Some(path)) => {
                if flow.has_bytes()
                    && let Some((_, sealed)) = &flow.sealed
                {
                    let sealed = sealed.clone();
                    flow.drive = DriveExport::Saving;
                    return vec![Effect::SaveSealed { path, sealed }];
                }
            }
            ExportMsg::OpenInDrive => {
                if let DriveExport::Uploaded {
                    file:
                        DriveFile {
                            link: Some(link), ..
                        },
                    ..
                } = &flow.drive
                {
                    return vec![Effect::OpenDrivePage(Redacted(link.clone()))];
                }
            }
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
        if !self.export_ready() || self.export.as_ref().is_some_and(ExportFlow::finished) {
            return Vec::new();
        }
        if self
            .export
            .as_ref()
            .is_some_and(|flow| flow.destination == Destination::Drive)
        {
            return self.submit_to_drive();
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
        if !flow.checking
            || flow.destination != Destination::File
            || path_of(&flow.path).ok() != Some(path)
        {
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
        // A new export: the bytes of a failed upload can no longer stand in for it.
        flow.sealed = None;
        flow.drive = DriveExport::Idle;
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
    /// passwords and questions; any other goes (and its sealed bytes with it). A connect
    /// that waits for the browser is cancelled.
    pub(super) fn export_left(&mut self) -> Vec<Effect> {
        if self.export_shown() {
            return Vec::new();
        }
        match self.export.as_mut() {
            Some(flow) if flow.running() => flow.forget_passwords(),
            Some(_) => self.export = None,
            None => {}
        }
        if matches!(self.drive_link, DriveLink::Waiting { .. }) {
            self.drive_link = DriveLink::Disconnected;
            return vec![Effect::DriveCancel];
        }
        Vec::new()
    }

    /// The marker in `settings.toml` (Settings draws its line from it), saved if changed.
    fn set_drive_marker(&mut self, on: bool) -> Vec<Effect> {
        if self.settings.google_drive == on {
            return Vec::new();
        }
        self.settings.google_drive = on;
        self.save_settings_effect().into_iter().collect()
    }

    /// File or Google Drive; not while an export runs, Google Drive only with an OAuth
    /// client in the build. Picking Google Drive is the only place a form reads the
    /// keyring: the first time, or again after the keyring could not be read.
    fn pick_destination(&mut self, destination: Destination) -> Vec<Effect> {
        if destination == Destination::Drive && !self.drive_available {
            return Vec::new();
        }
        let Some(flow) = self.export.as_mut() else {
            return Vec::new();
        };
        if flow.running() {
            return Vec::new();
        }
        flow.destination = destination;
        flow.field_error = None;
        // A check or question about the file has no meaning any more.
        flow.checking = false;
        flow.confirm_overwrite = false;
        let unread = match self.drive_link {
            DriveLink::Unknown => true,
            DriveLink::Disconnected => self.drive_unreadable,
            _ => false,
        };
        if destination == Destination::Drive && unread {
            self.drive_link = DriveLink::Reading;
            self.drive_error = None;
            return vec![Effect::DriveStatus];
        }
        Vec::new()
    }

    /// The keyring's answer. Absent: the marker goes. Unreadable (locked, its unlock
    /// refused): the error shows, the marker stays, and the next pick asks again.
    fn drive_status(&mut self, result: Result<Option<DriveAccount>, Error>) -> Vec<Effect> {
        if self.drive_link != DriveLink::Reading {
            return Vec::new();
        }
        self.drive_unreadable = false;
        match result {
            Ok(Some(account)) => {
                self.drive_link = DriveLink::Connected(account);
                self.set_drive_marker(true)
            }
            Ok(None) => {
                self.drive_link = DriveLink::Disconnected;
                self.set_drive_marker(false)
            }
            Err(error) => {
                // Shown as not connected (Connect still works, for this session if the
                // keyring stays out of reach); the next pick asks the keyring again.
                self.drive_link = DriveLink::Disconnected;
                self.drive_unreadable = true;
                self.drive_error = Some(error);
                Vec::new()
            }
        }
    }

    /// "Connect to Google Drive…": only when not connected and not already waiting.
    fn drive_connect(&mut self) -> Vec<Effect> {
        if !self.drive_available
            || self.drive_link != DriveLink::Disconnected
            || self
                .export
                .as_ref()
                .is_none_or(|flow| flow.destination != Destination::Drive)
        {
            return Vec::new();
        }
        self.drive_link = DriveLink::Waiting { url: None };
        self.drive_error = None;
        vec![Effect::DriveConnect]
    }

    fn drive_connecting(&mut self, event: ConnectEvent) -> Vec<Effect> {
        match event {
            ConnectEvent::Waiting { url } => {
                if self.drive_link == (DriveLink::Waiting { url: None }) {
                    self.drive_link = DriveLink::Waiting {
                        url: Some(url.clone()),
                    };
                    return vec![Effect::OpenDrivePage(Redacted(url))];
                }
                // Cancelled (or the form left) before the backend's wait began, when the
                // cancel could not reach it: cancelled again now that it can.
                vec![Effect::DriveCancel]
            }
            // A success that raced a cancel is kept.
            ConnectEvent::Done(Ok(account)) => {
                self.drive_link = DriveLink::Connected(account);
                self.drive_unreadable = false;
                self.drive_error = None;
                self.set_drive_marker(true)
            }
            ConnectEvent::Done(Err(error)) => {
                if matches!(self.drive_link, DriveLink::Waiting { .. }) {
                    self.drive_link = DriveLink::Disconnected;
                    if error.code != Some("drive-connect-cancelled") {
                        self.drive_error = Some(error);
                    }
                }
                Vec::new()
            }
        }
    }

    /// Cancel while the browser is awaited. A cancel sent before the backend's wait began
    /// is lost there: the `Waiting` that follows is then cancelled at once
    /// ([`App::drive_connecting`]).
    fn cancel_connect(&mut self) -> Vec<Effect> {
        if !matches!(self.drive_link, DriveLink::Waiting { .. }) {
            return Vec::new();
        }
        self.drive_link = DriveLink::Disconnected;
        vec![Effect::DriveCancel]
    }

    /// "Disconnect" (from the form or Settings): not in a build without Google Drive, nor
    /// while an export runs or the browser is awaited.
    fn drive_disconnect(&mut self) -> Vec<Effect> {
        if !self.drive_available
            || self.export_running()
            || matches!(self.drive_link, DriveLink::Waiting { .. })
        {
            return Vec::new();
        }
        vec![Effect::DriveDisconnect]
    }

    /// Disconnected: whatever the outcome, the connection is gone here (a failure to
    /// delete the entry shows).
    fn drive_disconnected(&mut self, result: Result<(), Error>) -> Vec<Effect> {
        if !matches!(self.drive_link, DriveLink::Waiting { .. }) {
            self.drive_link = DriveLink::Disconnected;
        }
        // A failure may have left the entry there: the next pick reads it again.
        self.drive_unreadable = result.is_err();
        self.drive_error = result.err();
        self.set_drive_marker(false)
    }

    /// Export to Google Drive: connected, the name, then the passwords as for a file;
    /// the password leaves the form with the effect.
    fn submit_to_drive(&mut self) -> Vec<Effect> {
        if !matches!(self.drive_link, DriveLink::Connected(_)) {
            return Vec::new();
        }
        let Some(flow) = self.export.as_mut() else {
            return Vec::new();
        };
        if flow.drive_name.trim().is_empty() {
            flow.field_error = Some(strings::drive_name_required());
            return Vec::new();
        }
        if flow.password.0.is_empty() {
            flow.field_error = Some(strings::backup_password_required());
            return Vec::new();
        }
        if flow.password != flow.confirm {
            flow.field_error = Some(strings::backup_passwords_differ());
            return Vec::new();
        }
        flow.field_error = None;
        // From here on only the effect holds the password.
        let password = SecretString::from(std::mem::take(&mut flow.password.0));
        flow.confirm = Password::default();
        flow.sealed = None;
        flow.drive = DriveExport::Sealing;
        vec![Effect::SealExport {
            scope: flow.scope.clone(),
            password,
        }]
    }

    /// The flow a Drive step's result lands in: `None` when that step does not run, and
    /// the flow is dropped (its bytes with it) when the form was left meanwhile.
    fn drive_step(&mut self, step: fn(&DriveExport) -> bool) -> Option<&mut ExportFlow> {
        if !self.export.as_ref().is_some_and(|flow| step(&flow.drive)) {
            return None;
        }
        if !self.export_shown() {
            self.export = None;
            return None;
        }
        self.export.as_mut()
    }

    fn seal_done(&mut self, result: Result<(ExportReport, Sealed), Error>) -> Vec<Effect> {
        let Some(flow) = self.drive_step(|drive| matches!(drive, DriveExport::Sealing)) else {
            return Vec::new();
        };
        match result {
            Ok((report, sealed)) => {
                flow.sealed = Some((report, sealed.clone()));
                flow.drive = DriveExport::Uploading;
                vec![Effect::DriveUpload {
                    name: flow.drive_name.trim().to_owned(),
                    sealed,
                }]
            }
            Err(error) => {
                flow.drive = DriveExport::Failed(error);
                Vec::new()
            }
        }
    }

    fn upload_done(&mut self, result: Result<DriveFile, Error>) -> Vec<Effect> {
        if !self
            .export
            .as_ref()
            .is_some_and(|flow| matches!(flow.drive, DriveExport::Uploading))
        {
            return Vec::new();
        }
        // Google refused the grant (revoked from the account page): Connect again.
        let mut effects = Vec::new();
        if let Err(error) = &result
            && error.code == Some("drive-not-connected")
        {
            self.drive_link = DriveLink::Disconnected;
            effects = self.set_drive_marker(false);
        }
        let Some(flow) = self.drive_step(|drive| matches!(drive, DriveExport::Uploading)) else {
            return effects;
        };
        match result {
            Ok(file) => {
                flow.drive = match flow.sealed.take() {
                    Some((report, _)) => DriveExport::Uploaded { report, file },
                    None => DriveExport::Idle,
                };
            }
            Err(error) => flow.drive = DriveExport::Failed(error),
        }
        effects
    }

    fn sealed_saved(&mut self, path: PathBuf, result: Result<(), Error>) {
        let Some(flow) = self.drive_step(|drive| matches!(drive, DriveExport::Saving)) else {
            return;
        };
        match result {
            Ok(()) => {
                flow.drive = match flow.sealed.take() {
                    Some((report, _)) => DriveExport::Saved { report, path },
                    None => DriveExport::Idle,
                };
            }
            Err(error) => flow.drive = DriveExport::Failed(error),
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
