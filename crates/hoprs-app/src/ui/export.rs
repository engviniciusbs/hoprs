//! The export screen: one site, one workspace or everything, sealed into one encrypted
//! file. A centre form like the workspace form (no modal); its state is
//! [`App::export`](crate::app::App::export).

use hoprs_vault::{ExportReport, ExportScope};
use iced::widget::{button, column, container, row, scrollable, space, text, text_input};
use iced::{Alignment, Element, Length, Padding};

use crate::app::export::{Destination, DriveExport, DriveLink, ExportFlow, ExportMsg, ExportTitle};
use crate::app::settings::BackupState;
use crate::app::{App, Message};
use crate::ui::settings::{names, outcome_error};
use crate::ui::site_form::Password;
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use crate::ui::widgets::{self, busy, confirm_with, failure, field};

fn msg(m: ExportMsg) -> Message {
    Message::Export(m)
}

/// What the screen says it exports.
pub fn title(title: &ExportTitle) -> String {
    match title {
        ExportTitle::Site { name } => strings::export_title_site(name),
        ExportTitle::Workspace { name, sites } => strings::export_title_workspace(name, *sites),
        ExportTitle::All { sites, workspaces } => strings::export_title_all(*sites, *workspaces),
    }
}

/// The note under the title of a partial export (one site or one workspace).
pub fn partial_note(scope: &ExportScope) -> Option<&'static str> {
    match scope {
        ExportScope::All => None,
        ExportScope::Site(_) | ExportScope::Workspace(_) => Some(strings::export_partial_note()),
    }
}

/// The Google Drive row under Destination.
#[derive(Debug, PartialEq)]
pub enum DriveRow {
    /// The keyring is being read: nothing under Destination yet.
    Reading,
    Connect,
    /// The browser is awaited; "Copy the link" waits for the URL.
    Waiting {
        can_copy: bool,
    },
    Account {
        label: String,
        session_only: bool,
    },
}

/// The Drive row for the flow's destination; `None` unless it is Google Drive.
pub fn drive_row(app: &App) -> Option<DriveRow> {
    let flow = app.export.as_ref()?;
    if flow.destination != Destination::Drive {
        return None;
    }
    Some(match &app.drive_link {
        DriveLink::Unknown | DriveLink::Reading => DriveRow::Reading,
        DriveLink::Disconnected => DriveRow::Connect,
        DriveLink::Waiting { url } => DriveRow::Waiting {
            can_copy: url.is_some(),
        },
        DriveLink::Connected(account) => DriveRow::Account {
            label: account
                .email
                .clone()
                .unwrap_or_else(|| strings::drive_connected().to_owned()),
            session_only: !account.stored,
        },
    })
}

/// What the Google Drive export has done so far.
#[derive(Debug, PartialEq)]
pub enum DriveOutcome {
    None,
    Busy(&'static str),
    Uploaded { path: String, link: bool },
    Failed { try_again: bool, save: bool },
    Saved(String),
}

/// The Drive outcome; only for the Google Drive destination (switching to File hides it,
/// switching back shows it again).
pub fn drive_outcome(app: &App) -> DriveOutcome {
    let Some(flow) = app.export.as_ref() else {
        return DriveOutcome::None;
    };
    if flow.destination != Destination::Drive {
        return DriveOutcome::None;
    }
    match &flow.drive {
        DriveExport::Idle => DriveOutcome::None,
        DriveExport::Sealing => DriveOutcome::Busy(strings::exporting()),
        DriveExport::Uploading | DriveExport::Saving => {
            DriveOutcome::Busy(strings::drive_uploading())
        }
        DriveExport::Uploaded { file, .. } => DriveOutcome::Uploaded {
            path: format!("hoprs/{}", file.name),
            link: file.link.is_some(),
        },
        DriveExport::Failed(_) => {
            let bytes = flow.has_bytes();
            DriveOutcome::Failed {
                try_again: bytes && matches!(app.drive_link, DriveLink::Connected(_)),
                save: bytes,
            }
        }
        DriveExport::Saved { path, .. } => DriveOutcome::Saved(path.to_string_lossy().into_owned()),
    }
}

/// The export succeeded (to the file, or to or from Google Drive): the primary button is
/// Done.
pub fn export_finished(app: &App) -> bool {
    let Some(flow) = app.export.as_ref() else {
        return false;
    };
    match flow.destination {
        Destination::File => matches!(flow.state, BackupState::Done(_)),
        Destination::Drive => matches!(
            drive_outcome(app),
            DriveOutcome::Uploaded { .. } | DriveOutcome::Saved(_)
        ),
    }
}

/// Whether Export can be pressed: the model's rule and, for Google Drive, a connection.
pub fn export_enabled(app: &App) -> bool {
    let drive = app
        .export
        .as_ref()
        .is_some_and(|flow| flow.destination == Destination::Drive);
    app.export_ready() && (!drive || matches!(app.drive_link, DriveLink::Connected(_)))
}

fn muted(label: &str) -> Element<'_, Message> {
    text(label)
        .size(theme::TEXT_SM)
        .color(palette().muted)
        .into()
}

