//! hoprs: desktop SSH + SFTP + FTP/FTPS client.
//!
//! The binary only calls [`run`], which owns the process-level concerns: the
//! single-instance lock, logging and handing the [`App`] to iced. The crate is a
//! library so that the benchmarks in `benches/` reach the view ([`bench`]) and the
//! start-up probe; nothing else is public.

mod app;
mod backend;
#[doc(hidden)]
pub mod bench;
pub mod cli;
mod drive;
mod effects;
mod i18n;
mod icon;
mod logbuf;
mod opener;
mod probe;
mod renderer;
mod settings;
mod terminal;
#[cfg(test)]
pub(crate) mod test_https;
mod ui;
mod update;

use std::borrow::Cow;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use iced::Task;

use crate::app::{App, Message};
use crate::backend::{Backend, BackendApi, BackendConfig, LazyKeyringStore};
pub use crate::icon::{APP_ID, window_icon};

/// Smallest window that still fits the sidebar and two file panes.
const MIN_WINDOW: iced::Size = iced::Size::new(960.0, 640.0);
const INITIAL_WINDOW: iced::Size =
    iced::Size::new(app::INITIAL_WINDOW_WIDTH, app::INITIAL_WINDOW_HEIGHT);
/// A frame of the indeterminate progress bar.
const BUSY_FRAME: std::time::Duration = std::time::Duration::from_millis(50);

/// The default filter, used when `RUST_LOG` is unset or empty: `info`, with suppaftp
/// at its own default and the graphics stack's start-up chatter (adapters, surfaces,
/// "No config found!", "Ignoring unknown button type") left out.
const DEFAULT_LOG_FILTER: &str = "info,suppaftp=warn,wgpu_hal=error,wgpu_core=warn,iced_winit=warn,iced_wgpu=warn,sctk_adwaita=error";
/// suppaftp's own default; added unless `RUST_LOG` sets plain `suppaftp`.
const SUPPAFTP_DEFAULT: &str = "suppaftp=warn";
/// The most suppaftp may ever log at: at trace it logs `PASS <password>`.
const SUPPAFTP_CAP: &str = "debug";
/// Crates that log terminal bytes (unhandled OSC/DCS/CSI payloads) at debug
/// and trace: never above warn, whatever `RUST_LOG` says.
const TERMINAL_LOG_CRATES: [&str; 2] = ["vte", "alacritty_terminal"];
const TERMINAL_LOG_CAP: &str = "warn";

/// Runs hoprs until its window closes. With `HOPRS_PROBE_STARTUP=1` it runs the start-up
/// probe instead (see `probe`): the same start-up, then two lines on stdout, then exit.
pub fn run() -> iced::Result {
    let started = Instant::now();
    // First, while hoprs is still single-threaded: it may set `ICED_BACKEND`.
    let renderer = renderer::pick();
    let probe = probe::enabled().then(|| probe::Probe::new(started));
    let filter = log_filter(std::env::var("RUST_LOG").ok().as_deref());
    let log = logbuf::LogBuffer::new();
    init_logging(&filter, log.clone());
    tracing::info!("{}", renderer.0);

    let _instance = match acquire_first_instance_lock(&instance_lock_paths()) {
        Ok(Some(lock)) => Some(lock),
        Ok(None) => {
            println!("{}", ui::strings::already_running());
            std::process::exit(0);
        }
        Err(err) => {
            // No usable location at all: start anyway, unprotected.
            tracing::warn!(%err, "could not take the single-instance lock anywhere");
            None
        }
    };

    iced::application(move || boot(log.clone(), probe), update, view)
        .subscription(subscription)
        .title(ui::strings::app_title())
        .theme(|_: &Hoprs| ui::theme::theme())
        .settings(iced::Settings {
            fonts: ui::theme::FONTS.map(Cow::Borrowed).to_vec(),
            default_font: ui::theme::INTER,
            default_text_size: ui::theme::TEXT_SM.into(),
            ..iced::Settings::default()
        })
        .window(iced::window::Settings {
            size: INITIAL_WINDOW,
            min_size: Some(MIN_WINDOW),
            // A close request goes to `update`: tabs saved, backend shut down, then closed.
            exit_on_close_request: false,
            icon: window_icon(),
            platform_specific: platform_specific(),
            ..iced::window::Settings::default()
        })
        .run()
}

/// On Linux the window's app id (Wayland `app_id`, X11 `WM_CLASS`) is what the dock
/// matches against the `.desktop` file. iced has no such field on Windows or macOS.
#[cfg(target_os = "linux")]
fn platform_specific() -> iced::window::settings::PlatformSpecific {
    iced::window::settings::PlatformSpecific {
        application_id: APP_ID.to_owned(),
        ..iced::window::settings::PlatformSpecific::default()
    }
}

#[cfg(not(target_os = "linux"))]
fn platform_specific() -> iced::window::settings::PlatformSpecific {
    iced::window::settings::PlatformSpecific::default()
}

