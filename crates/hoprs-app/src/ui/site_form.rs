//! The site form: create or edit a saved site, in the center area (no modal).
//!
//! The form keeps its fields as typed. Saving validates them inline (host, port), then
//! hands the site and the password change to the app as a [`SiteFormOutcome::Save`]; the
//! password leaves the form at that moment and is never shown again.

use std::fmt;
use std::path::PathBuf;

use hoprs_core::{AgentAccess, Error, FtpMode, OpenAs, Protocol, Site, SiteId};
use iced::widget::{
    button, checkbox, column, container, pick_list, row, scrollable, space, text, text_input,
};
use iced::{Alignment, Element, Length, Padding};
use secrecy::SecretString;

use crate::app::Message;
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use crate::ui::widgets::{confirm, failure, field};

/// A password as typed. Its `Debug` never shows it.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Password(pub String);

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(***)")
    }
}

/// What saving does to the site's stored password.
#[derive(Debug)]
pub enum PasswordChange {
    Keep,
    Set(SecretString),
    Forget,
}

#[derive(Debug, Clone)]
pub enum SiteFormMsg {
    Name(String),
    Host(String),
    Port(String),
    User(String),
    Protocol(Protocol),
    Password(Password),
    ForgetPassword(bool),
    UseAgent(bool),
    KeyFile(String),
    OpenAs(OpenAs),
    FtpMode(FtpMode),
    NatWorkaround(bool),
    RemoteDir(String),
    LocalDir(String),
    AgentAccess(AgentAccess),
    Save,
    Cancel,
    Export,
    Delete,
    ConfirmDelete,
    CancelDelete,
}

/// What the app must do after a form message.
#[derive(Debug)]
pub enum SiteFormOutcome {
    Nothing,
    Close,
    Save {
        site: Box<Site>,
        password: PasswordChange,
    },
    /// The export screen for the site as saved.
    Export(SiteId),
    Delete(SiteId),
}

/// Inline validation messages.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SiteErrors {
    pub host: Option<&'static str>,
    pub port: Option<&'static str>,
}

impl SiteErrors {
    pub fn is_empty(&self) -> bool {
        self.host.is_none() && self.port.is_none()
    }
}

#[derive(Debug)]
pub struct SiteForm {
    /// Editing a saved site (`false`: a new one).
    pub editing: bool,
    /// The site as loaded (or a fresh one): keeps its id and every field the form does
    /// not show.
    base: Site,
    pub name: String,
    pub host: String,
    pub port: String,
    /// The user typed a port: changing the protocol no longer replaces it.
    pub port_edited: bool,
    pub user: String,
    pub protocol: Protocol,
    /// Typed here only; emptied when the save is emitted.
    pub password: Password,
    pub forget_password: bool,
    pub use_agent: bool,
    pub key_file: String,
    pub open_as: OpenAs,
    pub ftp_mode: FtpMode,
    pub nat_workaround: bool,
    pub remote_dir: String,
    pub local_dir: String,
    pub agent_access: AgentAccess,
    /// Shown once a save was attempted, then kept current while typing.
    pub errors: Option<SiteErrors>,
    pub confirm_delete: bool,
    /// A save or delete is in flight.
    pub busy: bool,
    /// The backend refused the last save or delete.
    pub failure: Option<Error>,
}

impl SiteForm {
    pub fn new_site() -> Self {
        let mut form = Self::edit(&Site::new("", "", "", Protocol::Ssh));
        form.editing = false;
        form.port_edited = false;
        form
    }

    pub fn edit(site: &Site) -> Self {
        Self {
            editing: true,
            base: site.clone(),
            name: site.name.clone(),
            host: site.host.clone(),
            port: site.port.to_string(),
            port_edited: site.port != site.protocol.default_port(),
            user: site.user.clone(),
            protocol: site.protocol,
            password: Password::default(),
            forget_password: false,
            use_agent: site.ssh.use_agent,
            key_file: site
                .ssh
                .key_file
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            open_as: site.ssh.open_as,
            ftp_mode: site.ftp.mode,
            nat_workaround: site.ftp.nat_workaround,
            remote_dir: site.remote_dir.clone().unwrap_or_default(),
            local_dir: site
                .local_dir
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            agent_access: site.agent_access,
            errors: None,
            confirm_delete: false,
            busy: false,
            failure: None,
        }
    }

