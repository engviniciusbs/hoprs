//! Plan 16 Task 3: the export screen (file destination), a centre form for one site, one
//! workspace or everything.

use std::path::PathBuf;

use chrono::NaiveDate;
use hoprs_vault::{ExportReport, ExportScope};

use super::*;
use crate::app::export::{ExportFlow, ExportMsg, ExportTitle};
use crate::app::settings::{BackupMsg, BackupState, SettingsMsg};

fn export(app: &mut App, msg: ExportMsg) -> Vec<Effect> {
    app.update(Message::Export(msg))
}

fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 22).unwrap()
}

fn flow(app: &App) -> &ExportFlow {
    app.export.as_ref().expect("an export flow")
}

fn typed(app: &mut App, password: &str, confirm: &str) {
    export(app, ExportMsg::Password(Password(password.into())));
    export(app, ExportMsg::Confirm(Password(confirm.into())));
}

fn report() -> ExportReport {
    ExportReport {
        sites: 1,
        workspaces: 0,
        secrets: 1,
        skipped_unreadable: Vec::new(),
        missing_secrets: Vec::new(),
    }
}

struct Fixture {
    app: App,
    fake: Arc<FakeBackend>,
    dir: tempfile::TempDir,
    a: Site,
    team: Workspace,
}

/// An app with "Cliente Acme" and "b", the workspace "Team" holding both, and its export
/// folder in a temporary directory.
async fn fixture() -> Fixture {
    let a = site("Cliente Acme", "a.example", "u", Protocol::Ssh);
    let b = site("b", "b.example", "u", Protocol::Ftp);
    let team = workspace("Team", &[&a.id, &b.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_site(b)
            .with_workspace(team.clone()),
    );
    let mut app = loaded(&fake).await;
    let dir = tempfile::tempdir().unwrap();
    app.export_dir = Some(dir.path().to_path_buf());
    Fixture {
        app,
        fake,
        dir,
        a,
        team,
    }
}

/// Opens the form for `scope`, picks `dir/picked.hoprs` and types `password` twice: the
/// next `Submit` starts at once.
fn ready_to_start(f: &mut Fixture, scope: ExportScope, password: &str) -> PathBuf {
    export(&mut f.app, ExportMsg::Open(scope));
    let picked = f.dir.path().join("picked.hoprs");
    export(&mut f.app, ExportMsg::Picked(Some(picked.clone())));
    typed(&mut f.app, password, password);
    picked
}

/// Starts an export of site `a` to a picked file; returns its effect.
fn started(f: &mut Fixture) -> Effect {
    let scope = ExportScope::Site(f.a.id.clone());
    ready_to_start(f, scope, "pw");
    let mut effects = export(&mut f.app, ExportMsg::Submit);
    assert!(
        matches!(effects[..], [Effect::ExportBackup { .. }]),
        "{effects:?}"
    );
    effects.remove(0)
}

#[test]
fn flow_open_prefills_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let flow = ExportFlow::open(
        ExportScope::Site(SiteId::new()),
        ExportTitle::Site {
            name: "Cliente Acme".into(),
        },
        Some(dir.path()),
        day(),
    );
    assert_eq!(flow.file_name, "hoprs-cliente-acme-2026-11-22.hoprs");
    assert_eq!(PathBuf::from(&flow.path), dir.path().join(&flow.file_name));
    assert!(!flow.path_picked && !flow.checking && !flow.confirm_overwrite);
    assert!(flow.password.0.is_empty() && flow.confirm.0.is_empty());
    assert_eq!(flow.field_error, None);
    assert_eq!(flow.state, BackupState::Idle);
    assert!(!flow.running());

    let all = ExportFlow::open(
        ExportScope::All,
        ExportTitle::All {
            sites: 2,
            workspaces: 1,
        },
        Some(dir.path()),
        day(),
    );
    assert_eq!(all.file_name, "hoprs-backup-2026-11-22.hoprs");
    assert_eq!(PathBuf::from(&all.path), dir.path().join(&all.file_name));

    let nowhere = ExportFlow::open(
        ExportScope::All,
        ExportTitle::All {
            sites: 0,
            workspaces: 0,
        },
        None,
        day(),
    );
    assert_eq!(nowhere.path, "");
    assert_eq!(nowhere.file_name, "hoprs-backup-2026-11-22.hoprs");
}

#[tokio::test]
async fn open_shows_the_form_for_each_scope() {
    let mut f = fixture().await;
    // Whatever form was shown, the export screen replaces it.
    f.app.update(Message::NewSite);
    let cases = [
        (
            ExportScope::Site(f.a.id.clone()),
            ExportTitle::Site {
                name: "Cliente Acme".into(),
            },
        ),
        (
            ExportScope::Workspace(f.team.id.clone()),
            ExportTitle::Workspace {
                name: "Team".into(),
                sites: 2,
            },
        ),
        (
            ExportScope::All,
            ExportTitle::All {
                sites: 2,
                workspaces: 1,
            },
        ),
    ];
    for (scope, title) in cases {
        let effects = export(&mut f.app, ExportMsg::Open(scope.clone()));
        assert!(effects.is_empty(), "{effects:?}");
        assert!(matches!(f.app.form, Some(Form::Export)));
        let flow = flow(&f.app);
        assert_eq!(flow.scope, scope);
        assert_eq!(flow.title, title);
        assert_eq!(
            PathBuf::from(&flow.path),
            f.dir.path().join(&flow.file_name)
        );
        let _ = crate::ui::export::view(&f.app);
    }
}

/// RF1: a site or workspace that is not in the profiles opens nothing.
#[tokio::test]
async fn open_for_a_missing_target_does_nothing() {
    let mut f = fixture().await;
    for scope in [
        ExportScope::Site(SiteId::new()),
        ExportScope::Workspace(WorkspaceId::new()),
    ] {
        assert!(export(&mut f.app, ExportMsg::Open(scope)).is_empty());
        assert!(f.app.form.is_none());
        assert!(f.app.export.is_none());
    }
    // An open form stays as it is.
    export(
        &mut f.app,
        ExportMsg::Open(ExportScope::Site(f.a.id.clone())),
    );
    typed(&mut f.app, "pw", "pw");
    export(
        &mut f.app,
        ExportMsg::Open(ExportScope::Site(SiteId::new())),
    );
    assert!(matches!(f.app.form, Some(Form::Export)));
    assert_eq!(flow(&f.app).scope, ExportScope::Site(f.a.id.clone()));
    assert_eq!(flow(&f.app).password.0, "pw");
}

#[tokio::test]
async fn validation_in_order() {
    let mut f = fixture().await;
    export(
        &mut f.app,
        ExportMsg::Open(ExportScope::Site(f.a.id.clone())),
    );
    typed(&mut f.app, "a", "b");

    export(&mut f.app, ExportMsg::Path("  ".into()));
    assert!(export(&mut f.app, ExportMsg::Submit).is_empty());
    assert_eq!(
        flow(&f.app).field_error,
        Some(strings::backup_path_required())
    );

    let path = f.dir.path().join("x.hoprs");
    export(
        &mut f.app,
        ExportMsg::Path(path.to_string_lossy().into_owned()),
    );
    typed(&mut f.app, "", "");
    assert!(export(&mut f.app, ExportMsg::Submit).is_empty());
    assert_eq!(
        flow(&f.app).field_error,
        Some(strings::backup_password_required())
    );

    typed(&mut f.app, "a", "b");
    assert!(export(&mut f.app, ExportMsg::Submit).is_empty());
    assert_eq!(
        flow(&f.app).field_error,
        Some(strings::backup_passwords_differ())
    );
    assert_eq!(flow(&f.app).state, BackupState::Idle);
    let _ = crate::ui::export::view(&f.app);
}

