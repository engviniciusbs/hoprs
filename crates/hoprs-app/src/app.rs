//! The application state and its pure update: no I/O here, only state
//! changes and the [`Effect`]s for `effects::run` to perform.

use crate::terminal::Redacted;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use hoprs_core::{
    ConnectionState, Error, RemoteEntry, RemotePath, Site, SiteId, TabKind, Workspace, WorkspaceId,
};
use hoprs_import::ImportReport;
use hoprs_transfer::{ConflictPolicy, Event, TransferId, TransferSpec};
use hoprs_vault::{BackupImportReport, ExportScope, ImportMode, UnreadableEntry};
use secrecy::SecretString;

use crate::backend::{
    AuthMethod, BrokerEvent, ConnectResult, ControlOp, KnownHostEntry, ProfileSnapshot,
    PromptAnswer, PromptId, SessionId, ShellId, Side, TransferSnapshot, WorkspaceTabs,
};
use crate::settings::{Settings, SettingsLoad};
use crate::ui::sidebar;
use crate::ui::site_form::{PasswordChange, SiteForm, SiteFormMsg, SiteFormOutcome};
use crate::ui::workspace_form::{WorkspaceForm, WorkspaceFormMsg, WorkspaceFormOutcome};

/// Where the saved sites are.
#[derive(Debug, Clone, PartialEq)]
pub enum Load {
    Loading,
    Loaded,
    Failed(Error),
}

/// The form shown in the center, in place of the tabs.
#[derive(Debug)]
// At most one form exists, so its size doesn't matter.
#[allow(clippy::large_enum_variant)]
pub enum Form {
    Site(SiteForm),
    Workspace(WorkspaceForm),
    /// "Import from FileZilla" ([`App::import`] holds its state, which outlives the view
    /// while an import runs).
    Import,
    /// The export screen ([`App::export`] holds its state, which outlives the view while
    /// an export runs).
    Export,
    /// The question about unsent remote edits before the window or a workspace closes
    /// ([`App::center_ask`]).
    QuitEdits,
}

/// What a system file dialog is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickTarget {
    /// The `sitemanager.xml` to import (open).
    Sitemanager,
    /// The backup to restore (open).
    RestoreBackup,
}

pub mod actions;
pub mod compare;
pub mod dialogs;
pub mod edit;
pub mod export;
pub mod files;
pub mod import;
pub mod lifecycle;
pub mod log;
pub mod menu;
pub mod palette;
pub mod queue;
pub mod reconnect;
pub mod settings;
pub mod tabs;
pub mod terminal;
pub mod transfers;
pub mod update;

pub use actions::FileAction;
pub use compare::{CompareItem, CompareMsg, CompareRunId};
pub use dialogs::{Dialog, DialogMsg, Dialogs, FailureAction, KnownHostsFix, KnownHostsMsg, Typed};
pub use edit::EditMsg;
pub use export::ExportMsg;
pub use files::{FileKey, Mods, PaneMsg, RequestId};
pub use lifecycle::ConnStatus;
pub use log::{LogLevel, LogMsg};
pub use menu::{ContextMenu, MenuMsg, MenuTarget, TabRef};
pub use palette::PaletteMsg;
pub use queue::{Queue, QueueMsg, QueueTab, RowAction};
pub use settings::{SettingsTab, SidebarMsg, SidebarSection};
pub use tabs::{SiteConn, Tab, TabId, TabState};
pub use terminal::TerminalMsg;
pub use update::UpdateMsg;

/// A workspace's entry as the sidebar lists it.
#[derive(Debug, PartialEq)]
pub enum WorkspaceRow<'a> {
    Site(&'a Site),
    /// The site is no longer in the vault (or this version can't read it): shown as
    /// "Site missing" and skipped by "Open workspace".
    Missing(SiteId),
}

/// A panel whose height the user drags ([`crate::ui::divider`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Panel {
    Compare,
    Queue,
}

/// How far (logical px) the pointer must move from a press on a row before a drag
/// between panes starts; up to it the press stays a click.
pub const DRAG_THRESHOLD: f32 = 4.0;

/// A press on a row of pane `from` that may become a drag to the other pane: `origin` is
/// where the press was, `at` where the pointer is now; `active` once it moved more than
/// [`DRAG_THRESHOLD`] (and from then on, until the button is up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drag {
    pub tab: TabId,
    pub from: Side,
    pub origin: iced::Point,
    pub at: iced::Point,
    pub active: bool,
}

/// A divider drag. The anchor (`start_y`, `start_height`) is where the press was and how
/// tall the panel was drawn then; it moves to the pointer whenever the height hits a
/// bound, so the panel follows the pointer at once when it turns back.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Resize {
    pub panel: Panel,
    pub start_y: f32,
    pub start_height: f32,
    /// The height the panel was drawn at when pressed (fixed for the whole drag).
    pub drawn: f32,
    /// The most the panel can be drawn at in the window now.
    pub cap: f32,
    /// The panel's setting when pressed (`None`: the compare panel's default).
    pub saved_at_press: Option<f32>,
}

/// Something on a panel's divider.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum DividerMsg {
    /// A press: a resize starts from `height`, the height the panel was drawn at, up to
    /// `cap`, the most the window lets it have now.
    Press { panel: Panel, height: f32, cap: f32 },
    /// A double-click: the panel's default height.
    Reset(Panel),
}