    pub fn site_id(&self) -> &SiteId {
        &self.base.id
    }

    /// The saved site has a password in the keyring.
    pub fn has_password(&self) -> bool {
        self.editing && self.base.has_password
    }

    pub fn validate(&self) -> SiteErrors {
        SiteErrors {
            host: self
                .host
                .trim()
                .is_empty()
                .then_some(strings::host_required()),
            port: match self.port.trim().parse::<u16>() {
                Ok(port) if port > 0 => None,
                _ => Some(strings::port_invalid()),
            },
        }
    }

    /// The site to save; `None` while a field is invalid.
    pub fn build(&self) -> Option<Site> {
        if !self.validate().is_empty() {
            return None;
        }
        let mut site = self.base.clone();
        let host = self.host.trim().to_owned();
        site.name = match self.name.trim() {
            "" => host.clone(),
            name => name.to_owned(),
        };
        site.host = host;
        site.port = self.port.trim().parse().ok()?;
        site.user = self.user.trim().to_owned();
        site.protocol = self.protocol;
        site.ssh.use_agent = self.use_agent;
        site.ssh.key_file = non_empty(&self.key_file).map(PathBuf::from);
        site.ssh.open_as = self.open_as;
        site.ftp.mode = self.ftp_mode;
        site.ftp.nat_workaround = self.nat_workaround;
        site.remote_dir = non_empty(&self.remote_dir);
        site.local_dir = non_empty(&self.local_dir).map(PathBuf::from);
        site.agent_access = self.agent_access;
        Some(site)
    }

    pub fn update(&mut self, msg: SiteFormMsg) -> SiteFormOutcome {
        match msg {
            SiteFormMsg::Name(v) => self.name = v,
            SiteFormMsg::Host(v) => self.host = v,
            SiteFormMsg::Port(v) => {
                self.port = v;
                self.port_edited = true;
            }
            SiteFormMsg::User(v) => self.user = v,
            SiteFormMsg::Protocol(protocol) => {
                self.protocol = protocol;
                if !self.port_edited {
                    self.port = protocol.default_port().to_string();
                }
            }
            SiteFormMsg::Password(v) => self.password = v,
            SiteFormMsg::ForgetPassword(v) => self.forget_password = v,
            SiteFormMsg::UseAgent(v) => self.use_agent = v,
            SiteFormMsg::KeyFile(v) => self.key_file = v,
            SiteFormMsg::OpenAs(v) => self.open_as = v,
            SiteFormMsg::FtpMode(v) => self.ftp_mode = v,
            SiteFormMsg::NatWorkaround(v) => self.nat_workaround = v,
            SiteFormMsg::RemoteDir(v) => self.remote_dir = v,
            SiteFormMsg::LocalDir(v) => self.local_dir = v,
            SiteFormMsg::AgentAccess(v) => self.agent_access = v,
            SiteFormMsg::Save => return self.save(),
            SiteFormMsg::Cancel => return SiteFormOutcome::Close,
            SiteFormMsg::Export => {
                if self.editing && !self.busy {
                    return SiteFormOutcome::Export(self.base.id.clone());
                }
            }
            SiteFormMsg::Delete => self.confirm_delete = self.editing,
            SiteFormMsg::CancelDelete => self.confirm_delete = false,
            SiteFormMsg::ConfirmDelete => {
                if self.editing && self.confirm_delete && !self.busy {
                    self.confirm_delete = false;
                    self.busy = true;
                    self.failure = None;
                    return SiteFormOutcome::Delete(self.base.id.clone());
                }
            }
        }
        if self.errors.is_some() {
            self.errors = Some(self.validate());
        }
        SiteFormOutcome::Nothing
    }

    fn save(&mut self) -> SiteFormOutcome {
        if self.busy {
            return SiteFormOutcome::Nothing;
        }
        let Some(site) = self.build() else {
            self.errors = Some(self.validate());
            return SiteFormOutcome::Nothing;
        };
        self.errors = None;
        self.busy = true;
        self.failure = None;
        // The password leaves the UI state here: from now on only the effect holds it.
        let typed = std::mem::take(&mut self.password.0);
        let password = if !typed.is_empty() {
            PasswordChange::Set(SecretString::from(typed))
        } else if self.forget_password {
            PasswordChange::Forget
        } else {
            PasswordChange::Keep
        };
        SiteFormOutcome::Save {
            site: Box::new(site),
            password,
        }
    }