#[tokio::test]
async fn typed_path_is_checked_then_asks_before_overwriting() {
    let mut f = fixture().await;
    let scope = ExportScope::Workspace(f.team.id.clone());
    export(&mut f.app, ExportMsg::Open(scope.clone()));
    typed(&mut f.app, "pw", "pw");
    let path = PathBuf::from(&flow(&f.app).path);
    let effects = export(&mut f.app, ExportMsg::Submit);
    match &effects[..] {
        [Effect::CheckExportTarget(checked)] => assert_eq!(checked, &path),
        other => panic!("expected one CheckExportTarget, got {other:?}"),
    }
    assert_eq!(flow(&f.app).field_error, None);
    assert!(flow(&f.app).checking);
    assert!(!f.app.export_ready());
    // While it is checked, Submit does nothing more.
    assert!(export(&mut f.app, ExportMsg::Submit).is_empty());

    let checked = |app: &mut App, path: &PathBuf, exists: bool| {
        export(
            app,
            ExportMsg::TargetChecked {
                path: path.clone(),
                exists,
            },
        )
    };
    assert!(checked(&mut f.app, &path, true).is_empty());
    assert!(flow(&f.app).confirm_overwrite, "asked in place");
    assert_eq!(flow(&f.app).state, BackupState::Idle);
    assert!(!f.app.export_ready());
    assert!(export(&mut f.app, ExportMsg::Submit).is_empty());
    let _ = crate::ui::export::view(&f.app);

    // Keep: nothing written; the question goes away.
    assert!(export(&mut f.app, ExportMsg::CancelOverwrite).is_empty());
    assert!(!flow(&f.app).confirm_overwrite);
    assert!(export(&mut f.app, ExportMsg::ConfirmOverwrite).is_empty());

    // Again, and Overwrite.
    export(&mut f.app, ExportMsg::Submit);
    checked(&mut f.app, &path, true);
    let effects = export(&mut f.app, ExportMsg::ConfirmOverwrite);
    match &effects[..] {
        [
            Effect::ExportBackup {
                scope: sent,
                path: to,
                ..
            },
        ] => {
            assert_eq!(sent, &scope);
            assert_eq!(to, &path);
        }
        other => panic!("expected one ExportBackup, got {other:?}"),
    }
    assert!(!flow(&f.app).confirm_overwrite);

    // An answer for a path edited meanwhile is dropped; one that does not exist starts.
    let mut f = fixture().await;
    export(&mut f.app, ExportMsg::Open(ExportScope::All));
    typed(&mut f.app, "pw", "pw");
    let first = PathBuf::from(&flow(&f.app).path);
    export(&mut f.app, ExportMsg::Submit);
    let other = f.dir.path().join("other.hoprs");
    export(
        &mut f.app,
        ExportMsg::Path(other.to_string_lossy().into_owned()),
    );
    assert!(!flow(&f.app).checking);
    assert!(checked(&mut f.app, &first, true).is_empty());
    assert!(!flow(&f.app).confirm_overwrite);
    assert!(matches!(
        export(&mut f.app, ExportMsg::Submit)[..],
        [Effect::CheckExportTarget(_)]
    ));
    assert!(checked(&mut f.app, &first, false).is_empty());
    let effects = checked(&mut f.app, &other, false);
    match &effects[..] {
        [Effect::ExportBackup { scope, path, .. }] => {
            assert_eq!(scope, &ExportScope::All);
            assert_eq!(path, &other);
        }
        other => panic!("expected one ExportBackup, got {other:?}"),
    }
    assert!(flow(&f.app).running());
}

#[tokio::test]
async fn picked_path_skips_the_question() {
    let mut f = fixture().await;
    export(
        &mut f.app,
        ExportMsg::Open(ExportScope::Site(f.a.id.clone())),
    );
    let before = flow(&f.app).path.clone();
    let effects = export(&mut f.app, ExportMsg::Choose);
    match &effects[..] {
        [Effect::PickExportFile { name }] => {
            assert_eq!(name, &flow(&f.app).file_name);
            assert_eq!(name, &format!("hoprs-cliente-acme-{}.hoprs", today()));
        }
        other => panic!("expected one PickExportFile, got {other:?}"),
    }
    // Cancelled: nothing changes.
    export(&mut f.app, ExportMsg::Picked(None));
    assert_eq!(flow(&f.app).path, before);
    assert!(!flow(&f.app).path_picked);

    let picked = f.dir.path().join("picked.hoprs");
    export(&mut f.app, ExportMsg::Picked(Some(picked.clone())));
    assert_eq!(PathBuf::from(&flow(&f.app).path), picked);
    assert!(flow(&f.app).path_picked);
    export(&mut f.app, ExportMsg::Picked(None));
    assert!(flow(&f.app).path_picked);

    typed(&mut f.app, "pw", "pw");
    let effects = export(&mut f.app, ExportMsg::Submit);
    match &effects[..] {
        [Effect::ExportBackup { path, .. }] => assert_eq!(path, &picked),
        other => panic!("expected one ExportBackup, got {other:?}"),
    }
}

fn today() -> String {
    chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string()
}

#[tokio::test]
async fn password_leaves_the_form_with_the_effect() {
    let typed_message = Message::Export(ExportMsg::Password(Password("hunter2".into())));
    let confirm = ExportMsg::Confirm(Password("hunter3".into()));
    let debug = format!("{typed_message:?} {confirm:?}");
    assert!(!debug.contains("hunter"), "{debug}");

    let mut f = fixture().await;
    let scope = ExportScope::Site(f.a.id.clone());
    ready_to_start(&mut f, scope, "correct horse");
    assert!(format!("{:?}", f.app.export).contains("Password(***)"));
    let effects = export(&mut f.app, ExportMsg::Submit);
    match &effects[..] {
        [Effect::ExportBackup { password, .. }] => {
            assert_eq!(password.expose_secret(), "correct horse");
        }
        other => panic!("expected one ExportBackup, got {other:?}"),
    }
    assert!(!format!("{effects:?}").contains("correct horse"));
    let flow = flow(&f.app);
    assert!(flow.password.0.is_empty());
    assert!(flow.confirm.0.is_empty());
    assert_eq!(flow.state, BackupState::Running);
    assert!(!format!("{:?}", f.app.export).contains("correct horse"));
    assert!(f.app.busy());
    let _ = crate::ui::export::view(&f.app);
}

#[tokio::test]
async fn one_export_at_a_time_and_not_during_a_restore() {
    let mut f = fixture().await;
    started(&mut f);
    typed(&mut f.app, "again", "again");
    assert!(!f.app.export_ready());
    assert!(export(&mut f.app, ExportMsg::Submit).is_empty());

    // A restore does not start while the export runs.
    f.app.update(Message::OpenSettings);
    let restore = |app: &mut App, msg| app.update(Message::Settings(SettingsMsg::Backup(msg)));
    restore(&mut f.app, BackupMsg::OpenRestore);
    restore(&mut f.app, BackupMsg::RestorePath("/b.hoprs".into()));
    restore(
        &mut f.app,
        BackupMsg::RestorePassword(Password("pw".into())),
    );
    let effects = restore(&mut f.app, BackupMsg::Restore);
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::RestoreBackup { .. })),
        "{effects:?}"
    );
    assert_eq!(f.app.backup.restore, BackupState::Idle);
    // Replace asks nothing while the export runs: a confirmed Replace could not start.
    restore(
        &mut f.app,
        BackupMsg::RestoreMode(hoprs_vault::ImportMode::Replace),
    );
    let effects = restore(&mut f.app, BackupMsg::Restore);
    assert!(effects.is_empty(), "{effects:?}");
    assert!(!f.app.backup.confirm_replace);
    assert_eq!(f.app.backup.restore, BackupState::Idle);

    // An export does not start while a restore runs.
    let mut f = fixture().await;
    let scope = ExportScope::Site(f.a.id.clone());
    ready_to_start(&mut f, scope, "pw");
    assert!(f.app.export_ready());
    f.app.backup.restore = BackupState::Running;
    assert!(!f.app.export_ready());
    assert!(export(&mut f.app, ExportMsg::Submit).is_empty());
    assert_eq!(flow(&f.app).state, BackupState::Idle);
}

