//! Turns each [`Effect`] into an [`iced::Task`]: the only place that calls
//! the backend.

use crate::terminal::Redacted;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{Stream, StreamExt, stream};
use hoprs_core::{Error, RemotePath, Site, SiteId, WorkspaceId};
use hoprs_transfer::{Event, TransferSpec};
use tokio::sync::{broadcast, mpsc, watch};

use hoprs_import::ImportOptions;
use hoprs_vault::{ExportScope, ImportMode};
use secrecy::SecretString;

use crate::app::edit::{EditsFor, FLUSH_LIMIT, Settle, Settled};
use crate::app::terminal::ShellSignal;
use crate::app::{
    CompareItem, CompareMsg, CompareRunId, EditMsg, Effect, ExportMsg, FileAction, Message,
    PickTarget, ProfileOp, RequestId, TabId, TerminalMsg, UpdateMsg,
};
use crate::backend::{
    BackendApi, KeptEdit, KnownHostEntry, PromptAnswer, PromptId, SessionId, ShellId, ShellOutput,
    Side, WorkspaceTabs,
};
use crate::drive::{Pages, Sealed};
use crate::settings::Settings;
use crate::ui::site_form::PasswordChange;
use crate::ui::{dialogs, files, strings, tabs};

/// Runs one effect against the backend.
pub fn run(
    backend: Arc<dyn BackendApi>,
    runs: &CompareRuns,
    effect: Effect,
) -> iced::Task<Message> {
    use iced::Task;
    match effect {
        Effect::LoadProfiles(request) => {
            Task::perform(async move { backend.profiles().await }, move |result| {
                Message::ProfilesLoaded { request, result }
            })
        }
        Effect::SaveSite { site, password } => {
            let op = ProfileOp::SaveSite(site.id.clone());
            Task::perform(save_site(backend, site, password), move |result| {
                Message::FormDone { op, result }
            })
        }
        Effect::DeleteSite(id) => {
            let op = ProfileOp::DeleteSite(id.clone());
            Task::perform(
                async move { backend.delete_site(id).await },
                move |result| Message::FormDone { op, result },
            )
        }
        Effect::SaveWorkspace(workspace) => {
            let op = ProfileOp::SaveWorkspace(workspace.id.clone());
            Task::perform(
                async move { backend.save_workspace(workspace).await },
                move |result| Message::FormDone { op, result },
            )
        }
        Effect::DeleteWorkspace(id) => {
            let op = ProfileOp::DeleteWorkspace(id.clone());
            Task::perform(
                async move { backend.delete_workspace(id).await },
                move |result| Message::FormDone { op, result },
            )
        }
        Effect::ConnectSites(sites) => Task::run(connect_sites(backend, sites), |m| m),
        Effect::CloseSession(session) => Task::future(close_session(backend, session))
            .and_then(|(kept, kept_in)| Task::done(Message::Edit(EditMsg::Kept { kept, kept_in }))),
        Effect::OpenProfileFolder => Task::perform(
            crate::opener::open(backend.profile_dir().into_os_string()),
            Message::ProfileFolderOpened,
        ),
        // Never cancelled: a listing the user moved away from finishes, and `update`
        // drops its result by `request_id`.
        Effect::List {
            tab,
            side,
            request_id,
            session,
            path,
        } => Task::future(list(backend, tab, side, request_id, session, path)),
        Effect::ScrollTabs { x } => iced::widget::operation::snap_to(
            tabs::strip_id(),
            iced::widget::operation::RelativeOffset {
                x: Some(x),
                y: None,
            },
        ),
        Effect::ScrollPane {
            tab,
            side,
            offset_y,
        } => iced::widget::operation::scroll_to(
            files::scroll_id(tab, side),
            iced::widget::operation::AbsoluteOffset {
                x: None,
                y: Some(offset_y),
            },
        ),
        Effect::ScrollCompare { tab, offset_y } => iced::widget::operation::scroll_to(
            crate::ui::compare::scroll_id(tab),
            iced::widget::operation::AbsoluteOffset {
                x: None,
                y: Some(offset_y),
            },
        ),
        // Never cancelled either: the result finds its pane by `op`, or is dropped.
        Effect::FileOp {
            tab,
            side,
            op,
            session,
            action,
        } => Task::run(file_op(backend, tab, side, op, session, action), |m| m),
        Effect::FocusEdit {
            tab,
            side,
            select_end,
        } => {
            let id = files::edit_id(tab, side);
            iced::widget::operation::focus(id.clone())
                .chain(iced::widget::operation::select_range(id, 0, select_end))
        }
        Effect::KeyUnlessTyping { key, shift } => {
            iced::advanced::widget::operate(files::text_focused()).then(move |typing| {
                if typing {
                    Task::none()
                } else {
                    Task::done(Message::Key { key, shift })
                }
            })
        }
        Effect::Compare {
            tab,
            run,
            session,
            local_root,
            remote_root,
            options,
        } => {
            let stream = compare_run(
                backend,
                runs.clone(),
                (tab, run),
                session,
                local_root,
                remote_root,
                options,
            );
            let (task, handle) = Task::run(stream, |m| m).abortable();
            if let Some(entry) = runs.lock().get_mut(&(tab, run)) {
                entry.abort = Some(handle);
            }
            task
        }
        Effect::CompareResume { tab, run } => {
            if let Some(entry) = runs.lock().get(&(tab, run)) {
                let _ = entry.resume.send(true);
            }
            Task::none()
        }
        Effect::CompareCancel { tab, run } => {
            if let Some(entry) = runs.lock().get(&(tab, run)) {
                let _ = entry.stop.send(true);
            }
            Task::none()
        }
        Effect::CompareAbort { tab, run } => {
            if let Some(entry) = runs.lock().remove(&(tab, run))
                && let Some(handle) = entry.abort
            {
                handle.abort();
            }
            Task::none()
        }
        Effect::Enqueue(specs) => Task::perform(enqueue_all(backend, specs), Message::Enqueued),
        Effect::EnqueueCompare {
            tab,
            run,
            roots,
            items,
        } => Task::perform(enqueue_compare(backend, items), move |results| {
            Message::Compare(CompareMsg::Enqueued {
                tab,
                run,
                roots,
                results,
            })
        }),
        Effect::Control { id, op } => Task::perform(
            async move { backend.control(id, op).await },
            move |result| Message::ControlDone { id, result },
        ),
        Effect::SnapshotQueue(request) => Task::perform(
            async move { backend.transfer_snapshot().await },
            move |snapshot| Message::QueueSnapshot { request, snapshot },
        ),
        Effect::ClearFinished => Task::perform(
            async move { backend.clear_finished().await },
            Message::Cleared,
        ),
        Effect::AnswerPrompt { id, answer } => {
            Task::future(answer_prompt(backend, id, answer)).discard()
        }
        Effect::FocusDialogField(n) => iced::widget::operation::focus(dialogs::field_id(n)),
        Effect::Unfocus => {
            iced::advanced::widget::operate(iced::advanced::widget::operation::focusable::unfocus())
        }
        Effect::KnownHostEntries { site, host, port } => {
            Task::future(known_host_entries(backend, site, host, port))
        }
        Effect::RemoveKnownHost {
            site,
            host,
            port,
            expected,
        } => Task::future(remove_known_host(backend, site, host, port, expected)),
        Effect::RestartSite { close, site } => Task::run(restart_site(backend, close, site), |m| m),
        Effect::Reconnect { session } => Task::future(reconnect(backend, session)),
        Effect::SaveTabs { workspace, tabs } => Task::future(save_tabs(backend, workspace, tabs)),
        Effect::Shutdown { save, window } => {
            Task::perform(shutdown(backend, save, SHUTDOWN_LIMIT), move |_| {
                Message::ShutdownDone(window)
            })
        }
        // A forced exit (or the end of the shutdown) keeps every unsent edit first,
        // locally, within a short bound.
        Effect::ExitWindow(window) => Task::future(preserve_before_exit(backend, PRESERVE_LIMIT))
            .then(move |_| {
                exit_watchdog();
                iced::window::close(window)
            }),
        Effect::CheckUpdate { manual } => Task::perform(
            async move { backend.latest_release().await },
            move |result| Message::Update(UpdateMsg::Checked { manual, result }),
        ),
        Effect::OpenPage(url) => Task::perform(crate::opener::open(url.into()), |result| {
            Message::Update(UpdateMsg::PageOpened(result))
        }),
        Effect::DownloadUpdate(job) => Task::run(backend.download_update(job), |event| {
            Message::Update(UpdateMsg::Download(event))
        }),
        Effect::OpenInstaller(file) => {
            Task::perform(crate::opener::open(file.into_os_string()), |result| {
                Message::Update(UpdateMsg::InstallerOpened(result))
            })
        }
        Effect::RequestClose => {
            iced::window::oldest().and_then(|window| Task::done(Message::CloseRequested(window)))
        }
        Effect::LoadSettings => Task::done(Message::SettingsLoaded(backend.settings())),
        Effect::LoadQueueStatus => Task::done(Message::QueueStatus(backend.queue_error())),
        Effect::SaveSettings(settings) => Task::future(save_settings(backend, settings)),
        Effect::SetPolicy(policy) => {
            Task::future(async move { backend.set_policy(policy).await }).discard()
        }
        Effect::SetAgents(on) => Task::future(set_agents(backend, on)).discard(),
        Effect::SetWorkers(workers) => {
            backend.set_workers(workers);
            Task::none()
        }
        Effect::SetKeepalive(secs) => {
            backend.set_keepalive(secs);
            Task::none()
        }
        Effect::CheckExportTarget(path) => Task::future(async move {
            // An error (no permission to look) counts as "not there": the export itself
            // then reports what is wrong.
            let probe = path.clone();
            let exists = tokio::task::spawn_blocking(move || probe.try_exists())
                .await
                .is_ok_and(|found| found.unwrap_or(false));
            Message::Export(ExportMsg::TargetChecked { path, exists })
        }),
        Effect::PickFile(target) => Task::perform(pick_file(target), move |path| {
            Message::FilePicked { target, path }
        }),
        Effect::PickExportFile { name } => Task::perform(pick_export_file(name), |path| {
            Message::Export(ExportMsg::Picked(path))
        }),
        Effect::ImportFilezilla {
            path,
            folders_as_workspaces,
        } => Task::future(import_filezilla(backend, path, folders_as_workspaces)),
        Effect::DeleteSitemanager(path) => Task::future(delete_sitemanager(backend, path)),
        Effect::ExportBackup {
            scope,
            path,
            password,
        } => Task::future(export_backup(backend, scope, path, password)),
        Effect::DriveStatus => Task::future(drive_status(backend)),
        Effect::DriveConnect { attempt } => Task::run(drive_connect(backend, attempt), |m| m),
        Effect::DriveCancel { attempt } => {
            backend.drive_cancel(attempt);
            Task::none()
        }
        Effect::DriveDisconnect => Task::future(drive_disconnect(backend)),
        Effect::SealExport { scope, password } => {
            Task::future(seal_export(backend, scope, password))
        }
        Effect::DriveUpload { name, sealed } => Task::future(drive_upload(backend, name, sealed)),
        Effect::PickSealedTarget { name } => Task::perform(pick_export_file(name), |path| {
            Message::Export(ExportMsg::SavePicked(path))
        }),
        Effect::SaveSealed { path, sealed } => Task::future(save_sealed(backend, path, sealed)),
        Effect::OpenDrivePage(url) => Task::perform(crate::opener::open(url.0.into()), |result| {
            Message::Export(ExportMsg::PageOpened(result))
        }),
        Effect::RestoreBackup {
            path,
            password,
            mode,
        } => Task::future(restore_backup(backend, path, password, mode)),
        Effect::OpenShell {
            tab,
            request,
            session,
            cols,
            rows,
        } => Task::future(async move {
            let result = backend.open_shell(session, cols, rows).await;
            Message::ShellOpened {
                tab,
                request,
                result,
            }
        }),
        Effect::ShellInput { tab, shell, data } => Task::future(async move {
            // A failure needs no message: a dead shell says so through its output. The
            // bytes are never logged.
            let _ = backend.shell_input(shell, data.0).await;
            Message::Terminal {
                tab,
                msg: TerminalMsg::InputSent(shell),
            }
        }),
        Effect::ShellResize { shell, cols, rows } => Task::future(async move {
            if let Err(error) = backend.shell_resize(shell, cols, rows).await {
                tracing::debug!(%error, "terminal resize not delivered");
            }
        })
        .discard(),
        Effect::CloseShell(shell) => {
            backend.close_shell(shell);
            Task::none()
        }
        Effect::WriteClipboard(text) => iced::clipboard::write(text.0),
        Effect::ReadClipboard(tab) => iced::clipboard::read().map(move |text| Message::Terminal {
            tab,
            msg: TerminalMsg::Pasted(text.into()),
        }),
        Effect::Later { after, message } => Task::future(later(after, *message)),
        Effect::FocusTerminalSearch(tab) => {
            iced::widget::operation::focus(crate::ui::terminal::search_id(tab))
        }
        Effect::FocusPalette => iced::widget::operation::focus(crate::ui::palette::input_id()),
        Effect::StartEdit {
            tab,
            session,
            remote,
            editor,
        } => Task::future(async move {
            let result = backend.start_edit(session, remote.clone(), editor).await;
            Message::Edit(EditMsg::Started {
                tab,
                session,
                remote,
                result,
            })
        }),
        Effect::ResolveEdit { id, choice } => Task::future(async move {
            let result = backend.resolve_edit(id, choice).await;
            Message::Edit(EditMsg::Done { id, result })
        }),
        Effect::RetryEdit(id) => Task::future(async move {
            let result = backend.retry_edit(id).await;
            Message::Edit(EditMsg::Done { id, result })
        }),
        Effect::CheckEdits { purpose, sessions } => {
            Task::future(check_edits(backend, purpose, sessions))
        }
        Effect::SettleEdits {
            purpose,
            sessions,
            how,
        } => Task::future(settle_edits(backend, purpose, sessions, how, FLUSH_LIMIT)),
        Effect::ScrollLogToEnd => iced::widget::operation::snap_to(
            crate::ui::log::scroll_id(),
            iced::widget::operation::RelativeOffset {
                x: None,
                y: Some(1.0),
            },
        ),
    }
}