/// The whole UI state.
#[derive(Debug)]
pub struct App {
    /// Whether the transfer queue is expanded.
    pub queue_open: bool,
    /// The technical log panel.
    pub log: log::LogPanel,
    /// The Ctrl+K palette, while it is open ([`palette`]).
    pub palette: Option<palette::Palette>,
    /// The sidebar's site filter, as typed.
    pub site_filter: String,
    pub load: Load,
    /// Saved sites and workspaces, in file order.
    pub sites: Vec<Site>,
    pub workspaces: Vec<Workspace>,
    /// Entries `sites.toml` / `workspaces.toml` hold but this version can't read.
    pub unreadable: Vec<UnreadableEntry>,
    /// Workspaces expanded in the sidebar.
    pub expanded: HashSet<WorkspaceId>,
    pub form: Option<Form>,
    /// The session tabs, in strip order: a site's tabs (terminal or files) share its
    /// session ([`tabs`]).
    pub tabs: Vec<Tab>,
    /// The connection of each site with tabs, shared by those tabs ([`tabs`]).
    pub conns: HashMap<SiteId, SiteConn>,
    pub active_tab: Option<usize>,
    /// The settings tab, drawn after the session tabs while it is open ([`settings`]).
    /// Selected (`active`) only while `active_tab` is `None`.
    pub settings_tab: Option<SettingsTab>,
    /// A sidebar action failed (opening the profile folder).
    pub sidebar_error: Option<Error>,
    /// Where a local pane starts when its site names no folder.
    pub home: Option<RemotePath>,
    /// The modifiers held now, for clicks in the file lists.
    pub mods: Mods,
    /// The transfer queue panel.
    pub queue: Queue,
    /// A press on a row started a possible drag from its pane (until the button is up).
    pub drag: Option<Drag>,
    /// The pane under the pointer.
    pub pointer: Option<(TabId, Side)>,
    /// Files from the system file manager hover the window.
    pub os_drag: bool,
    /// How many files the system said hover the window (one `FileHovered` each; 0 while
    /// it has not said yet).
    pub os_drag_count: usize,
    /// The security and credential dialogs: the one shown and those waiting.
    pub dialogs: Dialogs,
    /// The workspace "Open workspace" opened last, until it is closed: its tabs and
    /// folders are saved when it closes or the window does.
    pub open_workspace: Option<WorkspaceId>,
    /// The window asked to close: the backend is shutting down.
    pub closing: bool,
    /// FileZilla's usual `sitemanager.xml`, where the import starts.
    pub filezilla_default: Option<PathBuf>,
    pub import: import::ImportFlow,
    /// The settings in force (the engine and the sessions follow them).
    pub settings: Settings,
    /// `settings.toml` was unusable at start: the defaults apply and nothing is saved.
    pub settings_load_error: Option<Error>,
    /// The last save of `settings.toml` failed.
    pub settings_save_error: Option<Error>,
    /// A save of `settings.toml` runs.
    settings_saving: bool,
    /// The settings changed while a save ran: saved again when it ends.
    settings_dirty: bool,
    pub backup: settings::Backup,
    /// The export screen, while it is shown or an export runs ([`export`]).
    pub export: Option<export::ExportFlow>,
    /// Where the export screen's File field starts: Documents, else home.
    pub export_dir: Option<PathBuf>,
    /// This build has an OAuth client: the export screen offers Google Drive.
    pub drive_available: bool,
    /// The Google Drive connection, as known for this session ([`export`]).
    pub drive_link: export::DriveLink,
    /// The last Google Drive step that failed outside an export (reading the keyring,
    /// connecting, opening the browser, disconnecting).
    pub drive_error: Option<Error>,
    /// The keyring could not be read (or the entry deleted): the next pick of the Google
    /// Drive destination reads it again.
    drive_unreadable: bool,
    /// Advances while something runs without a known end (import, backup): drives the
    /// indeterminate progress bar.
    pub busy_phase: u32,
    /// The window's width, as the last resize reported it: what fits in the file panes,
    /// toolbars and queue head ([`crate::ui::fit`]).
    pub window_width: f32,
    /// The window's height, as the last resize reported it (where a menu fits).
    pub window_height: f32,
    /// Where the last left or right press in the window was, in logical pixels (the
    /// root [`crate::ui::pointer`] widget publishes it before the press reaches anything
    /// else): where a menu opens.
    pub last_press: iced::Point,
    /// A divider being dragged: the panel it resizes, until the button is up.
    pub resize: Option<Resize>,
    /// The context menu, while one is open ([`menu`]).
    pub menu: Option<ContextMenu>,
    /// The remote files being edited, in the order they started ([`edit`]).
    pub edits: Vec<edit::RemoteEdit>,
    /// Edits whose download and editor are on their way.
    pub edit_opening: Vec<edit::Opening>,
    /// The notice at the bottom right (an upload, kept copies).
    pub toast: Option<edit::Toast>,
    /// Last tabs of a session whose close waits for the answer about its edits.
    pub close_asks: HashMap<TabId, edit::CloseAsk>,
    /// The window's (or a workspace's) close waits for the answer about unsent edits.
    pub center_ask: Option<edit::CenterAsk>,
    /// A site or workspace form a close question stepped in front of (back after it).
    pub parked_forms: Vec<Form>,
    /// The window asked to close while a workspace question was busy: asked after it.
    pub queued_quit: Option<iced::window::Id>,
    /// Unsent edits kept as recovered copies by a close, until dismissed.
    pub kept_notice: Option<edit::KeptNotice>,
    /// The editor command as typed on the settings page, while it is not valid.
    pub editor_input: Option<String>,
    /// The compare ignore field as typed, while the settings page is open.
    pub compare_ignore_input: Option<String>,
    /// The update check, the card and the download ([`update`]).
    pub updater: update::Updater,
    /// Actions waiting for their site's lost session to reconnect ([`reconnect`]).
    held: HashMap<SiteId, Vec<Effect>>,
    /// Listings sent again after a reconnection, which the reconnect's result does not
    /// repeat ([`reconnect`]).
    replayed: HashSet<files::RequestId>,
    /// The re-list delay after finished transfers is running ([`transfers`]).
    relist_armed: bool,
    /// The last tab or listing id handed out.
    next_id: u64,
    /// The last profile read asked for, and the one whose result was applied last.
    profiles_requested: RequestId,
    profiles_applied: RequestId,
}