/// iced's state: the pure [`App`] plus the backend its effects run against.
struct Hoprs {
    app: App,
    backend: Arc<dyn BackendApi>,
    /// The running folder comparisons' handles.
    compare_runs: effects::CompareRuns,
    /// Only with `HOPRS_PROBE_STARTUP=1`.
    probe: Option<probe::Probe>,
}

/// What iced carries: the app's messages, and the start-up probe's steps, which never
/// reach the [`App`].
// `Message` is large; boxing it would allocate on every message for a probe that is
// rarely on.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
enum Root {
    App(Message),
    Probe(probe::Step),
}

fn boot(log: logbuf::LogBuffer, probe: Option<probe::Probe>) -> (Hoprs, Task<Root>) {
    let backend = match open_backend() {
        Ok(backend) => backend,
        Err(err) => {
            // No profiles, no app: there is nothing to show yet without them.
            tracing::error!(%err, "cannot open the profiles");
            eprintln!("{}: {err}", ui::strings::cannot_open_profiles());
            std::process::exit(1);
        }
    };
    // The language before the first view: the setting, or the OS's.
    i18n::set_language(i18n::resolve(backend.settings().settings.language));
    let backend: Arc<dyn BackendApi> = Arc::new(backend);
    let (mut app, effects) = App::boot();
    app.attach_log(log);
    let compare_runs = effects::CompareRuns::default();
    let task = run_effects(&backend, &compare_runs, effects);
    (
        Hoprs {
            app,
            backend,
            compare_runs,
            probe,
        },
        task,
    )
}

/// The real backend at the standard locations, secrets in the system keyring.
fn open_backend() -> hoprs_core::Result<Backend> {
    let config = BackendConfig::standard().ok_or_else(|| {
        hoprs_core::Error::coded(
            hoprs_core::ErrorKind::Io,
            "app-no-config-dir",
            "No configuration directory",
        )
    })?;
    let keyring = Arc::new(LazyKeyringStore::new());
    Ok(Backend::open(config, keyring.clone())?.with_app_secrets(keyring))
}

/// Thin wrapper: the pure update decides, `effects::run` performs.
fn update(state: &mut Hoprs, message: Root) -> Task<Root> {
    match message {
        Root::App(message) => {
            let effects = state.app.update(message);
            run_effects(&state.backend, &state.compare_runs, effects)
        }
        Root::Probe(step) => match &mut state.probe {
            Some(probe) => probe.step(step).map(Root::Probe),
            None => Task::none(),
        },
    }
}

fn run_effects(
    backend: &Arc<dyn BackendApi>,
    runs: &effects::CompareRuns,
    effects: Vec<app::Effect>,
) -> Task<Root> {
    Task::batch(
        effects
            .into_iter()
            .map(|effect| effects::run(Arc::clone(backend), runs, effect)),
    )
    .map(Root::App)
}

fn view(state: &Hoprs) -> iced::Element<'_, Root> {
    ui::layout::view(&state.app).map(Root::App)
}

/// The app's subscriptions, and the probe's frames while it waits for the first one.
fn subscription(state: &Hoprs) -> iced::Subscription<Root> {
    let probe = state
        .probe
        .as_ref()
        .and_then(probe::Probe::subscription)
        .map(|frames| frames.map(Root::Probe));
    iced::Subscription::batch(std::iter::once(app_subscription(state).map(Root::App)).chain(probe))
}

/// The keyboard for the file panes and dialogs, the mouse button and dropped files
/// (drags), the transfer engine's events, the prompt broker's dialogs, the remote edits'
/// events, each connected
/// session's state, each terminal's shell output, and the window's close requests.
fn app_subscription(state: &Hoprs) -> iced::Subscription<Message> {
    let fixed = [
        iced::event::listen_with(ui::files::key_message),
        iced::event::listen_with(ui::queue::pointer_message),
        iced::Subscription::run_with(TransferFeed(Arc::clone(&state.backend)), |feed| {
            effects::transfer_feed(feed.0.as_ref())
        }),
        iced::Subscription::run_with(PromptFeed(Arc::clone(&state.backend)), |feed| {
            effects::prompt_feed(feed.0.as_ref())
        }),
        iced::Subscription::run_with(EditFeed(Arc::clone(&state.backend)), |feed| {
            effects::edit_feed(feed.0.as_ref())
        }),
        iced::window::close_requests().map(Message::CloseRequested),
        iced::window::resize_events().map(|(_, size)| Message::WindowResized(size)),
    ];
    // The indeterminate progress bar moves only while an import or a backup runs.
    let busy = state
        .app
        .busy()
        .then(|| iced::time::every(BUSY_FRAME).map(|_| Message::BusyTick));
    // One feed per session, keyed by its id: it starts when a tab connects and stops when
    // the tab closes or fails (a new session is a new feed).
    let sessions = state.app.watched_sessions().into_iter().map(|session| {
        iced::Subscription::run_with(SessionFeed(session, Arc::clone(&state.backend)), |feed| {
            effects::session_feed(Arc::clone(&feed.1), feed.0)
        })
    });
    // One output feed per open shell, keyed by tab and shell: it starts when the shell
    // opens and stops when the tab closes or the shell is replaced.
    let shells = state.app.terminal_feeds().into_iter().map(|(tab, shell)| {
        iced::Subscription::run_with(ShellFeed(tab, shell, Arc::clone(&state.backend)), |feed| {
            effects::shell_feed(feed.2.as_ref(), feed.0, feed.1)
        })
    });
    // The pointer's moves, only while a divider or a row is dragged.
    let pointer = state
        .app
        .tracks_pointer()
        .then(|| iced::event::listen_with(ui::pointer::moved_message));
    // The log panel takes the new lines while it is open.
    let log = state
        .app
        .log
        .open
        .then(|| iced::time::every(app::log::POLL).map(|_| Message::Log(app::LogMsg::Tick)));
    iced::Subscription::batch(
        fixed
            .into_iter()
            .chain(busy)
            .chain(log)
            .chain(pointer)
            .chain(sessions)
            .chain(shells),
    )
}