/// `message`, `after` from now (the debounces; tokio's clock, so tests can pause it).
pub async fn later(after: Duration, message: Message) -> Message {
    tokio::time::sleep(after).await;
    message
}

/// The longest a terminal frame lasts: output is delivered at most this often.
pub const FRAME: Duration = Duration::from_millis(16);
/// The most output one batch holds. With [`MAX_IN_FLIGHT`] batches waiting at most,
/// memory stays bounded and a shell can deliver up to about 125 MiB/s. Measured in
/// release: feeding 2 MiB of `cat` output takes 23–32 ms (80- to 250-column text, with
/// and without colours); 4 MiB took 46–54 ms, too close to a UI hitch.
pub const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;
/// Batches of one shell that may wait for `update` at once. While they all wait, the feed
/// pulls nothing more: the shell's pump holds the output, and past its own limit closes
/// the shell and says so (`Overflowed`), instead of the app queueing without bound.
pub const MAX_IN_FLIGHT: usize = 2;

/// One batch's place among the [`MAX_IN_FLIGHT`] of its shell; the place frees when the
/// last clone is dropped, i.e. once `update` has handled the [`Message::TerminalOutput`]
/// carrying it. `Default` holds no place.
#[derive(Clone, Debug, Default)]
pub struct Credit {
    /// Held only to be dropped.
    _permit: Option<Arc<tokio::sync::OwnedSemaphorePermit>>,
}

