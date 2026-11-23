//! The security and credential dialogs (the app's only modals) and the changed host key
//! explanation shown in a tab.
//!
//! Each dialog is described by a [`Content`] (what it says and asks), built from its
//! request by [`content`]; [`view`] draws it. The tests check the content.

use iced::widget::{
    button, checkbox, column, container, opaque, row, space, stack, text, text_input,
};
use iced::{Alignment, Element, Length, Padding};

use crate::app::{
    App, Dialog, DialogMsg, FailureAction, KnownHostsFix, KnownHostsMsg, Message, Tab, Typed,
};
use crate::backend::KnownHostEntry;
use crate::backend::broker::PromptKind;
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use hoprs_remote::CertProblem;

/// The dialog box's width.
const WIDTH: f32 = 460.0;

/// The id of a dialog's field `index` (the first one gets the focus when shown).
pub fn field_id(index: usize) -> iced::widget::Id {
    iced::widget::Id::from(format!("dialog-field-{index}"))
}

/// What a dialog says and asks.
#[derive(Debug, Clone, PartialEq)]
pub struct Content {
    pub title: String,
    pub intro: Option<String>,
    /// Label and value; values are data (host, fingerprint, path), shown in mono.
    pub rows: Vec<(&'static str, String)>,
    /// Shown in the error colour.
    pub warning: Option<String>,
    /// The inputs: label and whether the answer may be shown.
    pub fields: Vec<(String, bool)>,
    /// Offers "Save in the system keyring".
    pub save: bool,
    /// The action besides Cancel: "Trust and connect", "Connect"…
    pub action: &'static str,
    /// A trust decision: Cancel is the default (white) button and Enter means Cancel.
    pub decision: bool,
    /// An agent asked for this connection: one muted line says so. Information only; it
    /// changes no button and no key.
    pub by_agent: bool,
}

/// The content of `dialog`.
pub fn content(dialog: &Dialog) -> Content {
    let request = &dialog.request;
    let base = Content {
        title: String::new(),
        intro: None,
        rows: Vec::new(),
        warning: None,
        fields: Vec::new(),
        save: dialog.can_save(),
        action: strings::connect(),
        decision: dialog.is_decision(),
        by_agent: request.by_agent,
    };
    match &request.kind {
        PromptKind::HostKey {
            key_type,
            fingerprint,
            other_key_types,
            ..
        } => Content {
            title: strings::host_key_title().to_owned(),
            intro: Some(strings::host_key_intro().to_owned()),
            rows: vec![
                (strings::label_host(), request.key.clone()),
                (strings::label_key_type(), key_type.clone()),
                (strings::label_fingerprint(), fingerprint.clone()),
            ],
            warning: (!other_key_types.is_empty())
                .then(|| strings::other_key_types(other_key_types)),
            action: strings::trust_and_connect(),
            ..base
        },
        PromptKind::Certificate {
            fingerprint,
            problem,
            ..
        } => {
            let host = (strings::label_host(), request.key.clone());
            match problem {
                CertProblem::Untrusted { reason } => Content {
                    title: strings::cert_untrusted_title().to_owned(),
                    intro: Some(strings::cert_untrusted_intro().to_owned()),
                    rows: vec![
                        host,
                        (strings::label_reason(), strings::cert_reason(reason)),
                        (strings::label_fingerprint(), fingerprint.clone()),
                    ],
                    action: strings::trust_certificate(),
                    ..base
                },
                CertProblem::Changed { pinned, reason } => Content {
                    title: strings::cert_changed_title().to_owned(),
                    intro: None,
                    rows: vec![
                        host,
                        (strings::label_reason(), strings::cert_reason(reason)),
                        (strings::label_trusted_before(), pinned.clone()),
                        (strings::label_presented_now(), fingerprint.clone()),
                    ],
                    warning: Some(strings::cert_changed_intro().to_owned()),
                    action: strings::trust_certificate(),
                    ..base
                },
            }
        }
        PromptKind::Password { user, .. } => Content {
            title: strings::password_title().to_owned(),
            rows: vec![(strings::label_account(), format!("{user}@{}", request.key))],
            fields: vec![(strings::field_password().to_owned(), false)],
            ..base
        },
        PromptKind::Passphrase { path } => Content {
            title: strings::passphrase_title().to_owned(),
            rows: vec![
                (strings::label_host(), request.key.clone()),
                (strings::label_key_file(), path.display().to_string()),
            ],
            fields: vec![(strings::field_passphrase().to_owned(), false)],
            action: strings::unlock(),
            ..base
        },
        PromptKind::KeyboardInteractive {
            name,
            instructions,
            prompts,
        } => Content {
            title: if name.trim().is_empty() {
                strings::kbd_title().to_owned()
            } else {
                name.clone()
            },
            intro: (!instructions.trim().is_empty()).then(|| instructions.clone()),
            rows: vec![(strings::label_host(), request.key.clone())],
            fields: prompts
                .iter()
                .map(|p| (p.text.trim().to_owned(), p.echo))
                .collect(),
            action: strings::continue_(),
            ..base
        },
    }
}

/// `base` with the open dialog over it, if any: a backdrop that takes every click, and
/// the dialog box in the middle.
pub fn overlay<'a>(app: &'a App, base: Element<'a, Message>) -> Element<'a, Message> {
    let Some(dialog) = &app.dialogs.current else {
        return base;
    };
    let waiting = app.dialogs.queued().count();
    let mut boxed = column![view(dialog)]
        .spacing(theme::SPACE_XS)
        .align_x(Alignment::Center);
    if waiting > 0 {
        boxed = boxed.push(
            text(strings::dialogs_waiting(waiting))
                .size(theme::TEXT_XS)
                .color(palette().muted),
        );
    }
    let layer = opaque(container(boxed).center(Length::Fill).style(theme::backdrop));
    stack![base, layer].into()
}

fn msg(m: DialogMsg) -> Message {
    Message::Dialog(m)
}

/// The dialog box.
pub fn view(dialog: &Dialog) -> Element<'_, Message> {
    let p = palette();
    let content = content(dialog);
    let mut body = column![
        text(content.title)
            .size(theme::TEXT_MD)
            .font(theme::INTER_SEMIBOLD)
    ]
    .spacing(theme::SPACE_SM);
    if content.by_agent {
        body = body.push(
            text(strings::prompt_by_agent())
                .size(theme::TEXT_SM)
                .color(p.muted),
        );
    }
    if let Some(intro) = content.intro {
        body = body.push(text(intro).size(theme::TEXT_SM).color(p.ink_2));
    }
    for (label, value) in content.rows {
        body = body.push(
            column![
                text(label).size(theme::TEXT_XS).color(p.muted),
                text(value)
                    .font(theme::MONO)
                    .size(theme::TEXT_MONO)
                    .wrapping(text::Wrapping::WordOrGlyph),
            ]
            .spacing(theme::SPACE_3XS),
        );
    }
    if let Some(warning) = content.warning {
        body = body.push(text(warning).size(theme::TEXT_SM).color(p.error));
    }
    for (index, ((label, echo), typed)) in
        content.fields.into_iter().zip(&dialog.fields).enumerate()
    {
        let input = text_input("", &typed.0)
            .id(field_id(index))
            .on_input(move |v| msg(DialogMsg::Input(index, Typed(v))))
            .on_submit(msg(DialogMsg::Submit))
            .secure(!echo)
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
            .size(theme::TEXT_SM)
            .style(theme::search);
        body = body.push(
            column![text(label).size(theme::TEXT_XS).color(p.muted), input]
                .spacing(theme::SPACE_2XS),
        );
    }
    if content.save {
        body = body.push(
            column![
                checkbox(dialog.save)
                    .label(strings::save_in_keyring())
                    .text_size(theme::TEXT_SM)
                    .style(theme::checkbox)
                    .on_toggle(|v| msg(DialogMsg::Save(v))),
                text(strings::save_in_keyring_hint())
                    .size(theme::TEXT_XS)
                    .color(p.muted),
            ]
            .spacing(theme::SPACE_3XS),
        );
    }
    body = body.push(buttons(content.action, content.decision));
    container(body)
        .width(WIDTH)
        .padding(theme::SPACE_LG)
        .style(theme::dialog_box)
        .into()
}

