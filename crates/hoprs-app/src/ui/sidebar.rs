//! The 256 px sidebar of mockup v2: filter, Workspaces (expandable, with "Open
//! workspace"), All sites, the unreadable entries and the footer.

use hoprs_core::{Error, OpenAs, Protocol, Site, SiteId, Workspace};
use iced::widget::{
    button, column, container, hover, mouse_area, row, rule, scrollable, space, text, text_input,
    tooltip,
};
use iced::{Alignment, Element, Length, Padding};

use crate::app::{
    App, ConnStatus, Load, MenuMsg, MenuTarget, Message, SidebarMsg, SidebarSection, TabState,
};
use crate::ui::fit;
use crate::ui::icons::{self, Icon};
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use crate::ui::widgets;

pub const WIDTH: f32 = 256.0;
pub const SEARCH_HEIGHT: f32 = 32.0;
/// The footer row ("Import from FileZilla", settings) with the rule above it: the update
/// card floats this far above the window's bottom edge.
pub const FOOTER_HEIGHT: f32 = FOOTER_ROW + theme::HAIRLINE;
/// The footer row's height: an icon button and 4 px above and below.
const FOOTER_ROW: f32 = widgets::ICON_HIT + 2.0 * theme::SPACE_2XS;

/// Whether `site` matches the filter: name, host or user contains it, ignoring case.
pub fn site_matches(site: &Site, filter: &str) -> bool {
    let filter = filter.trim().to_lowercase();
    filter.is_empty()
        || [&site.name, &site.host, &site.user]
            .iter()
            .any(|field| field.to_lowercase().contains(&filter))
}

/// The protocol tag shown next to a site, as in the mockup: what the site opens.
pub fn protocol_tag(site: &Site) -> &'static str {
    match site.protocol {
        Protocol::Ssh => match site.ssh.open_as {
            OpenAs::Both => "ssh + sftp",
            OpenAs::Files => "sftp",
            OpenAs::Terminal => "ssh",
        },
        Protocol::Ftp => "ftp",
        Protocol::FtpsExplicit | Protocol::FtpsImplicit => "ftps",
    }
}

pub fn view(app: &App) -> Element<'_, Message> {
    let search = container(
        text_input(strings::filter_sites(), &app.site_filter)
            .on_input(Message::FilterSites)
            .padding(Padding::from([0.0, theme::SPACE_XS]))
            .size(theme::TEXT_SM)
            .line_height(iced::Pixels(SEARCH_HEIGHT - 2.0 * theme::HAIRLINE))
            .style(theme::search),
    )
    .padding(theme::SPACE_SM);

    let footer = row![
        widgets::tool_button(
            Icon::DownloadSimple,
            strings::import_filezilla(),
            None,
            true,
            Some(Message::OpenImport),
        ),
        space::horizontal(),
        // The `Ctrl ,` hint joins the tooltip with the shortcut (Plan 13 task 8).
        widgets::icon_button(
            Icon::GearSix,
            strings::settings(),
            None,
            Some(Message::OpenSettings),
        ),
    ]
    .align_y(Alignment::Center);

    let body: Element<'_, Message> = match &app.load {
        Load::Loading => skeleton(),
        Load::Failed(error) => column![
            group_rule(),
            container(
                column![
                    text(strings::cannot_load_sites()).color(palette().error),
                    text(crate::ui::strings::error(error))
                        .size(theme::TEXT_XS)
                        .color(palette().muted),
                    action(strings::retry(), Message::ReloadProfiles),
                ]
                .spacing(theme::SPACE_2XS)
            )
            .padding(Padding::from([theme::SPACE_XS, theme::SPACE_MD])),
        ]
        .into(),
        Load::Loaded => {
            let mut lists = column![];
            if !app.settings.workspaces.hidden {
                lists = lists.push(workspaces(app));
            }
            if !app.settings.sites.hidden {
                lists = lists.push(all_sites(app));
            }
            if app.settings.workspaces.hidden && app.settings.sites.hidden {
                lists = lists.push(column![group_rule(), note(strings::sidebar_all_hidden())]);
            }
            if !app.unreadable.is_empty() || app.sidebar_error.is_some() {
                lists = lists.push(unreadable(app));
            }
            scrollable(lists).height(Length::Fill).into()
        }
    };

    let mut content =
        column![search, container(body).height(Length::Fill), group_rule()].height(Length::Fill);
    // "↓ 1.4.0 available" above the footer row, while the update card is collapsed.
    if let Some(line) = crate::ui::update::footer_line(app) {
        content = content.push(
            container(line).padding(Padding::from([theme::SPACE_3XS, theme::SPACE_SM]).bottom(0)),
        );
    }
    let content = content.push(
        container(footer)
            .center_y(FOOTER_ROW)
            .padding(Padding::from([0.0, theme::SPACE_SM])),
    );

    container(content)
        .width(WIDTH - theme::HAIRLINE)
        .height(Length::Fill)
        .style(theme::sidebar)
        .into()
}