/// What [`batch_output`] yields.
#[derive(Debug)]
pub enum Batched {
    /// Every byte gathered for one frame, with its credit.
    Data(Redacted<Vec<u8>>, Credit),
    Signal(ShellSignal),
}

/// A shell's output as the terminal tab takes it: [`batch_output`] over the stream the
/// backend hands out once, as [`Message::TerminalOutput`] (data) and
/// [`Message::ShellEvent`] (the rest). An output already taken, or one that ends without
/// `Closed`, ends with `Closed`.
pub fn shell_feed(
    backend: &dyn BackendApi,
    tab: TabId,
    shell: ShellId,
) -> impl Stream<Item = Message> + Send + use<> {
    let output = backend
        .shell_output(shell)
        .unwrap_or_else(|| stream::empty().boxed());
    let credits = Arc::new(tokio::sync::Semaphore::new(MAX_IN_FLIGHT));
    let mut closed = false;
    batch_output(output, FRAME, credits)
        .chain(stream::once(std::future::ready(Batched::Signal(
            ShellSignal::Closed,
        ))))
        .filter_map(move |item| {
            // Exactly one `Closed`, last.
            let item = (!closed).then_some(item);
            closed |= matches!(item, Some(Batched::Signal(ShellSignal::Closed)));
            std::future::ready(item)
        })
        .map(move |item| match item {
            Batched::Data(bytes, credit) => Message::TerminalOutput {
                tab,
                shell,
                bytes,
                credit,
            },
            Batched::Signal(event) => Message::ShellEvent { tab, shell, event },
        })
}

/// Joins a shell's `Data` into at most one item per `frame` (Review Focus 1):
/// - a burst after a quiet moment goes out at once, with whatever else is already
///   waiting; output that keeps coming is held until a frame after the previous item,
///   then goes out as one;
/// - an item holds at most [`MAX_FRAME_BYTES`] (past it the stream stops pulling until
///   the frame ends);
/// - each data item takes one of `credits` before the stream pulls its first byte, and
///   gives it back when its [`Credit`] is dropped: while the consumer has not handled
///   them, nothing more is pulled (the producer's own bound applies);
/// - `Exit`, `Overflowed` and `Closed` keep their order: data received before one of them
///   goes out first, at the end of its frame, and the event right after it. An event with
///   no data before it goes out at once.
pub fn batch_output<S>(
    output: S,
    frame: Duration,
    credits: Arc<tokio::sync::Semaphore>,
) -> impl Stream<Item = Batched> + Send
where
    S: Stream<Item = ShellOutput> + Send + Unpin + 'static,
{
    struct Batcher<S> {
        output: S,
        credits: Arc<tokio::sync::Semaphore>,
        /// When the last item went out.
        last: Option<tokio::time::Instant>,
        /// An event that ended a batch, to send after it.
        held: Option<ShellSignal>,
        ended: bool,
    }

    fn signal(output: ShellOutput) -> Option<ShellSignal> {
        match output {
            ShellOutput::Data(_) => None,
            ShellOutput::Exit(code) => Some(ShellSignal::Exit(code)),
            ShellOutput::Overflowed => Some(ShellSignal::Overflowed),
            ShellOutput::Closed => Some(ShellSignal::Closed),
        }
    }

    let state = Batcher {
        output,
        credits,
        last: None,
        held: None,
        ended: false,
    };
    stream::unfold(state, move |mut st| async move {
        if let Some(event) = st.held.take() {
            return Some((Batched::Signal(event), st));
        }
        if st.ended {
            return None;
        }
        // Backpressure: no credit, no pulling.
        let permit = st.credits.clone().acquire_owned().await.ok();
        let mut batch = match st.output.next().await? {
            ShellOutput::Data(bytes) => bytes,
            event => return signal(event).map(|s| (Batched::Signal(s), st)),
        };
        let now = tokio::time::Instant::now();
        let due = st.last.map_or(now, |last| (last + frame).max(now));
        let wait = tokio::time::sleep_until(due);
        tokio::pin!(wait);
        loop {
            if batch.len() >= MAX_FRAME_BYTES {
                wait.as_mut().await;
                break;
            }
            tokio::select! {
                // What is already waiting joins the batch before the frame closes.
                biased;
                item = st.output.next() => match item {
                    Some(ShellOutput::Data(bytes)) => batch.extend_from_slice(&bytes),
                    // The data keeps its frame; the event follows it.
                    Some(event) => {
                        st.held = signal(event);
                        wait.as_mut().await;
                        break;
                    }
                    None => {
                        st.ended = true;
                        wait.as_mut().await;
                        break;
                    }
                },
                () = &mut wait => break,
            }
        }
        st.last = Some(tokio::time::Instant::now());
        Some((
            Batched::Data(
                batch.into(),
                Credit {
                    _permit: permit.map(Arc::new),
                },
            ),
            st,
        ))
    })
}