/// A decision: "Trust…" outlined, then Cancel as the default (white) button. A
/// credential: Cancel as text, then the action in white.
fn buttons<'a>(action: &'a str, decision: bool) -> Element<'a, Message> {
    let pad = Padding::from([theme::SPACE_2XS, theme::SPACE_MD]);
    let cancel = button(text(strings::cancel()).font(theme::INTER_SEMIBOLD))
        .padding(pad)
        .on_press(msg(DialogMsg::Cancel));
    let row = if decision {
        row![
            space::horizontal(),
            button(text(action))
                .padding(pad)
                .style(theme::button_outline)
                .on_press(msg(DialogMsg::Trust)),
            cancel.style(theme::button_primary),
        ]
    } else {
        row![
            space::horizontal(),
            cancel.style(theme::button_text),
            button(text(action).font(theme::INTER_SEMIBOLD))
                .padding(pad)
                .style(theme::button_primary)
                .on_press(msg(DialogMsg::Submit)),
        ]
    };
    row.spacing(theme::SPACE_XS)
        .align_y(Alignment::Center)
        .into()
}

/// The warning to show before removing `entries`: some also name other hosts.
pub fn known_hosts_warning(entries: &[KnownHostEntry]) -> Option<String> {
    let lines: Vec<usize> = entries
        .iter()
        .filter(|e| e.covers_other_hosts)
        .map(|e| e.line)
        .collect();
    (!lines.is_empty()).then(|| strings::also_covers_other_hosts(&lines))
}