fn group_rule<'a>() -> Element<'a, Message> {
    rule::horizontal(theme::HAIRLINE)
        .style(theme::rule_1px)
        .into()
}

/// A muted line in the group's padding.
fn note(label: &str) -> Element<'_, Message> {
    container(text(label).size(theme::TEXT_SM).color(palette().muted))
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
        .into()
}

fn action(label: &str, on_press: Message) -> Element<'_, Message> {
    button(text(label).size(theme::TEXT_XS))
        .padding(0)
        .style(theme::button_text)
        .on_press(on_press)
        .into()
}

/// A sidebar group: rule, heading (fold caret, title, then "+" to create, or the count
/// when folded), then its body unless folded.
fn group<'a>(
    section: SidebarSection,
    collapsed: bool,
    title: &'a str,
    count: usize,
    new_label: &'a str,
    on_new: Message,
    body: Element<'a, Message>,
) -> Element<'a, Message> {
    let caret = widgets::icon_button_small(
        if collapsed {
            Icon::CaretRight
        } else {
            Icon::CaretDown
        },
        if collapsed {
            strings::sidebar_expand()
        } else {
            strings::sidebar_collapse()
        },
        None,
        Some(Message::Sidebar(SidebarMsg::ToggleCollapsed(section))),
    );
    let end: Element<'a, Message> = if collapsed {
        text(count.to_string())
            .size(theme::TEXT_XS)
            .color(palette().muted)
            .into()
    } else {
        widgets::icon_button_small(Icon::Plus, new_label, None, Some(on_new))
    };
    let head = row![
        caret,
        text(title).size(theme::TEXT_XS).color(palette().muted),
        space::horizontal(),
        end,
    ]
    .spacing(theme::SPACE_2XS)
    .align_y(Alignment::Center)
    .padding(Padding::from([0.0, theme::SPACE_SM]));

    let content: Element<'a, Message> = if collapsed {
        head.into()
    } else {
        column![head, body].spacing(theme::SPACE_2XS).into()
    };
    column![
        group_rule(),
        container(content).padding(Padding::from([theme::SPACE_XS, 0.0])),
    ]
    .into()
}
/// How a site's connection reads in the sidebar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SiteState<'a> {
    /// No connection record: the site has no tabs.
    Idle,
    Connecting,
    Connected,
    Failed(&'a Error),
}

/// The state of `site`'s connection. Reads the record itself, not [`App::conn`], which
/// answers "connecting" for a site without tabs.
pub fn site_state<'a>(app: &'a App, site: &SiteId) -> SiteState<'a> {
    let Some(conn) = app.conns.get(site) else {
        return SiteState::Idle;
    };
    match &conn.state {
        TabState::Failed(failure) => SiteState::Failed(&failure.error),
        _ => match conn.conn_status() {
            ConnStatus::Connected => SiteState::Connected,
            ConnStatus::Disconnected(error) => SiteState::Failed(error),
            _ => SiteState::Connecting,
        },
    }
}

/// The icon of a site by what it opens: a terminal for SSH, a drive for FTP.
pub fn site_icon(site: &Site) -> Icon {
    match site.protocol {
        Protocol::Ssh => Icon::TerminalWindow,
        Protocol::Ftp | Protocol::FtpsExplicit | Protocol::FtpsImplicit => Icon::HardDrives,
    }
}

/// The icon, its colour and the name's colour of a site row by `state`. Connecting reads
/// as busy (arrows, name dimmed) so it differs from Idle.
fn state_look(site: &Site, state: SiteState<'_>) -> (Icon, iced::Color, iced::Color) {
    let p = palette();
    match state {
        SiteState::Idle => (site_icon(site), p.muted, p.ink_2),
        SiteState::Connecting => (Icon::ArrowsClockwise, p.muted, p.ink_2),
        SiteState::Connected => (Icon::PlugsConnected, p.success, p.ink),
        SiteState::Failed(_) => (Icon::Warning, p.error, p.ink_2),
    }
}

/// The workspace head's icon and name colours: green and bright while it is the open one.
fn workspace_look(is_open_workspace: bool, expanded: bool) -> (iced::Color, iced::Color) {
    let p = palette();
    let icon = if is_open_workspace {
        p.success
    } else {
        p.muted
    };
    let name = if is_open_workspace || expanded {
        p.ink
    } else {
        p.ink_2
    };
    (icon, name)
}