/// "Import from FileZilla": the backend reads, parses and applies the file on the
/// blocking pool.
pub async fn import_filezilla(
    backend: Arc<dyn BackendApi>,
    path: PathBuf,
    folders_as_workspaces: bool,
) -> Message {
    let options = ImportOptions {
        folders_as_workspaces,
    };
    Message::Imported(backend.import_filezilla(path, options).await)
}

pub async fn delete_sitemanager(backend: Arc<dyn BackendApi>, path: PathBuf) -> Message {
    Message::SitemanagerDeleted(backend.delete_sitemanager(path).await)
}

/// Writes a backup; the password goes straight from the effect to the vault.
pub async fn export_backup(
    backend: Arc<dyn BackendApi>,
    scope: ExportScope,
    path: PathBuf,
    password: SecretString,
) -> Message {
    Message::Export(ExportMsg::Done(
        backend.export_backup(scope, path, password).await,
    ))
}

/// The Google Drive connection: memory, else the keyring (once).
pub async fn drive_status(backend: Arc<dyn BackendApi>) -> Message {
    Message::Export(ExportMsg::DriveStatus(backend.drive_status().await))
}

/// Signs in through the browser; the pages it shows afterwards are in the app's language.
pub fn drive_connect(
    backend: Arc<dyn BackendApi>,
    attempt: u64,
) -> impl Stream<Item = Message> + use<> {
    let pages = Pages {
        connected: strings::drive_page_connected().to_owned(),
        failed: strings::drive_page_failed().to_owned(),
    };
    backend
        .drive_connect(attempt, pages)
        .map(move |event| Message::Export(ExportMsg::Connecting(attempt, event)))
}

pub async fn drive_disconnect(backend: Arc<dyn BackendApi>) -> Message {
    Message::Export(ExportMsg::Disconnected(backend.drive_disconnect().await))
}

/// Seals a backup in memory; the password goes straight from the effect to the vault.
pub async fn seal_export(
    backend: Arc<dyn BackendApi>,
    scope: ExportScope,
    password: SecretString,
) -> Message {
    Message::Export(ExportMsg::SealDone(
        backend.seal_export(scope, password).await,
    ))
}

pub async fn drive_upload(backend: Arc<dyn BackendApi>, name: String, sealed: Sealed) -> Message {
    Message::Export(ExportMsg::UploadDone(
        backend.drive_upload(name, sealed).await,
    ))
}

pub async fn save_sealed(backend: Arc<dyn BackendApi>, path: PathBuf, sealed: Sealed) -> Message {
    let result = backend.save_sealed(path.clone(), sealed).await;
    Message::Export(ExportMsg::SaveDone { path, result })
}

pub async fn restore_backup(
    backend: Arc<dyn BackendApi>,
    path: PathBuf,
    password: SecretString,
    mode: ImportMode,
) -> Message {
    Message::BackupRestored(backend.restore_backup(path, password, mode).await)
}

/// Tells the backend the master switch for agents.
pub async fn set_agents(backend: Arc<dyn BackendApi>, on: bool) {
    backend.set_agents_enabled(on).await;
}

pub async fn save_settings(backend: Arc<dyn BackendApi>, settings: Settings) -> Message {
    Message::SettingsSaved(backend.save_settings(settings).await)
}

/// The system file dialog (on Linux the XDG desktop portal, else zenity), on its own
/// thread: the UI keeps running. `None` if cancelled or no dialog is available.
async fn pick_file(target: PickTarget) -> Option<PathBuf> {
    let home = dirs::home_dir();
    let dialog = rfd::AsyncFileDialog::new();
    let dialog = match &home {
        Some(home) => dialog.set_directory(home),
        None => dialog,
    };
    let handle = match target {
        PickTarget::Sitemanager => {
            let dialog = match hoprs_import::default_sitemanager_path()
                .as_deref()
                .and_then(Path::parent)
                .filter(|dir| dir.is_dir())
            {
                Some(dir) => dialog.set_directory(dir),
                None => dialog,
            };
            dialog
                .set_title(strings::pick_sitemanager())
                .add_filter(strings::pick_filter_xml(), &["xml"])
                .pick_file()
                .await
        }
        PickTarget::RestoreBackup => dialog.set_title(strings::pick_restore()).pick_file().await,
    };
    handle.map(|handle| handle.path().to_path_buf())
}

/// The export's save dialog, proposing `name` in Documents (else home), as the File field
/// does. `None` if cancelled or no dialog is available.
async fn pick_export_file(name: String) -> Option<PathBuf> {
    let dialog = rfd::AsyncFileDialog::new()
        .set_title(strings::pick_export())
        .set_file_name(name);
    let dialog = match dirs::document_dir().or_else(dirs::home_dir) {
        Some(dir) => dialog.set_directory(dir),
        None => dialog,
    };
    dialog
        .save_file()
        .await
        .map(|handle| handle.path().to_path_buf())
}

/// The longest the window waits for the backend to shut down once asked to close.
pub const SHUTDOWN_LIMIT: Duration = Duration::from_secs(5);
/// The part of [`SHUTDOWN_LIMIT`] the tab save may take: the engine always gets the rest.
pub const SAVE_LIMIT: Duration = Duration::from_secs(2);
/// After the window is asked to close, the process ends at the latest this much later,
/// even if something still holds the runtime (an abandoned blocking save, for one).
pub const EXIT_GRACE: Duration = Duration::from_secs(2);

/// How [`shutdown`] ended. The window closes whatever it is.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ShutdownOutcome {
    Done,
    /// `limit` ran out first; what was left is abandoned (the process exits).
    TimedOut,
    /// The backend's shutdown panicked.
    Failed,
}