    /// The backend finished the save or delete.
    pub fn finished(&mut self, failure: Option<Error>) {
        self.busy = false;
        self.failure = failure;
    }
}

/// The line under the level buttons: what an agent may do. "Everything" means files only
/// on an FTP site, and the hint says so.
pub fn agent_hint(access: AgentAccess, protocol: Protocol) -> Option<&'static str> {
    match access {
        AgentAccess::None => None,
        AgentAccess::Read => Some(strings::agent_access_read_hint()),
        AgentAccess::All if protocol == Protocol::Ssh => Some(strings::agent_access_all_hint_ssh()),
        AgentAccess::All => Some(strings::agent_access_all_hint_ftp()),
    }
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// `pick_list` needs `Display`: these show the UI labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProtocolChoice(Protocol);

impl fmt::Display for ProtocolChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.label())
    }
}

const PROTOCOLS: [ProtocolChoice; 4] = [
    ProtocolChoice(Protocol::Ssh),
    ProtocolChoice(Protocol::Ftp),
    ProtocolChoice(Protocol::FtpsExplicit),
    ProtocolChoice(Protocol::FtpsImplicit),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OpenAsChoice(OpenAs);

impl fmt::Display for OpenAsChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            OpenAs::Terminal => strings::open_as_terminal(),
            OpenAs::Files => strings::open_as_files(),
            OpenAs::Both => strings::open_as_both(),
        })
    }
}

const OPEN_AS: [OpenAsChoice; 3] = [
    OpenAsChoice(OpenAs::Both),
    OpenAsChoice(OpenAs::Files),
    OpenAsChoice(OpenAs::Terminal),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ModeChoice(FtpMode);

impl fmt::Display for ModeChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            FtpMode::Passive => strings::ftp_passive(),
            FtpMode::Active => strings::ftp_active(),
        })
    }
}

const MODES: [ModeChoice; 2] = [ModeChoice(FtpMode::Passive), ModeChoice(FtpMode::Active)];

/// The width of the form column.
const FORM_WIDTH: f32 = 520.0;

fn msg(m: SiteFormMsg) -> Message {
    Message::SiteForm(m)
}