/// The tab's blocking explanation of a changed host key: no way to accept the key, only
/// "Remove the old entry…" (with its confirmation in place) or "Edit site".
pub fn host_key_changed<'a>(app: &'a App, tab: &'a Tab) -> Element<'a, Message> {
    container(host_key_changed_body(app, tab))
        .center(Length::Fill)
        .into()
}

/// The changed host key's explanation and actions, without the centering: the failed
/// tab's remote pane centers it, a lost session's banner spans the tab with it.
pub fn host_key_changed_body<'a>(app: &'a App, tab: &'a Tab) -> iced::widget::Column<'a, Message> {
    let p = palette();
    let fix = |m: KnownHostsMsg| Message::KnownHosts {
        tab: tab.id,
        msg: m,
    };
    let pad = Padding::from([theme::SPACE_2XS, theme::SPACE_XS]);
    let conn = app.conn(&tab.site);
    let line = conn.host_key_change().and_then(|change| change.line);
    let explanation = if line.is_some() {
        strings::host_key_changed_body()
    } else {
        strings::host_key_changed_session_body()
    };
    let mut body = column![
        text(strings::host_key_changed_title())
            .font(theme::INTER_SEMIBOLD)
            .color(p.error),
        text(explanation).size(theme::TEXT_SM).color(p.ink_2),
    ]
    .spacing(theme::SPACE_XS)
    .max_width(WIDTH);
    if let Some(line) = line {
        body = body.push(
            text(strings::known_hosts_line(line))
                .font(theme::MONO)
                .size(theme::TEXT_MONO)
                .color(p.muted),
        );
    }
    let host = conn
        .failed_endpoint
        .as_ref()
        .map(|(host, port)| format!("{host}:{port}"))
        .unwrap_or_default();
    let mut actions = row![].spacing(theme::SPACE_XS);
    for action in app.failure_actions(tab) {
        let (label, style, on_press): (_, fn(&iced::Theme, _) -> _, _) = match action {
            FailureAction::RemoveOldEntry => (
                strings::remove_old_entry(),
                theme::button_outline,
                fix(KnownHostsMsg::Start),
            ),
            FailureAction::CloseAndReconnect => (
                strings::close_and_reconnect(),
                theme::button_outline,
                Message::RestartSession(tab.id),
            ),
            FailureAction::EditSite => (
                strings::edit_site(),
                theme::button_text,
                Message::EditSite(tab.site.clone()),
            ),
            FailureAction::Reconnect => continue,
        };
        actions = actions.push(
            button(text(label))
                .padding(pad)
                .style(style)
                .on_press(on_press),
        );
    }
    let muted = |label: &'a str| text(label).size(theme::TEXT_SM).color(p.muted);
    body = match &conn.known_hosts {
        KnownHostsFix::Idle => body.push(actions),
        KnownHostsFix::Checking => body.push(muted(strings::reading_known_hosts())),
        KnownHostsFix::Removing => body.push(muted(strings::removing_known_hosts())),
        KnownHostsFix::Failed(error) => body
            .push(crate::ui::widgets::failure(Some(error)))
            .push(actions),
        KnownHostsFix::Confirm(entries) => body.push(confirm(entries, host, fix)),
    };
    body
}