/// A terminal tab's shell, as the identity of its output feed (the backend rides along).
struct ShellFeed(app::TabId, backend::ShellId, Arc<dyn BackendApi>);

impl std::hash::Hash for ShellFeed {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "shell-feed".hash(state);
        self.0.hash(state);
        self.1.hash(state);
    }
}

/// A session, as the identity of its state feed (the backend rides along).
struct SessionFeed(backend::SessionId, Arc<dyn BackendApi>);

impl std::hash::Hash for SessionFeed {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "session-feed".hash(state);
        self.0.hash(state);
    }
}

/// The backend, as the identity of the dialogs' subscription (one for the whole run: the
/// broker hands its channel out once).
struct PromptFeed(Arc<dyn BackendApi>);

impl std::hash::Hash for PromptFeed {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "prompt-feed".hash(state);
    }
}

/// The backend, as the identity of the remote edits' events (one for the whole run).
struct EditFeed(Arc<dyn BackendApi>);

impl std::hash::Hash for EditFeed {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "edit-feed".hash(state);
    }
}

/// The backend, as the identity of the transfer events' subscription: there is one
/// backend, so one subscription for the whole run.
struct TransferFeed(Arc<dyn BackendApi>);

impl std::hash::Hash for TransferFeed {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "transfer-feed".hash(state);
    }
}

/// Where the lock may live, in order: `$XDG_RUNTIME_DIR/hoprs/`, the
/// config directory, the temp directory.
fn instance_lock_paths() -> Vec<PathBuf> {
    [
        dirs::runtime_dir(),
        dirs::config_dir(),
        Some(std::env::temp_dir()),
    ]
    .into_iter()
    .flatten()
    .map(|base| base.join("hoprs").join("instance.lock"))
    .collect()
}

/// Takes the lock at the first location that works. A location that fails
/// (can't be created, opened or locked) falls through to the next one;
/// one held by another instance stops the search with `Ok(None)`.
fn acquire_first_instance_lock(paths: &[PathBuf]) -> io::Result<Option<File>> {
    let mut last_err = io::Error::new(io::ErrorKind::NotFound, "no lock location");
    for path in paths {
        match acquire_instance_lock(path) {
            Ok(lock) => return Ok(lock),
            Err(err) => {
                tracing::warn!(%err, path = %path.display(), "lock location unusable; trying the next");
                last_err = err;
            }
        }
    }
    Err(last_err)
}

/// Takes the exclusive lock at `path`, creating its directory.
///
/// `Ok(None)` means another process holds it. The lock lives as long as
/// the returned file.
fn acquire_instance_lock(path: &Path) -> io::Result<Option<File>> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    // Called through the trait: std's inherent `File::try_lock` shadows it.
    match fs4::FileExt::try_lock(&file) {
        Ok(()) => Ok(Some(file)),
        Err(fs4::TryLockError::WouldBlock) => Ok(None),
        Err(fs4::TryLockError::Error(err)) => Err(err),
    }
}

/// The `EnvFilter` directives to log with, and whether a request for
/// suppaftp at trace had to be lowered.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LogFilter {
    directives: String,
    capped: bool,
}

