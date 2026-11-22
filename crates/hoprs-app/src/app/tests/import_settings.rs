//! Task 11: "Import from FileZilla" (three steps in the center, no modal), the encrypted
//! backup (export and restore) and the settings page.

use hoprs_import::{ImportReport, ImportedSite, SkippedSite};
use hoprs_transfer::ConflictPolicy;
use hoprs_vault::{BackupImportReport, ExportReport, ExportScope, ImportMode};

use std::path::PathBuf;

use secrecy::SecretString;

use super::*;
use crate::app::import::{ImportMsg, ImportStep, KeepSource, SourceDelete};
use crate::app::settings::{BackupMsg, BackupPanel, BackupState, SettingsMsg};
use crate::backend::fake::BackupCall;
use crate::settings::{Settings, SettingsLoad};
use crate::ui::import as import_view;

fn import(app: &mut App, msg: ImportMsg) -> Vec<Effect> {
    app.update(Message::Import(msg))
}

fn settings(app: &mut App, msg: SettingsMsg) -> Vec<Effect> {
    app.update(Message::Settings(msg))
}

fn backup(app: &mut App, msg: BackupMsg) -> Vec<Effect> {
    settings(app, SettingsMsg::Backup(msg))
}

fn report() -> ImportReport {
    ImportReport {
        imported: vec![
            ImportedSite {
                name: "Prod".into(),
                id: SiteId::new(),
                notes: Vec::new(),
                secret_not_saved: false,
            },
            ImportedSite {
                name: "Legacy FTP".into(),
                id: SiteId::new(),
                notes: vec!["password not saved: keyring unavailable".into()],
                secret_not_saved: true,
            },
        ],
        skipped: vec![SkippedSite {
            name: "Old S3".into(),
            reason: "unsupported protocol".into(),
        }],
        workspaces_created: vec!["Clients".into()],
        skipped_workspaces: vec!["Work: a workspace with this name already exists".into()],
        failures: Vec::new(),
    }
}

/// [`report`] with every password saved: nothing keeps the source file.
fn saved_report() -> ImportReport {
    let mut report = report();
    report.imported[1].notes = vec!["FileZilla used TLS only if available".into()];
    report.imported[1].secret_not_saved = false;
    report
}

/// An app with the import flow at its report, from `path`.
fn at_report(path: &str, report: ImportReport) -> App {
    let mut app = App::new();
    app.update(Message::OpenImport);
    import(&mut app, ImportMsg::Path(path.into()));
    import(&mut app, ImportMsg::Next);
    import(
        &mut app,
        ImportMsg::Start {
            folders_as_workspaces: false,
        },
    );
    app.update(Message::Imported(Ok(report)));
    assert_eq!(app.import.step, ImportStep::Report);
    app
}

#[test]
fn import_flow_steps() {
    let mut app = App::new();
    app.filezilla_default = Some(PathBuf::from("/home/me/.config/filezilla/sitemanager.xml"));
    assert!(app.update(Message::OpenImport).is_empty());
    assert!(
        matches!(app.form, Some(Form::Import)),
        "in the center, no modal"
    );
    assert!(!app.dialogs.is_open());

    // Step 1: the file, FileZilla's default path to start with.
    assert_eq!(app.import.step, ImportStep::File);
    assert_eq!(
        app.import.path,
        "/home/me/.config/filezilla/sitemanager.xml"
    );
    // An empty path is refused inline.
    import(&mut app, ImportMsg::Path("  ".into()));
    assert!(import(&mut app, ImportMsg::Next).is_empty());
    assert_eq!(app.import.step, ImportStep::File);
    assert_eq!(app.import.path_error, Some(strings::import_path_required()));
    // "Choose…" opens the system file dialog; its answer fills the path.
    let effects = import(&mut app, ImportMsg::Choose);
    assert!(matches!(
        effects[..],
        [Effect::PickFile(PickTarget::Sitemanager)]
    ));
    app.update(Message::FilePicked {
        target: PickTarget::Sitemanager,
        path: Some(PathBuf::from("/tmp/x/sitemanager.xml")),
    });
    assert_eq!(app.import.path, "/tmp/x/sitemanager.xml");
    assert_eq!(app.import.path_error, None);
    // A cancelled dialog changes nothing.
    app.update(Message::FilePicked {
        target: PickTarget::Sitemanager,
        path: None,
    });
    assert_eq!(app.import.path, "/tmp/x/sitemanager.xml");

    // Step 2: the folders question; Back returns to the file.
    assert!(import(&mut app, ImportMsg::Next).is_empty());
    assert_eq!(app.import.step, ImportStep::Folders);
    import(&mut app, ImportMsg::Back);
    assert_eq!(app.import.step, ImportStep::File);
    import(&mut app, ImportMsg::Next);

    // The answer starts the import, off the UI thread.
    let effects = import(
        &mut app,
        ImportMsg::Start {
            folders_as_workspaces: true,
        },
    );
    match &effects[..] {
        [
            Effect::ImportFilezilla {
                path,
                folders_as_workspaces: true,
            },
        ] => assert_eq!(path, &PathBuf::from("/tmp/x/sitemanager.xml")),
        other => panic!("expected one ImportFilezilla, got {other:?}"),
    }
    assert_eq!(app.import.step, ImportStep::Running);
    assert!(app.busy(), "the progress indicator runs");
    // A second start while it runs does nothing.
    assert!(
        import(
            &mut app,
            ImportMsg::Start {
                folders_as_workspaces: false
            }
        )
        .is_empty()
    );

    // Step 3: the report, and the sidebar reloads.
    let done = report();
    let effects = app.update(Message::Imported(Ok(done.clone())));
    assert!(matches!(effects[..], [Effect::LoadProfiles(_)]));
    assert_eq!(app.import.step, ImportStep::Report);
    assert_eq!(app.import.report, Some(done));
    assert!(!app.busy());

    // A file that can't be read or parsed: back to step 1 with the error inline.
    let mut app = App::new();
    app.update(Message::OpenImport);
    import(&mut app, ImportMsg::Path("/nope.xml".into()));
    import(&mut app, ImportMsg::Next);
    import(
        &mut app,
        ImportMsg::Start {
            folders_as_workspaces: false,
        },
    );
    let error = Error::new(ErrorKind::NotFound, "No such file");
    assert!(app.update(Message::Imported(Err(error.clone()))).is_empty());
    assert_eq!(app.import.step, ImportStep::File);
    assert_eq!(app.import.error, Some(error));
    assert_eq!(app.import.path, "/nope.xml");

    // Done closes the flow; opening it again starts over.
    let mut app = at_report("/a.xml", report());
    import(&mut app, ImportMsg::Close);
    assert!(app.form.is_none());
    app.update(Message::OpenImport);
    assert_eq!(app.import.step, ImportStep::File);
    assert_eq!(app.import.report, None);
}