/// The window is closing (Review Focus 5), in this order and within `limit` in all:
/// 1. saves the open workspace's tabs (`save`), through the vault's atomic write, within
///    [`SAVE_LIMIT`]; a failure, a panic or a save that takes too long is logged and
///    abandoned (never waited for), and the shutdown goes on;
/// 2. shuts the transfer engine down, so its running items reopen paused next time;
/// 3. closes the sessions ([`BackendApi::shutdown`] does 2 and 3), in what is left of
///    `limit`.
///
/// Each step runs in its own task, so a panic becomes an outcome instead of a lost
/// message. The caller closes the window when this returns, whatever the outcome.
pub async fn shutdown(
    backend: Arc<dyn BackendApi>,
    save: Option<(WorkspaceId, WorkspaceTabs)>,
    limit: Duration,
) -> ShutdownOutcome {
    let deadline = tokio::time::Instant::now() + limit;
    if let Some((workspace, tabs)) = save {
        let saver = backend.clone();
        let mut saving =
            tokio::spawn(async move { saver.save_workspace_tabs(workspace, tabs).await });
        match tokio::time::timeout(SAVE_LIMIT.min(limit), &mut saving).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(error))) => {
                tracing::warn!(%error, "cannot save the workspace's tabs on close");
            }
            Ok(Err(error)) => {
                tracing::error!(%error, "saving the workspace's tabs on close panicked");
            }
            Err(_) => {
                saving.abort();
                tracing::warn!(limit = ?SAVE_LIMIT, "saving the workspace's tabs on close took too long; skipped");
            }
        }
    }
    let mut stopping = tokio::spawn(async move { backend.shutdown().await });
    match tokio::time::timeout_at(deadline, &mut stopping).await {
        Ok(Ok(())) => ShutdownOutcome::Done,
        Ok(Err(error)) => {
            tracing::error!(%error, "the backend's shutdown panicked; closing anyway");
            ShutdownOutcome::Failed
        }
        Err(_) => {
            stopping.abort();
            tracing::warn!(
                ?limit,
                "the backend did not shut down in time; closing anyway"
            );
            ShutdownOutcome::TimedOut
        }
    }
}

/// Ends the process [`EXIT_GRACE`] from now unless it ended already: closing the window
/// ends iced's loop, but dropping the runtime waits for blocking tasks still running.
fn exit_watchdog() {
    let spawned = std::thread::Builder::new()
        .name("exit-watchdog".into())
        .spawn(|| {
            std::thread::sleep(EXIT_GRACE);
            tracing::warn!("still running after the window closed; exiting");
            std::process::exit(0);
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "cannot start the exit watchdog");
    }
}

/// Saves a workspace's tabs and folders.
pub async fn save_tabs(
    backend: Arc<dyn BackendApi>,
    workspace: WorkspaceId,
    tabs: WorkspaceTabs,
) -> Message {
    Message::TabsSaved(backend.save_workspace_tabs(workspace, tabs).await)
}

/// The banner's Reconnect: the backend's reconnect of the session (`reconnect_now`, then
/// the wait that also settles the secrets typed with "save").
pub async fn reconnect(backend: Arc<dyn BackendApi>, session: SessionId) -> Message {
    let result = backend.reconnect(session).await;
    Message::Reconnected { session, result }
}

/// One session's feed for its subscription: the state now, then each change, as
/// [`Message::SessionState`] with how the session logged in. Ends when the session is
/// closed (its watch goes away), or at once if it is already.
pub fn session_feed(
    backend: Arc<dyn BackendApi>,
    session: SessionId,
) -> impl Stream<Item = Message> + Send + 'static {
    let watch = backend.session_state(session);
    stream::unfold((watch, true), move |(watch, first)| {
        let backend = backend.clone();
        async move {
            let mut watch = watch?;
            if !first {
                watch.changed().await.ok()?;
            }
            let state = watch.borrow_and_update().clone();
            let message = Message::SessionState {
                session,
                state,
                auth: backend.session_auth(session),
            };
            Some((message, (Some(watch), false)))
        }
    })
}

/// Ends a session's remote edits, then closes it: all sent, their files go (an upload
/// under way is waited for, never cut by the close); unsent ones (a close that did not
/// ask, e.g. the site was deleted) are kept as recovered copies, nothing deleted. Returns
/// the copies kept and their root (none: no message).
pub async fn close_session(
    backend: Arc<dyn BackendApi>,
    session: SessionId,
) -> Option<(Vec<KeptEdit>, PathBuf)> {
    let kept = match backend.finish_edits(session, false).await {
        Ok(unsent) if unsent.is_empty() => None,
        Ok(_) => match backend.keep_edits(session).await {
            Ok(kept) => Some((kept, backend.recovered_dir())),
            Err(error) => {
                tracing::warn!(session = session.0, %error, "cannot keep the unsent remote edits; their files stay");
                None
            }
        },
        Err(error) => {
            tracing::warn!(session = session.0, %error, "cannot end the remote edits; their files stay");
            None
        }
    };
    backend.close(session).await;
    kept
}

/// How long a forced exit waits for the unsent edits to be kept.
pub const PRESERVE_LIMIT: Duration = Duration::from_secs(2);

/// Before the window closes for good: every unsent edit is saved as a recovered copy,
/// locally (at most `limit`; the edits a finished shutdown ended are already safe).
pub async fn preserve_before_exit(backend: Arc<dyn BackendApi>, limit: Duration) -> Vec<KeptEdit> {
    match tokio::time::timeout(limit, backend.preserve_edits()).await {
        Ok(kept) => {
            if !kept.is_empty() {
                tracing::info!(count = kept.len(), dir = %backend.recovered_dir().display(), "unsent remote edits kept before exit");
            }
            kept
        }
        Err(_) => {
            tracing::warn!(
                ?limit,
                "keeping the unsent remote edits took too long; exiting"
            );
            Vec::new()
        }
    }
}

/// Which edits of `sessions` are unsent, for a close question.
pub async fn check_edits(
    backend: Arc<dyn BackendApi>,
    purpose: EditsFor,
    sessions: Vec<SessionId>,
) -> Message {
    let mut unsent = Vec::new();
    for session in sessions {
        unsent.extend(backend.edits_pending(session).await);
    }
    Message::Edit(EditMsg::Checked {
        purpose,
        unsent,
        kept_in: backend.recovered_dir(),
    })
}