fn primary(label: &str, on_press: Option<Message>) -> Element<'_, Message> {
    button(text(label).font(theme::INTER_SEMIBOLD))
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
        .style(theme::button_primary)
        .on_press_maybe(on_press)
        .into()
}

fn plain(label: &str, on_press: Message) -> Element<'_, Message> {
    button(text(label))
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
        .style(theme::button_text)
        .on_press(on_press)
        .into()
}

fn password<'a>(
    value: &'a str,
    on_input: impl Fn(String) -> Message + 'a,
) -> iced::widget::TextInput<'a, Message> {
    text_input("", value)
        .on_input(on_input)
        .secure(true)
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
        .size(theme::TEXT_SM)
        .style(theme::search)
}

pub fn view(app: &App) -> Element<'_, Message> {
    let Some(flow) = &app.export else {
        return space().into();
    };
    let mut content = column![
        text(title(&flow.title))
            .size(theme::TEXT_MD)
            .font(theme::INTER_SEMIBOLD),
    ];
    if let Some(note) = partial_note(&flow.scope) {
        content = content.push(muted(note));
    }
    let drive_row = drive_row(app);
    if app.drive_available {
        content = content.push(field(
            strings::export_destination(),
            row![
                destination(
                    strings::export_destination_file(),
                    flow.destination,
                    Destination::File
                ),
                destination(
                    strings::export_destination_drive(),
                    flow.destination,
                    Destination::Drive
                ),
            ]
            .spacing(theme::SPACE_XS)
            .into(),
            None,
        ));
    }
    match &drive_row {
        None => content = content.push(field(strings::backup_file(), file_row(flow), None)),
        Some(DriveRow::Reading) => {}
        Some(DriveRow::Connect) => {
            content = content.push(widgets::outlined(
                button(text(strings::drive_connect()))
                    .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_SM]))
                    .on_press(msg(ExportMsg::Connect)),
                true,
            ));
        }
        Some(DriveRow::Waiting { can_copy }) => {
            content = content
                .push(text(strings::drive_waiting()))
                .push(
                    row![
                        plain(strings::cancel(), msg(ExportMsg::CancelConnect)),
                        outlined_button(
                            strings::drive_copy_link(),
                            can_copy.then(|| msg(ExportMsg::CopyLink)),
                        ),
                    ]
                    .spacing(theme::SPACE_XS)
                    .align_y(Alignment::Center),
                )
                .push(muted(strings::drive_browser_hint()));
        }
        Some(DriveRow::Account {
            label,
            session_only,
        }) => {
            content = content.push(field(
                strings::drive_account(),
                row![
                    text(label.clone()),
                    space().width(Length::Fill),
                    plain_maybe(
                        strings::drive_disconnect(),
                        (!flow.running()).then(|| msg(ExportMsg::Disconnect)),
                    ),
                ]
                .align_y(Alignment::Center)
                .into(),
                None,
            ));
            if *session_only {
                content = content.push(muted(strings::drive_session_only()));
            }
            content = content.push(field(
                strings::drive_file_name(),
                text_input("", &flow.drive_name)
                    .on_input(|v| msg(ExportMsg::DriveName(v)))
                    .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                    .style(theme::search)
                    .font(theme::MONO)
                    .size(theme::TEXT_MONO)
                    .into(),
                None,
            ));
        }
    }
    if let Some(error) = &app.drive_error
        && drive_row.is_some()
    {
        content = content.push(failure(Some(error)));
    }
    content = content
        .push(
            row![
                field(
                    strings::backup_password(),
                    password(&flow.password.0, |v| msg(ExportMsg::Password(Password(v)))).into(),
                    None,
                ),
                field(
                    strings::backup_password_confirm(),
                    password(&flow.confirm.0, |v| msg(ExportMsg::Confirm(Password(v))))
                        .on_submit(msg(ExportMsg::Submit))
                        .into(),
                    None,
                ),
            ]
            .spacing(theme::SPACE_SM),
        )
        .push(muted(strings::export_password_hint()));
    if let Some(error) = flow.field_error {
        content = content.push(text(error).size(theme::TEXT_XS).color(palette().error));
    }
    if drive_row.is_some() {
        content = content.push(drive_outcome_view(app, flow));
    } else {
        match &flow.state {
            BackupState::Idle => {}
            BackupState::Running => {
                content = content.push(busy(strings::exporting(), app.busy_phase));
            }
            BackupState::Done(report) => content = content.push(export_report(report)),
            BackupState::Failed(error) => {
                content = content.push(outcome_error(strings::export_failed(), error));
            }
        }
    }
    content = content.push(actions(app, flow));

    scrollable(
        container(content.spacing(theme::SPACE_SM).max_width(520))
            .width(Length::Fill)
            .padding(theme::SPACE_LG),
    )
    .height(Length::Fill)
    .into()
}