#[tokio::test]
async fn result_shows_report_or_error() {
    let mut f = fixture().await;
    started(&mut f);
    assert!(export(&mut f.app, ExportMsg::Done(Ok(report()))).is_empty());
    assert_eq!(flow(&f.app).state, BackupState::Done(report()));
    assert!(matches!(f.app.form, Some(Form::Export)));
    assert!(!f.app.busy());
    let _ = crate::ui::export::view(&f.app);

    let mut f = fixture().await;
    started(&mut f);
    let error = Error::new(ErrorKind::Io, "disk full");
    export(&mut f.app, ExportMsg::Done(Err(error.clone())));
    assert_eq!(flow(&f.app).state, BackupState::Failed(error));
    let _ = crate::ui::export::view(&f.app);
}

/// RF1: the site was deleted between opening the form and the export: the backend's
/// error shows inline, the form stays. A result while nothing runs is ignored.
#[tokio::test]
async fn export_of_a_vanished_target_fails_inline() {
    let mut f = fixture().await;
    let error = Error::new(ErrorKind::NotFound, "site not found");
    f.fake.fail_next_export(error.clone());
    let Effect::ExportBackup {
        scope,
        path,
        password,
    } = started(&mut f)
    else {
        unreachable!()
    };
    let backend: Arc<dyn BackendApi> = f.fake.clone();
    let message = effects::export_backup(backend, scope, path, password).await;
    assert!(
        matches!(&message, Message::Export(ExportMsg::Done(Err(e))) if e.kind == ErrorKind::NotFound),
        "{message:?}"
    );
    f.app.update(message);
    assert_eq!(flow(&f.app).state, BackupState::Failed(error.clone()));
    assert!(matches!(f.app.form, Some(Form::Export)));

    // Not running: ignored.
    export(&mut f.app, ExportMsg::Done(Ok(report())));
    assert_eq!(flow(&f.app).state, BackupState::Failed(error));
    let mut f = fixture().await;
    export(&mut f.app, ExportMsg::Open(ExportScope::All));
    export(&mut f.app, ExportMsg::Done(Ok(report())));
    assert_eq!(flow(&f.app).state, BackupState::Idle);
    let mut app = App::new();
    assert!(
        app.update(Message::Export(ExportMsg::Done(Ok(report()))))
            .is_empty()
    );
    assert!(app.export.is_none());
}

#[tokio::test]
async fn leaving_the_form_drops_an_idle_flow() {
    let mut f = fixture().await;
    for leave in [
        Message::ConnectSite(f.a.id.clone()),
        Message::OpenSettings,
        Message::Export(ExportMsg::Close),
    ] {
        export(
            &mut f.app,
            ExportMsg::Open(ExportScope::Site(f.a.id.clone())),
        );
        typed(&mut f.app, "e", "e");
        assert!(f.app.export.is_some());
        f.app.update(leave.clone());
        assert!(!matches!(f.app.form, Some(Form::Export)), "{leave:?}");
        assert!(f.app.export.is_none(), "{leave:?}");
    }
}

#[tokio::test]
async fn leaving_while_running_keeps_the_flow_without_passwords() {
    let mut f = fixture().await;
    started(&mut f);
    typed(&mut f.app, "late", "late");
    f.app.update(Message::OpenSettings);
    let flow_now = flow(&f.app);
    assert!(flow_now.running());
    assert!(flow_now.password.0.is_empty() && flow_now.confirm.0.is_empty());
    assert!(!flow_now.checking && !flow_now.confirm_overwrite);
    assert!(f.app.busy());

    // Opening the form again shows the running export, whatever scope was asked.
    export(
        &mut f.app,
        ExportMsg::Open(ExportScope::Workspace(f.team.id.clone())),
    );
    assert!(matches!(f.app.form, Some(Form::Export)));
    assert_eq!(flow(&f.app).scope, ExportScope::Site(f.a.id.clone()));
    assert!(flow(&f.app).running());

    // Left again: the result arrives with the form gone and drops the flow.
    f.app.update(Message::NewSite);
    assert!(f.app.export.is_some());
    export(&mut f.app, ExportMsg::Done(Ok(report())));
    assert!(f.app.export.is_none());
    assert!(!f.app.busy());
}

#[tokio::test]
async fn export_input_ignored_while_closing_or_behind_a_dialog() {
    for msg in [
        ExportMsg::Open(ExportScope::All),
        ExportMsg::Path(String::new()),
        ExportMsg::Choose,
        ExportMsg::Password(Password::default()),
        ExportMsg::Confirm(Password::default()),
        ExportMsg::Submit,
        ExportMsg::ConfirmOverwrite,
        ExportMsg::CancelOverwrite,
        ExportMsg::Close,
    ] {
        assert!(msg.is_user(), "{msg:?}");
    }
    for msg in [
        ExportMsg::Picked(None),
        ExportMsg::TargetChecked {
            path: PathBuf::new(),
            exists: false,
        },
        ExportMsg::Done(Ok(report())),
    ] {
        assert!(!msg.is_user(), "{msg:?}");
    }

    // Behind the palette: input does nothing, a result still lands.
    let mut f = fixture().await;
    started(&mut f);
    f.app.update(Message::Palette(PaletteMsg::Open));
    assert!(f.app.palette.is_some());
    export(&mut f.app, ExportMsg::Path("/elsewhere".into()));
    assert_ne!(flow(&f.app).path, "/elsewhere");
    export(&mut f.app, ExportMsg::Close);
    assert!(matches!(f.app.form, Some(Form::Export)));
    export(&mut f.app, ExportMsg::Done(Ok(report())));
    assert_eq!(flow(&f.app).state, BackupState::Done(report()));
    f.app.update(Message::Palette(PaletteMsg::Close));

    // While the window closes: nothing opens or changes.
    let mut f = fixture().await;
    f.app.closing = true;
    assert!(export(&mut f.app, ExportMsg::Open(ExportScope::All)).is_empty());
    assert!(f.app.export.is_none() && f.app.form.is_none());
    let mut f = fixture().await;
    export(&mut f.app, ExportMsg::Open(ExportScope::All));
    let path = flow(&f.app).path.clone();
    f.app.closing = true;
    export(&mut f.app, ExportMsg::Path("/elsewhere".into()));
    typed(&mut f.app, "pw", "pw");
    assert!(export(&mut f.app, ExportMsg::Submit).is_empty());
    assert_eq!(flow(&f.app).path, path);
    assert!(flow(&f.app).password.0.is_empty());
}

// Plan 16 Task 11: the Google Drive destination (connect, seal, upload, fall back to a
// file).

use crate::app::export::{Destination, DriveExport, DriveLink};
use crate::drive::{ConnectEvent, DriveAccount, DriveFile, Sealed};

/// An authorisation URL as the backend builds it; its `state` must never show in a
/// `Debug`.
const AUTH_URL: &str =
    "https://accounts.google.com/o/oauth2/v2/auth?client_id=c&state=st4te-n0nce&code_challenge=x";
