//! The export screen: one site, one workspace or everything, sealed into one encrypted
//! file. A centre form like the workspace form (no modal); its state is
//! [`App::export`](crate::app::App::export).

use hoprs_vault::{ExportReport, ExportScope};
use iced::widget::{button, column, container, row, scrollable, space, text, text_input};
use iced::{Alignment, Element, Length, Padding};

use crate::app::export::{ExportFlow, ExportMsg, ExportTitle};
use crate::app::settings::BackupState;
use crate::app::{App, Message};
use crate::ui::settings::{names, outcome_error};
use crate::ui::site_form::Password;
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use crate::ui::widgets::{self, busy, confirm_with, field};

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
    let file = row![
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
    .align_y(Alignment::Center);
    content = content
        .push(field(strings::backup_file(), file.into(), None))
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
    match &flow.state {
        BackupState::Idle => {}
        BackupState::Running => content = content.push(busy(strings::exporting(), app.busy_phase)),
        BackupState::Done(report) => content = content.push(export_report(report)),
        BackupState::Failed(error) => {
            content = content.push(outcome_error(strings::export_failed(), error));
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
    if matches!(flow.state, BackupState::Done(_)) {
        return primary(strings::done(), Some(msg(ExportMsg::Close)));
    }
    row![
        primary(
            strings::export(),
            app.export_ready().then(|| msg(ExportMsg::Submit))
        ),
        plain(strings::cancel(), msg(ExportMsg::Close)),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center)
    .into()
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
    use crate::i18n::{Language, set_language};
    use hoprs_core::{SiteId, WorkspaceId};

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
}