#[test]
fn import_report_sections() {
    let full = report();
    let sections = import_view::report_sections(&full);
    let heads: Vec<(&str, usize)> = sections.iter().map(|s| (s.title, s.count)).collect();
    assert_eq!(
        heads,
        [
            (strings::import_imported(), 2),
            (strings::import_skipped(), 1),
            (strings::import_workspaces_created(), 1),
            (strings::import_workspaces_not_created(), 1),
            (strings::import_failures(), 0),
        ]
    );
    // Imported sites with their notes; skipped ones with the reason.
    assert_eq!(sections[0].lines[0].name, "Prod");
    assert!(sections[0].lines[0].notes.is_empty());
    assert_eq!(sections[0].lines[1].name, "Legacy FTP");
    assert_eq!(
        sections[0].lines[1].notes,
        ["password not saved: keyring unavailable"]
    );
    assert_eq!(sections[1].lines[0].name, "Old S3");
    assert_eq!(sections[1].lines[0].notes, ["unsupported protocol"]);
    assert_eq!(
        sections[3].lines[0].name,
        "Work: a workspace with this name already exists"
    );

    // Failures are listed with their count.
    let mut failed = report();
    failed.failures = vec!["A: disk full".into(), "B: disk full".into()];
    let sections = import_view::report_sections(&failed);
    assert_eq!(sections[4].count, 2);
    assert_eq!(sections[4].lines[1].name, "B: disk full");

    // The view renders every step without panicking.
    let mut app = at_report("/a.xml", failed);
    let _ = import_view::view(&app);
    app.import.step = ImportStep::Running;
    let _ = import_view::view(&app);
}

#[test]
fn delete_source_needs_confirmation() {
    let mut app = at_report("/home/me/sitemanager.xml", saved_report());
    assert_eq!(app.import.delete, SourceDelete::Offered);

    // The first click only asks; Keep backs out.
    assert!(import(&mut app, ImportMsg::Delete).is_empty());
    assert_eq!(app.import.delete, SourceDelete::Confirming);
    assert!(import(&mut app, ImportMsg::CancelDelete).is_empty());
    assert_eq!(app.import.delete, SourceDelete::Offered);
    // Confirming without being asked does nothing.
    assert!(import(&mut app, ImportMsg::ConfirmDelete).is_empty());

    import(&mut app, ImportMsg::Delete);
    // The file the report came from, even if the path field were edited since.
    let effects = import(&mut app, ImportMsg::ConfirmDelete);
    match &effects[..] {
        [Effect::DeleteSitemanager(path)] => {
            assert_eq!(path, &PathBuf::from("/home/me/sitemanager.xml"));
        }
        other => panic!("expected one DeleteSitemanager, got {other:?}"),
    }
    assert_eq!(app.import.delete, SourceDelete::Deleting);
    assert!(import(&mut app, ImportMsg::ConfirmDelete).is_empty());

    app.update(Message::SitemanagerDeleted(Ok(())));
    assert_eq!(app.import.delete, SourceDelete::Deleted);

    // A refusal (not a FileZilla file, a symlink…) shows inline.
    let mut app = at_report("/x.xml", saved_report());
    import(&mut app, ImportMsg::Delete);
    import(&mut app, ImportMsg::ConfirmDelete);
    let error = Error::new(ErrorKind::Protocol, "not deleting: the path is a symlink");
    app.update(Message::SitemanagerDeleted(Err(error.clone())));
    assert_eq!(app.import.delete, SourceDelete::Failed(error));

    // A report with failures doesn't offer to delete the source.
    let mut failed = saved_report();
    failed.failures = vec!["A: disk full".into()];
    let mut app = at_report("/x.xml", failed);
    assert_eq!(
        app.import.delete,
        SourceDelete::Unavailable(KeepSource::Failures)
    );
    import(&mut app, ImportMsg::Delete);
    assert!(import(&mut app, ImportMsg::ConfirmDelete).is_empty());

    // A password the keyring refused: the file is its only copy, so no delete. The
    // reason names the sites (from the structured flag, not the note's text).
    let mut app = at_report("/x.xml", report());
    assert_eq!(
        app.import.delete,
        SourceDelete::Unavailable(KeepSource::PasswordsNotSaved(vec!["Legacy FTP".into()]))
    );
    assert!(import(&mut app, ImportMsg::Delete).is_empty());
    assert!(import(&mut app, ImportMsg::ConfirmDelete).is_empty());
    let mut flagged_without_note = saved_report();
    flagged_without_note.imported[0].secret_not_saved = true;
    let app = at_report("/x.xml", flagged_without_note);
    assert_eq!(
        app.import.delete,
        SourceDelete::Unavailable(KeepSource::PasswordsNotSaved(vec!["Prod".into()]))
    );
    let _ = import_view::view(&app);
}