/// `full` on one clipped line cut to `width`; the whole name in a tooltip only when cut.
fn named<'a>(
    full: &'a str,
    width: f32,
    font: iced::Font,
    color: iced::Color,
) -> Element<'a, Message> {
    let shown = fit::ellipsize(full, width, theme::TEXT_SM, fit::INTER_ADVANCE);
    let cut = matches!(shown, std::borrow::Cow::Owned(_));
    let line: Element<'a, Message> = container(
        text(shown)
            .font(font)
            .size(theme::TEXT_SM)
            .color(color)
            .wrapping(text::Wrapping::None),
    )
    .width(Length::Fill)
    .clip(true)
    .into();
    if cut {
        widgets::tip(line, full, None)
    } else {
        line
    }
}

/// Horizontal room around a site row in the All sites list: the list's and the button's
/// padding.
const ROW_CHROME: f32 = 4.0 * theme::SPACE_XS;
/// The same inside an open workspace: the nested list's indent, rule and padding.
const NESTED_CHROME: f32 =
    theme::SPACE_LG + 3.0 * theme::SPACE_XS + theme::HAIRLINE + 2.0 * theme::SPACE_XS;

/// Room kept right of a row for the scrollbar.
const SCROLL_GUTTER: f32 = 10.0;

/// The pencil that shows over a hovered row (right-aligned, on a patch that hides what is
/// under it) and opens its form.
fn hover_edit<'a>(row_view: Element<'a, Message>, on_edit: Message) -> Element<'a, Message> {
    let pencil = container(widgets::icon_button(
        Icon::PencilSimple,
        strings::edit(),
        None,
        Some(on_edit),
    ))
    .padding(Padding::from([0.0, theme::SPACE_2XS]))
    .style(theme::hover_patch);
    hover(
        row_view,
        container(pencil)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_right(Length::Fill)
            .center_y(Length::Fill),
    )
}

/// A site row: state icon, name, protocol tag in mono on the right; a click connects.
/// With `edit`, a pencil shows over the row while it is hovered and opens the site form.
/// `chrome` is the horizontal room the row's surroundings take, for fitting the name.
fn site_row<'a>(app: &'a App, site: &'a Site, edit: bool, chrome: f32) -> Element<'a, Message> {
    let p = palette();
    let state = site_state(app, &site.id);
    let tag = protocol_tag(site);
    let tag_width = tag.chars().count() as f32 * theme::TEXT_TAG * fit::MONO_ADVANCE;
    let name_width = (WIDTH
        - theme::HAIRLINE
        - chrome
        - icons::SMALL
        - 2.0 * theme::SPACE_XS
        - tag_width
        - SCROLL_GUTTER)
        .max(0.0);
    let (glyph, glyph_color, name_color) = state_look(site, state);
    let glyph = icons::icon(glyph, icons::SMALL).color(glyph_color);
    let glyph: Element<'a, Message> = match state {
        SiteState::Idle => glyph.into(),
        SiteState::Connecting => state_tip(glyph, strings::connecting().to_owned()),
        SiteState::Connected => state_tip(glyph, strings::connected().to_owned()),
        SiteState::Failed(error) => state_tip(glyph, strings::error(error)),
    };
    let name = named(&site.name, name_width, theme::INTER, name_color);
    let open = button(
        row![
            glyph,
            name,
            text(tag)
                .font(theme::MONO)
                .size(theme::TEXT_TAG)
                .color(p.muted)
                .wrapping(text::Wrapping::None),
        ]
        .spacing(theme::SPACE_XS)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
    .style(theme::row)
    .on_press(Message::ConnectSite(site.id.clone()));
    let open = if edit {
        hover_edit(open.into(), Message::EditSite(site.id.clone()))
    } else {
        open.into()
    };
    // A right press: Open files, Edit, Export… (mouse only; the first two have their click and the palette,
    // Export… its button in the site's form).
    mouse_area(open)
        .on_right_press(Message::Menu(MenuMsg::Open(MenuTarget::Site(
            site.id.clone(),
        ))))
        .into()
}

/// `glyph` with `tip` as its tooltip.
fn state_tip<'a>(glyph: iced::widget::Text<'a>, tip: String) -> Element<'a, Message> {
    tooltip(
        glyph,
        container(text(tip).size(theme::TEXT_XS))
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
            .style(theme::dialog_box),
        tooltip::Position::Bottom,
    )
    .into()
}