/// Builds the filter from `RUST_LOG`; suppaftp never goes above debug, and
/// the terminal crates ([`TERMINAL_LOG_CRATES`]) never above warn. The
/// unset default is [`DEFAULT_LOG_FILTER`]: [`terminal_log_cap`] caps the
/// terminal crates at run time in every case, span directives included.
fn log_filter(rust_log: Option<&str>) -> LogFilter {
    let Some(rust_log) = rust_log.map(str::trim).filter(|s| !s.is_empty()) else {
        return LogFilter {
            directives: DEFAULT_LOG_FILTER.to_owned(),
            capped: false,
        };
    };

    let mut directives = Vec::new();
    let mut capped = false;
    let mut sets_suppaftp = false;
    let mut sets_terminal = [false; TERMINAL_LOG_CRATES.len()];
    for directive in rust_log.split(',').map(str::trim).filter(|d| !d.is_empty()) {
        // `selector=level`; the level never contains `]` or `}`, which
        // tells it apart from a `=` inside a span field filter.
        let (selector, level) = match directive.rsplit_once('=') {
            Some((selector, level)) if !level.contains([']', '}']) => (selector, Some(level)),
            _ => (directive, None),
        };
        let (target, level) = match level {
            Some(level) => (selector.split('[').next().unwrap_or(""), level_rank(level)),
            // A bare level is global; a bare target means trace for it.
            None => match level_rank(selector) {
                Some(rank) => ("", Some(rank)),
                None => (selector.split('[').next().unwrap_or(""), Some(TRACE)),
            },
        };
        let is_suppaftp = target == "suppaftp" || target.starts_with("suppaftp::");
        // A span-only directive (`[span]=level`) matches every target.
        let reaches_suppaftp = is_suppaftp || (target.is_empty() && selector.contains('['));
        // Only a plain `suppaftp=<valid level>` covers the whole target; a
        // span-scoped or submodule directive leaves the rest of suppaftp at
        // the global level. An unparsable level is dropped by `EnvFilter`.
        sets_suppaftp |= selector == "suppaftp" && level.is_some();

        // A directive for a terminal crate (or a module of it) above warn
        // is lowered to warn; at or below warn it is honoured.
        let terminal = TERMINAL_LOG_CRATES
            .iter()
            .position(|name| target == *name || target.starts_with(&format!("{name}::")));
        if let Some(index) = terminal {
            sets_terminal[index] |= selector == TERMINAL_LOG_CRATES[index] && level.is_some();
            if level.is_some_and(|rank| rank > WARN) {
                directives.push(format!("{selector}={TERMINAL_LOG_CAP}"));
                continue;
            }
        }

        if reaches_suppaftp && level == Some(TRACE) {
            directives.push(format!("{selector}={SUPPAFTP_CAP}"));
            capped = true;
        } else {
            directives.push(directive.to_owned());
        }
    }
    // Otherwise suppaftp (or the parts of it not covered by a scoped
    // directive) would inherit the global level, trace included.
    if !sets_suppaftp {
        directives.push(SUPPAFTP_DEFAULT.to_owned());
    }
    for (name, set) in TERMINAL_LOG_CRATES.iter().zip(sets_terminal) {
        if !set {
            directives.push(format!("{name}={TERMINAL_LOG_CAP}"));
        }
    }

    LogFilter {
        directives: directives.join(","),
        capped,
    }
}

const WARN: u8 = 2;
const TRACE: u8 = 5;

/// A `tracing` level filter as a number, 0 (off) to 5 (trace), the way
/// `EnvFilter` parses it.
fn level_rank(level: &str) -> Option<u8> {
    const NAMES: [&str; 6] = ["off", "error", "warn", "info", "debug", "trace"];
    if let Ok(rank) = level.parse::<u8>() {
        return (rank <= TRACE).then_some(rank);
    }
    NAMES
        .iter()
        .position(|name| level.eq_ignore_ascii_case(name))
        .and_then(|rank| u8::try_from(rank).ok())
}

/// A hard per-target cap on top of the `EnvFilter`: the terminal crates
/// stay at warn even inside a span a `[span]=debug` directive enables.
fn terminal_log_cap() -> tracing_subscriber::filter::Targets {
    TERMINAL_LOG_CRATES.iter().fold(
        tracing_subscriber::filter::Targets::new()
            .with_default(tracing_subscriber::filter::LevelFilter::TRACE),
        |targets, name| targets.with_target(*name, tracing::Level::WARN),
    )
}

/// The subscriber's filter: `RUST_LOG` (as rewritten by [`log_filter`])
/// and the terminal cap.
fn subscriber_filter<S>(filter: &LogFilter) -> impl tracing_subscriber::layer::Filter<S> + use<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    use tracing_subscriber::filter::FilterExt;
    tracing_subscriber::EnvFilter::new(&filter.directives).and(terminal_log_cap())
}

/// What `main` logs to: the terminal output (`writer`) and the log panel's
/// `buffer`, each behind the same filter ([`subscriber_filter`]), so the panel
/// never shows what the terminal wouldn't.
fn log_subscriber<W>(
    filter: &LogFilter,
    buffer: logbuf::LogBuffer,
    writer: W,
) -> impl tracing::Subscriber + Send + Sync + 'static
where
    W: for<'w> tracing_subscriber::fmt::MakeWriter<'w> + Send + Sync + 'static,
{
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::SubscriberExt;
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(writer)
                .with_filter(subscriber_filter(filter)),
        )
        .with(buffer.with_filter(subscriber_filter(filter)))
}

fn init_logging(filter: &LogFilter, buffer: logbuf::LogBuffer) {
    use tracing_subscriber::util::SubscriberInitExt;
    // `init` also installs the `log` bridge, through which vte and
    // alacritty_terminal log.
    log_subscriber(filter, buffer, io::stdout).init();
    if filter.capped {
        tracing::warn!(
            "RUST_LOG would log suppaftp at trace, where it logs passwords; lowered to debug"
        );
    }
}