/// A close question's answer: "Upload" sends every session's unsent edits (all at once,
/// within `limit`), then ends the sessions with nothing left unsent (their files go);
/// "Keep" saves the unsent edits as recovered copies and ends them, deleting nothing.
pub async fn settle_edits(
    backend: Arc<dyn BackendApi>,
    purpose: EditsFor,
    sessions: Vec<SessionId>,
    how: Settle,
    limit: Duration,
) -> Message {
    let result = async {
        let mut settled = Settled::default();
        match how {
            Settle::Upload => {
                let flushes = sessions
                    .iter()
                    .map(|session| backend.flush_edits(*session, limit));
                let left = futures_util::future::join_all(flushes).await;
                for (session, left) in sessions.iter().zip(left) {
                    if left.is_empty() {
                        // A save made since the flush is reported, not deleted.
                        settled
                            .unsent
                            .extend(backend.finish_edits(*session, false).await?);
                    } else {
                        settled.unsent.extend(left);
                    }
                }
            }
            Settle::Keep => {
                for session in &sessions {
                    settled.kept.extend(backend.keep_edits(*session).await?);
                }
            }
        }
        settled.kept_in = backend.recovered_dir();
        Ok(settled)
    }
    .await;
    Message::Edit(EditMsg::Settled {
        purpose,
        how,
        result,
    })
}

/// The edits' events for their subscription. A receiver that lagged lost events: the
/// strip may be stale until the next event of each edit (logged).
pub fn edit_feed(backend: &dyn BackendApi) -> impl Stream<Item = Message> + Send + use<> {
    let events = backend.subscribe_edits();
    stream::unfold(events, |mut events| async move {
        loop {
            match events.recv().await {
                Ok(event) => return Some((Message::Edit(EditMsg::Event(event)), events)),
                Err(broadcast::error::RecvError::Lagged(lost)) => {
                    tracing::warn!(lost, "the remote edit notices lagged behind");
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    })
}

/// Delivers a dialog's answer (the effect's secret goes straight to the backend).
pub async fn answer_prompt(backend: Arc<dyn BackendApi>, id: PromptId, answer: PromptAnswer) {
    backend.answer_prompt(id, answer).await;
}

/// Reads the `known_hosts` lines "Remove the old entry" would delete.
pub async fn known_host_entries(
    backend: Arc<dyn BackendApi>,
    site: SiteId,
    host: String,
    port: u16,
) -> Message {
    let result = backend.known_host_entries(host, port).await;
    Message::KnownHostEntries { site, result }
}

/// Removes the `known_hosts` lines of `host:port`, if still the ones confirmed.
pub async fn remove_known_host(
    backend: Arc<dyn BackendApi>,
    site: SiteId,
    host: String,
    port: u16,
    expected: Vec<KnownHostEntry>,
) -> Message {
    let result = backend.remove_known_host(host, port, expected).await;
    Message::KnownHostRemoved { site, result }
}

/// Closes `close` first (as [`close_session`]: its unsent edits kept, said by
/// [`EditMsg::Kept`]), then connects `site`: the connect never finds (and reuses) the old
/// session.
pub fn restart_site(
    backend: Arc<dyn BackendApi>,
    close: Option<SessionId>,
    site: SiteId,
) -> impl Stream<Item = Message> + Send + 'static {
    let closer = backend.clone();
    // Closed as any session is: its remote edits settle first, unsent ones kept and said.
    stream::once(async move {
        let kept = match close {
            Some(session) => close_session(closer, session).await,
            None => None,
        };
        stream::iter(kept.map(|(kept, kept_in)| Message::Edit(EditMsg::Kept { kept, kept_in })))
    })
    .flatten()
    // Lazy: the connect starts only once the close is done.
    .chain(connect_sites(backend, vec![site]))
}

/// The broker's dialogs for the UI's subscription: each request or withdrawal as
/// [`Message::Prompt`]. The backend hands its channel out once; without it (a second
/// subscription) the stream is empty.
pub fn prompt_feed(backend: &dyn BackendApi) -> impl Stream<Item = Message> + Send + use<> {
    let events = backend.prompt_events();
    stream::unfold(events, |events| async move {
        let mut events = events?;
        let event = events.recv().await?;
        Some((Message::Prompt(event), Some(events)))
    })
}

/// What the agent side reports, for the UI's subscription: each event as
/// [`Message::Agent`]. The backend hands its channel out once; without it (a second
/// subscription) the stream is empty.
pub fn agent_feed(backend: &dyn BackendApi) -> impl Stream<Item = Message> + Send + use<> {
    let events = backend.agent_events();
    stream::unfold(events, |events| async move {
        let mut events = events?;
        let event = events.recv().await?;
        Some((Message::Agent(event), Some(events)))
    })
}

/// Queues each transfer in order; stops at the first failure.
pub async fn enqueue_all(
    backend: Arc<dyn BackendApi>,
    specs: Vec<TransferSpec>,
) -> Result<(), Error> {
    for spec in specs {
        backend.enqueue(spec).await?;
    }
    Ok(())
}

/// Queues the compare panel's items in order: a file with `Overwrite` for itself alone, a
/// folder with the normal policy. Each item has its own result (a failure does not stop the
/// rest).
pub async fn enqueue_compare(
    backend: Arc<dyn BackendApi>,
    items: Vec<CompareItem>,
) -> Vec<crate::app::compare::Queued> {
    let mut results = Vec::with_capacity(items.len());
    for item in items {
        let result = if item.overwrite {
            backend
                .enqueue_with_policy(item.spec, hoprs_transfer::ConflictPolicy::Overwrite)
                .await
        } else {
            backend.enqueue(item.spec).await
        };
        results.push(crate::app::compare::Queued {
            path: item.path,
            size: item.size,
            result,
        });
    }
    results
}

/// The engine's events for the queue's subscription. It subscribes at once, then starts
/// with [`Message::QueueResync`], so the snapshot it leads to is taken after the
/// subscription (nothing falls between the two).
pub fn transfer_feed(backend: &dyn BackendApi) -> impl Stream<Item = Message> + Send + use<> {
    engine_messages(backend.transfer_events())
}

/// Each event as [`Message::TransferEvent`], after a first [`Message::QueueResync`]. A
/// receiver that lagged (it lost events) gives another `QueueResync`; the stream ends with
/// the engine.
pub fn engine_messages(
    events: broadcast::Receiver<Event>,
) -> impl Stream<Item = Message> + Send + 'static {
    let events = stream::unfold(events, |mut events| async move {
        match events.recv().await {
            Ok(event) => Some((Message::TransferEvent(event), events)),
            Err(broadcast::error::RecvError::Lagged(lost)) => {
                tracing::warn!(
                    lost,
                    "the queue lagged behind the transfer engine; resyncing"
                );
                Some((Message::QueueResync, events))
            }
            Err(broadcast::error::RecvError::Closed) => None,
        }
    });
    stream::once(std::future::ready(Message::QueueResync)).chain(events)
}

/// A running comparison's handles.
struct CompareRun {
    resume: watch::Sender<bool>,
    stop: watch::Sender<bool>,
    /// Set right after the task is made; aborting drops the run's future.
    abort: Option<iced::task::Handle>,
}

/// The running comparisons by tab and run, owned by the app's shell (so a registry never
/// outlives it, and tests with their own never meet). An entry leaves when the run ends or
/// is aborted.
#[derive(Clone, Default)]
pub struct CompareRuns(
    Arc<std::sync::Mutex<std::collections::HashMap<(TabId, CompareRunId), CompareRun>>>,
);

impl CompareRuns {
    fn lock(
        &self,
    ) -> std::sync::MutexGuard<'_, std::collections::HashMap<(TabId, CompareRunId), CompareRun>>
    {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(test)]
    fn contains(&self, tab: TabId, run: CompareRunId) -> bool {
        self.lock().contains_key(&(tab, run))
    }
}

/// One comparison: its progress as it comes, then [`CompareMsg::Finished`] (last). The
/// run's signals are registered before the first message, so a cancel sent right after
/// the effect always finds them.
pub fn compare_run(
    backend: Arc<dyn BackendApi>,
    runs: CompareRuns,
    (tab, run): (TabId, CompareRunId),
    session: SessionId,
    local_root: RemotePath,
    remote_root: RemotePath,
    options: hoprs_remote::CompareOptions,
) -> impl Stream<Item = Message> + Send + 'static {
    let (resume_tx, resume) = watch::channel(false);
    let (stop_tx, stop) = watch::channel(false);
    runs.lock().insert(
        (tab, run),
        CompareRun {
            resume: resume_tx,
            stop: stop_tx,
            abort: None,
        },
    );
    let (tx, rx) = mpsc::unbounded_channel();
    let done = async move {
        let signals = hoprs_remote::CompareSignals { resume, stop };
        let result = backend
            .compare(session, local_root, remote_root, options, tx, signals)
            .await;
        runs.lock().remove(&(tab, run));
        Message::Compare(CompareMsg::Finished { tab, run, result })
    };
    // The progress sender lives in the run, so the progress stream ends with it, before
    // the final message.
    let progress = stream::unfold(rx, move |mut rx| async move {
        rx.recv().await.map(|progress| {
            (
                Message::Compare(CompareMsg::Progress { tab, run, progress }),
                rx,
            )
        })
    });
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    let driver = stream::once(async move {
        let _ = result_tx.send(done.await);
    })
    .filter_map(|()| std::future::ready(None));
    let last = stream::once(result_rx).filter_map(|r| std::future::ready(r.ok()));
    stream::select(driver, progress.chain(last))
}

