//! The context menu overlay: a small box pinned where the menu was asked for
//! ([`origin`] keeps it inside the window) over a transparent full-window layer that takes
//! every press, right press and scroll outside the box and closes the menu. Items read
//! their icon, label, shortcut (mono, `muted`) and, for check items, a check glyph; a
//! destructive item is in the error colour after a rule; a disabled item is drawn in the
//! `disabled` token and cannot be clicked. No shadow, 1 px outline, 4 px corners.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Shell, Widget, overlay};
use iced::widget::{
    button, column, container, mouse_area, opaque, pin, row, rule, space, stack, text,
};
use iced::{
    Alignment, Color, Element, Event, Length, Padding, Point, Rectangle, Size, Vector, keyboard,
    mouse,
};

use crate::app::menu::{MenuAction, MenuItem};
use crate::app::{App, FileKey, MenuMsg, Message};
use crate::backend::Side;
use crate::ui::icons::{self, Icon};
use crate::ui::strings;
use crate::ui::theme::{self, palette};

/// The box's width.
pub const WIDTH: f32 = 224.0;
/// An item's height.
pub const ITEM: f32 = 28.0;
/// A rule between items: 1 px with 4 px above and below.
pub const RULE: f32 = 9.0;
/// The box's inner padding (all sides).
const PADDING: f32 = theme::SPACE_2XS;
/// The least distance between the box and the window's edges.
const EDGE: f32 = 4.0;

fn msg(m: MenuMsg) -> Message {
    Message::Menu(m)
}

/// The item's label.
pub fn label(action: MenuAction) -> &'static str {
    match action {
        MenuAction::ShowWorkspaces => strings::menu_show_workspaces(),
        MenuAction::ShowSites => strings::menu_show_sites(),
        MenuAction::Transfer(Side::Local) => strings::upload(),
        MenuAction::Transfer(Side::Remote) => strings::download(),
        MenuAction::Edit => strings::edit_file(),
        MenuAction::Open => strings::menu_open(),
        MenuAction::Rename => strings::rename(),
        MenuAction::Permissions => strings::permissions(),
        MenuAction::CopyPath => strings::menu_copy_path(),
        MenuAction::Refresh => strings::menu_refresh(),
        MenuAction::NewFolder => strings::new_folder(),
        MenuAction::Delete => strings::delete(),
        MenuAction::Compare => strings::compare_button(),
        MenuAction::CompareUpload => strings::upload(),
        MenuAction::CompareDownload => strings::download(),
        MenuAction::ShowInPanes => strings::menu_show_in_panes(),
        MenuAction::Queue(action) => crate::ui::queue::action_label(action),
        MenuAction::OpenSite => strings::menu_open_files(),
        MenuAction::EditSite | MenuAction::EditWorkspace => strings::edit(),
        MenuAction::OpenAll => strings::menu_open_all(),
        MenuAction::CloseAll => strings::menu_close_all(),
        MenuAction::Export => strings::export_ellipsis(),
        MenuAction::ExportAll => strings::export_everything(),
        MenuAction::CloseTab => strings::close_tab(),
        MenuAction::CloseOtherTabs => strings::menu_close_other_tabs(),
    }
}

/// The item's icon.
pub fn icon(action: MenuAction) -> Icon {
    match action {
        MenuAction::ShowWorkspaces => Icon::Stack,
        MenuAction::ShowSites => Icon::HardDrives,
        MenuAction::Transfer(Side::Local) => Icon::UploadSimple,
        MenuAction::Transfer(Side::Remote) => Icon::DownloadSimple,
        MenuAction::Edit => Icon::PencilSimpleLine,
        MenuAction::Open => Icon::Folder,
        MenuAction::Rename => Icon::Textbox,
        MenuAction::Permissions => Icon::LockKey,
        MenuAction::CopyPath => Icon::Copy,
        MenuAction::Refresh => Icon::ArrowsClockwise,
        MenuAction::NewFolder => Icon::FolderSimplePlus,
        MenuAction::Delete => Icon::Trash,
        MenuAction::Compare => Icon::GitDiff,
        MenuAction::CompareUpload => Icon::UploadSimple,
        MenuAction::CompareDownload => Icon::DownloadSimple,
        MenuAction::ShowInPanes => Icon::Folder,
        // A conflict's answers have no icon, and are never in a menu.
        MenuAction::Queue(action) => crate::ui::queue::action_icon(action).unwrap_or(Icon::Check),
        MenuAction::OpenSite => Icon::HardDrives,
        MenuAction::EditSite | MenuAction::EditWorkspace => Icon::PencilSimple,
        MenuAction::OpenAll => Icon::Stack,
        MenuAction::CloseAll => Icon::X,
        MenuAction::CloseTab | MenuAction::CloseOtherTabs => Icon::X,
        MenuAction::Export | MenuAction::ExportAll => Icon::ShieldCheck,
    }
}