/// Serializes the tests that install `tracing` subscribers. Callsite
/// interest is cached across all live dispatchers: a concurrent test's
/// `trace` subscriber would mark a callsite `always` and skip ours.
#[cfg(test)]
fn subscriber_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Installs [`log_subscriber`] (as `main` does, the terminal output discarded) on
/// this thread until the guard drops; the buffer gets what the app logs here, and,
/// through [`LogCapture::runtime`], on every thread of a runtime (workers and the
/// blocking pool).
///
/// A second, silent dispatcher keeps a callsite's cached interest from another
/// thread (no subscriber: "never") from hiding it from this one. Hold
/// [`subscriber_test_lock`] meanwhile.
#[cfg(test)]
fn capture_log(filter: &LogFilter) -> (logbuf::LogBuffer, LogCapture) {
    let buffer = logbuf::LogBuffer::new();
    let dispatch = tracing::Dispatch::new(log_subscriber(filter, buffer.clone(), io::sink));
    let capture = LogCapture {
        _default: tracing::dispatcher::set_default(&dispatch),
        dispatch,
        _other: tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()),
    };
    (buffer, capture)
}

/// What [`capture_log`] keeps alive.
#[cfg(test)]
struct LogCapture {
    dispatch: tracing::Dispatch,
    _default: tracing::subscriber::DefaultGuard,
    _other: tracing::Dispatch,
}

#[cfg(test)]
impl LogCapture {
    /// A multi-thread runtime whose every thread (workers and the blocking pool) logs
    /// to the capture: `spawn`ed tasks and `spawn_blocking` work included. Its threads
    /// end with it.
    fn runtime(&self) -> tokio::runtime::Runtime {
        let dispatch = self.dispatch.clone();
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .on_thread_start(move || {
                // The thread keeps it until it exits.
                std::mem::forget(tracing::dispatcher::set_default(&dispatch));
            })
            .build()
            .unwrap()
    }
}

/// The marker [`log_canaries`] logs.
#[cfg(test)]
const CANARY: &str = "log-capture-canary";

/// Logs [`CANARY`] from a worker task and from the blocking pool (call inside a
/// [`LogCapture::runtime`]): with [`assert_no_secrets`], proof the capture saw those
/// threads.
#[cfg(test)]
async fn log_canaries() {
    tokio::spawn(async { tracing::info!(target: "hoprs_app", "{CANARY} worker") })
        .await
        .unwrap();
    tokio::task::spawn_blocking(|| tracing::info!(target: "hoprs_app", "{CANARY} blocking"))
        .await
        .unwrap();
}

