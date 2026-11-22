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