const STATE: &str = "st4te-n0nce";

fn account() -> DriveAccount {
    DriveAccount {
        email: Some("me@example.com".into()),
        stored: true,
    }
}

fn coded(code: &'static str) -> Error {
    Error::coded(ErrorKind::Io, code, code)
}

fn uploaded_file() -> DriveFile {
    DriveFile {
        name: "n.hoprs".into(),
        link: Some("https://drive.google.com/file/d/x/view".into()),
    }
}

/// [`fixture`] with an OAuth client in the build.
async fn drive_fixture() -> Fixture {
    let mut f = fixture().await;
    f.app.drive_available = true;
    f
}

/// The form for site `a` with the Google Drive destination picked; the keyring answer
/// not back yet.
fn on_drive(f: &mut Fixture) -> Vec<Effect> {
    export(
        &mut f.app,
        ExportMsg::Open(ExportScope::Site(f.a.id.clone())),
    );
    export(&mut f.app, ExportMsg::Destination(Destination::Drive))
}

/// The Drive destination, connected (the keyring had the entry).
fn connected(f: &mut Fixture) {
    on_drive(f);
    export(&mut f.app, ExportMsg::DriveStatus(Ok(Some(account()))));
    assert_eq!(f.app.drive_link, DriveLink::Connected(account()));
    // The marker was saved: its save is over.
    f.app.update(Message::SettingsSaved(Ok(())));
}

/// Connected, the passwords typed, Export pressed: sealing.
fn sealing(f: &mut Fixture) {
    connected(f);
    typed(&mut f.app, "pw", "pw");
    let effects = export(&mut f.app, ExportMsg::Submit);
    assert!(
        matches!(effects[..], [Effect::SealExport { .. }]),
        "{effects:?}"
    );
}

/// Sealed: the upload runs; returns the bytes the upload got.
fn uploading(f: &mut Fixture) -> Sealed {
    sealing(f);
    let effects = export(
        &mut f.app,
        ExportMsg::SealDone(Ok((report(), Sealed::new(b"sealed".to_vec())))),
    );
    match &effects[..] {
        [Effect::DriveUpload { sealed, .. }] => sealed.clone(),
        other => panic!("expected one DriveUpload, got {other:?}"),
    }
}

/// The upload failed: the bytes are kept.
fn failed_upload(f: &mut Fixture) -> Sealed {
    let sealed = uploading(f);
    export(
        &mut f.app,
        ExportMsg::UploadDone(Err(Error::new(ErrorKind::Network, "down"))),
    );
    assert!(matches!(flow(&f.app).drive, DriveExport::Failed(_)));
    sealed
}

fn kept_bytes(app: &App) -> &Sealed {
    &flow(app).sealed.as_ref().expect("the sealed bytes").1
}

fn has_drive_status(effects: &[Effect]) -> bool {
    effects.iter().any(|e| matches!(e, Effect::DriveStatus))
}

fn saves_settings(effects: &[Effect]) -> bool {
    effects.iter().any(|e| matches!(e, Effect::SaveSettings(_)))
}

#[tokio::test]
async fn drive_is_not_offered_without_a_client() {
    let mut f = fixture().await;
    f.app.drive_available = false;
    let effects = on_drive(&mut f);
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(flow(&f.app).destination, Destination::File);
    assert_eq!(f.app.drive_link, DriveLink::Unknown);
}

/// RF5: the keyring is read only when the Google Drive destination is picked, once.
#[tokio::test]
async fn opening_the_form_never_reads_the_keyring() {
    let mut f = drive_fixture().await;
    let mut effects = Vec::new();
    for scope in [
        ExportScope::Site(f.a.id.clone()),
        ExportScope::Workspace(f.team.id.clone()),
        ExportScope::All,
    ] {
        effects.extend(export(&mut f.app, ExportMsg::Open(scope)));
    }
    let picked = f.dir.path().join("p.hoprs");
    for msg in [
        ExportMsg::Destination(Destination::File),
        ExportMsg::Path("x".into()),
        ExportMsg::Choose,
        ExportMsg::Picked(Some(picked)),
        ExportMsg::Password(Password("pw".into())),
        ExportMsg::Confirm(Password("pw".into())),
        ExportMsg::Submit,
        ExportMsg::Done(Ok(report())),
        ExportMsg::Close,
    ] {
        effects.extend(export(&mut f.app, msg));
    }
    assert!(!has_drive_status(&effects), "{effects:?}");
    assert_eq!(f.app.drive_link, DriveLink::Unknown);

    let effects = on_drive(&mut f);
    assert!(matches!(effects[..], [Effect::DriveStatus]), "{effects:?}");
    assert_eq!(f.app.drive_link, DriveLink::Reading);
    assert_eq!(flow(&f.app).destination, Destination::Drive);
    // Back and forth while it is read: asked once.
    assert!(export(&mut f.app, ExportMsg::Destination(Destination::File)).is_empty());
    assert!(export(&mut f.app, ExportMsg::Destination(Destination::Drive)).is_empty());
    export(&mut f.app, ExportMsg::DriveStatus(Ok(None)));
    assert_eq!(f.app.drive_link, DriveLink::Disconnected);
    assert!(export(&mut f.app, ExportMsg::Destination(Destination::File)).is_empty());
    assert!(export(&mut f.app, ExportMsg::Destination(Destination::Drive)).is_empty());
    // The link outlives the form (ruling 11).
    export(&mut f.app, ExportMsg::Close);
    let effects = on_drive(&mut f);
    assert!(!has_drive_status(&effects), "{effects:?}");
    assert_eq!(f.app.drive_link, DriveLink::Disconnected);
}

/// RF5: a marker whose keyring entry is gone is cleared; an unreadable keyring leaves
/// the marker alone.
#[tokio::test]
async fn stale_marker_is_cleared_when_the_entry_is_gone() {
    let mut f = drive_fixture().await;
    f.app.settings.google_drive = true;
    on_drive(&mut f);
    let effects = export(&mut f.app, ExportMsg::DriveStatus(Ok(None)));
    assert_eq!(f.app.drive_link, DriveLink::Disconnected);
    assert!(!f.app.settings.google_drive);
    assert!(saves_settings(&effects), "{effects:?}");
    assert_eq!(f.app.drive_error, None);

    let mut f = drive_fixture().await;
    f.app.settings.google_drive = true;
    on_drive(&mut f);
    let error = Error::new(ErrorKind::Io, "keyring locked");
    let effects = export(&mut f.app, ExportMsg::DriveStatus(Err(error.clone())));
    assert_eq!(f.app.drive_link, DriveLink::Disconnected);
    assert_eq!(f.app.drive_error, Some(error));
    assert!(f.app.settings.google_drive, "marker untouched");
    assert!(!saves_settings(&effects), "{effects:?}");
}