/// Export / Cancel, Done after a successful export, or the inline question about an
/// existing file.
fn actions<'a>(app: &App, flow: &ExportFlow) -> Element<'a, Message> {
    if flow.confirm_overwrite {
        return confirm_with(
            strings::export_overwrite_confirm(),
            strings::overwrite(),
            msg(ExportMsg::ConfirmOverwrite),
            msg(ExportMsg::CancelOverwrite),
        );
    }
    if export_finished(app) {
        return primary(strings::done(), Some(msg(ExportMsg::Close)));
    }
    row![
        primary(
            strings::export(),
            export_enabled(app).then(|| msg(ExportMsg::Submit))
        ),
        plain(strings::cancel(), msg(ExportMsg::Close)),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center)
    .into()
}

fn plain_maybe(label: &str, on_press: Option<Message>) -> Element<'_, Message> {
    button(text(label))
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
        .style(theme::button_text)
        .on_press_maybe(on_press)
        .into()
}

fn outlined_button(label: &str, on_press: Option<Message>) -> Element<'_, Message> {
    let enabled = on_press.is_some();
    widgets::outlined(
        button(text(label))
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_SM]))
            .on_press_maybe(on_press),
        enabled,
    )
}

/// One choice of the Destination control, as the restore panel's Merge / Replace.
fn destination(
    label: &str,
    current: Destination,
    this: Destination,
) -> iced::widget::Button<'_, Message> {
    button(text(label))
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
        .style(theme::segment(current == this))
        .on_press(msg(ExportMsg::Destination(this)))
}

/// The File destination's path and "Choose…".
fn file_row(flow: &ExportFlow) -> Element<'_, Message> {
    row![
        text_input("", &flow.path)
            .on_input(|v| msg(ExportMsg::Path(v)))
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
            .style(theme::search)
            // Paths are data: mono.
            .font(theme::MONO)
            .size(theme::TEXT_MONO),
        widgets::outlined(
            button(text(strings::choose_file()))
                .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_SM]))
                .on_press(msg(ExportMsg::Choose)),
            true,
        ),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center)
    .into()
}

/// What the Google Drive export reports: progress, the result and its buttons.
fn drive_outcome_view<'a>(app: &'a App, flow: &'a ExportFlow) -> Element<'a, Message> {
    let report = match &flow.drive {
        DriveExport::Uploaded { report, .. } | DriveExport::Saved { report, .. } => {
            Some(export_report(report))
        }
        _ => None,
    };
    let mut col = column![].spacing(theme::SPACE_SM);
    match drive_outcome(app) {
        DriveOutcome::None => {}
        DriveOutcome::Busy(label) => col = col.push(busy(label, app.busy_phase)),
        DriveOutcome::Uploaded { path, link } => {
            col = col
                .extend(report)
                .push(text(strings::drive_uploaded(&path)));
            if link {
                col = col.push(plain(strings::drive_open(), msg(ExportMsg::OpenInDrive)));
            }
        }
        DriveOutcome::Failed { try_again, save } => {
            if let DriveExport::Failed(error) = &flow.drive {
                col = col.push(outcome_error(strings::export_failed(), error));
            }
            let mut buttons = row![].spacing(theme::SPACE_XS).align_y(Alignment::Center);
            if try_again {
                buttons =
                    buttons.push(plain(strings::update_try_again(), msg(ExportMsg::TryAgain)));
            }
            if save {
                buttons = buttons.push(plain(
                    strings::drive_save_local(),
                    msg(ExportMsg::SaveLocal),
                ));
            }
            if try_again || save {
                col = col.push(buttons);
            }
        }
        DriveOutcome::Saved(path) => {
            col = col
                .extend(report)
                .push(text(strings::drive_saved_local(&path)));
        }
    }
    col.into()
}