#[test]
fn export_refuses_empty_password_in_ui() {
    let mut app = App::new();
    app.update(Message::OpenSettings);
    assert!(app.settings_shown());
    backup(&mut app, BackupMsg::OpenExport);
    assert_eq!(app.backup.panel, BackupPanel::Export);
    backup(
        &mut app,
        BackupMsg::ExportPath("/home/me/hoprs.backup".into()),
    );

    // Empty password: refused inline, nothing sent.
    assert!(backup(&mut app, BackupMsg::Export).is_empty());
    assert_eq!(
        app.backup.field_error,
        Some(strings::backup_password_required())
    );
    assert_eq!(app.backup.export, BackupState::Idle);

    // A confirmation that differs: refused too.
    backup(
        &mut app,
        BackupMsg::ExportPassword(Password("correct horse".into())),
    );
    backup(
        &mut app,
        BackupMsg::ExportConfirm(Password("correct hose".into())),
    );
    assert!(backup(&mut app, BackupMsg::Export).is_empty());
    assert_eq!(
        app.backup.field_error,
        Some(strings::backup_passwords_differ())
    );

    // No file: refused.
    backup(
        &mut app,
        BackupMsg::ExportConfirm(Password("correct horse".into())),
    );
    backup(&mut app, BackupMsg::ExportPath(String::new()));
    assert!(backup(&mut app, BackupMsg::Export).is_empty());
    assert_eq!(
        app.backup.field_error,
        Some(strings::backup_path_required())
    );
    backup(
        &mut app,
        BackupMsg::ExportPath("/home/me/hoprs.backup".into()),
    );

    // A typed path: whether the file exists is checked first (off the UI thread).
    let effects = backup(&mut app, BackupMsg::Export);
    match &effects[..] {
        [Effect::CheckExportTarget(path)] => {
            assert_eq!(path, &PathBuf::from("/home/me/hoprs.backup"));
        }
        other => panic!("expected one CheckExportTarget, got {other:?}"),
    }
    assert!(!app.busy(), "nothing runs yet");
    assert_eq!(
        app.backup.export_password.0, "correct horse",
        "kept until sent"
    );

    // The password leaves the UI state with the effect; its Debug never shows it.
    let effects = app.update(Message::ExportTargetChecked {
        path: PathBuf::from("/home/me/hoprs.backup"),
        exists: false,
    });
    match &effects[..] {
        [
            Effect::ExportBackup {
                scope,
                path,
                password,
            },
        ] => {
            assert_eq!(scope, &ExportScope::All);
            assert_eq!(path, &PathBuf::from("/home/me/hoprs.backup"));
            assert_eq!(password.expose_secret(), "correct horse");
        }
        other => panic!("expected one ExportBackup, got {other:?}"),
    }
    assert!(!format!("{effects:?}").contains("correct horse"));
    assert!(app.backup.export_password.0.is_empty());
    assert!(app.backup.export_confirm.0.is_empty());
    assert!(!format!("{app:?}").contains("correct horse"));
    assert_eq!(app.backup.field_error, None);
    assert_eq!(app.backup.export, BackupState::Running);
    assert!(app.busy());
    // A second export while one runs: nothing.
    backup(
        &mut app,
        BackupMsg::ExportPassword(Password("again".into())),
    );
    backup(&mut app, BackupMsg::ExportConfirm(Password("again".into())));
    assert!(backup(&mut app, BackupMsg::Export).is_empty());

    // The report.
    let report = ExportReport {
        sites: 3,
        workspaces: 1,
        secrets: 2,
        skipped_unreadable: Vec::new(),
        missing_secrets: vec!["Old box".into()],
    };
    assert!(
        app.update(Message::BackupExported(Ok(report.clone())))
            .is_empty()
    );
    assert_eq!(app.backup.export, BackupState::Done(report));
    assert!(!app.busy());

    // "Choose…" asks where to save.
    let effects = backup(&mut app, BackupMsg::Choose(PickTarget::ExportBackup));
    assert!(matches!(
        effects[..],
        [Effect::PickFile(PickTarget::ExportBackup)]
    ));
    app.update(Message::FilePicked {
        target: PickTarget::ExportBackup,
        path: Some(PathBuf::from("/media/usb/hoprs.backup")),
    });
    assert_eq!(app.backup.export_path, "/media/usb/hoprs.backup");
    // A path from the save dialog (which asked about overwriting) exports at once.
    backup(&mut app, BackupMsg::ExportPassword(Password("pw".into())));
    backup(&mut app, BackupMsg::ExportConfirm(Password("pw".into())));
    let effects = backup(&mut app, BackupMsg::Export);
    assert!(
        matches!(effects[..], [Effect::ExportBackup { .. }]),
        "{effects:?}"
    );
}