#[derive(Debug, Clone)]
pub enum Message {
    /// Collapse or expand the transfer queue.
    ToggleQueue,
    /// Something in the technical log panel, or its poll.
    Log(LogMsg),
    /// A folder comparison's progress or end ([`compare`]).
    Compare(CompareMsg),
    /// The Ctrl+K palette: open, typed into, run or closed ([`palette`]).
    Palette(PaletteMsg),
    /// The sidebar filter changed.
    FilterSites(String),
    ReloadProfiles,
    /// The profiles read for `request`; dropped if older than the read applied last.
    ProfilesLoaded {
        request: RequestId,
        result: Result<ProfileSnapshot, Error>,
    },
    ToggleWorkspace(WorkspaceId),
    OpenWorkspace(WorkspaceId),
    /// A site was clicked: open (or retry) its tabs, as its "Open as" says.
    ConnectSite(SiteId),
    /// One site's connect finished (each site of a batch on its own).
    SiteConnected {
        site: SiteId,
        result: ConnectResult,
    },
    SelectTab(usize),
    NewSite,
    EditSite(SiteId),
    NewWorkspace,
    EditWorkspace(WorkspaceId),
    SiteForm(SiteFormMsg),
    WorkspaceForm(WorkspaceFormMsg),
    /// A form's save or delete finished. `op` says which; a result whose form is no
    /// longer the one open still updates the profiles and tabs, never the open form.
    FormDone {
        op: ProfileOp,
        result: Result<(), Error>,
    },
    OpenProfileFolder,
    ProfileFolderOpened(Result<(), Error>),
    /// The × of a tab: closes it, and its session if no other tab uses it.
    CloseTab(TabId),
    /// A tab's menu, "Close other tabs": every other tab closes as its × would, and the
    /// kept one is selected.
    CloseOtherTabs(TabRef),
    /// "New terminal tab" / "New files tab" on a connected SSH tab: another tab of
    /// `kind` on the same session.
    NewTab {
        tab: TabId,
        kind: TabKind,
    },
    /// Something in one pane of a file tab.
    Pane {
        tab: TabId,
        side: Side,
        msg: PaneMsg,
    },
    /// A listing finished; dropped unless `request_id` is the pane's latest request.
    Listed {
        tab: TabId,
        side: Side,
        request_id: RequestId,
        result: Result<Vec<RemoteEntry>, Error>,
    },
    /// A key from the keyboard subscription. Unless it is Escape, it reaches the panes
    /// (as [`Message::Key`]) only if no text field has the focus.
    KeyPressed {
        key: FileKey,
        shift: bool,
    },
    /// A key for the focused pane: no widget took it and no text field has the focus.
    Key {
        key: FileKey,
        shift: bool,
    },
    Modifiers(Mods),
    /// A delete's running count of removed items.
    FileOpProgress {
        tab: TabId,
        side: Side,
        op: RequestId,
        done: u64,
    },
    /// A file operation finished; dropped unless it is the pane's operation in flight.
    FileOpDone {
        tab: TabId,
        side: Side,
        op: RequestId,
        result: Result<(), Error>,
    },
    /// Something in the queue panel.
    Queue(QueueMsg),
    /// An engine event, from the queue's subscription.
    TransferEvent(Event),
    /// The re-list delay after finished transfers ran out: the panes waiting for it list
    /// their folder again ([`transfers`]).
    RelistPanes,
    /// The queue must start over from a snapshot: the subscription just started, or its
    /// receiver lagged and lost events.
    QueueResync,
    /// The snapshot asked for by `request`; dropped unless it is the latest request.
    QueueSnapshot {
        request: RequestId,
        snapshot: TransferSnapshot,
    },
    /// The transfers of one [`Effect::Enqueue`] were queued (or the first failure).
    Enqueued(Result<(), Error>),
    ControlDone {
        id: TransferId,
        result: Result<(), Error>,
    },
    /// "Clear finished" ended: the number of items removed.
    Cleared(Result<usize, Error>),
    /// A file or folder dropped on the window from the system file manager.
    FileDropped(PathBuf),
    /// Files from the system file manager entered (`true`, once per file) or left the
    /// window.
    FilesHovered(bool),
    /// A mouse button went up, anywhere: ends a drag between panes (dropping it only over
    /// the other pane).
    DragEnd,
    /// The pointer left the window: a drag ends without dropping.
    DragCancel,
    /// A dialog to show or withdraw, from the prompt broker's subscription.
    Prompt(BrokerEvent),
    /// Something in the open dialog.
    Dialog(DialogMsg),
    /// "Remove the old entry…" in a tab whose host key changed.
    KnownHosts {
        tab: TabId,
        msg: KnownHostsMsg,
    },
    /// The `known_hosts` lines that removing would delete (the site's fix).
    KnownHostEntries {
        site: SiteId,
        result: Result<Vec<KnownHostEntry>, Error>,
    },
    /// The old `known_hosts` entries were removed (how many; the site's fix).
    KnownHostRemoved {
        site: SiteId,
        result: Result<usize, Error>,
    },
    /// "Close and reconnect": the tab's failed session is closed and a new one started.
    RestartSession(TabId),
    /// A session's connection state (and, for SSH, how it logged in), from its feed.
    SessionState {
        session: SessionId,
        state: ConnectionState,
        auth: Option<AuthMethod>,
    },
    /// "Reconnect" on a tab whose connection was lost.
    ReconnectSession(TabId),
    /// The backend's reconnect of `session` finished (for every tab still on it).
    Reconnected {
        session: SessionId,
        result: Result<(), Error>,
    },
    /// Close the open workspace: its tabs are saved, then closed.
    CloseWorkspace(WorkspaceId),
    /// The workspace's tabs were saved (or not: logged only).
    TabsSaved(Result<(), Error>),
    /// The window's close button (or the window manager) asks to close.
    CloseRequested(iced::window::Id),
    /// The backend is shut down (or ran out of time): the window may close.
    ShutdownDone(iced::window::Id),
    /// The sidebar's "Import from FileZilla".
    OpenImport,
    Import(import::ImportMsg),
    /// The import finished: its report, or why the file could not be read.
    Imported(Result<ImportReport, Error>),
    SitemanagerDeleted(Result<(), Error>),
    /// The sidebar's gear, the palette or Ctrl+, (Cmd+, on macOS): opens the settings
    /// tab, or selects it.
    OpenSettings,
    /// A click on the settings tab in the strip.
    SelectSettings,
    /// The settings tab's ×.
    CloseSettings,
    Settings(settings::SettingsMsg),
    /// Folding or hiding a sidebar section.
    Sidebar(SidebarMsg),
    SettingsLoaded(SettingsLoad),
    /// Why the persistent queue could not be opened at start, if it could not.
    QueueStatus(Option<Error>),
    SettingsSaved(Result<(), Error>),
    BackupRestored(Result<BackupImportReport, Error>),
    /// A system file dialog closed: the path chosen, `None` if cancelled.
    FilePicked {
        target: PickTarget,
        path: Option<PathBuf>,
    },
    /// A frame of the indeterminate progress bar.
    BusyTick,
    /// The window was resized to this size (logical pixels).
    WindowResized(iced::Size),
    /// A left or right press in the window, at this position (logical pixels), before
    /// the widget under it sees it.
    PointerPressed(iced::Point),
    /// The pointer moved, at this position (logical pixels): sent only while
    /// [`App::tracks_pointer`].
    PointerMoved(iced::Point),
    /// A press or double-click on the divider at the top of a panel.
    Divider(DividerMsg),
    /// Something in the context menu ([`menu`]).
    Menu(MenuMsg),
    /// Something in a terminal tab: its widget, keyboard, search bar or timers.
    Terminal {
        tab: TabId,
        msg: TerminalMsg,
    },
    /// A terminal widget's size in cells changed (the shell follows after
    /// [`terminal::RESIZE_DEBOUNCE`]).
    TerminalResized {
        tab: TabId,
        cols: u16,
        rows: u16,
    },
    /// One frame's worth of a shell's output (every byte since the last one). Its
    /// `credit` lets the feed pull more once this message is handled (and dropped).
    TerminalOutput {
        tab: TabId,
        shell: ShellId,
        bytes: Redacted<Vec<u8>>,
        credit: crate::effects::Credit,
    },
    /// The end of a shell's output: its exit status, an overflow, then `Closed`.
    ShellEvent {
        tab: TabId,
        shell: ShellId,
        event: terminal::ShellSignal,
    },
    /// The shell [`Effect::OpenShell`] asked for under `request` opened (or not).
    ShellOpened {
        tab: TabId,
        request: RequestId,
        result: Result<ShellId, Error>,
    },
    /// Remote editing: the strip's choices, the close questions, the backend's events
    /// and results ([`edit`]).
    Edit(EditMsg),
    /// The update check, the card's buttons and the download ([`update`]).
    Update(UpdateMsg),
    /// The export screen: opened, typed into, its checks and its result ([`export`]).
    Export(ExportMsg),
}