fn export_report(report: &ExportReport) -> Element<'_, Message> {
    let mut col = column![text(strings::export_done(
        report.sites,
        report.workspaces,
        report.secrets
    ))]
    .spacing(theme::SPACE_SM);
    col = names(
        col,
        strings::backup_missing_secrets(),
        report.missing_secrets.iter().cloned(),
        false,
    );
    col = names(
        col,
        strings::backup_skipped_unreadable(),
        report
            .skipped_unreadable
            .iter()
            .map(|entry| strings::unreadable_name(entry.name.as_deref(), entry.file, entry.index)),
        false,
    );
    col.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::export::{Destination, DriveExport, DriveLink, ExportFlow};
    use crate::drive::{DriveAccount, DriveFile, Sealed};
    use crate::i18n::{Language, set_language};
    use hoprs_core::{Error, ErrorKind, SiteId, WorkspaceId};

    #[test]
    fn title_per_scope() {
        set_language(Language::En);
        assert_eq!(
            title(&ExportTitle::Site {
                name: "Cliente Acme".into()
            }),
            "Export \"Cliente Acme\" (site)"
        );
        assert_eq!(
            title(&ExportTitle::Workspace {
                name: "Cliente Acme".into(),
                sites: 4
            }),
            "Export \"Cliente Acme\" (workspace, 4 sites)"
        );
        assert_eq!(
            title(&ExportTitle::All {
                sites: 12,
                workspaces: 3
            }),
            "Export everything (12 sites, 3 workspaces)"
        );
    }

    #[test]
    fn partial_note_only_for_partial_scopes() {
        set_language(Language::En);
        assert_eq!(partial_note(&ExportScope::All), None);
        for scope in [
            ExportScope::Site(SiteId::new()),
            ExportScope::Workspace(WorkspaceId::new()),
        ] {
            assert_eq!(
                partial_note(&scope),
                Some(strings::export_partial_note()),
                "{scope:?}"
            );
        }
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

    fn error() -> Error {
        Error::coded(
            ErrorKind::Network,
            "drive-upload-failed",
            "Could not upload to Google Drive",
        )
    }

    fn account(email: Option<&str>, stored: bool) -> DriveAccount {
        DriveAccount {
            email: email.map(str::to_owned),
            stored,
        }
    }

    /// An app with an export flow on `destination`.
    fn app_on(destination: Destination, link: DriveLink) -> App {
        let mut app = App::new();
        let mut flow = ExportFlow::open(
            ExportScope::All,
            ExportTitle::All {
                sites: 1,
                workspaces: 0,
            },
            None,
            chrono::NaiveDate::from_ymd_opt(2026, 11, 22).unwrap(),
        );
        flow.destination = destination;
        app.export = Some(flow);
        app.drive_link = link;
        app
    }

    fn drive_app(link: DriveLink, drive: DriveExport, bytes: bool) -> App {
        let mut app = app_on(Destination::Drive, link);
        let flow = app.export.as_mut().unwrap();
        flow.drive = drive;
        if bytes {
            flow.sealed = Some((report(), Sealed::new(vec![1, 2, 3])));
        }
        app
    }

    fn connected() -> DriveLink {
        DriveLink::Connected(account(Some("ana@example.com"), true))
    }

    #[test]
    fn no_drive_row_for_the_file_destination() {
        for link in [DriveLink::Disconnected, connected()] {
            let app = app_on(Destination::File, link);
            assert_eq!(drive_row(&app), None);
        }
        assert_eq!(drive_row(&App::new()), None);
    }

    #[test]
    fn drive_row_per_link() {
        set_language(Language::En);
        let row = |link| drive_row(&app_on(Destination::Drive, link));
        assert_eq!(row(DriveLink::Unknown), Some(DriveRow::Reading));
        assert_eq!(row(DriveLink::Reading), Some(DriveRow::Reading));
        assert_eq!(row(DriveLink::Disconnected), Some(DriveRow::Connect));
        assert_eq!(
            row(DriveLink::Waiting { url: None }),
            Some(DriveRow::Waiting { can_copy: false })
        );
        assert_eq!(
            row(DriveLink::Waiting {
                url: Some("https://example.com/auth".into())
            }),
            Some(DriveRow::Waiting { can_copy: true })
        );
        assert_eq!(
            row(DriveLink::Connected(account(Some("ana@example.com"), true))),
            Some(DriveRow::Account {
                label: "ana@example.com".into(),
                session_only: false
            })
        );
        assert_eq!(
            row(DriveLink::Connected(account(None, false))),
            Some(DriveRow::Account {
                label: "Connected".into(),
                session_only: true
            })
        );
    }

    #[test]
    fn drive_outcome_per_state() {
        set_language(Language::En);
        let outcome = |link, drive, bytes| drive_outcome(&drive_app(link, drive, bytes));
        assert_eq!(
            outcome(connected(), DriveExport::Idle, false),
            DriveOutcome::None
        );
        assert_eq!(
            outcome(connected(), DriveExport::Sealing, false),
            DriveOutcome::Busy(strings::exporting())
        );
        assert_eq!(
            outcome(connected(), DriveExport::Uploading, true),
            DriveOutcome::Busy("Uploading to Google Drive…")
        );
        assert_eq!(
            outcome(connected(), DriveExport::Saving, true),
            DriveOutcome::Busy("Uploading to Google Drive…")
        );
        let file = |link: Option<&str>| DriveFile {
            name: "hoprs-cliente-acme-2026-11-22.hoprs".into(),
            link: link.map(str::to_owned),
        };
        assert_eq!(
            outcome(
                connected(),
                DriveExport::Uploaded {
                    report: report(),
                    file: file(Some("https://drive.google.com/file/d/1/view"))
                },
                false
            ),
            DriveOutcome::Uploaded {
                path: "hoprs/hoprs-cliente-acme-2026-11-22.hoprs".into(),
                link: true
            }
        );
        assert_eq!(
            outcome(
                connected(),
                DriveExport::Uploaded {
                    report: report(),
                    file: file(None)
                },
                false
            ),
            DriveOutcome::Uploaded {
                path: "hoprs/hoprs-cliente-acme-2026-11-22.hoprs".into(),
                link: false
            }
        );
        let failed = |link, bytes| outcome(link, DriveExport::Failed(error()), bytes);
        assert_eq!(
            failed(connected(), true),
            DriveOutcome::Failed {
                try_again: true,
                save: true
            }
        );
        assert_eq!(
            failed(DriveLink::Disconnected, true),
            DriveOutcome::Failed {
                try_again: false,
                save: true
            }
        );
        assert_eq!(
            failed(connected(), false),
            DriveOutcome::Failed {
                try_again: false,
                save: false
            }
        );
        let path = std::path::Path::new("dir").join("a.hoprs");
        assert_eq!(
            outcome(
                connected(),
                DriveExport::Saved {
                    report: report(),
                    path: path.clone()
                },
                false
            ),
            DriveOutcome::Saved(path.to_string_lossy().into_owned())
        );
    }

    #[test]
    fn the_file_destination_shows_nothing_of_a_drive_outcome() {
        let mut app = drive_app(connected(), DriveExport::Failed(error()), true);
        assert!(matches!(drive_outcome(&app), DriveOutcome::Failed { .. }));
        app.export.as_mut().unwrap().destination = Destination::File;
        assert_eq!(drive_outcome(&app), DriveOutcome::None);
        app.export.as_mut().unwrap().destination = Destination::Drive;
        assert!(matches!(drive_outcome(&app), DriveOutcome::Failed { .. }));
    }

    #[test]
    fn export_enabled_follows_the_destination_and_the_link() {
        let links = [
            (DriveLink::Unknown, false),
            (DriveLink::Reading, false),
            (DriveLink::Disconnected, false),
            (DriveLink::Waiting { url: None }, false),
            (connected(), true),
        ];
        for (link, on_drive) in links {
            let file = app_on(Destination::File, link.clone());
            assert!(export_enabled(&file), "file {link:?}");
            let drive = app_on(Destination::Drive, link.clone());
            assert_eq!(export_enabled(&drive), on_drive, "drive {link:?}");
        }
        let mut busy = drive_app(connected(), DriveExport::Uploading, true);
        assert!(!export_enabled(&busy));
        busy.export.as_mut().unwrap().drive = DriveExport::Idle;
        assert!(export_enabled(&busy));
        assert!(!export_enabled(&App::new()));
    }

    #[test]
    fn export_finished_per_destination() {
        let uploaded = DriveExport::Uploaded {
            report: report(),
            file: DriveFile {
                name: "a.hoprs".into(),
                link: None,
            },
        };
        let mut app = drive_app(connected(), uploaded, false);
        assert!(export_finished(&app));
        app.export.as_mut().unwrap().destination = Destination::File;
        assert!(!export_finished(&app));
        app.export.as_mut().unwrap().state = BackupState::Done(report());
        assert!(export_finished(&app));
        let failed = drive_app(connected(), DriveExport::Failed(error()), true);
        assert!(!export_finished(&failed));
        assert!(!export_finished(&App::new()));
    }
}