/// Runs one file operation: a delete's progress as it goes, then
/// [`Message::FileOpDone`] (last).
pub fn file_op(
    backend: Arc<dyn BackendApi>,
    tab: TabId,
    side: Side,
    op: RequestId,
    session: Option<SessionId>,
    action: FileAction,
) -> impl Stream<Item = Message> + Send + 'static {
    let (tx, rx) = mpsc::unbounded_channel();
    let done = async move {
        let result = match action {
            FileAction::Mkdir(path) => backend.mkdir(session, side, path).await,
            FileAction::Rename { from, to } => backend.rename(session, side, from, to).await,
            FileAction::Delete(targets) => {
                backend.delete(session, side, targets, tx).await.map(|_| ())
            }
            FileAction::Chmod { paths, mode } => {
                chmod_all(&*backend, session, side, paths, mode).await
            }
        };
        Message::FileOpDone {
            tab,
            side,
            op,
            result,
        }
    };
    // The progress sender lives in the operation, so the progress stream ends with it,
    // before the final message.
    let progress = stream::unfold(rx, move |mut rx| async move {
        rx.recv().await.map(|done| {
            (
                Message::FileOpProgress {
                    tab,
                    side,
                    op,
                    done,
                },
                rx,
            )
        })
    });
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    let driver = stream::once(async move {
        let _ = result_tx.send(done.await);
    })
    .filter_map(|()| std::future::ready(None));
    let last = stream::once(result_rx).filter_map(|r| std::future::ready(r.ok()));
    stream::select(driver, progress.chain(last))
}

/// The same mode on each path, stopping at the first failure.
async fn chmod_all(
    backend: &dyn BackendApi,
    session: Option<SessionId>,
    side: Side,
    paths: Vec<RemotePath>,
    mode: u32,
) -> Result<(), Error> {
    for path in paths {
        backend.chmod(session, side, path, mode).await?;
    }
    Ok(())
}

/// Saves the site, then stores or forgets its password (the site must exist first).
pub async fn save_site(
    backend: Arc<dyn BackendApi>,
    site: Site,
    password: PasswordChange,
) -> Result<(), Error> {
    let id = site.id.clone();
    backend.save_site(site).await?;
    match password {
        PasswordChange::Keep => Ok(()),
        PasswordChange::Set(password) => backend.set_password(id, Some(password)).await,
        PasswordChange::Forget => backend.set_password(id, None).await,
    }
}

/// Runs one listing to its end and tags the result with its request.
pub async fn list(
    backend: Arc<dyn BackendApi>,
    tab: TabId,
    side: Side,
    request_id: RequestId,
    session: Option<SessionId>,
    path: RemotePath,
) -> Message {
    let result = backend.list(session, side, path).await;
    Message::Listed {
        tab,
        side,
        request_id,
        result,
    }
}

/// Open workspace: one [`Message::SiteConnected`] per site, each as soon as that site is
/// done. The backend reads the secrets once for the whole batch.
pub fn connect_sites(
    backend: Arc<dyn BackendApi>,
    sites: Vec<SiteId>,
) -> impl Stream<Item = Message> + Send + 'static {
    let (tx, rx) = mpsc::unbounded_channel();
    // Drives the batch; yields nothing itself. The channel closes when it returns.
    let driver = stream::once(async move { backend.connect_batch(sites, tx).await })
        .filter_map(|()| std::future::ready(None));
    let results = stream::unfold(rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|(site, result)| (Message::SiteConnected { site, result }, rx))
    });
    stream::select(driver, results)
}

#[cfg(test)]
mod compare_tests {
    use super::*;
    use crate::backend::fake::FakeBackend;
    use hoprs_core::Protocol;
    use hoprs_remote::{CompareOptions, Progress, Status, Summary};
    use std::time::SystemTime;

    const TAB: TabId = TabId(7);

    fn p(path: &str) -> RemotePath {
        RemotePath::new(path).unwrap()
    }