/// The profile change a form asked for, carried back with its result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileOp {
    SaveSite(SiteId),
    DeleteSite(SiteId),
    SaveWorkspace(WorkspaceId),
    DeleteWorkspace(WorkspaceId),
}

/// Work for the backend.
#[derive(Debug)]
pub enum Effect {
    /// Reads the profiles; back as [`Message::ProfilesLoaded`] with the same id.
    LoadProfiles(RequestId),
    /// Saves the site, then applies the password change (the only holder of a new
    /// password once the form emitted it).
    SaveSite {
        site: Site,
        password: PasswordChange,
    },
    DeleteSite(SiteId),
    SaveWorkspace(Workspace),
    DeleteWorkspace(WorkspaceId),
    /// Starts a comparison of the local folder `local_root` with the remote folder
    /// `remote_root` of `session`; reports as [`Message::Compare`] (progress, then one
    /// `Finished`). `options.concurrency_*` are set by the backend; a run id is
    /// used once per tab.
    Compare {
        tab: TabId,
        run: CompareRunId,
        session: SessionId,
        local_root: RemotePath,
        remote_root: RemotePath,
        options: hoprs_remote::CompareOptions,
    },
    /// Lets a run that stopped at the limit continue without one.
    CompareResume {
        tab: TabId,
        run: CompareRunId,
    },
    /// Cooperative cancel (folder change, Stop, auto-off): no new listing starts, the one
    /// in flight finishes, and the run ends with `Finished(Ok(stopped))`.
    CompareCancel {
        tab: TabId,
        run: CompareRunId,
    },
    /// Abrupt cancel for a closed tab or a gone session: drops the run's future (an FTP
    /// session may reconnect) and its registry entry; no `Finished` follows.
    CompareAbort {
        tab: TabId,
        run: CompareRunId,
    },
    /// Connects the sites in parallel with one secret batch; one
    /// [`Message::SiteConnected`] per site, as each finishes.
    ConnectSites(Vec<SiteId>),
    CloseSession(SessionId),
    /// Opens the directory of `sites.toml` with the system file manager.
    OpenProfileFolder,
    /// Lists a pane's folder; the result comes back as [`Message::Listed`]. The remote
    /// side needs `session`; the local side has none.
    List {
        tab: TabId,
        side: Side,
        request_id: RequestId,
        session: Option<SessionId>,
        path: RemotePath,
    },
    /// Scrolls the tab strip to `x` (relative, 0 = start, 1 = end) so the active tab
    /// shows ([`tab_strip_offset`]).
    ScrollTabs {
        x: f32,
    },
    /// Scrolls a pane's list to `offset_y` (a new folder, the keyboard cursor, a tab
    /// shown again).
    ScrollPane {
        tab: TabId,
        side: Side,
        offset_y: f32,
    },
    /// Scrolls the differences panel's list to `offset_y` (the keyboard cursor).
    ScrollCompare {
        tab: TabId,
        offset_y: f32,
    },
    /// A new folder, rename, delete or chmod in one pane; progress comes back as
    /// [`Message::FileOpProgress`], the end as [`Message::FileOpDone`]. The remote side
    /// needs `session`.
    FileOp {
        tab: TabId,
        side: Side,
        op: RequestId,
        session: Option<SessionId>,
        action: FileAction,
    },
    /// Focuses the pane's inline name editor and selects its first `select_end`
    /// characters.
    FocusEdit {
        tab: TabId,
        side: Side,
        select_end: usize,
    },
    /// Sends [`Message::Key`] unless a text field has the focus (the sidebar filter keeps
    /// its arrows, Tab, F2, F7…).
    KeyUnlessTyping {
        key: FileKey,
        shift: bool,
    },
    /// Adds transfers to the engine's queue; one [`Message::Enqueued`] for all.
    Enqueue(Vec<TransferSpec>),
    /// Queues the compare panel's transfers (files with their own `Overwrite` policy); one
    /// [`CompareMsg::Enqueued`] with each item's id.
    EnqueueCompare {
        tab: TabId,
        /// The run and roots the rows belong to.
        run: CompareRunId,
        roots: (RemotePath, RemotePath),
        items: Vec<compare::CompareItem>,
    },
    /// Pause, resume, cancel, retry or resolve one item.
    Control {
        id: TransferId,
        op: ControlOp,
    },
    /// Takes the queue's snapshot; comes back as [`Message::QueueSnapshot`].
    SnapshotQueue(RequestId),
    ClearFinished,
    /// The user's answer to a dialog (the only holder of a typed secret once sent).
    AnswerPrompt {
        id: PromptId,
        answer: PromptAnswer,
    },
    /// Focuses field `n` of the open dialog (shown, or Tab).
    FocusDialogField(usize),
    /// Takes the keyboard focus from every widget (a dialog without fields opened).
    Unfocus,
    /// Reads the `known_hosts` lines for `host:port`; back as
    /// [`Message::KnownHostEntries`].
    KnownHostEntries {
        site: SiteId,
        host: String,
        port: u16,
    },
    /// Removes the `known_hosts` lines for `host:port` if they are still `expected` (what
    /// the user confirmed); back as [`Message::KnownHostRemoved`].
    RemoveKnownHost {
        site: SiteId,
        host: String,
        port: u16,
        expected: Vec<KnownHostEntry>,
    },
    /// Closes `close` (if any), then connects `site` with a new session; back as
    /// [`Message::SiteConnected`].
    RestartSite {
        close: Option<SessionId>,
        site: SiteId,
    },
    /// Retries a lost session (shared by all its tabs) through the backend's reconnect;
    /// back as [`Message::Reconnected`].
    Reconnect {
        session: SessionId,
    },
    /// Saves a workspace's tabs and folders; back as [`Message::TabsSaved`].
    SaveTabs {
        workspace: WorkspaceId,
        tabs: WorkspaceTabs,
    },
    /// The window is closing: saves `save` (the open workspace's tabs), shuts the backend
    /// down (engine, then sessions), all within 5 s; back as [`Message::ShutdownDone`].
    Shutdown {
        save: Option<(WorkspaceId, WorkspaceTabs)>,
        window: iced::window::Id,
    },
    /// Closes the window, which ends the app.
    ExitWindow(iced::window::Id),
    /// Reads the settings the backend started with; back as [`Message::SettingsLoaded`].
    LoadSettings,
    /// Asks whether the persistent queue opened; back as [`Message::QueueStatus`].
    LoadQueueStatus,
    /// Writes `settings.toml`; back as [`Message::SettingsSaved`].
    SaveSettings(Settings),
    /// The engine's conflict policy from now on.
    SetPolicy(ConflictPolicy),
    /// Workers per site for the next connections.
    SetWorkers(usize),
    /// Keepalive (seconds) of the next SSH and FTP connections.
    SetKeepalive(u64),
    /// Checks whether the typed export file exists; back as
    /// [`ExportMsg::TargetChecked`].
    CheckExportTarget(PathBuf),
    /// Opens the system file dialog; back as [`Message::FilePicked`].
    PickFile(PickTarget),
    /// The save dialog for an export, proposing `name`; back as [`ExportMsg::Picked`].
    PickExportFile {
        name: String,
    },
    /// Imports a `sitemanager.xml`, off the UI thread; back as [`Message::Imported`].
    ImportFilezilla {
        path: PathBuf,
        folders_as_workspaces: bool,
    },
    /// Deletes the imported file; back as [`Message::SitemanagerDeleted`].
    DeleteSitemanager(PathBuf),
    /// Writes an encrypted backup of `scope`; back as [`ExportMsg::Done`]. The only holder
    /// of the password once emitted (its `Debug` is redacted).
    ExportBackup {
        scope: ExportScope,
        path: PathBuf,
        password: SecretString,
    },
    /// Reads the Google Drive connection (the keyring, once); back as
    /// [`ExportMsg::DriveStatus`].
    DriveStatus,
    /// Signs in to Google Drive through the browser; its events come back as
    /// [`ExportMsg::Connecting`].
    DriveConnect,
    /// Ends the wait of a running connect (no message: its `Done` says it).
    DriveCancel,
    /// Disconnects Google Drive; back as [`ExportMsg::Disconnected`].
    DriveDisconnect,
    /// Seals the encrypted backup of `scope` in memory; back as [`ExportMsg::SealDone`].
    /// The only holder of the password once emitted.
    SealExport {
        scope: ExportScope,
        password: SecretString,
    },
    /// Uploads the sealed bytes as `name`; back as [`ExportMsg::UploadDone`].
    DriveUpload {
        name: String,
        sealed: crate::drive::Sealed,
    },
    /// The save dialog for the sealed bytes, proposing `name`; back as
    /// [`ExportMsg::SavePicked`].
    PickSealedTarget {
        name: String,
    },
    /// Writes the sealed bytes to `path`; back as [`ExportMsg::SaveDone`].
    SaveSealed {
        path: PathBuf,
        sealed: crate::drive::Sealed,
    },
    /// Opens a Google page in the browser (the authorisation URL, which holds the `state`,
    /// or the uploaded file's); back as [`ExportMsg::PageOpened`].
    OpenDrivePage(Redacted<String>),
    /// Restores a backup; back as [`Message::BackupRestored`]. The only holder of the
    /// password once emitted.
    RestoreBackup {
        path: PathBuf,
        password: SecretString,
        mode: ImportMode,
    },
    /// Opens a terminal of `cols` x `rows` on `session`; back as [`Message::ShellOpened`].
    OpenShell {
        tab: TabId,
        request: RequestId,
        session: SessionId,
        cols: u16,
        rows: u16,
    },
    /// Writes to the shell; back as [`TerminalMsg::InputSent`] (one write in flight per
    /// shell, so bytes keep their order).
    ShellInput {
        tab: TabId,
        shell: ShellId,
        data: Redacted<Vec<u8>>,
    },
    ShellResize {
        shell: ShellId,
        cols: u16,
        rows: u16,
    },
    /// Closes the shell (and forgets it in the backend).
    CloseShell(ShellId),
    WriteClipboard(Redacted<String>),
    /// Reads the clipboard for a paste; back as [`TerminalMsg::Pasted`].
    ReadClipboard(TabId),
    /// Sends `message` back after `after` (the debounces).
    Later {
        after: std::time::Duration,
        message: Box<Message>,
    },
    /// Focuses the terminal tab's search field.
    FocusTerminalSearch(TabId),
    /// Focuses the palette's search field.
    FocusPalette,
    /// Scrolls the log panel to its newest line.
    ScrollLogToEnd,
    /// Downloads `remote` and opens it with `editor`; back as [`EditMsg::Started`].
    StartEdit {
        tab: TabId,
        session: SessionId,
        remote: RemotePath,
        editor: String,
    },
    /// The user's choice on a conflict; back as [`EditMsg::Done`].
    ResolveEdit {
        id: crate::backend::EditId,
        choice: crate::backend::EditChoice,
    },
    /// "Retry" on a failed upload; back as [`EditMsg::Done`].
    RetryEdit(crate::backend::EditId),
    /// Asks which edits of `sessions` are unsent; back as [`EditMsg::Checked`].
    CheckEdits {
        purpose: edit::EditsFor,
        sessions: Vec<SessionId>,
    },
    /// Uploads or keeps the edits of `sessions` (see [`edit::Settle`]); back as
    /// [`EditMsg::Settled`].
    SettleEdits {
        purpose: edit::EditsFor,
        sessions: Vec<SessionId>,
        how: edit::Settle,
    },
    /// Asks for the latest release; back as [`UpdateMsg::Checked`].
    CheckUpdate {
        manual: bool,
    },
    /// Opens a release page in the browser; back as [`UpdateMsg::PageOpened`].
    OpenPage(String),
    /// Downloads and verifies the update; its events come back as [`UpdateMsg::Download`].
    DownloadUpdate(crate::update::download::DownloadJob),
    /// Opens a verified installer with the system handler; back as
    /// [`UpdateMsg::InstallerOpened`].
    OpenInstaller(std::path::PathBuf),
    /// Closes the window the normal way ([`Message::CloseRequested`]: the unsent-edits
    /// question, tabs saved, shutdown), so the Windows installer can replace hoprs.
    RequestClose,
}