/// The loading state: grey bars where the groups' rows will be.
fn skeleton<'a>() -> Element<'a, Message> {
    let bar = |width: f32| {
        container(space())
            .width(width)
            .height(theme::TEXT_SM)
            .style(theme::skeleton)
    };
    let rows = [120.0, 168.0, 96.0, 144.0, 112.0];
    column![
        group_rule(),
        container(column(rows.into_iter().map(|w| bar(w).into())).spacing(theme::SPACE_SM))
            .padding(Padding::from([theme::SPACE_SM, theme::SPACE_MD])),
    ]
    .into()
}

fn workspaces(app: &App) -> Element<'_, Message> {
    let visible: Vec<&Workspace> = app
        .workspaces
        .iter()
        .filter(|ws| {
            app.site_filter.trim().is_empty()
                || ws
                    .name
                    .to_lowercase()
                    .contains(&app.site_filter.trim().to_lowercase())
                || ws.sites.iter().any(|entry| {
                    app.site(&entry.site)
                        .is_some_and(|site| site_matches(site, &app.site_filter))
                })
        })
        .collect();
    let body: Element<'_, Message> = if app.workspaces.is_empty() {
        note(strings::no_workspaces())
    } else if visible.is_empty() {
        note(strings::no_matches())
    } else {
        column(visible.into_iter().map(|ws| workspace(app, ws)))
            .spacing(theme::SPACE_3XS)
            .into()
    };
    group(
        SidebarSection::Workspaces,
        app.settings.workspaces.collapsed,
        strings::workspaces(),
        app.workspaces.len(),
        strings::workspace_form_new(),
        Message::NewWorkspace,
        body,
    )
}

fn workspace<'a>(app: &'a App, ws: &'a Workspace) -> Element<'a, Message> {
    let open = app.expanded.contains(&ws.id);
    let p = palette();
    let count = strings::workspace_site_count(
        ws.sites
            .iter()
            .filter(|entry| app.site(&entry.site).is_some())
            .count(),
        ws.sites.len(),
    );
    let count_width = count.chars().count() as f32 * theme::TEXT_XS * fit::INTER_ADVANCE;
    let name_width = (WIDTH
        - theme::HAIRLINE
        - 4.0 * theme::SPACE_XS
        - icons::SMALL
        - 2.0 * theme::SPACE_XS
        - count_width
        - SCROLL_GUTTER)
        .max(0.0);
    let is_open_workspace = app.open_workspace.as_ref() == Some(&ws.id);
    let (icon_color, name_color) = workspace_look(is_open_workspace, open);
    let glyph = icons::icon(Icon::Stack, icons::SMALL).color(icon_color);
    let stack_icon: Element<'a, Message> = if is_open_workspace {
        state_tip(glyph, strings::workspace_open_tip().to_string())
    } else {
        glyph.into()
    };
    let head = button(
        row![
            stack_icon,
            named(&ws.name, name_width, theme::INTER_MEDIUM, name_color),
            text(count)
                .size(theme::TEXT_XS)
                .color(p.muted)
                .wrapping(text::Wrapping::None),
        ]
        .spacing(theme::SPACE_XS)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding(theme::SPACE_XS)
    .style(if open {
        theme::row_selected
    } else {
        theme::row
    })
    .on_press(Message::ToggleWorkspace(ws.id.clone()));
    let head = hover_edit(head.into(), Message::EditWorkspace(ws.id.clone()));
    // A right press: Open all, Close all, Edit, Export… (mouse only, like the site's; Export… has its
    // button in the workspace's form).
    let head = mouse_area(head).on_right_press(Message::Menu(MenuMsg::Open(
        MenuTarget::Workspace(ws.id.clone()),
    )));
    let head = container(head).padding(Padding::from([0.0, theme::SPACE_XS]));
    if !open {
        return head.into();
    }

    let sites = column(ws.sites.iter().map(|entry| {
        match app.site(&entry.site) {
            Some(site) => site_row(app, site, false, NESTED_CHROME),
            None => container(
                text(strings::site_missing())
                    .size(theme::TEXT_SM)
                    .color(palette().error),
            )
            .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
            .into(),
        }
    }));
    // The 1 px `rule_2` line on the left of the nested list.
    let nested = row![
        rule::vertical(theme::HAIRLINE).style(theme::rule_strong),
        container(sites).padding(Padding::from([0.0, theme::SPACE_XS])),
    ]
    .height(Length::Shrink);
    let nested = container(nested).padding(
        Padding::ZERO
            .top(theme::SPACE_2XS)
            .left(theme::SPACE_LG)
            .right(theme::SPACE_XS),
    );

    column![head, nested].into()
}