pub fn view(form: &SiteForm, delete_note: Option<String>) -> Element<'_, Message> {
    let errors = form.errors.clone().unwrap_or_default();
    let title = if form.editing {
        strings::site_form_edit()
    } else {
        strings::site_form_new()
    };

    let name = field(
        strings::field_name(),
        input(
            strings::field_name_hint(),
            &form.name,
            SiteFormMsg::Name,
            false,
        ),
        None,
    );
    let protocol = field(
        strings::field_protocol(),
        pick_list(PROTOCOLS, Some(ProtocolChoice(form.protocol)), |c| {
            msg(SiteFormMsg::Protocol(c.0))
        })
        .font(theme::MONO)
        .text_size(theme::TEXT_MONO)
        .width(Length::Fill)
        .style(theme::pick_list)
        .menu_style(theme::menu)
        .into(),
        None,
    );
    let host = field(
        strings::field_host(),
        input("", &form.host, SiteFormMsg::Host, true),
        errors.host,
    );
    let port = container(field(
        strings::field_port(),
        input("", &form.port, SiteFormMsg::Port, true),
        errors.port,
    ))
    .width(140);
    let user = field(
        strings::field_user(),
        input("", &form.user, SiteFormMsg::User, true),
        None,
    );

    let password_hint = if form.has_password() {
        strings::password_replace()
    } else {
        strings::password_optional()
    };
    let mut password = column![field(
        strings::field_password(),
        text_input(password_hint, &form.password.0)
            .on_input(|v| msg(SiteFormMsg::Password(Password(v))))
            .secure(true)
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
            .size(theme::TEXT_SM)
            .style(theme::search)
            .into(),
        None,
    )]
    .spacing(theme::SPACE_2XS);
    if form.has_password() {
        password = password.push(
            row![
                muted(strings::password_saved()),
                space::horizontal(),
                checkbox(form.forget_password)
                    .label(strings::forget_password())
                    .text_size(theme::TEXT_XS)
                    .style(theme::checkbox)
                    .on_toggle(|v| msg(SiteFormMsg::ForgetPassword(v))),
            ]
            .align_y(Alignment::Center),
        );
    }

    let options: Element<'_, Message> = match form.protocol {
        Protocol::Ssh => section(
            strings::ssh_options(),
            column![
                checkbox(form.use_agent)
                    .label(strings::use_agent())
                    .text_size(theme::TEXT_SM)
                    .style(theme::checkbox)
                    .on_toggle(|v| msg(SiteFormMsg::UseAgent(v))),
                field(
                    strings::key_file(),
                    input(
                        strings::key_file_hint(),
                        &form.key_file,
                        SiteFormMsg::KeyFile,
                        true
                    ),
                    None,
                ),
                field(
                    strings::open_as(),
                    pick_list(OPEN_AS, Some(OpenAsChoice(form.open_as)), |c| {
                        msg(SiteFormMsg::OpenAs(c.0))
                    })
                    .text_size(theme::TEXT_SM)
                    .width(Length::Fill)
                    .style(theme::pick_list)
                    .menu_style(theme::menu)
                    .into(),
                    None,
                ),
            ]
            .spacing(theme::SPACE_XS)
            .into(),
        ),
        Protocol::Ftp | Protocol::FtpsExplicit | Protocol::FtpsImplicit => section(
            strings::ftp_options(),
            column![
                field(
                    strings::ftp_mode(),
                    pick_list(MODES, Some(ModeChoice(form.ftp_mode)), |c| {
                        msg(SiteFormMsg::FtpMode(c.0))
                    })
                    .text_size(theme::TEXT_SM)
                    .width(Length::Fill)
                    .style(theme::pick_list)
                    .menu_style(theme::menu)
                    .into(),
                    None,
                ),
                checkbox(form.nat_workaround)
                    .label(strings::nat_workaround())
                    .text_size(theme::TEXT_SM)
                    .style(theme::checkbox)
                    .on_toggle(|v| msg(SiteFormMsg::NatWorkaround(v))),
            ]
            .spacing(theme::SPACE_XS)
            .into(),
        ),
    };

    let folders = section(
        strings::start_folders(),
        row![
            field(
                strings::remote_dir(),
                input(
                    strings::folder_hint(),
                    &form.remote_dir,
                    SiteFormMsg::RemoteDir,
                    true
                ),
                None,
            ),
            field(
                strings::local_dir(),
                input(
                    strings::folder_hint(),
                    &form.local_dir,
                    SiteFormMsg::LocalDir,
                    true
                ),
                None,
            ),
        ]
        .spacing(theme::SPACE_SM)
        .into(),
    );

    let level = |label, access: AgentAccess| {
        button(text(label))
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
            .style(theme::segment(form.agent_access == access))
            .on_press(msg(SiteFormMsg::AgentAccess(access)))
    };
    let mut agents_body = column![
        row![
            level(strings::agent_access_none(), AgentAccess::None),
            level(strings::agent_access_read(), AgentAccess::Read),
            level(strings::agent_access_all(), AgentAccess::All),
        ]
        .spacing(theme::SPACE_2XS)
    ]
    .spacing(theme::SPACE_XS);
    if let Some(hint) = agent_hint(form.agent_access, form.protocol) {
        agents_body = agents_body.push(muted(hint));
    }
    let agents = section(strings::agents_section(), agents_body.into());

    let content = column![
        text(title).size(theme::TEXT_MD).font(theme::INTER_SEMIBOLD),
        name,
        protocol,
        row![host, port].spacing(theme::SPACE_SM),
        user,
        password,
        options,
        folders,
        agents,
        failure(form.failure.as_ref()),
        actions(form, delete_note),
    ]
    .spacing(theme::SPACE_SM)
    .max_width(FORM_WIDTH);

    scrollable(
        container(content)
            .width(Length::Fill)
            .padding(theme::SPACE_LG),
    )
    .height(Length::Fill)
    .into()
}