/// "Remove N lines for host from known_hosts?", the lines, the warning, Remove / Keep.
fn confirm<'a>(
    entries: &'a [KnownHostEntry],
    host: String,
    fix: impl Fn(KnownHostsMsg) -> Message,
) -> Element<'a, Message> {
    let p = palette();
    let pad = Padding::from([theme::SPACE_2XS, theme::SPACE_MD]);
    let (question, yes) = if entries.is_empty() {
        (
            strings::no_known_hosts_entry().to_owned(),
            strings::connect_again(),
        )
    } else {
        (
            strings::remove_known_hosts_question(entries.len(), &host),
            strings::remove(),
        )
    };
    let mut body =
        column![text(question).size(theme::TEXT_SM).color(p.ink_2)].spacing(theme::SPACE_XS);
    for entry in entries {
        body = body.push(
            text(format!(
                "{}  {}",
                strings::known_hosts_line(entry.line),
                entry.key_type
            ))
            .font(theme::MONO)
            .size(theme::TEXT_MONO)
            .color(p.ink_2),
        );
    }
    if let Some(warning) = known_hosts_warning(entries) {
        body = body.push(text(warning).size(theme::TEXT_SM).color(p.error));
    }
    body = body.push(
        row![
            button(text(yes).font(theme::INTER_SEMIBOLD))
                .padding(pad)
                .style(theme::button_primary)
                .on_press(fix(KnownHostsMsg::Confirm)),
            button(text(strings::keep()))
                .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                .style(theme::button_text)
                .on_press(fix(KnownHostsMsg::Cancel)),
        ]
        .spacing(theme::SPACE_XS),
    );
    container(body)
        .padding(theme::SPACE_SM)
        .style(theme::outline_box)
        .into()
}

#[cfg(test)]
mod tests {
    use hoprs_core::SiteId;
    use hoprs_remote::{CertProblem, CertReason};

    use super::*;
    use crate::backend::broker::{PromptKind, PromptRequest};
    use crate::backend::{BrokerEvent, PromptId, SessionId};

    /// The content of the dialog the app shows for such a request.
    fn shown(kind: PromptKind, by_agent: bool) -> Content {
        let mut app = App::new();
        app.update(Message::Prompt(BrokerEvent::Request(PromptRequest {
            id: PromptId(1),
            session: SessionId(1),
            site: SiteId::new(),
            key: "h:22".into(),
            kind,
            by_agent,
        })));
        content(app.dialogs.current.as_ref().expect("the dialog is shown"))
    }

    fn kinds() -> Vec<PromptKind> {
        vec![
            PromptKind::HostKey {
                host: "h".into(),
                port: 22,
                key_type: "ssh-ed25519".into(),
                fingerprint: "SHA256:x".into(),
                other_key_types: Vec::new(),
            },
            PromptKind::Certificate {
                host: "h".into(),
                port: 21,
                fingerprint: "SHA256:y".into(),
                problem: CertProblem::Untrusted {
                    reason: CertReason::UnknownIssuer,
                },
            },
            PromptKind::Password {
                user: "u".into(),
                host: "h".into(),
            },
            PromptKind::Passphrase { path: "/k".into() },
            PromptKind::KeyboardInteractive {
                name: String::new(),
                instructions: String::new(),
                prompts: Vec::new(),
            },
        ]
    }

    #[test]
    fn agent_dialogs_say_so() {
        for kind in kinds() {
            let flagged = shown(kind.clone(), true);
            assert!(flagged.by_agent, "{kind:?}");
            let plain = shown(kind.clone(), false);
            assert!(!plain.by_agent, "{kind:?}");
            // Information only: the buttons and the default are the same.
            assert_eq!(
                (
                    flagged.action,
                    flagged.decision,
                    flagged.save,
                    flagged.fields.len()
                ),
                (plain.action, plain.decision, plain.save, plain.fields.len()),
                "{kind:?}"
            );
        }
    }
}
