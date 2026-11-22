//! The shell of mockup v2: top bar across the window; below it the
//! 256 px sidebar on the left and, on the right, the center, the
//! collapsible transfer queue and the status bar.

use iced::widget::{button, column, container, image, row, rule, space, text};
use iced::{Alignment, Element, Length, Padding};

use crate::app::{App, Form, MenuMsg, MenuTarget, Message, PaletteMsg};
use crate::ui::icons::Icon;
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use crate::ui::{
    dialogs, drag, edits, export, files, import, log, menu, palette as command_palette, pointer,
    queue, settings, sidebar, site_form, status, tabs, terminal, update, widgets, workspace_form,
};

pub const SIDEBAR_WIDTH: f32 = sidebar::WIDTH;
/// 4 px above the 36 px tabs.
pub const TOP_BAR_HEIGHT: f32 = 40.0;
/// Side of the logo in the brand cell.
pub const BRAND_LOGO: f32 = 22.0;
pub const QUEUE_HEAD_HEIGHT: f32 = 36.0;
pub const QUEUE_MAX_HEIGHT: f32 = 210.0;
pub const STATUS_HEIGHT: f32 = 26.0;
/// The log panel's lines (six rows); its head is as tall as the queue's.
pub const LOG_BODY_HEIGHT: f32 = 180.0;
/// The accent line under an active tab.
pub const UNDERLINE: f32 = 2.0;

pub fn view(app: &App) -> Element<'_, Message> {
    let mut right = column![];
    if let Some(notice) = edits::kept_notice(app) {
        right = right.push(notice).push(hrule());
    }
    let right = right.push(edits::with_toast(app, center(app)));
    // The settings tab has no transfers to show: no queue under it (the log and the
    // status bar stay).
    let right = if app.settings_shown() {
        right
    } else {
        right.push(queue::view(app))
    };
    let right = right
        .push(log::view(app))
        .push(status::view(app))
        .width(Length::Fill);
    let body = row![sidebar::view(app), vrule(), right].height(Length::Fill);

    let base = container(column![top_bar(app), hrule(), body])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::paper)
        .into();
    // Never more than one: a security dialog closes the palette and a menu, the palette
    // closes a menu, and neither opens under a dialog. A drag's chip goes over a menu (a
    // right press during a drag opens one) and under the palette and the dialogs. The
    // update card is the lowest layer: everything else covers it.
    let shown = dialogs::overlay(
        app,
        command_palette::overlay(
            app,
            drag::overlay(app, menu::overlay(app, update::overlay(app, base))),
        ),
    );
    // Every left or right press is published with its position first (menus open there).
    pointer::tracker(shown)
}

fn hrule<'a>() -> Element<'a, Message> {
    rule::horizontal(theme::HAIRLINE)
        .style(theme::rule_1px)
        .into()
}

fn vrule<'a>() -> Element<'a, Message> {
    rule::vertical(theme::HAIRLINE)
        .style(theme::rule_1px)
        .into()
}

fn muted<'a>(label: &'a str, size: f32) -> Element<'a, Message> {
    text(label).size(size).color(palette().muted).into()
}

fn top_bar(app: &App) -> Element<'_, Message> {
    let brand = container(
        row![
            image(crate::icon::logo())
                .width(BRAND_LOGO)
                .height(BRAND_LOGO),
            text(strings::brand())
                .size(theme::TEXT_MD)
                .font(theme::INTER_SEMIBOLD),
            space::horizontal(),
            widgets::icon_button(
                Icon::DotsThree,
                strings::sidebar_options(),
                None,
                Some(Message::Menu(MenuMsg::Open(MenuTarget::SidebarOptions))),
            ),
        ]
        .spacing(theme::SPACE_XS)
        .align_y(Alignment::Center),
    )
    .width(SIDEBAR_WIDTH - theme::HAIRLINE)
    .height(Length::Fill)
    .padding(Padding::ZERO.left(theme::SPACE_MD).right(theme::SPACE_MD))
    .align_y(Alignment::Center);

    let tabs = tabs::view(app);

    // The Ctrl+K palette's button.
    let palette_slot = container(
        button(
            container(
                row![
                    text(strings::go_to_anything()),
                    container(text(strings::go_to_anything_key()).size(theme::TEXT_XS))
                        .padding(Padding::ZERO.left(theme::SPACE_2XS).right(theme::SPACE_2XS))
                        .style(theme::kbd),
                ]
                .spacing(theme::SPACE_SM)
                .align_y(Alignment::Center),
            )
            .height(Length::Fill)
            .align_y(Alignment::Center),
        )
        .height(Length::Fill)
        .padding(Padding::ZERO.left(theme::SPACE_SM).right(theme::SPACE_SM))
        .style(theme::button_outline)
        .on_press(Message::Palette(PaletteMsg::Open)),
    )
    .height(Length::Fill)
    .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_SM]));

    row![brand, vrule(), tabs, palette_slot]
        .height(TOP_BAR_HEIGHT)
        .into()
}

/// The center: the open form, else the settings tab or the active tab, else the empty
/// state.
fn center(app: &App) -> Element<'_, Message> {
    match &app.form {
        Some(Form::Site(form)) => {
            return site_form::view(form, app.site_delete_note(form.site_id()));
        }
        Some(Form::Workspace(form)) => return workspace_form::view(form, &app.sites),
        Some(Form::Import) => return import::view(app),
        Some(Form::Export) => return export::view(app),
        Some(Form::QuitEdits) => return edits::quit_view(app),
        None => {}
    }
    if app.settings_shown()
        && let Some(tab) = &app.settings_tab
    {
        return settings::view(app, tab);
    }
    if let Some(tab) = app.active_tab.and_then(|i| app.tabs.get(i)) {
        return match tab.terminal() {
            Some(term) => terminal::view(app, tab, term),
            None => files::view(app, tab),
        };
    }
    container(
        column![
            text(strings::no_session_title()).font(theme::INTER_SEMIBOLD),
            muted(strings::no_session_hint(), theme::TEXT_SM),
        ]
        .spacing(theme::SPACE_2XS)
        .align_x(Alignment::Center),
    )
    .center(Length::Fill)
    .into()
}