/// After an unreadable keyring, picking the destination again asks again.
#[tokio::test]
async fn unreadable_keyring_is_asked_again() {
    let mut f = drive_fixture().await;
    on_drive(&mut f);
    export(
        &mut f.app,
        ExportMsg::DriveStatus(Err(Error::new(ErrorKind::Io, "locked"))),
    );
    export(&mut f.app, ExportMsg::Destination(Destination::File));
    let effects = export(&mut f.app, ExportMsg::Destination(Destination::Drive));
    assert!(matches!(effects[..], [Effect::DriveStatus]), "{effects:?}");
    assert_eq!(f.app.drive_link, DriveLink::Reading);
    assert_eq!(f.app.drive_error, None);
    // The form left and opened again: still asked, as it never answered.
    export(
        &mut f.app,
        ExportMsg::DriveStatus(Err(Error::new(ErrorKind::Io, "locked"))),
    );
    export(&mut f.app, ExportMsg::Close);
    let effects = on_drive(&mut f);
    assert!(matches!(effects[..], [Effect::DriveStatus]), "{effects:?}");
    export(&mut f.app, ExportMsg::DriveStatus(Ok(Some(account()))));
    assert_eq!(f.app.drive_link, DriveLink::Connected(account()));
    export(&mut f.app, ExportMsg::Destination(Destination::File));
    assert!(export(&mut f.app, ExportMsg::Destination(Destination::Drive)).is_empty());

    // Connecting is still possible after a failed read (session only if need be).
    let mut f = drive_fixture().await;
    on_drive(&mut f);
    export(
        &mut f.app,
        ExportMsg::DriveStatus(Err(Error::new(ErrorKind::Io, "locked"))),
    );
    let effects = export(&mut f.app, ExportMsg::Connect);
    assert!(matches!(effects[..], [Effect::DriveConnect]), "{effects:?}");
    assert_eq!(f.app.drive_error, None);
}

#[tokio::test]
async fn connect_waits_then_connects() {
    let mut f = drive_fixture().await;
    on_drive(&mut f);
    export(&mut f.app, ExportMsg::DriveStatus(Ok(None)));
    let effects = export(&mut f.app, ExportMsg::Connect);
    assert!(matches!(effects[..], [Effect::DriveConnect]), "{effects:?}");
    assert_eq!(f.app.drive_link, DriveLink::Waiting { url: None });
    let effects = export(
        &mut f.app,
        ExportMsg::Connecting(ConnectEvent::Waiting {
            url: AUTH_URL.into(),
        }),
    );
    match &effects[..] {
        [Effect::OpenDrivePage(url)] => assert_eq!(url.0, AUTH_URL),
        other => panic!("expected one OpenDrivePage, got {other:?}"),
    }
    assert_eq!(
        f.app.drive_link,
        DriveLink::Waiting {
            url: Some(AUTH_URL.into())
        }
    );
    // "Copy the link".
    let effects = export(&mut f.app, ExportMsg::CopyLink);
    match &effects[..] {
        [Effect::WriteClipboard(text)] => assert_eq!(text.0, AUTH_URL),
        other => panic!("expected one WriteClipboard, got {other:?}"),
    }
    // The browser did not open: said inline.
    let error = Error::new(ErrorKind::Io, "xdg-open failed");
    export(&mut f.app, ExportMsg::PageOpened(Err(error.clone())));
    assert_eq!(f.app.drive_error, Some(error));

    let effects = export(
        &mut f.app,
        ExportMsg::Connecting(ConnectEvent::Done(Ok(account()))),
    );
    assert_eq!(f.app.drive_link, DriveLink::Connected(account()));
    assert!(f.app.settings.google_drive);
    assert!(saves_settings(&effects), "{effects:?}");
    assert_eq!(f.app.drive_error, None);
    assert_eq!(flow(&f.app).drive_name, flow(&f.app).file_name);
}

/// No `Debug` shows the authorisation URL (it holds the `state`).
#[tokio::test]
async fn debug_never_shows_the_url() {
    let link = DriveLink::Waiting {
        url: Some(AUTH_URL.into()),
    };
    let message = Message::Export(ExportMsg::Connecting(ConnectEvent::Waiting {
        url: AUTH_URL.into(),
    }));
    let mut f = drive_fixture().await;
    on_drive(&mut f);
    export(&mut f.app, ExportMsg::DriveStatus(Ok(None)));
    export(&mut f.app, ExportMsg::Connect);
    let opened = f.app.update(message.clone());
    let copied = export(&mut f.app, ExportMsg::CopyLink);
    let debug = format!("{link:?} {message:?} {opened:?} {copied:?} {:?}", f.app);
    assert!(!debug.contains(STATE), "{debug}");
    assert!(format!("{link:?}").contains("Waiting"));
}

/// RF4: a second click on Connect while it waits starts nothing.
#[tokio::test]
async fn second_connect_click_is_ignored() {
    let mut f = drive_fixture().await;
    on_drive(&mut f);
    export(&mut f.app, ExportMsg::DriveStatus(Ok(None)));
    export(&mut f.app, ExportMsg::Connect);
    assert!(export(&mut f.app, ExportMsg::Connect).is_empty());
    export(
        &mut f.app,
        ExportMsg::Connecting(ConnectEvent::Waiting {
            url: AUTH_URL.into(),
        }),
    );
    assert!(export(&mut f.app, ExportMsg::Connect).is_empty());
    // Not from Reading or Connected either.
    let mut f = drive_fixture().await;
    on_drive(&mut f);
    assert!(export(&mut f.app, ExportMsg::Connect).is_empty());
    export(&mut f.app, ExportMsg::DriveStatus(Ok(Some(account()))));
    assert!(export(&mut f.app, ExportMsg::Connect).is_empty());
}

#[tokio::test]
async fn cancel_and_timeout_states() {
    let mut f = drive_fixture().await;
    on_drive(&mut f);
    export(&mut f.app, ExportMsg::DriveStatus(Ok(None)));
    export(&mut f.app, ExportMsg::Connect);
    export(
        &mut f.app,
        ExportMsg::Connecting(ConnectEvent::Waiting {
            url: AUTH_URL.into(),
        }),
    );
    let effects = export(&mut f.app, ExportMsg::CancelConnect);
    assert!(matches!(effects[..], [Effect::DriveCancel]), "{effects:?}");
    assert_eq!(f.app.drive_link, DriveLink::Disconnected);
    let effects = export(
        &mut f.app,
        ExportMsg::Connecting(ConnectEvent::Done(Err(coded("drive-connect-cancelled")))),
    );
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(f.app.drive_link, DriveLink::Disconnected);
    assert_eq!(f.app.drive_error, None);
    // Nothing to cancel any more.
    assert!(export(&mut f.app, ExportMsg::CancelConnect).is_empty());

    // The browser never answered.
    export(&mut f.app, ExportMsg::Connect);
    let timed_out = coded("drive-connect-timed-out");
    export(
        &mut f.app,
        ExportMsg::Connecting(ConnectEvent::Done(Err(timed_out.clone()))),
    );
    assert_eq!(f.app.drive_link, DriveLink::Disconnected);
    assert_eq!(f.app.drive_error, Some(timed_out.clone()));
    // A late failure while not waiting is ignored.
    export(
        &mut f.app,
        ExportMsg::Connecting(ConnectEvent::Done(Err(coded("drive-connect-denied")))),
    );
    assert_eq!(f.app.drive_error, Some(timed_out));
}

/// A cancel clicked before the backend's wait began is not lost: the `Waiting` that
/// arrives afterwards is cancelled at once, and the browser is not opened.
#[tokio::test]
async fn early_cancel_is_sent_again_once_the_wait_begins() {
    let mut f = drive_fixture().await;
    on_drive(&mut f);
    export(&mut f.app, ExportMsg::DriveStatus(Ok(None)));
    export(&mut f.app, ExportMsg::Connect);
    let effects = export(&mut f.app, ExportMsg::CancelConnect);
    assert!(matches!(effects[..], [Effect::DriveCancel]), "{effects:?}");
    assert_eq!(f.app.drive_link, DriveLink::Disconnected);
    let effects = export(
        &mut f.app,
        ExportMsg::Connecting(ConnectEvent::Waiting {
            url: AUTH_URL.into(),
        }),
    );
    assert!(matches!(effects[..], [Effect::DriveCancel]), "{effects:?}");
    assert_eq!(f.app.drive_link, DriveLink::Disconnected);
    export(
        &mut f.app,
        ExportMsg::Connecting(ConnectEvent::Done(Err(coded("drive-connect-cancelled")))),
    );
    assert_eq!(f.app.drive_link, DriveLink::Disconnected);
    assert_eq!(f.app.drive_error, None);
}