/// Review fix: a typed export path that already exists needs an inline "Overwrite?".
#[test]
fn export_over_an_existing_file_needs_confirmation() {
    let mut app = App::new();
    app.update(Message::OpenSettings);
    backup(&mut app, BackupMsg::OpenExport);
    backup(&mut app, BackupMsg::ExportPath("/b.hoprs".into()));
    backup(&mut app, BackupMsg::ExportPassword(Password("pw".into())));
    backup(&mut app, BackupMsg::ExportConfirm(Password("pw".into())));
    assert!(matches!(
        backup(&mut app, BackupMsg::Export)[..],
        [Effect::CheckExportTarget(_)]
    ));
    // While it is checked, Export does nothing more.
    assert!(backup(&mut app, BackupMsg::Export).is_empty());
    let exists = |app: &mut App, path: &str| {
        app.update(Message::ExportTargetChecked {
            path: PathBuf::from(path),
            exists: true,
        })
    };
    // An answer for another path (edited meanwhile) is dropped.
    assert!(exists(&mut app, "/other.hoprs").is_empty());
    assert!(!app.backup.confirm_overwrite);
    assert!(exists(&mut app, "/b.hoprs").is_empty());
    assert!(app.backup.confirm_overwrite, "asked in place");
    assert_eq!(app.backup.export, BackupState::Idle);
    assert!(backup(&mut app, BackupMsg::Export).is_empty());

    // Keep: nothing written; the question goes away.
    assert!(backup(&mut app, BackupMsg::CancelOverwrite).is_empty());
    assert!(!app.backup.confirm_overwrite);
    assert!(backup(&mut app, BackupMsg::ConfirmOverwrite).is_empty());

    // Again, and Overwrite.
    backup(&mut app, BackupMsg::Export);
    exists(&mut app, "/b.hoprs");
    let effects = backup(&mut app, BackupMsg::ConfirmOverwrite);
    assert!(
        matches!(effects[..], [Effect::ExportBackup { .. }]),
        "{effects:?}"
    );
    assert!(app.backup.export_password.0.is_empty());

    // Editing the path drops a pending question.
    let mut app = App::new();
    app.update(Message::OpenSettings);
    backup(&mut app, BackupMsg::OpenExport);
    backup(&mut app, BackupMsg::ExportPath("/b.hoprs".into()));
    backup(&mut app, BackupMsg::ExportPassword(Password("pw".into())));
    backup(&mut app, BackupMsg::ExportConfirm(Password("pw".into())));
    backup(&mut app, BackupMsg::Export);
    exists(&mut app, "/b.hoprs");
    backup(&mut app, BackupMsg::ExportPath("/c.hoprs".into()));
    assert!(!app.backup.confirm_overwrite);
    assert!(backup(&mut app, BackupMsg::ConfirmOverwrite).is_empty());
}

/// Review fixes: typed passwords travel redacted in messages, are forgotten whenever
/// the page is left (any route) or the panel changes, and a pending "Replace" question is
/// dropped when the restore file changes.
#[test]
fn backup_passwords_redacted_and_forgotten() {
    let message = Message::Settings(SettingsMsg::Backup(BackupMsg::ExportPassword(Password(
        "hunter2".into(),
    ))));
    let restore = BackupMsg::RestorePassword(Password("hunter3".into()));
    let confirm = BackupMsg::ExportConfirm(Password("hunter4".into()));
    let debug = format!("{message:?} {restore:?} {confirm:?}");
    assert!(!debug.contains("hunter"), "{debug}");

    let typed = |app: &mut App| {
        backup(app, BackupMsg::ExportPassword(Password("e".into())));
        backup(app, BackupMsg::ExportConfirm(Password("e".into())));
        backup(app, BackupMsg::RestorePassword(Password("r".into())));
    };
    let empty = |app: &App| {
        app.backup.export_password.0.is_empty()
            && app.backup.export_confirm.0.is_empty()
            && app.backup.restore_password.0.is_empty()
    };
    // Left through the sidebar (another page), a site click, or a tab.
    let mut app = App::new();
    for leave in [
        Message::NewSite,
        Message::OpenImport,
        Message::NewWorkspace,
        Message::ConnectSite(SiteId::new()),
    ] {
        app.update(Message::OpenSettings);
        backup(&mut app, BackupMsg::OpenExport);
        typed(&mut app);
        assert!(!empty(&app));
        app.update(leave.clone());
        assert!(empty(&app), "{leave:?}");
    }
    // Switching panels.
    app.update(Message::OpenSettings);
    backup(&mut app, BackupMsg::OpenExport);
    typed(&mut app);
    backup(&mut app, BackupMsg::OpenRestore);
    assert!(empty(&app));
    // Re-opening the same panel keeps what is typed.
    typed(&mut app);
    backup(&mut app, BackupMsg::OpenRestore);
    assert_eq!(app.backup.restore_password.0, "r");

    // A new restore file, typed or chosen, drops the pending Replace question.
    backup(&mut app, BackupMsg::RestorePath("/a.hoprs".into()));
    backup(&mut app, BackupMsg::RestoreMode(ImportMode::Replace));
    backup(&mut app, BackupMsg::Restore);
    assert!(app.backup.confirm_replace);
    backup(&mut app, BackupMsg::RestorePath("/b.hoprs".into()));
    assert!(!app.backup.confirm_replace);
    assert!(backup(&mut app, BackupMsg::ConfirmReplace).is_empty());
    backup(&mut app, BackupMsg::Restore);
    assert!(app.backup.confirm_replace);
    app.update(Message::FilePicked {
        target: PickTarget::RestoreBackup,
        path: Some(PathBuf::from("/c.hoprs")),
    });
    assert!(!app.backup.confirm_replace);
    assert!(backup(&mut app, BackupMsg::ConfirmReplace).is_empty());
}