/// The buffer saw [`log_canaries`] and holds none of `secrets`.
#[cfg(test)]
fn assert_no_secrets(buffer: &logbuf::LogBuffer, secrets: &[&str]) {
    let text = buffer.text();
    for canary in [format!("{CANARY} worker"), format!("{CANARY} blocking")] {
        assert!(text.contains(&canary), "{canary} missing: {text}");
    }
    for secret in secrets {
        assert!(!text.contains(secret), "{secret} reached the log: {text}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_instance_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hoprs").join("instance.lock");

        let first = acquire_instance_lock(&path).unwrap();
        assert!(first.is_some(), "the first instance takes the lock");
        assert!(
            acquire_instance_lock(&path).unwrap().is_none(),
            "a second instance is refused while the first runs"
        );

        drop(first);
        assert!(
            acquire_instance_lock(&path).unwrap().is_some(),
            "the lock is free again once the first instance exits"
        );
    }

    /// Whether the real `EnvFilter` built from `directives` lets a trace
    /// event through for `target`.
    fn trace_enabled(directives: &str, target: &str) -> bool {
        use tracing_subscriber::layer::SubscriberExt;

        let subscriber =
            tracing_subscriber::registry().with(tracing_subscriber::EnvFilter::new(directives));
        tracing::subscriber::with_default(subscriber, || match target {
            "suppaftp" => tracing::enabled!(target: "suppaftp", tracing::Level::TRACE),
            "suppaftp::async_ftp" => {
                tracing::enabled!(target: "suppaftp::async_ftp", tracing::Level::TRACE)
            }
            "suppaftp::foo" => tracing::enabled!(target: "suppaftp::foo", tracing::Level::TRACE),
            "hoprs_ssh" => tracing::enabled!(target: "hoprs_ssh", tracing::Level::TRACE),
            other => panic!("no callsite for {other}"),
        })
    }

    #[test]
    fn real_env_filter_never_traces_suppaftp() {
        let _lock = subscriber_test_lock();
        let inputs = [
            None,
            Some("suppaftp=trace"),
            Some("trace"),
            Some("debug"),
            Some("trace,suppaftp[cmd]=warn"),
            Some("trace,suppaftp::foo=warn"),
            Some("trace,suppaftp::foo=trace,suppaftp[cmd]=trace"),
        ];
        for input in inputs {
            let directives = log_filter(input).directives;
            for target in ["suppaftp", "suppaftp::async_ftp", "suppaftp::foo"] {
                assert!(
                    !trace_enabled(&directives, target),
                    "{input:?} -> {directives:?} traces {target}"
                );
            }
        }
        // The check is not vacuous: other targets still trace.
        assert!(trace_enabled(
            &log_filter(Some("trace")).directives,
            "hoprs_ssh"
        ));
        assert!(!trace_enabled(&log_filter(None).directives, "hoprs_ssh"));
    }

    #[test]
    fn lock_falls_back_when_the_first_location_fails() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the runtime directory should be: create_dir_all fails.
        let blocker = dir.path().join("not-a-dir");
        fs::write(&blocker, b"").unwrap();
        let broken = blocker.join("hoprs").join("instance.lock");
        let fallback = dir
            .path()
            .join("config")
            .join("hoprs")
            .join("instance.lock");
        let candidates = [broken, fallback.clone()];

        let first = acquire_first_instance_lock(&candidates).unwrap();
        assert!(first.is_some(), "the fallback location is locked");
        assert!(fallback.exists());
        assert!(
            acquire_first_instance_lock(&candidates).unwrap().is_none(),
            "a second instance hits the same fallback lock"
        );

        // Every location broken: the error is reported.
        assert!(acquire_first_instance_lock(&candidates[..1]).is_err());
    }

    #[test]
    fn trace_for_suppaftp_is_capped() {
        // Absent or empty: the default.
        for unset in [None, Some(""), Some("  ")] {
            let filter = log_filter(unset);
            assert_eq!(
                filter.directives,
                "info,suppaftp=warn,wgpu_hal=error,wgpu_core=warn,iced_winit=warn,iced_wgpu=warn,sctk_adwaita=error"
            );
            assert!(!filter.capped);
        }

        // Explicit trace for suppaftp is lowered to debug, with a warning.
        let filter = log_filter(Some("info,suppaftp=trace"));
        assert_eq!(
            filter.directives,
            "info,suppaftp=debug,vte=warn,alacritty_terminal=warn"
        );
        assert!(filter.capped);

        // A bare target means trace for it.
        let filter = log_filter(Some("suppaftp"));
        assert_eq!(
            filter.directives,
            "suppaftp=debug,vte=warn,alacritty_terminal=warn"
        );
        assert!(filter.capped);

        // Submodules, numeric levels and span filters are capped too.
        let filter = log_filter(Some("suppaftp::async_ftp=TRACE,suppaftp[cmd]=5"));
        assert_eq!(
            filter.directives,
            "suppaftp::async_ftp=debug,suppaftp[cmd]=debug,suppaftp=warn,vte=warn,alacritty_terminal=warn"
        );
        assert!(filter.capped);

        // A global trace would reach suppaftp: it keeps its default.
        let filter = log_filter(Some("trace"));
        assert_eq!(
            filter.directives,
            "trace,suppaftp=warn,vte=warn,alacritty_terminal=warn"
        );
        assert!(!filter.capped);

        // A span-only directive applies to every target, suppaftp included.
        let filter = log_filter(Some("info,[connect]=trace"));
        assert_eq!(
            filter.directives,
            "info,[connect]=debug,suppaftp=warn,vte=warn,alacritty_terminal=warn"
        );
        assert!(filter.capped);

        // Levels at or below debug are honoured; other targets untouched.
        let filter = log_filter(Some("hoprs_ssh=trace,suppaftp=debug"));
        assert_eq!(
            filter.directives,
            "hoprs_ssh=trace,suppaftp=debug,vte=warn,alacritty_terminal=warn"
        );
        assert!(!filter.capped);
        let filter = log_filter(Some("warn,suppaftp=off"));
        assert_eq!(
            filter.directives,
            "warn,suppaftp=off,vte=warn,alacritty_terminal=warn"
        );
        assert!(!filter.capped);

        // A target that only starts with the same letters is not suppaftp.
        let filter = log_filter(Some("suppaftp_ext=trace"));
        assert_eq!(
            filter.directives,
            "suppaftp_ext=trace,suppaftp=warn,vte=warn,alacritty_terminal=warn"
        );
        assert!(!filter.capped);

        // Scoped suppaftp directives don't set the `suppaftp` target
        // itself: the cap still applies to it and its other modules.
        let filter = log_filter(Some("trace,suppaftp[cmd]=warn"));
        assert_eq!(
            filter.directives,
            "trace,suppaftp[cmd]=warn,suppaftp=warn,vte=warn,alacritty_terminal=warn"
        );
        let filter = log_filter(Some("trace,suppaftp::foo=warn"));
        assert_eq!(
            filter.directives,
            "trace,suppaftp::foo=warn,suppaftp=warn,vte=warn,alacritty_terminal=warn"
        );

        // An unparsable level sets nothing, so the default still applies.
        let filter = log_filter(Some("trace,suppaftp=loud"));
        assert_eq!(
            filter.directives,
            "trace,suppaftp=loud,suppaftp=warn,vte=warn,alacritty_terminal=warn"
        );

        // Everything produced is valid `EnvFilter` syntax.
        for input in ["suppaftp", "[connect]=trace", "suppaftp[cmd{id=1}]=trace"] {
            let directives = log_filter(Some(input)).directives;
            assert!(
                tracing_subscriber::EnvFilter::builder()
                    .parse(&directives)
                    .is_ok(),
                "{directives}"
            );
        }
        assert_eq!(
            log_filter(Some("suppaftp[cmd{id=1}]=trace")).directives,
            "suppaftp[cmd{id=1}]=debug,suppaftp=warn,vte=warn,alacritty_terminal=warn"
        );
    }

    #[test]
    fn terminal_crates_are_capped_at_warn() {
        // Appended whenever RUST_LOG is set.
        let filter = log_filter(Some("debug"));
        assert_eq!(
            filter.directives,
            "debug,suppaftp=warn,vte=warn,alacritty_terminal=warn"
        );
        // Explicit requests above warn are lowered, modules included.
        let filter = log_filter(Some("vte=trace,alacritty_terminal::term=debug,vte"));
        assert_eq!(
            filter.directives,
            "vte=warn,alacritty_terminal::term=warn,vte=warn,suppaftp=warn,alacritty_terminal=warn"
        );
        // At or below warn is honoured, and not overridden afterwards.
        let filter = log_filter(Some("info,vte=error,alacritty_terminal=off"));
        assert_eq!(
            filter.directives,
            "info,vte=error,alacritty_terminal=off,suppaftp=warn"
        );
        // A crate that only shares the prefix is untouched.
        let filter = log_filter(Some("vtest=trace"));
        assert_eq!(
            filter.directives,
            "vtest=trace,suppaftp=warn,vte=warn,alacritty_terminal=warn"
        );
        assert!(!filter.capped);
    }

    /// Whether the full subscriber filter (`EnvFilter` + terminal cap)
    /// enables a debug event from `target`, optionally inside a span `span`.
    fn debug_enabled(rust_log: Option<&str>, target: &str, in_span: bool) -> bool {
        let filter = log_filter(rust_log);
        let subscriber = log_subscriber(&filter, logbuf::LogBuffer::new(), io::sink);
        tracing::subscriber::with_default(subscriber, || {
            let span = in_span.then(|| tracing::info_span!("connect").entered());
            let enabled = match target {
                "vte" => tracing::enabled!(target: "vte", tracing::Level::DEBUG),
                "vte::ansi" => tracing::enabled!(target: "vte::ansi", tracing::Level::DEBUG),
                "alacritty_terminal::term" => {
                    tracing::enabled!(target: "alacritty_terminal::term", tracing::Level::DEBUG)
                }
                "vte::warn" => tracing::enabled!(target: "vte", tracing::Level::WARN),
                "hoprs_app" => tracing::enabled!(target: "hoprs_app", tracing::Level::DEBUG),
                other => panic!("no callsite for {other}"),
            };
            drop(span);
            enabled
        })
    }

    #[test]
    fn real_filter_never_logs_terminal_bytes() {
        let _lock = subscriber_test_lock();
        for rust_log in [
            None,
            Some("debug"),
            Some("trace"),
            Some("vte=trace"),
            Some("vte::ansi=debug,alacritty_terminal=trace"),
            Some("info,[connect]=trace"),
        ] {
            for target in ["vte", "vte::ansi", "alacritty_terminal::term"] {
                for in_span in [false, true] {
                    assert!(
                        !debug_enabled(rust_log, target, in_span),
                        "{rust_log:?} enables debug for {target} (span: {in_span})"
                    );
                }
            }
        }
        // Not vacuous: warnings still pass, and other targets debug.
        assert!(debug_enabled(Some("debug"), "vte::warn", false));
        assert!(debug_enabled(Some("debug"), "hoprs_app", false));
        assert!(debug_enabled(
            Some("info,[connect]=trace"),
            "hoprs_app",
            true
        ));
        // The span case needs the per-target cap: `EnvFilter` alone lets
        // a span directive through for every target.
        use tracing_subscriber::layer::SubscriberExt;
        let directives = log_filter(Some("info,[connect]=trace")).directives;
        let env_only =
            tracing_subscriber::registry().with(tracing_subscriber::EnvFilter::new(directives));
        tracing::subscriber::with_default(env_only, || {
            let _span = tracing::info_span!("connect").entered();
            assert!(tracing::enabled!(target: "vte", tracing::Level::DEBUG));
        });
    }

    /// The panel's buffer sits behind the terminal's filter: the suppaftp cap (at
    /// trace it logs `PASS <password>`), the terminal crates' cap (span directives
    /// included) and the levels `RUST_LOG` asks for.
    #[test]
    fn layer_respects_the_filter_and_suppaftp_cap() {
        let _lock = subscriber_test_lock();
        let emit = || {
            tracing::trace!(target: "suppaftp", "PASS hunter2");
            tracing::trace!(target: "suppaftp::async_ftp", "PASS hunter3");
            tracing::debug!(target: "suppaftp", "suppaftp-debug");
            tracing::warn!(target: "suppaftp", "suppaftp-warn");
            tracing::debug!(target: "vte", "vte-bytes");
            tracing::warn!(target: "vte", "vte-warn");
            tracing::debug!(target: "alacritty_terminal::term", "alacritty-bytes");
            tracing::trace!(target: "hoprs_app", "app-trace");
            tracing::debug!(target: "hoprs_app", "app-debug");
            tracing::info!(target: "hoprs_app", "app-info");
            let _span = tracing::info_span!("connect").entered();
            tracing::debug!(target: "vte", "vte-in-span");
            tracing::trace!(target: "suppaftp", "PASS in-span");
            tracing::debug!(target: "hoprs_app", "app-in-span");
        };
        let logged = |rust_log: Option<&str>| {
            let (buffer, capture) = capture_log(&log_filter(rust_log));
            emit();
            drop(capture);
            buffer.text()
        };

        for rust_log in [
            Some("trace"),
            Some("suppaftp=trace,trace"),
            Some("info,[connect]=trace"),
            Some("vte=trace,alacritty_terminal=trace,debug"),
        ] {
            let text = logged(rust_log);
            for never in [
                "hunter2",
                "hunter3",
                "PASS",
                "vte-bytes",
                "alacritty-bytes",
                "vte-in-span",
            ] {
                assert!(!text.contains(never), "{rust_log:?} logs {never}: {text}");
            }
            assert!(text.contains("WARN suppaftp suppaftp-warn"), "{text}");
            assert!(text.contains("WARN vte vte-warn"), "{text}");
        }
        // An explicit RUST_LOG still decides, within the caps.
        let text = logged(Some("trace"));
        assert!(text.contains("TRACE hoprs_app app-trace"), "{text}");
        assert!(
            !text.contains("suppaftp-debug"),
            "suppaftp keeps warn: {text}"
        );
        let text = logged(Some("info,suppaftp=debug"));
        assert!(text.contains("DEBUG suppaftp suppaftp-debug"), "{text}");
        assert!(!text.contains("app-debug"), "{text}");
        assert!(text.contains("INFO hoprs_app app-info"), "{text}");
        let text = logged(Some("info,[connect]=trace"));
        assert!(text.contains("app-in-span"), "{text}");
        assert!(!text.contains("app-debug"), "{text}");
        // The default: info and up, suppaftp at warn.
        let text = logged(None);
        assert!(text.contains("INFO hoprs_app app-info"), "{text}");
        assert!(text.contains("suppaftp-warn"), "{text}");
        for never in [
            "app-debug",
            "app-trace",
            "suppaftp-debug",
            "PASS",
            "vte-bytes",
        ] {
            assert!(!text.contains(never), "the default logs {never}: {text}");
        }
    }

    /// The default leaves out the graphics stack's info chatter; the app's own info
    /// lines stay, and so do the stack's errors.
    #[test]
    fn default_filter_quiets_graphics_stack() {
        let _lock = subscriber_test_lock();
        let (buffer, capture) = capture_log(&log_filter(None));
        tracing::info!(target: "wgpu_hal::vulkan::instance", "adapter chatter");
        tracing::warn!(target: "wgpu_hal", "wgpu-hal-warn");
        tracing::info!(target: "wgpu_core::instance", "wgpu-core-info");
        tracing::info!(target: "iced_winit", "iced-winit-info");
        tracing::info!(target: "iced_wgpu::window::compositor", "iced-wgpu-info");
        tracing::warn!(target: "sctk_adwaita::config", "No config found!");
        tracing::info!(target: "hoprs_app", "app-info");
        tracing::error!(target: "wgpu_hal", "wgpu-hal-error");
        tracing::warn!(target: "iced_winit", "iced-winit-warn");
        drop(capture);
        let text = buffer.text();
        for quiet in [
            "adapter chatter",
            "wgpu-hal-warn",
            "wgpu-core-info",
            "iced-winit-info",
            "iced-wgpu-info",
            "No config found!",
        ] {
            assert!(!text.contains(quiet), "{quiet} in {text}");
        }
        for kept in [
            "INFO hoprs_app app-info",
            "wgpu-hal-error",
            "iced-winit-warn",
        ] {
            assert!(text.contains(kept), "{kept} missing from {text}");
        }
        // An explicit RUST_LOG replaces the default: the stack logs again.
        let (buffer, capture) = capture_log(&log_filter(Some("info")));
        tracing::info!(target: "iced_winit", "iced-winit-info-explicit");
        drop(capture);
        assert!(buffer.text().contains("iced-winit-info-explicit"));
    }

    /// The capture sees the runtime's worker threads and its blocking pool, not only
    /// the test's thread.
    #[test]
    fn capture_sees_workers_and_blocking_threads() {
        let _lock = subscriber_test_lock();
        let (buffer, capture) = capture_log(&log_filter(Some("trace")));
        capture.runtime().block_on(log_canaries());
        tracing::info!(target: "hoprs_app", "main-thread-canary");
        drop(capture);
        assert_no_secrets(&buffer, &[]);
        assert!(buffer.text().contains("main-thread-canary"));
    }
}