/// RF4: a success that raced the cancel is kept.
#[tokio::test]
async fn late_success_after_cancel_is_kept() {
    let mut f = drive_fixture().await;
    on_drive(&mut f);
    export(&mut f.app, ExportMsg::DriveStatus(Ok(None)));
    export(&mut f.app, ExportMsg::Connect);
    export(
        &mut f.app,
        ExportMsg::Connecting(ConnectEvent::Waiting {
            url: AUTH_URL.into(),
        }),
    );
    export(&mut f.app, ExportMsg::CancelConnect);
    let effects = export(
        &mut f.app,
        ExportMsg::Connecting(ConnectEvent::Done(Ok(account()))),
    );
    assert_eq!(f.app.drive_link, DriveLink::Connected(account()));
    assert!(f.app.settings.google_drive);
    assert!(saves_settings(&effects), "{effects:?}");
}

#[tokio::test]
async fn leaving_while_waiting_cancels() {
    let mut f = drive_fixture().await;
    on_drive(&mut f);
    export(&mut f.app, ExportMsg::DriveStatus(Ok(None)));
    export(&mut f.app, ExportMsg::Connect);
    export(
        &mut f.app,
        ExportMsg::Connecting(ConnectEvent::Waiting {
            url: AUTH_URL.into(),
        }),
    );
    let effects = f.app.update(Message::ConnectSite(f.a.id.clone()));
    assert!(
        effects.iter().any(|e| matches!(e, Effect::DriveCancel)),
        "{effects:?}"
    );
    assert_eq!(f.app.drive_link, DriveLink::Disconnected);
    assert!(f.app.export.is_none());
    // Once only.
    let effects = f.app.update(Message::OpenSettings);
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::DriveCancel)),
        "{effects:?}"
    );
}

#[tokio::test]
async fn export_to_drive_seals_then_uploads() {
    let mut f = drive_fixture().await;
    connected(&mut f);
    export(&mut f.app, ExportMsg::DriveName("my backup.hoprs".into()));
    typed(&mut f.app, "pw", "pw");
    let effects = export(&mut f.app, ExportMsg::Submit);
    match &effects[..] {
        [Effect::SealExport { scope, password }] => {
            assert_eq!(scope, &ExportScope::Site(f.a.id.clone()));
            assert_eq!(password.expose_secret(), "pw");
        }
        other => panic!("expected one SealExport, got {other:?}"),
    }
    assert!(matches!(flow(&f.app).drive, DriveExport::Sealing));
    assert!(flow(&f.app).password.0.is_empty() && flow(&f.app).confirm.0.is_empty());
    assert!(flow(&f.app).running());
    assert!(f.app.busy());
    // The file destination's state is untouched.
    assert_eq!(flow(&f.app).state, BackupState::Idle);

    let effects = export(
        &mut f.app,
        ExportMsg::SealDone(Ok((report(), Sealed::new(b"sealed".to_vec())))),
    );
    match &effects[..] {
        [Effect::DriveUpload { name, sealed }] => {
            assert_eq!(name, "my backup.hoprs");
            assert_eq!(sealed.bytes(), b"sealed");
        }
        other => panic!("expected one DriveUpload, got {other:?}"),
    }
    assert!(matches!(flow(&f.app).drive, DriveExport::Uploading));
    assert!(flow(&f.app).sealed.is_some());

    let effects = export(&mut f.app, ExportMsg::UploadDone(Ok(uploaded_file())));
    assert!(effects.is_empty(), "{effects:?}");
    match &flow(&f.app).drive {
        DriveExport::Uploaded { report: got, file } => {
            assert_eq!(got, &report());
            assert_eq!(file, &uploaded_file());
        }
        other => panic!("expected Uploaded, got {other:?}"),
    }
    assert!(flow(&f.app).sealed.is_none());
    assert!(!flow(&f.app).running());
    // "Open in Drive".
    let effects = export(&mut f.app, ExportMsg::OpenInDrive);
    match &effects[..] {
        [Effect::OpenDrivePage(url)] => assert_eq!(Some(url.0.clone()), uploaded_file().link),
        other => panic!("expected one OpenDrivePage, got {other:?}"),
    }

    // A blank name.
    let mut f = drive_fixture().await;
    connected(&mut f);
    export(&mut f.app, ExportMsg::DriveName("   ".into()));
    typed(&mut f.app, "pw", "pw");
    assert!(export(&mut f.app, ExportMsg::Submit).is_empty());
    assert_eq!(
        flow(&f.app).field_error,
        Some(strings::drive_name_required())
    );
    // Then the passwords, as for a file.
    export(&mut f.app, ExportMsg::DriveName("n".into()));
    typed(&mut f.app, "a", "b");
    assert!(export(&mut f.app, ExportMsg::Submit).is_empty());
    assert_eq!(
        flow(&f.app).field_error,
        Some(strings::backup_passwords_differ())
    );

    // Not connected: nothing.
    let mut f = drive_fixture().await;
    on_drive(&mut f);
    export(&mut f.app, ExportMsg::DriveStatus(Ok(None)));
    typed(&mut f.app, "pw", "pw");
    assert!(export(&mut f.app, ExportMsg::Submit).is_empty());
    assert!(matches!(flow(&f.app).drive, DriveExport::Idle));
}

#[tokio::test]
async fn upload_failure_offers_try_again_and_save() {
    let mut f = drive_fixture().await;
    let sent = failed_upload(&mut f);
    assert!(std::ptr::eq(kept_bytes(&f.app).bytes(), sent.bytes()));
    assert!(!flow(&f.app).running());

    let effects = export(&mut f.app, ExportMsg::TryAgain);
    match &effects[..] {
        [Effect::DriveUpload { name, sealed }] => {
            assert_eq!(name, &flow(&f.app).drive_name);
            assert!(std::ptr::eq(sealed.bytes(), sent.bytes()));
        }
        other => panic!("expected one DriveUpload, got {other:?}"),
    }
    assert!(matches!(flow(&f.app).drive, DriveExport::Uploading));
    export(
        &mut f.app,
        ExportMsg::UploadDone(Err(Error::new(ErrorKind::Network, "down"))),
    );

    let effects = export(&mut f.app, ExportMsg::SaveLocal);
    match &effects[..] {
        [Effect::PickSealedTarget { name }] => assert_eq!(name, &flow(&f.app).drive_name),
        other => panic!("expected one PickSealedTarget, got {other:?}"),
    }
    assert!(export(&mut f.app, ExportMsg::SavePicked(None)).is_empty());
    assert!(matches!(flow(&f.app).drive, DriveExport::Failed(_)));
    let path = f.dir.path().join("saved.hoprs");
    let effects = export(&mut f.app, ExportMsg::SavePicked(Some(path.clone())));
    match &effects[..] {
        [Effect::SaveSealed { path: to, sealed }] => {
            assert_eq!(to, &path);
            assert!(std::ptr::eq(sealed.bytes(), sent.bytes()));
        }
        other => panic!("expected one SaveSealed, got {other:?}"),
    }
    assert!(matches!(flow(&f.app).drive, DriveExport::Saving));
    assert!(flow(&f.app).running());

    // The save failed: back to the failure, the bytes still there.
    let error = Error::new(ErrorKind::Io, "disk full");
    export(
        &mut f.app,
        ExportMsg::SaveDone {
            path: path.clone(),
            result: Err(error.clone()),
        },
    );
    assert!(matches!(&flow(&f.app).drive, DriveExport::Failed(e) if e == &error));
    assert!(flow(&f.app).sealed.is_some());

    // Saved.
    export(&mut f.app, ExportMsg::SavePicked(Some(path.clone())));
    export(
        &mut f.app,
        ExportMsg::SaveDone {
            path: path.clone(),
            result: Ok(()),
        },
    );
    match &flow(&f.app).drive {
        DriveExport::Saved {
            report: got,
            path: to,
        } => {
            assert_eq!(got, &report());
            assert_eq!(to, &path);
        }
        other => panic!("expected Saved, got {other:?}"),
    }
    assert!(flow(&f.app).sealed.is_none());
    assert!(export(&mut f.app, ExportMsg::TryAgain).is_empty());
    assert!(export(&mut f.app, ExportMsg::SaveLocal).is_empty());
}