fn actions(form: &SiteForm, delete_note: Option<String>) -> Element<'_, Message> {
    if form.confirm_delete {
        let confirm = confirm(
            strings::delete_site_confirm(),
            msg(SiteFormMsg::ConfirmDelete),
            msg(SiteFormMsg::CancelDelete),
        );
        // Files of the site open for editing: what happens to their unsent changes.
        return match delete_note {
            Some(note) => column![
                text(note).size(theme::TEXT_SM).color(palette().muted),
                confirm
            ]
            .spacing(theme::SPACE_XS)
            .into(),
            None => confirm,
        };
    }
    let save_label = if form.busy {
        strings::saving()
    } else {
        strings::save()
    };
    let mut buttons = row![
        button(text(save_label).font(theme::INTER_SEMIBOLD))
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
            .style(theme::button_primary)
            .on_press_maybe((!form.busy).then(|| msg(SiteFormMsg::Save))),
        button(text(strings::cancel()))
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
            .style(theme::button_text)
            .on_press(msg(SiteFormMsg::Cancel)),
        space::horizontal(),
    ]
    .align_y(Alignment::Center)
    .spacing(theme::SPACE_XS);
    if form.editing {
        buttons = buttons.push(
            button(text(strings::export_ellipsis()))
                .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                .style(theme::button_text)
                .on_press_maybe((!form.busy).then(|| msg(SiteFormMsg::Export))),
        );
        buttons = buttons.push(
            button(text(strings::delete()))
                .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                .style(theme::button_text)
                .on_press_maybe((!form.busy).then(|| msg(SiteFormMsg::Delete))),
        );
    }
    buttons.into()
}

fn input<'a>(
    placeholder: &'a str,
    value: &'a str,
    on_input: fn(String) -> SiteFormMsg,
    mono: bool,
) -> Element<'a, Message> {
    let input = text_input(placeholder, value)
        .on_input(move |v| msg(on_input(v)))
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
        .style(theme::search);
    if mono {
        input.font(theme::MONO).size(theme::TEXT_MONO).into()
    } else {
        input.size(theme::TEXT_SM).into()
    }
}

/// A titled group of fields under a 1 px rule.
fn section<'a>(title: &'a str, body: Element<'a, Message>) -> Element<'a, Message> {
    column![
        iced::widget::rule::horizontal(theme::HAIRLINE).style(theme::rule_1px),
        text(title).size(theme::TEXT_XS).font(theme::INTER_SEMIBOLD),
        body,
    ]
    .spacing(theme::SPACE_XS)
    .into()
}

fn muted(label: &str) -> Element<'_, Message> {
    text(label)
        .size(theme::TEXT_XS)
        .color(palette().muted)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_access_starts_from_the_site_and_is_saved() {
        let mut site = Site::new("a", "h", "u", Protocol::Ssh);
        site.agent_access = AgentAccess::Read;
        let mut form = SiteForm::edit(&site);
        assert_eq!(form.agent_access, AgentAccess::Read);
        assert_eq!(form.build().unwrap().agent_access, AgentAccess::Read);
        form.update(SiteFormMsg::AgentAccess(AgentAccess::All));
        assert_eq!(form.build().unwrap().agent_access, AgentAccess::All);

        let mut fresh = SiteForm::new_site();
        fresh.update(SiteFormMsg::Host("h".into()));
        assert_eq!(fresh.agent_access, AgentAccess::None);
        assert_eq!(fresh.build().unwrap().agent_access, AgentAccess::None);
    }

    #[test]
    fn agent_hint_per_level_and_protocol() {
        assert_eq!(agent_hint(AgentAccess::None, Protocol::Ssh), None);
        assert_eq!(agent_hint(AgentAccess::None, Protocol::Ftp), None);
        assert_eq!(
            agent_hint(AgentAccess::Read, Protocol::Ssh),
            Some(strings::agent_access_read_hint())
        );
        assert_eq!(
            agent_hint(AgentAccess::All, Protocol::Ssh),
            Some(strings::agent_access_all_hint_ssh())
        );
        for protocol in [
            Protocol::Ftp,
            Protocol::FtpsExplicit,
            Protocol::FtpsImplicit,
        ] {
            assert_eq!(
                agent_hint(AgentAccess::All, protocol),
                Some(strings::agent_access_all_hint_ftp())
            );
        }
        assert_ne!(
            strings::agent_access_all_hint_ssh(),
            strings::agent_access_all_hint_ftp()
        );
    }

    #[test]
    fn changing_the_protocol_keeps_the_level() {
        let mut form = SiteForm::new_site();
        form.update(SiteFormMsg::Host("h".into()));
        form.update(SiteFormMsg::AgentAccess(AgentAccess::All));
        form.update(SiteFormMsg::Protocol(Protocol::Ftp));
        assert_eq!(form.agent_access, AgentAccess::All);
        assert_eq!(form.build().unwrap().agent_access, AgentAccess::All);
    }
}