/// Review fix: one settings save at a time; changes made meanwhile are saved (the latest
/// values) once it ends, so two writes never race.
#[test]
fn settings_saves_are_coalesced() {
    let mut app = App::new();
    app.update(Message::OpenSettings);
    let effects = settings(&mut app, SettingsMsg::Workers(5));
    assert!(matches!(
        effects[..],
        [Effect::SetWorkers(5), Effect::SaveSettings(_)]
    ));
    // While it runs: applied at once, saved later.
    let effects = settings(&mut app, SettingsMsg::Workers(6));
    assert!(
        matches!(effects[..], [Effect::SetWorkers(6)]),
        "{effects:?}"
    );
    let effects = settings(&mut app, SettingsMsg::Policy(ConflictPolicy::Skip));
    assert!(matches!(
        effects[..],
        [Effect::SetPolicy(ConflictPolicy::Skip)]
    ));
    // The first save ends (even failed): one more save, with the latest values.
    let effects = app.update(Message::SettingsSaved(Err(Error::new(ErrorKind::Io, "x"))));
    match &effects[..] {
        [Effect::SaveSettings(saved)] => assert_eq!(
            *saved,
            Settings {
                workers_per_site: 6,
                default_policy: ConflictPolicy::Skip,
                ..Settings::default()
            }
        ),
        other => panic!("{other:?}"),
    }
    // That one ends with nothing new: done.
    assert!(app.update(Message::SettingsSaved(Ok(()))).is_empty());
    assert_eq!(app.settings_save_error, None);
    let effects = settings(&mut app, SettingsMsg::Workers(7));
    assert!(matches!(
        effects[..],
        [Effect::SetWorkers(7), Effect::SaveSettings(_)]
    ));
}

/// The keepalive (Plan 6, Task 5): clamped to 10..=300 s, applied to the next
/// connections, saved.
#[test]
fn keepalive_setting_applies_to_next_connections() {
    let mut app = App::new();
    app.update(Message::OpenSettings);
    assert_eq!(app.settings.keepalive_secs, 30);
    let effects = settings(&mut app, SettingsMsg::Keepalive(5));
    match &effects[..] {
        [Effect::SetKeepalive(10), Effect::SaveSettings(saved)] => {
            assert_eq!(saved.keepalive_secs, 10);
        }
        other => panic!("{other:?}"),
    }
    app.update(Message::SettingsSaved(Ok(())));
    assert!(settings(&mut app, SettingsMsg::Keepalive(10)).is_empty());
    let effects = settings(&mut app, SettingsMsg::Keepalive(900));
    assert!(matches!(
        effects[..],
        [Effect::SetKeepalive(300), Effect::SaveSettings(_)]
    ));
}

#[test]
fn restore_replace_needs_extra_confirmation() {
    let mut app = App::new();
    app.update(Message::OpenSettings);
    backup(&mut app, BackupMsg::OpenRestore);
    assert_eq!(app.backup.panel, BackupPanel::Restore);
    backup(&mut app, BackupMsg::RestorePath("/b.backup".into()));
    assert!(backup(&mut app, BackupMsg::Restore).is_empty());
    assert_eq!(
        app.backup.field_error,
        Some(strings::backup_password_required())
    );
    backup(&mut app, BackupMsg::RestorePassword(Password("pw".into())));

    // Merge (the default) restores at once.
    assert_eq!(app.backup.restore_mode, ImportMode::Merge);
    let effects = backup(&mut app, BackupMsg::Restore);
    assert!(matches!(
        effects[..],
        [Effect::RestoreBackup {
            mode: ImportMode::Merge,
            ..
        }]
    ));
    assert!(app.backup.restore_password.0.is_empty());
    let done = BackupImportReport {
        added_sites: 2,
        secret_errors: vec!["Prod: keyring locked".into()],
        ..BackupImportReport::default()
    };
    // The restored profiles reach the sidebar.
    let effects = app.update(Message::BackupRestored(Ok(done.clone())));
    assert!(matches!(effects[..], [Effect::LoadProfiles(_)]));
    assert_eq!(app.backup.restore, BackupState::Done(done));

    // Replace: the first click only asks.
    backup(&mut app, BackupMsg::RestoreMode(ImportMode::Replace));
    backup(&mut app, BackupMsg::RestorePassword(Password("pw2".into())));
    assert!(backup(&mut app, BackupMsg::Restore).is_empty());
    assert!(app.backup.confirm_replace);
    assert_eq!(app.backup.restore_password.0, "pw2", "kept until confirmed");
    // Backing out keeps the vault as it is.
    assert!(backup(&mut app, BackupMsg::CancelReplace).is_empty());
    assert!(!app.backup.confirm_replace);
    // Confirming without being asked does nothing.
    assert!(backup(&mut app, BackupMsg::ConfirmReplace).is_empty());

    backup(&mut app, BackupMsg::Restore);
    let effects = backup(&mut app, BackupMsg::ConfirmReplace);
    match &effects[..] {
        [
            Effect::RestoreBackup {
                path,
                password,
                mode: ImportMode::Replace,
            },
        ] => {
            assert_eq!(path, &PathBuf::from("/b.backup"));
            assert_eq!(password.expose_secret(), "pw2");
        }
        other => panic!("expected one RestoreBackup, got {other:?}"),
    }
    assert!(!format!("{effects:?}").contains("pw2"));
    assert!(app.backup.restore_password.0.is_empty());
    assert!(!app.backup.confirm_replace);
    assert_eq!(app.backup.restore, BackupState::Running);

    // A wrong password comes back inline; nothing reloads.
    let error = Error::new(ErrorKind::Protocol, "Wrong password or damaged backup");
    assert!(
        app.update(Message::BackupRestored(Err(error.clone())))
            .is_empty()
    );
    assert_eq!(app.backup.restore, BackupState::Failed(error));

    // Switching back to Merge drops a pending Replace question.
    backup(&mut app, BackupMsg::RestorePassword(Password("pw3".into())));
    backup(&mut app, BackupMsg::Restore);
    assert!(app.backup.confirm_replace);
    backup(&mut app, BackupMsg::RestoreMode(ImportMode::Merge));
    assert!(!app.backup.confirm_replace);
}