/// The item's key, as the rest of the app shows it, if it has one.
pub fn shortcut(action: MenuAction) -> Option<&'static str> {
    match action {
        MenuAction::Transfer(_) => Some(strings::key_transfer()),
        MenuAction::Edit => Some(strings::key_edit()),
        MenuAction::Rename => Some(strings::key_rename()),
        MenuAction::Refresh => Some(strings::key_refresh()),
        MenuAction::NewFolder => Some(strings::key_new_folder()),
        MenuAction::Delete => Some(strings::key_delete()),
        MenuAction::ShowWorkspaces
        | MenuAction::ShowSites
        | MenuAction::Open
        | MenuAction::Permissions
        | MenuAction::CopyPath
        | MenuAction::Compare
        | MenuAction::CompareUpload
        | MenuAction::CompareDownload
        | MenuAction::ShowInPanes
        | MenuAction::Queue(_)
        | MenuAction::OpenSite
        | MenuAction::EditSite
        | MenuAction::OpenAll
        | MenuAction::CloseAll
        | MenuAction::EditWorkspace
        | MenuAction::Export
        | MenuAction::ExportAll
        | MenuAction::CloseTab
        | MenuAction::CloseOtherTabs => None,
    }
}

/// The box's size for `items`.
pub fn size(items: &[MenuItem]) -> Size {
    let rules = items.iter().filter(|item| item.rule_before).count();
    Size::new(
        WIDTH,
        items.len() as f32 * ITEM + rules as f32 * RULE + 2.0 * PADDING,
    )
}

/// Where the box of size `menu` goes for a menu asked for at `at` in a `window`: below
/// and right of `at` when it fits, else above and/or left of it; never closer than 4 px
/// to an edge (the top left wins when the window is smaller than the box).
pub fn origin(at: Point, menu: Size, window: Size) -> Point {
    let axis = |at: f32, len: f32, room: f32| {
        let start = if at + len <= room - EDGE {
            at
        } else {
            at - len
        };
        start.min(room - EDGE - len).max(EDGE)
    };
    Point::new(
        axis(at.x, menu.width, window.width),
        axis(at.y, menu.height, window.height),
    )
}

/// `base` with the open menu over it, if one is open.
///
/// Always a `stack` with `base` first, open or not: iced keeps a widget's state only while
/// its place in the tree keeps its kind, so wrapping `base` only while a menu is open
/// would reset every scroll offset and focus behind it each time one opens or closes.
pub fn overlay<'a>(app: &'a App, base: Element<'a, Message>) -> Element<'a, Message> {
    let Some(menu) = &app.menu else {
        return stack![base].into();
    };
    let items = app.menu_items(&menu.target);
    let window = Size::new(app.window_width, app.window_height);
    let at = origin(menu.at, size(&items), window);

    let mut list = column![];
    for (index, item) in items.iter().enumerate() {
        if item.rule_before {
            list = list.push(
                container(rule::horizontal(theme::HAIRLINE).style(theme::rule_1px))
                    .height(RULE)
                    .center_y(RULE),
            );
        }
        list = list.push(item_row(index, item, menu.cursor == Some(index)));
    }
    let boxed = container(list)
        .width(WIDTH)
        .padding(PADDING)
        .style(theme::dialog_box);

    // A press, right press or scroll anywhere outside the box closes the menu; nothing
    // behind it gets the pointer. The box takes its own presses (a disabled item's too).
    let backdrop = opaque(
        mouse_area(space().width(Length::Fill).height(Length::Fill))
            .on_press(msg(MenuMsg::Close))
            .on_right_press(msg(MenuMsg::Close))
            .on_middle_press(msg(MenuMsg::Close))
            .on_scroll(|_| msg(MenuMsg::Close)),
    );
    let layer = pin(opaque(boxed)).position(at);
    stack![base, backdrop, keys(layer.into())].into()
}

/// `layer` taking ↑, ↓ and Enter (without Ctrl or Alt) away from the window while a menu
/// is open: they are published as [`Message::KeyPressed`] and captured, so a text field
/// behind the menu (a rename in progress) neither moves its caret nor submits, and the
/// keyboard subscription, which drops captured keys, does not send them twice. Escape and
/// every other key go on as usual (the subscription closes the menu with them).
fn keys(layer: Element<'_, Message>) -> Element<'_, Message> {
    Element::new(MenuKeys { content: layer })
}

struct MenuKeys<'a> {
    content: Element<'a, Message>,
}