    /// A fake backend with one connected site, and the stream of a run on it.
    async fn setup() -> (Arc<FakeBackend>, SessionId) {
        let site = Site::new("s", "h", "u", Protocol::Ssh);
        let fake = Arc::new(FakeBackend::new().with_site(site.clone()));
        let session = fake.connect(site.id).await.unwrap().session;
        (fake, session)
    }

    fn start(
        fake: &Arc<FakeBackend>,
        runs: &CompareRuns,
        session: SessionId,
        run: u64,
        options: CompareOptions,
    ) -> std::pin::Pin<Box<dyn Stream<Item = Message> + Send>> {
        let backend: Arc<dyn BackendApi> = fake.clone();
        let run = CompareRunId(run);
        Box::pin(compare_run(
            backend,
            runs.clone(),
            (TAB, run),
            session,
            p("/l"),
            p("/r"),
            options,
        ))
    }

    fn effect(fake: &Arc<FakeBackend>, runs: &CompareRuns, effect: Effect) {
        let backend: Arc<dyn BackendApi> = fake.clone();
        let _ = run(backend, runs, effect);
    }

    fn finished(msg: Message) -> Result<Summary, Error> {
        match msg {
            Message::Compare(CompareMsg::Finished { tab, result, .. }) => {
                assert_eq!(tab, TAB);
                result
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn runs_a_comparison_over_the_session_and_the_local_fs() {
        let (fake, session) = setup().await;
        let runs = CompareRuns::default();
        let site = fake.session_site(session).unwrap();
        let t = SystemTime::UNIX_EPOCH;
        fake.local_fs().add_file("/l/same", b"x", t);
        fake.local_fs().add_file("/l/only-here", b"x", t);
        fake.remote_fs(&site).add_file("/r/same", b"x", t);
        fake.remote_fs(&site).add_file("/r/only-there", b"x", t);

        let messages: Vec<Message> = start(&fake, &runs, session, 1, CompareOptions::default())
            .collect()
            .await;
        let (last, progress) = messages.split_last().unwrap();
        let found: Vec<(String, Status)> = progress
            .iter()
            .filter_map(|m| match m {
                Message::Compare(CompareMsg::Progress {
                    run,
                    progress: Progress::Found(d),
                    ..
                }) => {
                    assert_eq!(*run, CompareRunId(1));
                    Some((d.path.clone(), d.status.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            found,
            vec![
                ("only-here".to_string(), Status::OnlyLocal),
                ("only-there".to_string(), Status::OnlyRemote),
            ]
        );
        let summary = finished(last.clone()).unwrap();
        assert_eq!((summary.identical, summary.differences), (1, 2));
        assert!(!summary.partial && !summary.stopped);
        assert_eq!(fake.compare_calls(), vec![(session, p("/l"), p("/r"))]);
        assert!(!runs.contains(TAB, CompareRunId(1)));
    }

    #[tokio::test]
    async fn resume_after_the_limit_completes_the_run() {
        let (fake, session) = setup().await;
        let runs = CompareRuns::default();
        let site = fake.session_site(session).unwrap();
        let t = SystemTime::UNIX_EPOCH;
        for root in ["/l", "/r"] {
            for name in ["d1/f", "d2/f", "top"] {
                if root == "/l" {
                    fake.local_fs().add_file(&format!("{root}/{name}"), b"x", t);
                } else {
                    fake.remote_fs(&site)
                        .add_file(&format!("{root}/{name}"), b"x", t);
                }
            }
        }
        let options = CompareOptions {
            limit: Some(3),
            ..Default::default()
        };
        let mut stream = start(&fake, &runs, session, 2, options);
        loop {
            match stream.next().await.expect("the run has not ended") {
                Message::Compare(CompareMsg::Progress {
                    progress: Progress::LimitReached { .. },
                    ..
                }) => break,
                Message::Compare(CompareMsg::Progress { .. }) => {}
                other => panic!("{other:?}"),
            }
        }
        effect(
            &fake,
            &runs,
            Effect::CompareResume {
                tab: TAB,
                run: CompareRunId(2),
            },
        );
        let last = loop {
            let message = stream.next().await.expect("Finished comes last");
            if matches!(message, Message::Compare(CompareMsg::Finished { .. })) {
                break message;
            }
        };
        let summary = finished(last).unwrap();
        assert!(!summary.partial);
        assert_eq!(summary.identical, 3);
        assert!(
            stream.next().await.is_none(),
            "Finished is the last message"
        );
    }

    #[tokio::test]
    async fn cancel_stops_cooperatively_and_ends_with_a_stopped_result() {
        let (fake, session) = setup().await;
        let runs = CompareRuns::default();
        // The scripted run sends one item, then waits until stopped.
        let _hold = fake.script_compare(
            vec![Progress::Seen {
                local: 1,
                remote: 1,
            }],
            Err(Error::new(hoprs_core::ErrorKind::Network, "not reached")),
            true,
        );
        let mut stream = start(&fake, &runs, session, 3, CompareOptions::default());
        assert!(matches!(
            stream.next().await,
            Some(Message::Compare(CompareMsg::Progress { .. }))
        ));
        effect(
            &fake,
            &runs,
            Effect::CompareCancel {
                tab: TAB,
                run: CompareRunId(3),
            },
        );
        let summary = finished(stream.next().await.unwrap()).unwrap();
        assert!(summary.stopped && summary.partial);
        assert!(stream.next().await.is_none());
        assert!(!runs.contains(TAB, CompareRunId(3)));
    }

    #[tokio::test]
    async fn abort_drops_the_entry_and_no_finished_follows() {
        let (fake, session) = setup().await;
        let runs = CompareRuns::default();
        let _hold = fake.script_compare(
            Vec::new(),
            Ok(Summary {
                identical: 0,
                differences: 0,
                partial: false,
                stopped: false,
            }),
            true,
        );
        let run = CompareRunId(4);
        let _stream = start(&fake, &runs, session, 4, CompareOptions::default());
        assert!(runs.contains(TAB, run));
        effect(&fake, &runs, Effect::CompareAbort { tab: TAB, run });
        assert!(!runs.contains(TAB, run));
    }

    #[tokio::test]
    async fn a_closed_session_ends_with_the_session_closed_error() {
        let (fake, session) = setup().await;
        let runs = CompareRuns::default();
        fake.close(session).await;
        let messages: Vec<Message> = start(&fake, &runs, session, 5, CompareOptions::default())
            .collect()
            .await;
        assert_eq!(messages.len(), 1);
        let error = finished(messages.into_iter().next().unwrap()).unwrap_err();
        assert_eq!(error.code, Some("session-closed"));
    }
}