/// The settings page: changes apply to the engine and the sessions, and are saved;
/// an unreadable file is never overwritten.
#[test]
fn settings_changes_apply_and_save() {
    let mut app = App::new();
    let (_, boot) = App::boot();
    assert!(matches!(
        boot[..],
        [
            Effect::LoadProfiles(_),
            Effect::LoadSettings,
            Effect::LoadQueueStatus
        ]
    ));
    app.update(Message::SettingsLoaded(SettingsLoad {
        settings: Settings {
            workers_per_site: 6,
            default_policy: ConflictPolicy::Skip,
            ..Settings::default()
        },
        error: None,
    }));
    assert_eq!(app.settings.workers_per_site, 6);
    app.update(Message::OpenSettings);

    // Workers: clamped, applied to the next connections, saved.
    let effects = settings(&mut app, SettingsMsg::Workers(40));
    assert_eq!(app.settings.workers_per_site, 16);
    match &effects[..] {
        [Effect::SetWorkers(16), Effect::SaveSettings(saved)] => {
            assert_eq!(saved.workers_per_site, 16);
            assert_eq!(saved.default_policy, ConflictPolicy::Skip);
        }
        other => panic!("{other:?}"),
    }
    app.update(Message::SettingsSaved(Ok(())));
    let effects = settings(&mut app, SettingsMsg::Workers(0));
    assert_eq!(app.settings.workers_per_site, 1);
    assert!(matches!(
        effects[..],
        [Effect::SetWorkers(1), Effect::SaveSettings(_)]
    ));
    app.update(Message::SettingsSaved(Ok(())));
    // The same value again: nothing to do.
    assert!(settings(&mut app, SettingsMsg::Workers(1)).is_empty());

    // The policy goes to the engine, and the queue's "this session" note goes away
    // (the engine now runs under the chosen policy).
    app.queue
        .set_session_policy(Some(ConflictPolicy::Overwrite));
    let effects = settings(&mut app, SettingsMsg::Policy(ConflictPolicy::Resume));
    match &effects[..] {
        [
            Effect::SetPolicy(ConflictPolicy::Resume),
            Effect::SaveSettings(saved),
        ] => {
            assert_eq!(saved.default_policy, ConflictPolicy::Resume);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(app.queue.session_policy(), None);

    // A failed save shows inline; the next success clears it.
    let error = Error::new(ErrorKind::Io, "Permission denied");
    app.update(Message::SettingsSaved(Err(error.clone())));
    assert_eq!(app.settings_save_error, Some(error));
    settings(&mut app, SettingsMsg::Workers(2));
    app.update(Message::SettingsSaved(Ok(())));
    assert_eq!(app.settings_save_error, None);

    // An unreadable file: the defaults, the error inline, and changes are applied but
    // never saved over the file.
    let mut app = App::new();
    let error = Error::new(ErrorKind::Protocol, "Can't read settings.toml");
    app.update(Message::SettingsLoaded(SettingsLoad {
        settings: Settings::default(),
        error: Some(error.clone()),
    }));
    assert_eq!(app.settings, Settings::default());
    assert_eq!(app.settings_load_error, Some(error));
    let effects = settings(&mut app, SettingsMsg::Workers(8));
    assert!(
        matches!(effects[..], [Effect::SetWorkers(8)]),
        "{effects:?}"
    );
    let effects = settings(&mut app, SettingsMsg::Policy(ConflictPolicy::Skip));
    assert!(matches!(
        effects[..],
        [Effect::SetPolicy(ConflictPolicy::Skip)]
    ));

    // Its × closes the tab.
    app.update(Message::CloseSettings);
    assert!(app.settings_tab.is_none());
}

/// The real effects against the fake backend: the import, the delete, the backup and
/// the settings reach it, and come back as messages.
#[tokio::test]
async fn effects_reach_the_backend() {
    let a = site("a", "a.example", "u", Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let backend: Arc<dyn BackendApi> = fake.clone();

    let message = effects::import_filezilla(backend.clone(), PathBuf::from("/s.xml"), true).await;
    assert!(matches!(message, Message::Imported(Ok(_))));
    let message = effects::delete_sitemanager(backend.clone(), PathBuf::from("/s.xml")).await;
    assert!(matches!(message, Message::SitemanagerDeleted(Ok(()))));
    let message = effects::export_backup(
        backend.clone(),
        ExportScope::Site(a.id.clone()),
        PathBuf::from("/b"),
        SecretString::from("pw"),
    )
    .await;
    assert!(matches!(message, Message::BackupExported(Ok(_))));
    let message = effects::restore_backup(
        backend.clone(),
        PathBuf::from("/b"),
        SecretString::from("pw"),
        ImportMode::Replace,
    )
    .await;
    assert!(matches!(message, Message::BackupRestored(Ok(_))));
    let saved = Settings {
        workers_per_site: 9,
        default_policy: ConflictPolicy::Overwrite,
        ..Settings::default()
    };
    let message = effects::save_settings(backend.clone(), saved.clone()).await;
    assert!(matches!(message, Message::SettingsSaved(Ok(()))));
    backend.set_policy(ConflictPolicy::Skip).await;
    backend.set_workers(3);

    assert_eq!(
        fake.backup_calls(),
        [
            BackupCall::Import {
                path: PathBuf::from("/s.xml"),
                folders_as_workspaces: true,
            },
            BackupCall::DeleteSitemanager(PathBuf::from("/s.xml")),
            BackupCall::Export {
                scope: ExportScope::Site(a.id.clone()),
                path: PathBuf::from("/b"),
                password: "pw".into(),
            },
            BackupCall::Restore {
                path: PathBuf::from("/b"),
                password: "pw".into(),
                mode: ImportMode::Replace,
            },
            BackupCall::SaveSettings(saved),
            BackupCall::SetPolicy(ConflictPolicy::Skip),
            BackupCall::SetWorkers(3),
        ]
    );
}

/// Review fix: new profiles (after a "Replace" restore, or any reload) reconcile the
/// tabs: a site that is gone loses its tab and session, a site whose host or port changed
/// loses its session, and a workspace that is gone is no longer the open one.
#[tokio::test]
async fn reloaded_profiles_reconcile_tabs() {
    let a = site("a", "ha", "u", Protocol::Ssh);
    let b = site("b", "hb", "u", Protocol::Ftp);
    let c = site("c", "hc", "u", Protocol::Ftp);
    let ws = workspace("w", &[&a.id, &b.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_site(b.clone())
            .with_site(c.clone())
            .with_workspace(ws.clone()),
    );
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::OpenWorkspace(ws.id.clone())));
    drive(&mut app, &fake, sites).await;
    let sites = connect_effect(app.update(Message::ConnectSite(c.id.clone())));
    drive(&mut app, &fake, sites).await;
    let session = |app: &App, id: &SiteId| app.tab_conn(app.tab_of(id).unwrap()).session().unwrap();
    let (sa, sb, sc) = (
        session(&app, &a.id),
        session(&app, &b.id),
        session(&app, &c.id),
    );
    assert_eq!(app.open_workspace, Some(ws.id.clone()));

    // The restored profiles: `a` and the workspace are gone, `b` moved to another host,
    // `c` is as it was (a user change only: its session stays).
    let mut moved = b.clone();
    moved.host = "new-hb".into();
    let mut renamed = c.clone();
    renamed.name = "c renamed".into();
    let effects = app.update(profiles_loaded(
        &app,
        Ok(ProfileSnapshot {
            sites: vec![moved.clone(), renamed],
            workspaces: Vec::new(),
            unreadable: Vec::new(),
        }),
    ));
    // The active tab may have moved (its strip scrolls to it); otherwise only closes.
    let mut closed: Vec<SessionId> = effects
        .iter()
        .filter(|e| !matches!(e, Effect::ScrollTabs { .. }))
        .map(|e| match e {
            Effect::CloseSession(s) => *s,
            other => panic!("only closes expected, got {other:?}"),
        })
        .collect();
    closed.sort_by_key(|s| s.0);
    let mut expected = vec![sa, sb];
    expected.sort_by_key(|s| s.0);
    assert_eq!(closed, expected);
    assert!(app.tab_of(&a.id).is_none(), "the gone site's tab is closed");
    assert_eq!(app.open_workspace, None);
    match tab_state(&app, &b.id) {
        TabState::Failed(failure) => {
            assert_eq!(failure.session, None, "a new session on reconnect");
            assert_eq!(
                strings::error(&failure.error),
                "The site's host or port changed: this session was closed. Reconnect to use the new settings."
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(app.tab_conn(app.tab_of(&b.id).unwrap()).session(), None);
    assert_eq!(session(&app, &c.id), sc, "unchanged endpoint: kept");
    assert!(app.active_tab.is_some_and(|i| i < app.tabs.len()));

    // Reconnect builds a new session on the saved endpoint.
    let sites = connect_effect(app.update(Message::ConnectSite(b.id.clone())));
    assert_eq!(sites, std::slice::from_ref(&b.id));

    // The same reload again changes nothing.
    let effects = app.update(profiles_loaded(
        &app,
        Ok(ProfileSnapshot {
            sites: vec![moved],
            workspaces: Vec::new(),
            unreadable: Vec::new(),
        }),
    ));
    assert!(
        effects
            .iter()
            .all(|e| matches!(e, Effect::CloseSession(s) if *s == sc)),
        "only `c`, now gone, is closed: {effects:?}"
    );
}

/// Puts the thread's language back to English when a test ends, however it ends.
struct RestoreEnglish;

impl Drop for RestoreEnglish {
    fn drop(&mut self) {
        crate::i18n::set_language(crate::i18n::Language::En);
    }
}

/// Picking a language on the settings page switches the labels the views build at once,
/// and saves the choice.
#[tokio::test]
async fn language_choice_switches_labels_and_saves() {
    use crate::i18n::Language;
    use crate::settings::LanguageChoice;

    let _restore = RestoreEnglish;
    let mut app = loaded(&FakeBackend::new()).await;
    assert_eq!(strings::language_system(), "System");
    assert_eq!(strings::app_title(), "hoprs");

    let effects = settings(
        &mut app,
        SettingsMsg::Language(LanguageChoice::Fixed(Language::PtBr)),
    );
    assert_eq!(crate::i18n::language(), Language::PtBr);
    assert_eq!(strings::language_system(), "Sistema");
    assert_eq!(strings::settings_language(), "Idioma");
    assert_eq!(app.settings.language, LanguageChoice::Fixed(Language::PtBr));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::SaveSettings(s) if s.language == LanguageChoice::Fixed(Language::PtBr)))
    );

    settings(
        &mut app,
        SettingsMsg::Language(LanguageChoice::Fixed(Language::En)),
    );
    assert_eq!(strings::language_system(), "System");
    assert_eq!(strings::settings_language(), "Language");
}

#[test]
fn sidebar_toggle_saves() {
    use crate::app::{SidebarMsg, SidebarSection};
    let mut app = App::new();
    let effects = app.update(Message::Sidebar(SidebarMsg::ToggleCollapsed(
        SidebarSection::Sites,
    )));
    assert!(app.settings.sites.collapsed);
    assert!(!app.settings.workspaces.collapsed);
    let [Effect::SaveSettings(saved)] = &effects[..] else {
        panic!("expected one SaveSettings, got {effects:?}");
    };
    assert!(saved.sites.collapsed);
}

#[test]
fn sidebar_menu_hides_and_shows() {
    use crate::app::menu::{MenuMsg, MenuTarget};
    let mut app = App::new();
    let open = || Message::Menu(MenuMsg::Open(MenuTarget::SidebarOptions));
    app.update(open());
    let items = app.menu_items(&MenuTarget::SidebarOptions);
    assert_eq!(items.len(), 2);
    assert!(items.iter().all(|item| item.enabled));
    assert_eq!(items[1].checked, Some(true));
    // Show all sites (checked): hides it, and the menu closes.
    let effects = app.update(Message::Menu(MenuMsg::Choose(1)));
    assert!(app.settings.sites.hidden);
    assert!(!app.settings.workspaces.hidden);
    assert!(matches!(&effects[..], [Effect::SaveSettings(_)]));
    assert!(app.menu.is_none());
    let items = app.menu_items(&MenuTarget::SidebarOptions);
    assert_eq!(items[1].checked, Some(false));
    assert_eq!(items[0].checked, Some(true));
    // Choosing it again shows the section.
    app.update(open());
    app.update(Message::Menu(MenuMsg::Choose(1)));
    assert!(!app.settings.sites.hidden);
}

/// Plan 13 Task 8: the settings page is a tab; a change made there applies and is saved
/// as before.
#[test]
fn settings_change_in_tab_is_saved() {
    let mut app = App::new();
    app.update(Message::OpenSettings);
    assert!(app.settings_shown());
    let effects = settings(&mut app, SettingsMsg::Workers(5));
    match &effects[..] {
        [Effect::SetWorkers(5), Effect::SaveSettings(saved)] => {
            assert_eq!(saved.workers_per_site, 5);
        }
        other => panic!("{other:?}"),
    }
    app.update(Message::SettingsSaved(Ok(())));
    let effects = settings(&mut app, SettingsMsg::Policy(ConflictPolicy::Skip));
    assert!(matches!(
        effects[..],
        [
            Effect::SetPolicy(ConflictPolicy::Skip),
            Effect::SaveSettings(_)
        ]
    ));
}

/// Typed backup passwords and fields go as soon as the settings tab is not shown (another
/// tab selected, or the tab closed), as they did when the page was a form.
#[tokio::test]
async fn typed_passwords_forgotten_when_settings_not_shown() {
    let site = site("prod", "h", "u", Protocol::Ftp);
    let fake = Arc::new(FakeBackend::new().with_site(site.clone()));
    let mut app = loaded(&fake).await;
    app.update(Message::ConnectSite(site.id.clone()));
    for leave in [Message::SelectTab(0), Message::CloseSettings] {
        app.update(Message::OpenSettings);
        backup(&mut app, BackupMsg::OpenExport);
        backup(&mut app, BackupMsg::ExportPassword(Password("e".into())));
        app.compare_ignore_input = Some("a, ".into());
        app.update(Message::Modifiers(Mods::default()));
        assert_eq!(app.backup.export_password.0, "e", "kept while shown");
        app.update(leave.clone());
        assert!(app.backup.export_password.0.is_empty(), "{leave:?}");
        assert_eq!(app.compare_ignore_input, None, "{leave:?}");
    }
}

#[test]
fn section_msg_switches_section() {
    use crate::app::settings::SettingsSection;
    let mut app = App::new();
    app.update(Message::OpenSettings);
    assert_eq!(
        app.settings_tab.as_ref().map(|tab| tab.section),
        Some(SettingsSection::General)
    );
    for section in SettingsSection::ALL {
        settings(&mut app, SettingsMsg::Section(section));
        assert_eq!(
            app.settings_tab.as_ref().map(|tab| tab.section),
            Some(section)
        );
    }
    // Without the tab it does nothing.
    app.update(Message::CloseSettings);
    settings(&mut app, SettingsMsg::Section(SettingsSection::Backup));
    assert!(app.settings_tab.is_none());
}

/// The export screen's title: what the scope names, from the profiles the app holds.
#[tokio::test]
async fn export_title_per_scope() {
    use crate::app::export::ExportTitle;

    let a = site("a", "a.example", "u", Protocol::Ssh);
    let b = site("b", "b.example", "u", Protocol::Ftp);
    let c = site("c", "c.example", "u", Protocol::Ftp);
    let missing = SiteId::new();
    let team = workspace("team", &[&a.id, &b.id, &missing]);
    let fake = FakeBackend::new()
        .with_site(a.clone())
        .with_site(b)
        .with_site(c)
        .with_workspace(team.clone());
    let app = loaded(&fake).await;

    let title = app.export_title(&ExportScope::Site(a.id.clone()));
    assert_eq!(title, Some(ExportTitle::Site { name: "a".into() }));
    assert_eq!(title.unwrap().name(), Some("a"));
    let title = app.export_title(&ExportScope::Workspace(team.id.clone()));
    assert_eq!(
        title,
        Some(ExportTitle::Workspace {
            name: "team".into(),
            sites: 2
        })
    );
    assert_eq!(title.unwrap().name(), Some("team"));
    let title = app.export_title(&ExportScope::All);
    assert_eq!(
        title,
        Some(ExportTitle::All {
            sites: 3,
            workspaces: 1
        })
    );
    assert_eq!(title.unwrap().name(), None);
    assert_eq!(app.export_title(&ExportScope::Site(SiteId::new())), None);
    assert_eq!(
        app.export_title(&ExportScope::Workspace(WorkspaceId::new())),
        None
    );
}
