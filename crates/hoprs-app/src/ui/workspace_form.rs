//! The workspace form: a name and which sites belong to it.
//!
//! Sites are listed (and saved) in the order of `sites.toml`. This version does not
//! reorder: an edited workspace keeps its entries where they are (with their saved tabs),
//! drops the ones unticked and appends the new ones in file order. Entries this version
//! cannot show (a site missing or unreadable) are kept untouched.

use std::collections::HashSet;

use hoprs_core::{Error, Site, SiteId, Workspace, WorkspaceId, WorkspaceSite};
use iced::widget::{button, checkbox, column, container, row, scrollable, space, text, text_input};
use iced::{Alignment, Element, Length, Padding};

use crate::app::Message;
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use crate::ui::widgets::{confirm, failure, field};

#[derive(Debug, Clone)]
pub enum WorkspaceFormMsg {
    Name(String),
    ToggleSite(SiteId, bool),
    Save,
    Cancel,
    Export,
    Delete,
    ConfirmDelete,
    CancelDelete,
}

#[derive(Debug)]
pub enum WorkspaceFormOutcome {
    Nothing,
    Close,
    Save(Workspace),
    /// The export screen for the workspace as saved.
    Export(WorkspaceId),
    Delete(WorkspaceId),
}

#[derive(Debug)]
pub struct WorkspaceForm {
    pub editing: bool,
    base: Workspace,
    pub name: String,
    pub selected: HashSet<SiteId>,
    /// Set once a save was attempted with an empty name.
    pub name_error: Option<&'static str>,
    pub confirm_delete: bool,
    pub busy: bool,
    pub failure: Option<Error>,
}

impl WorkspaceForm {
    pub fn new_workspace() -> Self {
        let mut form = Self::edit(&Workspace {
            id: WorkspaceId::new(),
            name: String::new(),
            sites: Vec::new(),
        });
        form.editing = false;
        form
    }

    pub fn edit(workspace: &Workspace) -> Self {
        Self {
            editing: true,
            base: workspace.clone(),
            name: workspace.name.clone(),
            selected: workspace.sites.iter().map(|s| s.site.clone()).collect(),
            name_error: None,
            confirm_delete: false,
            busy: false,
            failure: None,
        }
    }

    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.base.id
    }

    /// The workspace to save, given the readable sites in file order.
    pub fn build(&self, sites: &[Site]) -> Option<Workspace> {
        let name = self.name.trim();
        if name.is_empty() {
            return None;
        }
        let shown: HashSet<&SiteId> = sites.iter().map(|s| &s.id).collect();
        let mut entries: Vec<WorkspaceSite> = self
            .base
            .sites
            .iter()
            .filter(|entry| !shown.contains(&entry.site) || self.selected.contains(&entry.site))
            .cloned()
            .collect();
        for site in sites {
            let present = entries.iter().any(|entry| entry.site == site.id);
            if self.selected.contains(&site.id) && !present {
                entries.push(WorkspaceSite {
                    site: site.id.clone(),
                    tabs: Vec::new(),
                });
            }
        }
        Some(Workspace {
            id: self.base.id.clone(),
            name: name.to_owned(),
            sites: entries,
        })
    }

    pub fn update(&mut self, msg: WorkspaceFormMsg, sites: &[Site]) -> WorkspaceFormOutcome {
        match msg {
            WorkspaceFormMsg::Name(name) => {
                self.name = name;
                if self.name_error.is_some() && !self.name.trim().is_empty() {
                    self.name_error = None;
                }
            }
            WorkspaceFormMsg::ToggleSite(site, on) => {
                if on {
                    self.selected.insert(site);
                } else {
                    self.selected.remove(&site);
                }
            }
            WorkspaceFormMsg::Save => {
                if self.busy {
                    return WorkspaceFormOutcome::Nothing;
                }
                let Some(workspace) = self.build(sites) else {
                    self.name_error = Some(strings::name_required());
                    return WorkspaceFormOutcome::Nothing;
                };
                self.busy = true;
                self.failure = None;
                return WorkspaceFormOutcome::Save(workspace);
            }
            WorkspaceFormMsg::Cancel => return WorkspaceFormOutcome::Close,
            WorkspaceFormMsg::Export => {
                if self.editing && !self.busy {
                    return WorkspaceFormOutcome::Export(self.base.id.clone());
                }
            }
            WorkspaceFormMsg::Delete => self.confirm_delete = self.editing,
            WorkspaceFormMsg::CancelDelete => self.confirm_delete = false,
            WorkspaceFormMsg::ConfirmDelete => {
                if self.editing && self.confirm_delete && !self.busy {
                    self.confirm_delete = false;
                    self.busy = true;
                    self.failure = None;
                    return WorkspaceFormOutcome::Delete(self.base.id.clone());
                }
            }
        }
        WorkspaceFormOutcome::Nothing
    }

    pub fn finished(&mut self, failure: Option<Error>) {
        self.busy = false;
        self.failure = failure;
    }
}