#[tokio::test]
async fn refused_authorisation_asks_to_connect_again() {
    let mut f = drive_fixture().await;
    let sent = uploading(&mut f);
    assert!(f.app.settings.google_drive);
    let effects = export(
        &mut f.app,
        ExportMsg::UploadDone(Err(coded("drive-not-connected"))),
    );
    assert_eq!(f.app.drive_link, DriveLink::Disconnected);
    assert!(!f.app.settings.google_drive);
    assert!(saves_settings(&effects), "{effects:?}");
    f.app.update(Message::SettingsSaved(Ok(())));
    assert!(std::ptr::eq(kept_bytes(&f.app).bytes(), sent.bytes()));
    // Not connected: Try again waits for Connect.
    assert!(export(&mut f.app, ExportMsg::TryAgain).is_empty());

    export(&mut f.app, ExportMsg::Connect);
    export(
        &mut f.app,
        ExportMsg::Connecting(ConnectEvent::Waiting {
            url: AUTH_URL.into(),
        }),
    );
    export(
        &mut f.app,
        ExportMsg::Connecting(ConnectEvent::Done(Ok(account()))),
    );
    let effects = export(&mut f.app, ExportMsg::TryAgain);
    match &effects[..] {
        [Effect::DriveUpload { sealed, .. }] => {
            assert!(std::ptr::eq(sealed.bytes(), sent.bytes()));
        }
        other => panic!("expected one DriveUpload, got {other:?}"),
    }
}

#[tokio::test]
async fn seal_failure_has_no_bytes() {
    let mut f = drive_fixture().await;
    sealing(&mut f);
    let error = Error::new(ErrorKind::Io, "vault locked");
    export(&mut f.app, ExportMsg::SealDone(Err(error.clone())));
    assert!(matches!(&flow(&f.app).drive, DriveExport::Failed(e) if e == &error));
    assert!(flow(&f.app).sealed.is_none());
    assert!(export(&mut f.app, ExportMsg::TryAgain).is_empty());
    assert!(export(&mut f.app, ExportMsg::SaveLocal).is_empty());
}

#[tokio::test]
async fn one_export_at_a_time_for_drive_too() {
    for uploads in [false, true] {
        let mut f = drive_fixture().await;
        if uploads {
            uploading(&mut f);
        } else {
            sealing(&mut f);
        }
        typed(&mut f.app, "pw", "pw");
        assert!(!f.app.export_ready());
        for msg in [
            ExportMsg::Submit,
            ExportMsg::Destination(Destination::File),
            ExportMsg::Disconnect,
            ExportMsg::DriveName("other".into()),
            ExportMsg::TryAgain,
            ExportMsg::SaveLocal,
        ] {
            let effects = export(&mut f.app, msg.clone());
            assert!(effects.is_empty(), "{msg:?}: {effects:?}");
        }
        assert_eq!(flow(&f.app).destination, Destination::Drive);
        assert_ne!(flow(&f.app).drive_name, "other");

        f.app.update(Message::OpenSettings);
        let restore = |app: &mut App, msg| app.update(Message::Settings(SettingsMsg::Backup(msg)));
        restore(&mut f.app, BackupMsg::OpenRestore);
        restore(&mut f.app, BackupMsg::RestorePath("/b.hoprs".into()));
        restore(
            &mut f.app,
            BackupMsg::RestorePassword(Password("pw".into())),
        );
        let effects = restore(&mut f.app, BackupMsg::Restore);
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::RestoreBackup { .. })),
            "{effects:?}"
        );
        // Left while running: the flow stays; its result, with the form gone, drops it.
        assert!(f.app.export.is_some());
        let done = if uploads {
            ExportMsg::UploadDone(Ok(uploaded_file()))
        } else {
            ExportMsg::SealDone(Ok((report(), Sealed::new(vec![1]))))
        };
        assert!(export(&mut f.app, done).is_empty());
        assert!(f.app.export.is_none());
        assert!(!f.app.busy());
    }
}

/// Review fix: the Drive recovery actions only exist on the Drive destination while
/// nothing runs; a file export and an upload never run together.
#[tokio::test]
async fn recovery_only_on_drive_while_nothing_runs() {
    // Switched to File: the bytes stay, but Try again and Save do nothing there.
    let mut f = drive_fixture().await;
    let sent = failed_upload(&mut f);
    assert!(export(&mut f.app, ExportMsg::Destination(Destination::File)).is_empty());
    assert!(std::ptr::eq(kept_bytes(&f.app).bytes(), sent.bytes()));
    let path = f.dir.path().join("late.hoprs");
    for msg in [
        ExportMsg::TryAgain,
        ExportMsg::SaveLocal,
        ExportMsg::SavePicked(Some(path.clone())),
    ] {
        let effects = export(&mut f.app, msg.clone());
        assert!(effects.is_empty(), "{msg:?}: {effects:?}");
    }
    // Back on Drive: offered again, with the same bytes.
    export(&mut f.app, ExportMsg::Destination(Destination::Drive));
    match &export(&mut f.app, ExportMsg::TryAgain)[..] {
        [Effect::DriveUpload { sealed, .. }] => {
            assert!(std::ptr::eq(sealed.bytes(), sent.bytes()));
        }
        other => panic!("expected one DriveUpload, got {other:?}"),
    }

    // Failed with bytes, File, Submit: the file export runs and the Drive actions (and a
    // save dialog coming back late) start nothing; the old bytes are gone.
    let mut f = drive_fixture().await;
    failed_upload(&mut f);
    export(&mut f.app, ExportMsg::SaveLocal);
    export(&mut f.app, ExportMsg::Destination(Destination::File));
    let picked = f.dir.path().join("picked.hoprs");
    export(&mut f.app, ExportMsg::Picked(Some(picked)));
    typed(&mut f.app, "pw", "pw");
    let effects = export(&mut f.app, ExportMsg::Submit);
    assert!(
        matches!(effects[..], [Effect::ExportBackup { .. }]),
        "{effects:?}"
    );
    assert!(flow(&f.app).sealed.is_none());
    assert!(matches!(flow(&f.app).drive, DriveExport::Idle));
    for msg in [
        ExportMsg::TryAgain,
        ExportMsg::SaveLocal,
        ExportMsg::SavePicked(Some(path.clone())),
        ExportMsg::Destination(Destination::Drive),
    ] {
        let effects = export(&mut f.app, msg.clone());
        assert!(effects.is_empty(), "{msg:?}: {effects:?}");
    }
    export(&mut f.app, ExportMsg::Done(Ok(report())));
    assert_eq!(flow(&f.app).state, BackupState::Done(report()));
    // Back on Drive after it: nothing left to upload or save.
    export(&mut f.app, ExportMsg::Destination(Destination::Drive));
    assert!(export(&mut f.app, ExportMsg::TryAgain).is_empty());
    assert!(export(&mut f.app, ExportMsg::SaveLocal).is_empty());

    // A new Drive export drops the old bytes too: they never stand in for it.
    let mut f = drive_fixture().await;
    failed_upload(&mut f);
    typed(&mut f.app, "pw2", "pw2");
    let effects = export(&mut f.app, ExportMsg::Submit);
    assert!(
        matches!(effects[..], [Effect::SealExport { .. }]),
        "{effects:?}"
    );
    assert!(flow(&f.app).sealed.is_none());
    for msg in [
        ExportMsg::TryAgain,
        ExportMsg::SaveLocal,
        ExportMsg::SavePicked(Some(path.clone())),
    ] {
        let effects = export(&mut f.app, msg.clone());
        assert!(effects.is_empty(), "{msg:?}: {effects:?}");
    }
}