impl Widget<Message, iced::Theme, iced::Renderer> for MenuKeys<'_> {
    fn tag(&self) -> tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> tree::State {
        self.content.as_widget().state()
    }

    fn children(&self) -> Vec<Tree> {
        self.content.as_widget().children()
    }

    fn diff(&self, tree: &mut Tree) {
        self.content.as_widget().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content.as_widget_mut().layout(tree, renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(tree, layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if let Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(named),
            modifiers,
            ..
        }) = event
            && !modifiers.command()
            && !modifiers.alt()
        {
            let key = match named {
                keyboard::key::Named::ArrowUp => Some(FileKey::Up),
                keyboard::key::Named::ArrowDown => Some(FileKey::Down),
                keyboard::key::Named::Enter => Some(FileKey::Enter),
                _ => None,
            };
            if let Some(key) = key {
                shell.publish(Message::KeyPressed {
                    key,
                    shift: modifiers.shift(),
                });
                shell.capture_event();
                return;
            }
        }
        self.content.as_widget_mut().update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}

/// One item: icon, label, shortcut, check; chosen with a click when enabled. The pointer
/// over an enabled item highlights it ([`MenuMsg::Hover`]), as the arrows do: the row
/// lights up and its icon goes from `muted` to `ink`.
fn item_row<'a>(index: usize, item: &MenuItem, highlighted: bool) -> Element<'a, Message> {
    let p = palette();
    // `None` lets the label follow the row's hover colour.
    let (ink, glyph): (Option<Color>, Color) = if !item.enabled {
        (Some(p.disabled), p.disabled)
    } else if item.destructive {
        (Some(p.error), p.error)
    } else if highlighted {
        (None, p.ink)
    } else {
        (None, p.muted)
    };
    let mut label = text(label(item.action))
        .size(theme::TEXT_SM)
        .wrapping(text::Wrapping::None);
    if let Some(ink) = ink {
        label = label.color(ink);
    }
    let shortcut = shortcut(item.action).map(|key| {
        text(key)
            .font(theme::MONO)
            .size(theme::TEXT_TAG)
            .color(if item.enabled { p.muted } else { p.disabled })
    });
    // The check column keeps its width when unchecked, so labels line up.
    let check: Element<'a, Message> = match item.checked {
        Some(true) => icons::icon(Icon::Check, icons::ROW).color(glyph).into(),
        _ => space().width(icons::ROW).into(),
    };
    let line = row![
        icons::icon(icon(item.action), icons::ROW).color(glyph),
        label,
        space::horizontal(),
        shortcut,
        check,
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center);
    let button = button(
        container(line)
            .height(Length::Fill)
            .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .height(ITEM)
    .padding(Padding::ZERO.left(theme::SPACE_XS).right(theme::SPACE_XS))
    .style(if highlighted {
        theme::row_selected
    } else {
        theme::row
    })
    .on_press_maybe(item.enabled.then(|| msg(MenuMsg::Choose(index))));
    if item.enabled {
        mouse_area(button)
            .on_enter(msg(MenuMsg::Hover(index)))
            .into()
    } else {
        button.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::menu::{MenuAction, MenuItem};

    fn item(rule_before: bool) -> MenuItem {
        MenuItem {
            action: MenuAction::ShowWorkspaces,
            enabled: true,
            checked: None,
            destructive: false,
            rule_before,
        }
    }

    fn inside(origin: Point, menu: Size, window: Size) -> bool {
        origin.x >= EDGE
            && origin.y >= EDGE
            && origin.x + menu.width <= window.width - EDGE
            && origin.y + menu.height <= window.height - EDGE
    }

    /// ↑, ↓ and Enter go to the menu, even with a text field focused behind it: a rename
    /// editor neither gets nor submits them. Escape, Ctrl+Enter and letters pass.
    #[test]
    fn menu_takes_arrows_and_enter_from_a_text_field() {
        use iced::advanced::clipboard;
        use iced::widget::{stack, text_input};
        use iced_runtime::user_interface::{Cache, UserInterface};

        let mut renderer = iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(
            theme::INTER,
            theme::TEXT_SM.into(),
        ));
        let id = iced::widget::Id::unique();
        let field = text_input("", "name")
            .id(id.clone())
            .on_input(|_| Message::NewSite)
            .on_submit(Message::NewWorkspace);
        let content = stack![field, keys(space().width(10).height(10).into())];
        let mut ui = UserInterface::build(
            content,
            Size::new(200.0, 100.0),
            Cache::new(),
            &mut renderer,
        );
        // Focus the field.
        let mut focus = iced::advanced::widget::operation::focusable::focus(id);
        ui.operate(&renderer, &mut focus);
        let press = |named, modifiers| {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(named),
                modified_key: keyboard::Key::Named(named),
                physical_key: keyboard::key::Physical::Unidentified(
                    keyboard::key::NativeCode::Unidentified,
                ),
                location: keyboard::Location::Standard,
                modifiers,
                text: None,
                repeat: false,
            })
        };
        let none = keyboard::Modifiers::default();
        let mut messages = Vec::new();
        let (_, statuses) = ui.update(
            &[
                press(keyboard::key::Named::ArrowDown, none),
                press(keyboard::key::Named::ArrowUp, keyboard::Modifiers::SHIFT),
                press(keyboard::key::Named::Enter, none),
            ],
            mouse::Cursor::Unavailable,
            &mut renderer,
            &mut clipboard::Null,
            &mut messages,
        );
        let got: Vec<String> = messages.iter().map(|m| format!("{m:?}")).collect();
        let key = |key, shift| format!("{:?}", Message::KeyPressed { key, shift });
        assert_eq!(
            got,
            [
                key(FileKey::Down, false),
                key(FileKey::Up, true),
                key(FileKey::Enter, false)
            ],
            "the field never submitted"
        );
        assert!(statuses.iter().all(|s| *s == iced::event::Status::Captured));
        // Ctrl+Enter, letters and Escape are not the menu's (the letter reaches the
        // focused field: it was focused all along).
        let letter = Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Character("a".into()),
            modified_key: keyboard::Key::Character("a".into()),
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: none,
            text: Some("a".into()),
            repeat: false,
        });
        let mut messages = Vec::new();
        ui.update(
            &[
                letter,
                press(keyboard::key::Named::Enter, keyboard::Modifiers::COMMAND),
                press(keyboard::key::Named::Escape, none),
            ],
            mouse::Cursor::Unavailable,
            &mut renderer,
            &mut clipboard::Null,
            &mut messages,
        );
        assert!(
            !messages
                .iter()
                .any(|m| matches!(m, Message::KeyPressed { .. })),
            "{messages:?}"
        );
        assert!(
            messages.iter().any(|m| matches!(m, Message::NewSite)),
            "{messages:?}"
        );
    }

    #[test]
    fn origin_stays_inside_window() {
        let window = Size::new(1280.0, 800.0);
        let menu = Size::new(WIDTH, 3.0 * ITEM);
        // Near the bottom right corner: flipped up and left of the pointer.
        let at = Point::new(1270.0, 790.0);
        let o = origin(at, menu, window);
        assert!(inside(o, menu, window), "{o:?}");
        assert_eq!(o, Point::new(1270.0 - WIDTH, 790.0 - menu.height));
        // Room below and to the right: right at the pointer.
        assert_eq!(
            origin(Point::new(10.0, 10.0), menu, window),
            Point::new(10.0, 10.0)
        );
        // Too close to the top left edge: clamped 4 px in.
        assert_eq!(
            origin(Point::new(1.0, 0.0), menu, window),
            Point::new(EDGE, EDGE)
        );
        // A window smaller than the menu: pinned to the top left edge.
        let tiny = Size::new(100.0, 50.0);
        assert_eq!(
            origin(Point::new(60.0, 30.0), menu, tiny),
            Point::new(EDGE, EDGE)
        );
    }

    #[test]
    fn size_counts_items_and_rules() {
        let pad = 2.0 * PADDING;
        assert_eq!(size(&[]), Size::new(WIDTH, pad));
        assert_eq!(size(&[item(false)]), Size::new(WIDTH, ITEM + pad));
        assert_eq!(
            size(&[item(false), item(false), item(true)]),
            Size::new(WIDTH, 3.0 * ITEM + RULE + pad)
        );
    }

    #[test]
    fn every_action_has_a_label_and_icon() {
        for action in [MenuAction::ShowWorkspaces, MenuAction::ShowSites] {
            assert!(!label(action).is_empty());
            let _ = icon(action);
            assert_eq!(shortcut(action), None);
        }
        // The pane items, with the keys the toolbar shows.
        for (action, key) in [
            (MenuAction::Transfer(Side::Local), Some("Ctrl ↵")),
            (MenuAction::Transfer(Side::Remote), Some("Ctrl ↵")),
            (MenuAction::Edit, Some("F4")),
            (MenuAction::Open, None),
            (MenuAction::Rename, Some("F2")),
            (MenuAction::Permissions, None),
            (MenuAction::CopyPath, None),
            (MenuAction::Refresh, Some("F5")),
            (MenuAction::NewFolder, Some("F7")),
            (MenuAction::Delete, Some("Del")),
            (MenuAction::Compare, None),
        ] {
            assert!(!label(action).is_empty(), "{action:?}");
            let _ = icon(action);
            assert_eq!(shortcut(action), key, "{action:?}");
        }
        assert_ne!(
            label(MenuAction::Transfer(Side::Local)),
            label(MenuAction::Transfer(Side::Remote))
        );
    }
}