fn all_sites(app: &App) -> Element<'_, Message> {
    let visible = app.visible_sites();
    let body: Element<'_, Message> = if app.sites.is_empty() {
        container(
            column![
                text(strings::no_sites())
                    .size(theme::TEXT_SM)
                    .color(palette().muted),
                action(strings::new_site(), Message::NewSite),
            ]
            .spacing(theme::SPACE_2XS),
        )
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
        .into()
    } else if visible.is_empty() {
        note(strings::no_matches())
    } else {
        container(column(
            visible
                .into_iter()
                .map(|site| site_row(app, site, true, ROW_CHROME)),
        ))
        .padding(Padding::from([0.0, theme::SPACE_XS]))
        .into()
    };
    group(
        SidebarSection::Sites,
        app.settings.sites.collapsed,
        strings::all_sites(),
        app.sites.len(),
        strings::site_form_new(),
        Message::NewSite,
        body,
    )
}

/// Entries kept on disk but unreadable here, and the way to look at the file.
fn unreadable(app: &App) -> Element<'_, Message> {
    let mut list = column![].spacing(theme::SPACE_XS);
    for entry in &app.unreadable {
        let name = strings::unreadable_name(entry.name.as_deref(), entry.file, entry.index);
        list = list.push(
            column![
                text(name).font(theme::MONO).size(theme::TEXT_MONO),
                text(strings::unreadable_entry())
                    .size(theme::TEXT_XS)
                    .color(palette().error),
            ]
            .spacing(theme::SPACE_3XS),
        );
    }
    list = list.push(action(
        strings::open_profile_folder(),
        Message::OpenProfileFolder,
    ));
    if let Some(error) = &app.sidebar_error {
        list = list.push(
            text(strings::cannot_open_folder(&strings::error(error)))
                .size(theme::TEXT_XS)
                .color(palette().error),
        );
    }
    column![
        group_rule(),
        container(list).padding(Padding::from([theme::SPACE_SM, theme::SPACE_MD])),
    ]
    .into()
}

#[cfg(test)]
mod tests {
    use hoprs_core::ErrorKind;

    use super::*;
    use crate::app::SiteConn;
    use crate::backend::{ConnectError, Connected, SessionId};

    #[test]
    fn site_state_per_connection() {
        let mut app = App::new();
        let id = SiteId::new();
        assert_eq!(site_state(&app, &id), SiteState::Idle);

        app.conns.insert(id.clone(), SiteConn::connecting());
        assert_eq!(site_state(&app, &id), SiteState::Connecting);

        app.conns.get_mut(&id).unwrap().state = TabState::Connected(Connected {
            session: SessionId(1),
            keyring_unavailable: false,
        });
        assert_eq!(site_state(&app, &id), SiteState::Connected);

        let error = Error::new(ErrorKind::Protocol, "refused");
        app.conns.get_mut(&id).unwrap().state = TabState::Failed(ConnectError {
            session: None,
            error: error.clone(),
            keyring_unavailable: false,
        });
        assert_eq!(site_state(&app, &id), SiteState::Failed(&error));
    }

    #[test]
    fn a_name_is_cut_only_when_it_does_not_fit() {
        let cut = |name, width| {
            matches!(
                fit::ellipsize(name, width, theme::TEXT_SM, fit::INTER_ADVANCE),
                std::borrow::Cow::Owned(_)
            )
        };
        assert!(!cut("short", 200.0));
        assert!(cut("a very long site name that cannot fit", 60.0));
    }

    #[test]
    fn connecting_looks_busy_not_idle() {
        let site = Site::new("s", "h", "u", Protocol::Ssh);
        let idle = state_look(&site, SiteState::Idle);
        let connecting = state_look(&site, SiteState::Connecting);
        assert_eq!(idle.0, Icon::TerminalWindow);
        assert_eq!(connecting.0, Icon::ArrowsClockwise);
        assert_ne!(idle.0, connecting.0);
    }

    #[test]
    fn the_open_workspace_is_marked() {
        let p = palette();
        assert_eq!(workspace_look(true, false), (p.success, p.ink));
        assert_eq!(workspace_look(false, false), (p.muted, p.ink_2));
        assert_eq!(workspace_look(false, true), (p.muted, p.ink));
    }

    #[test]
    fn site_icon_by_protocol() {
        let icon = |protocol| site_icon(&Site::new("s", "h", "u", protocol));
        assert_eq!(icon(Protocol::Ssh), Icon::TerminalWindow);
        assert_eq!(icon(Protocol::Ftp), Icon::HardDrives);
        assert_eq!(icon(Protocol::FtpsExplicit), Icon::HardDrives);
        assert_eq!(icon(Protocol::FtpsImplicit), Icon::HardDrives);
    }
}