#[tokio::test]
async fn bytes_are_dropped_with_the_form() {
    let mut f = drive_fixture().await;
    failed_upload(&mut f);
    export(&mut f.app, ExportMsg::Close);
    assert!(f.app.export.is_none());
}

#[tokio::test]
async fn disconnect_clears_the_marker() {
    let mut f = drive_fixture().await;
    connected(&mut f);
    assert!(f.app.settings.google_drive);
    let effects = export(&mut f.app, ExportMsg::Disconnect);
    assert!(
        matches!(effects[..], [Effect::DriveDisconnect]),
        "{effects:?}"
    );
    let effects = export(&mut f.app, ExportMsg::Disconnected(Ok(())));
    assert_eq!(f.app.drive_link, DriveLink::Disconnected);
    assert!(!f.app.settings.google_drive);
    assert!(saves_settings(&effects), "{effects:?}");
    assert_eq!(f.app.drive_error, None);
    f.app.update(Message::SettingsSaved(Ok(())));

    let mut f = drive_fixture().await;
    connected(&mut f);
    let error = Error::new(ErrorKind::Io, "keyring");
    export(&mut f.app, ExportMsg::Disconnect);
    let effects = export(&mut f.app, ExportMsg::Disconnected(Err(error.clone())));
    assert_eq!(f.app.drive_link, DriveLink::Disconnected);
    assert!(!f.app.settings.google_drive);
    assert!(saves_settings(&effects), "{effects:?}");
    assert_eq!(f.app.drive_error, Some(error));

    // From Settings, with the form never opened (the marker draws the line there).
    let mut f = drive_fixture().await;
    f.app.settings.google_drive = true;
    f.app.update(Message::OpenSettings);
    let effects = export(&mut f.app, ExportMsg::Disconnect);
    assert!(
        matches!(effects[..], [Effect::DriveDisconnect]),
        "{effects:?}"
    );
    export(&mut f.app, ExportMsg::Disconnected(Ok(())));
    assert!(!f.app.settings.google_drive);

    // Not while it waits for the browser.
    let mut f = drive_fixture().await;
    on_drive(&mut f);
    export(&mut f.app, ExportMsg::DriveStatus(Ok(None)));
    export(&mut f.app, ExportMsg::Connect);
    assert!(export(&mut f.app, ExportMsg::Disconnect).is_empty());
}

#[test]
fn drive_messages_from_the_user() {
    for msg in [
        ExportMsg::Destination(Destination::Drive),
        ExportMsg::Connect,
        ExportMsg::CancelConnect,
        ExportMsg::CopyLink,
        ExportMsg::Disconnect,
        ExportMsg::DriveName(String::new()),
        ExportMsg::TryAgain,
        ExportMsg::SaveLocal,
        ExportMsg::OpenInDrive,
    ] {
        assert!(msg.is_user(), "{msg:?}");
    }
    for msg in [
        ExportMsg::DriveStatus(Ok(None)),
        ExportMsg::Connecting(ConnectEvent::Done(Ok(account()))),
        ExportMsg::Disconnected(Ok(())),
        ExportMsg::SealDone(Err(Error::new(ErrorKind::Io, "x"))),
        ExportMsg::UploadDone(Ok(uploaded_file())),
        ExportMsg::SavePicked(None),
        ExportMsg::SaveDone {
            path: PathBuf::new(),
            result: Ok(()),
        },
        ExportMsg::PageOpened(Ok(())),
    ] {
        assert!(!msg.is_user(), "{msg:?}");
    }
}

/// The Drive effects reach the backend and come back as the export flow's messages.
#[tokio::test]
async fn drive_effects_reach_the_backend() {
    let a = site("a", "a.example", "u", Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let backend: Arc<dyn BackendApi> = fake.clone();

    let message = effects::drive_status(backend.clone()).await;
    assert!(
        matches!(message, Message::Export(ExportMsg::DriveStatus(Ok(None)))),
        "{message:?}"
    );
    fake.set_drive_connect(vec![
        ConnectEvent::Waiting {
            url: AUTH_URL.into(),
        },
        ConnectEvent::Done(Ok(account())),
    ]);
    let messages: Vec<Message> = effects::drive_connect(backend.clone()).collect().await;
    assert!(
        matches!(
            messages.as_slice(),
            [
                Message::Export(ExportMsg::Connecting(ConnectEvent::Waiting { .. })),
                Message::Export(ExportMsg::Connecting(ConnectEvent::Done(Ok(_)))),
            ]
        ),
        "{messages:?}"
    );
    // The fake follows its connect.
    let message = effects::drive_status(backend.clone()).await;
    assert!(
        matches!(&message, Message::Export(ExportMsg::DriveStatus(Ok(Some(got)))) if got == &account()),
        "{message:?}"
    );
    let runs = effects::CompareRuns::default();
    let _ = effects::run(backend.clone(), &runs, Effect::DriveCancel);
    assert_eq!(fake.drive_cancels(), 1);

    let message = effects::seal_export(
        backend.clone(),
        ExportScope::Site(a.id.clone()),
        SecretString::from("pw"),
    )
    .await;
    let sealed = match message {
        Message::Export(ExportMsg::SealDone(Ok((report, sealed)))) => {
            assert_eq!(report.sites, 1);
            sealed
        }
        other => panic!("{other:?}"),
    };
    let message = effects::drive_upload(backend.clone(), "n.hoprs".into(), sealed.clone()).await;
    assert!(
        matches!(&message, Message::Export(ExportMsg::UploadDone(Ok(file))) if file.name == "n.hoprs"),
        "{message:?}"
    );
    assert_eq!(fake.drive_uploads(), vec![("n.hoprs".to_owned(), 6)]);
    let path = PathBuf::from("x.hoprs");
    let message = effects::save_sealed(backend.clone(), path.clone(), sealed.clone()).await;
    assert!(
        matches!(&message, Message::Export(ExportMsg::SaveDone { path: p, result: Ok(()) }) if p == &path),
        "{message:?}"
    );
    assert_eq!(fake.sealed_saves(), vec![path]);
    let message = effects::drive_disconnect(backend.clone()).await;
    assert!(
        matches!(message, Message::Export(ExportMsg::Disconnected(Ok(())))),
        "{message:?}"
    );
    assert_eq!(fake.drive_disconnects(), 1);
    // Disconnected: the fake answers as the real backend does.
    let message = effects::drive_upload(backend.clone(), "m.hoprs".into(), sealed).await;
    assert!(
        matches!(&message, Message::Export(ExportMsg::UploadDone(Err(e))) if e.code == Some("drive-not-connected")),
        "{message:?}"
    );
    fake.fail_next_seal(Error::new(ErrorKind::Io, "locked"));
    let message =
        effects::seal_export(backend.clone(), ExportScope::All, SecretString::from("pw")).await;
    assert!(
        matches!(message, Message::Export(ExportMsg::SealDone(Err(_)))),
        "{message:?}"
    );
}