/// The window's width before the first resize event (`main`'s initial size).
pub const INITIAL_WINDOW_WIDTH: f32 = 1280.0;
/// The window's height before the first resize event (`main`'s initial size).
pub const INITIAL_WINDOW_HEIGHT: f32 = 800.0;

/// Where the tab strip scrolls (relative, 0 to 1) to show tab `index` of `count`.
///
/// `index / (count - 1)`: with tabs of equal width `w`, a viewport `v >= w` wide and
/// `count * w` of tabs, the offset `index * (count * w - v) / (count - 1)` keeps the whole
/// tab in view (its left edge is at or right of the offset, its right edge at or left of
/// offset + `v`). Tabs of different widths land close to that.
pub fn tab_strip_offset(index: usize, count: usize) -> f32 {
    if count <= 1 {
        0.0
    } else {
        (index.min(count - 1) as f32) / ((count - 1) as f32)
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            queue_open: true,
            log: log::LogPanel::new(crate::logbuf::LogBuffer::new()),
            palette: None,
            site_filter: String::new(),
            load: Load::Loading,
            sites: Vec::new(),
            workspaces: Vec::new(),
            unreadable: Vec::new(),
            expanded: HashSet::new(),
            form: None,
            tabs: Vec::new(),
            conns: HashMap::new(),
            active_tab: None,
            settings_tab: None,
            sidebar_error: None,
            home: dirs::home_dir().as_deref().and_then(files::local_path),
            mods: Mods::default(),
            queue: Queue::new(),
            drag: None,
            pointer: None,
            os_drag: false,
            os_drag_count: 0,
            dialogs: Dialogs::default(),
            open_workspace: None,
            closing: false,
            filezilla_default: hoprs_import::default_sitemanager_path(),
            import: import::ImportFlow::new(None),
            settings: Settings::default(),
            settings_load_error: None,
            settings_save_error: None,
            settings_saving: false,
            settings_dirty: false,
            backup: settings::Backup::default(),
            export: None,
            export_dir: dirs::document_dir().or_else(dirs::home_dir),
            drive_available: crate::drive::client_configured(),
            drive_link: export::DriveLink::Unknown,
            drive_error: None,
            drive_unreadable: false,
            busy_phase: 0,
            window_width: INITIAL_WINDOW_WIDTH,
            window_height: INITIAL_WINDOW_HEIGHT,
            last_press: iced::Point::ORIGIN,
            resize: None,
            menu: None,
            edits: Vec::new(),
            edit_opening: Vec::new(),
            toast: None,
            close_asks: HashMap::new(),
            center_ask: None,
            parked_forms: Vec::new(),
            queued_quit: None,
            kept_notice: None,
            editor_input: None,
            compare_ignore_input: None,
            updater: update::Updater::new(update::UpdateEnv::for_app()),
            held: HashMap::new(),
            replayed: HashSet::new(),
            relist_armed: false,
            next_id: 0,
            profiles_requested: RequestId(0),
            profiles_applied: RequestId(0),
        }
    }

    /// Reads the profiles again, under a new request id.
    pub(crate) fn load_profiles(&mut self) -> Effect {
        let request = RequestId(self.next_id());
        self.profiles_requested = request;
        Effect::LoadProfiles(request)
    }

    /// The last profile read asked for.
    #[cfg(test)]
    pub(crate) fn profiles_requested(&self) -> RequestId {
        self.profiles_requested
    }

    /// A profile read finished. Reads can end out of order: one older than the read
    /// applied last is dropped (its snapshot, or its error, is stale).
    fn profiles_result(
        &mut self,
        request: RequestId,
        result: Result<ProfileSnapshot, Error>,
    ) -> Vec<Effect> {
        if request < self.profiles_applied {
            return Vec::new();
        }
        self.profiles_applied = request;
        match result {
            Ok(snapshot) => self.profiles_loaded(snapshot),
            Err(error) => {
                self.load = Load::Failed(error);
                Vec::new()
            }
        }
    }

    /// The app and the effects to run at startup.
    pub fn boot() -> (Self, Vec<Effect>) {
        let mut app = Self::new();
        let effects = vec![
            app.load_profiles(),
            Effect::LoadSettings,
            Effect::LoadQueueStatus,
        ];
        (app, effects)
    }

    pub fn site(&self, id: &SiteId) -> Option<&Site> {
        self.sites.iter().find(|s| &s.id == id)
    }

    pub fn workspace(&self, id: &WorkspaceId) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| &w.id == id)
    }

    /// The sites the sidebar lists under the current filter, in file order.
    pub fn visible_sites(&self) -> Vec<&Site> {
        self.sites
            .iter()
            .filter(|s| sidebar::site_matches(s, &self.site_filter))
            .collect()
    }

    /// The workspace's sites in its order, missing ones included.
    pub fn workspace_sites<'a>(&'a self, workspace: &Workspace) -> Vec<WorkspaceRow<'a>> {
        workspace
            .sites
            .iter()
            .map(|entry| match self.site(&entry.site) {
                Some(site) => WorkspaceRow::Site(site),
                None => WorkspaceRow::Missing(entry.site.clone()),
            })
            .collect()
    }

    /// Applies `msg`; returns the effects to run. Never blocks.
    pub fn update(&mut self, msg: Message) -> Vec<Effect> {
        // A palette row to run becomes the action's own message.
        let msg = self.palette_command(msg);
        // Escape cancels a drag under way first, unless a menu (a right press during the
        // drag opened it), a dialog or the palette is open: then that takes the Escape.
        let Some(msg) = self.drag_key(msg) else {
            return Vec::new();
        };
        // An open menu has the keyboard first (Enter chooses its highlighted item).
        let Some(msg) = self.menu_key(msg) else {
            return Vec::new();
        };
        // A menu item chosen becomes the action's own message too.
        let msg = self.menu_command(msg);
        let shown = self.shown_tab();
        let active = self.active_tab;
        let user = reconnect::is_user_action(&msg);
        let mut effects = self.apply(msg);
        // An action on a site whose session was lost reconnects it first.
        if user {
            effects = self.hold_for_reconnection(effects);
        }
        // Another tab became active (selected, opened or moved by a close): bring it into
        // view in the tab strip, which may be scrolled sideways.
        if self.active_tab != active
            && let Some(index) = self.active_tab
        {
            // The settings tab, last in the strip, counts too.
            let count = self.tabs.len() + usize::from(self.settings_tab.is_some());
            effects.push(Effect::ScrollTabs {
                x: tab_strip_offset(index, count),
            });
        }
        // A security dialog wins over the palette, and a closing window has none.
        if self.palette.is_some() && (self.dialogs.is_open() || self.closing) {
            self.palette = None;
        }
        // The same for a context menu, which the palette replaces too.
        if self.menu.is_some() && (self.dialogs.is_open() || self.closing || self.palette.is_some())
        {
            self.menu = None;
        }
        // A session tab became active (selected, opened, a close question shown): the
        // settings tab is no longer selected.
        if self.active_tab.is_some()
            && let Some(settings) = &mut self.settings_tab
        {
            settings.active = false;
        }
        // No session tab selected (the last one closed, or the workspace with it): the
        // settings tab, if open, is the remaining tab and becomes selected.
        if self.active_tab.is_none()
            && let Some(settings) = &mut self.settings_tab
        {
            settings.active = true;
        }
        // Typed backup passwords never outlive the settings page, however it was left.
        if !self.settings_shown() {
            self.backup.forget_passwords();
            // The same for the typed fields: the next visit starts from what is saved.
            self.compare_ignore_input = None;
            self.editor_input = None;
        }
        // The same for the export screen's: a flow that does not run goes with it.
        effects.extend(self.export_left());
        // The center shows another tab (or Settings, or a form): a menu about what it
        // showed goes.
        if self.shown_tab() != shown {
            self.menu = None;
        }
        // Everything the menu offered went (its queue row finished, its site, workspace
        // or tab is gone): no empty box stays behind.
        if let Some(menu) = &self.menu
            && self.menu_items(&menu.target).is_empty()
        {
            self.menu = None;
        }
        // The center now shows another tab (or a tab instead of a form): its lists'
        // scrollables must match the offsets the panes remember.
        if self.shown_tab() != shown && self.shown_tab().is_some() {
            effects.extend(self.sync_scroll());
        }
        effects
    }

    /// The tab the center shows, if any.
    pub(crate) fn shown_tab(&self) -> Option<TabId> {
        if self.form.is_some() {
            return None;
        }
        self.active_tab
            .and_then(|i| self.tabs.get(i))
            .map(|tab| tab.id)
    }

    fn apply(&mut self, msg: Message) -> Vec<Effect> {
        if self.closing
            && let Some(effects) = self.while_closing(&msg)
        {
            return effects;
        }
        if self.blocked_by_overlay(&msg) {
            return Vec::new();
        }
        match msg {
            Message::ToggleQueue => self.queue_open = !self.queue_open,
            Message::Log(msg) => return self.log_msg(msg),
            Message::Compare(msg) => return self.compare_msg(msg),
            Message::Palette(msg) => return self.palette_msg(msg),
            Message::WindowResized(size) => {
                self.window_width = size.width;
                self.window_height = size.height;
                self.menu = None;
            }
            Message::PointerPressed(at) => self.last_press = at,
            Message::PointerMoved(at) => self.pointer_moved(at),
            Message::Divider(msg) => return self.divider_msg(msg),
            Message::Menu(msg) => return self.menu_msg(msg),
            Message::Sidebar(msg) => return self.sidebar_msg(msg),
            Message::FilterSites(filter) => self.site_filter = filter,
            Message::ReloadProfiles => {
                self.load = Load::Loading;
                return vec![self.load_profiles()];
            }
            Message::ProfilesLoaded { request, result } => {
                return self.profiles_result(request, result);
            }
            Message::ToggleWorkspace(id) => {
                if !self.expanded.remove(&id) {
                    self.expanded.insert(id);
                }
            }
            Message::OpenWorkspace(id) => return self.open_workspace_tabs(&id),
            Message::ConnectSite(site) => {
                self.form = None;
                return self.open_tabs(vec![(site, Vec::new())]);
            }
            Message::SiteConnected { site, result } => return self.site_connected(site, result),
            Message::SelectTab(index) => {
                if index < self.tabs.len() {
                    self.select_session_tab(index);
                    self.form = None;
                }
            }
            Message::NewSite => self.form = Some(Form::Site(SiteForm::new_site())),
            Message::EditSite(id) => {
                if let Some(site) = self.site(&id) {
                    self.form = Some(Form::Site(SiteForm::edit(site)));
                }
            }
            Message::NewWorkspace => {
                self.form = Some(Form::Workspace(WorkspaceForm::new_workspace()));
            }
            Message::EditWorkspace(id) => {
                if let Some(workspace) = self.workspace(&id) {
                    self.form = Some(Form::Workspace(WorkspaceForm::edit(workspace)));
                }
            }
            Message::SiteForm(m) => return self.site_form(m),
            Message::WorkspaceForm(m) => return self.workspace_form(m),
            Message::FormDone { op, result } => return self.form_done(op, result),
            Message::OpenProfileFolder => return vec![Effect::OpenProfileFolder],
            Message::ProfileFolderOpened(result) => self.sidebar_error = result.err(),
            Message::CloseTab(id) => {
                if let Some(index) = self.tab_index(id) {
                    // The last tab of a session with remote edits asks first.
                    if let Some(effects) = self.ask_before_closing_tab(index) {
                        return effects;
                    }
                    return self.close_tab(index);
                }
            }
            Message::CloseOtherTabs(keep) => return self.close_other_tabs(keep),
            Message::NewTab { tab, kind } => return self.new_session_tab(tab, kind),
            Message::Pane { tab, side, msg } => return self.pane_msg(tab, side, msg),
            Message::Listed {
                tab,
                side,
                request_id,
                result,
            } => return self.listed(tab, side, request_id, result),
            Message::KeyPressed { key, shift } => return self.key_pressed(key, shift),
            Message::Key { key, shift } => return self.file_key(key, shift),
            Message::Modifiers(mods) => self.mods = mods,
            Message::FileOpProgress {
                tab,
                side,
                op,
                done,
            } => return self.file_op_progress(tab, side, op, done),
            Message::FileOpDone {
                tab,
                side,
                op,
                result,
            } => return self.file_op_done(tab, side, op, result),
            Message::Queue(msg) => return self.queue_msg(msg),
            Message::TransferEvent(event) => return self.transfer_event(event),
            Message::RelistPanes => return self.relist_panes(),
            Message::QueueResync => return self.resync_queue(),
            Message::QueueSnapshot { request, snapshot } => {
                return self.queue_snapshot(request, snapshot);
            }
            Message::Enqueued(result) => {
                if let Err(error) = result {
                    self.queue_error(error);
                }
            }
            Message::ControlDone { id, result } => self.control_done(id, result),
            Message::Cleared(result) => return self.cleared(result),
            Message::FileDropped(path) => return self.file_dropped(&path),
            Message::FilesHovered(hovered) => self.files_hovered(hovered),
            Message::DragEnd => return self.drag_end(),
            Message::DragCancel => return self.drag_cancel(),
            Message::Prompt(event) => return self.prompt_event(event),
            Message::Dialog(msg) => return self.dialog_msg(msg),
            Message::KnownHosts { tab, msg } => return self.known_hosts_msg(tab, msg),
            Message::KnownHostEntries { site, result } => {
                return self.known_host_entries(site, result);
            }
            Message::KnownHostRemoved { site, result } => {
                return self.known_host_removed(site, result);
            }
            Message::RestartSession(tab) => return self.restart_session(tab),
            Message::SessionState {
                session,
                state,
                auth,
            } => return self.session_state(session, state, auth),
            Message::ReconnectSession(tab) => return self.reconnect_session(tab),
            Message::Reconnected { session, result } => return self.reconnected(session, result),
            Message::CloseWorkspace(id) => return self.close_workspace(&id),
            Message::TabsSaved(result) => return self.tabs_saved(result),
            Message::CloseRequested(window) => return self.close_requested(window),
            Message::ShutdownDone(window) => return vec![Effect::ExitWindow(window)],
            Message::OpenImport => return self.open_import(),
            Message::Import(msg) => return self.import_msg(msg),
            Message::Imported(result) => return self.imported(result),
            Message::SitemanagerDeleted(result) => self.sitemanager_deleted(result),
            Message::OpenSettings => return self.open_settings(),
            Message::SelectSettings => {
                if self.settings_tab.is_some() {
                    return self.open_settings();
                }
            }
            Message::CloseSettings => return self.close_settings(),
            Message::Settings(msg) => return self.settings_msg(msg),
            Message::SettingsLoaded(loaded) => return self.settings_loaded(loaded),
            Message::QueueStatus(Some(error)) => self.queue_unavailable(error),
            Message::QueueStatus(None) => {}
            Message::SettingsSaved(result) => return self.settings_saved(result),
            Message::BackupRestored(result) => return self.backup_restored(result),
            Message::FilePicked { target, path } => self.file_picked(target, path),
            Message::BusyTick => self.busy_phase = self.busy_phase.wrapping_add(1),
            Message::Terminal { tab, msg } => return self.terminal_msg(tab, msg),
            Message::TerminalResized { tab, cols, rows } => {
                return self.terminal_resized(tab, cols, rows);
            }
            // The credit drops at the end of this arm: the feed may pull the next batch.
            Message::TerminalOutput {
                tab, shell, bytes, ..
            } => {
                return self.terminal_output(tab, shell, &bytes);
            }
            Message::ShellEvent { tab, shell, event } => {
                return self.shell_event(tab, shell, event);
            }
            Message::ShellOpened {
                tab,
                request,
                result,
            } => return self.shell_opened(tab, request, result),
            Message::Edit(msg) => return self.edit_msg(msg),
            Message::Update(msg) => return self.update_msg(msg),
            Message::Export(msg) => return self.export_msg(msg),
        }
        Vec::new()
    }

    fn site_form(&mut self, msg: SiteFormMsg) -> Vec<Effect> {
        let Some(Form::Site(form)) = &mut self.form else {
            return Vec::new();
        };
        match form.update(msg) {
            SiteFormOutcome::Nothing => Vec::new(),
            SiteFormOutcome::Close => {
                self.form = None;
                Vec::new()
            }
            SiteFormOutcome::Save { site, password } => vec![Effect::SaveSite {
                site: *site,
                password,
            }],
            SiteFormOutcome::Export(id) => self.export_msg(ExportMsg::Open(ExportScope::Site(id))),
            SiteFormOutcome::Delete(id) => vec![Effect::DeleteSite(id)],
        }
    }

    fn workspace_form(&mut self, msg: WorkspaceFormMsg) -> Vec<Effect> {
        let Some(Form::Workspace(form)) = &mut self.form else {
            return Vec::new();
        };
        match form.update(msg, &self.sites) {
            WorkspaceFormOutcome::Nothing => Vec::new(),
            WorkspaceFormOutcome::Close => {
                self.form = None;
                Vec::new()
            }
            WorkspaceFormOutcome::Save(workspace) => vec![Effect::SaveWorkspace(workspace)],
            WorkspaceFormOutcome::Export(id) => {
                self.export_msg(ExportMsg::Open(ExportScope::Workspace(id)))
            }
            WorkspaceFormOutcome::Delete(id) => vec![Effect::DeleteWorkspace(id)],
        }
    }

    /// A form's save or delete finished.
    ///
    /// Whatever form is open now, a success reloads the profiles and fixes up the tabs:
    /// - a saved site's session that is not connected is closed, so a retry starts over
    ///   with the new settings instead of retrying the old connection;
    /// - a deleted site's tabs and session go away, and its unfinished transfers are
    ///   cancelled.
    ///
    /// The form itself is closed (success) or shows the error (failure) only if it is the
    /// one that asked: same target, still waiting for this result.
    fn form_done(&mut self, op: ProfileOp, result: Result<(), Error>) -> Vec<Effect> {
        fn waiting_for(form: &Form, op: &ProfileOp) -> bool {
            match (form, op) {
                (Form::Site(form), ProfileOp::SaveSite(id) | ProfileOp::DeleteSite(id)) => {
                    form.busy && form.site_id() == id
                }
                (
                    Form::Workspace(form),
                    ProfileOp::SaveWorkspace(id) | ProfileOp::DeleteWorkspace(id),
                ) => form.busy && form.workspace_id() == id,
                _ => false,
            }
        }
        fn finished(form: &mut Form, error: &Error) {
            match form {
                Form::Site(form) => form.finished(Some(error.clone())),
                Form::Workspace(form) => form.finished(Some(error.clone())),
                Form::Import | Form::Export | Form::QuitEdits => {}
            }
        }
        let error = result.err();
        if self
            .form
            .as_ref()
            .is_some_and(|form| waiting_for(form, &op))
        {
            match (&error, &mut self.form) {
                (None, _) => self.form = None,
                (Some(error), Some(form)) => finished(form, error),
                (Some(_), None) => {}
            }
        } else if let Some(i) = self
            .parked_forms
            .iter()
            .position(|form| waiting_for(form, &op))
        {
            // A form parked behind a close question still gets its result.
            match &error {
                None => {
                    self.parked_forms.remove(i);
                }
                Some(error) => finished(&mut self.parked_forms[i], error),
            }
        }
        if error.is_some() {
            return Vec::new();
        }
        let mut effects = vec![self.load_profiles()];
        match op {
            ProfileOp::SaveSite(site) => effects.extend(self.drop_stale_session(&site)),
            ProfileOp::DeleteSite(site) => {
                effects.extend(self.close_site_tabs(&site));
                effects.extend(self.cancel_site_transfers(&site));
            }
            ProfileOp::SaveWorkspace(_) | ProfileOp::DeleteWorkspace(_) => {}
        }
        effects
    }

    /// New profiles (after a form, an import or a restore). The tabs follow them:
    /// - a site that is gone (deleted, or dropped by a "Replace" restore) loses all its tabs
    ///   and its session;
    /// - a site that is no longer SSH loses its terminal tabs;
    /// - a site whose host or port changed loses its session: a connected one is closed and
    ///   its tabs show why, with Reconnect starting a new session on the saved endpoint; a
    ///   failed one forgets it ([`drop_stale_session`](Self::drop_stale_session)). A
    ///   connect already in flight finishes on the old endpoint;
    /// - the open workspace is forgotten if it is gone.
    fn profiles_loaded(&mut self, snapshot: ProfileSnapshot) -> Vec<Effect> {
        let old = std::mem::replace(&mut self.sites, snapshot.sites);
        self.workspaces = snapshot.workspaces;
        self.unreadable = snapshot.unreadable;
        self.load = Load::Loaded;
        if let Some(open) = &self.open_workspace
            && self.workspace(open).is_none()
        {
            self.open_workspace = None;
        }

        let mut effects = Vec::new();
        let mut gone: Vec<SiteId> = Vec::new();
        for tab in &self.tabs {
            if self.site(&tab.site).is_none() && !gone.contains(&tab.site) {
                gone.push(tab.site.clone());
            }
        }
        for site in gone {
            effects.extend(self.close_site_tabs(&site));
        }
        // A site that is no longer SSH has no terminal: its terminal tabs close (the last
        // tab of the site closes its session).
        let no_terminal: Vec<TabId> = self
            .tabs
            .iter()
            .filter(|tab| {
                tab.kind() == TabKind::Terminal
                    && self
                        .site(&tab.site)
                        .is_some_and(|site| site.protocol != hoprs_core::Protocol::Ssh)
            })
            .map(|tab| tab.id)
            .collect();
        for id in no_terminal {
            if let Some(index) = self.tab_index(id) {
                effects.extend(self.close_tab(index));
            }
        }
        let moved: Vec<SiteId> = self
            .sites
            .iter()
            .filter(|new| {
                old.iter()
                    .any(|was| was.id == new.id && (was.host != new.host || was.port != new.port))
            })
            .map(|site| site.id.clone())
            .collect();
        for site in moved {
            effects.extend(self.drop_moved_session(&site));
        }
        effects
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
pub(crate) mod tests;