fn msg(m: WorkspaceFormMsg) -> Message {
    Message::WorkspaceForm(m)
}

pub fn view<'a>(form: &'a WorkspaceForm, sites: &'a [Site]) -> Element<'a, Message> {
    let title = if form.editing {
        strings::workspace_form_edit()
    } else {
        strings::workspace_form_new()
    };
    let name = field(
        strings::field_name(),
        text_input("", &form.name)
            .on_input(|v| msg(WorkspaceFormMsg::Name(v)))
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
            .size(theme::TEXT_SM)
            .style(theme::search)
            .into(),
        form.name_error,
    );

    let list: Element<'a, Message> = if sites.is_empty() {
        text(strings::workspace_no_sites())
            .size(theme::TEXT_SM)
            .color(palette().muted)
            .into()
    } else {
        column(sites.iter().map(|site| {
            let id = site.id.clone();
            row![
                checkbox(form.selected.contains(&site.id))
                    .label(&site.name)
                    .text_size(theme::TEXT_SM)
                    .style(theme::checkbox)
                    .on_toggle(move |on| msg(WorkspaceFormMsg::ToggleSite(id.clone(), on))),
                space::horizontal(),
                text(crate::ui::sidebar::protocol_tag(site))
                    .font(theme::MONO)
                    .size(theme::TEXT_TAG)
                    .color(palette().muted),
            ]
            .align_y(Alignment::Center)
            .into()
        }))
        .spacing(theme::SPACE_XS)
        .into()
    };

    let actions: Element<'a, Message> = if form.confirm_delete {
        confirm(
            strings::delete_workspace_confirm(),
            msg(WorkspaceFormMsg::ConfirmDelete),
            msg(WorkspaceFormMsg::CancelDelete),
        )
    } else {
        let mut buttons = row![
            button(
                text(if form.busy {
                    strings::saving()
                } else {
                    strings::save()
                })
                .font(theme::INTER_SEMIBOLD)
            )
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
            .style(theme::button_primary)
            .on_press_maybe((!form.busy).then(|| msg(WorkspaceFormMsg::Save))),
            button(text(strings::cancel()))
                .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                .style(theme::button_text)
                .on_press(msg(WorkspaceFormMsg::Cancel)),
            space::horizontal(),
        ]
        .align_y(Alignment::Center)
        .spacing(theme::SPACE_XS);
        if form.editing {
            buttons = buttons.push(
                button(text(strings::export_ellipsis()))
                    .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                    .style(theme::button_text)
                    .on_press_maybe((!form.busy).then(|| msg(WorkspaceFormMsg::Export))),
            );
            buttons = buttons.push(
                button(text(strings::delete()))
                    .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                    .style(theme::button_text)
                    .on_press_maybe((!form.busy).then(|| msg(WorkspaceFormMsg::Delete))),
            );
        }
        buttons.into()
    };

    let content = column![
        text(title).size(theme::TEXT_MD).font(theme::INTER_SEMIBOLD),
        name,
        iced::widget::rule::horizontal(theme::HAIRLINE).style(theme::rule_1px),
        text(strings::workspace_sites())
            .size(theme::TEXT_XS)
            .font(theme::INTER_SEMIBOLD),
        list,
        failure(form.failure.as_ref()),
        actions,
    ]
    .spacing(theme::SPACE_SM)
    .max_width(520);

    scrollable(
        container(content)
            .width(Length::Fill)
            .padding(theme::SPACE_LG),
    )
    .height(Length::Fill)
    .into()
}
