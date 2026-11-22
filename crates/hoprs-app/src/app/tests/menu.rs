//! Plan 13 Task 4: the context menu's model. It opens where the pointer last pressed,
//! closes on Escape, a click outside, a resize or a choice, never acts through a missing
//! or disabled item, and never opens over a security dialog.

use iced::{Point, Size};

use super::*;
use crate::app::ExportMsg;
use crate::app::menu::{MenuMsg, MenuTarget};
use crate::backend::broker::{PromptKind, PromptRequest};
use crate::backend::{BrokerEvent, PromptId};
use hoprs_vault::ExportScope;

fn open() -> Message {
    Message::Menu(MenuMsg::Open(MenuTarget::SidebarOptions))
}

pub(super) fn host_key_prompt(id: u64) -> Message {
    Message::Prompt(BrokerEvent::Request(PromptRequest {
        id: PromptId(id),
        session: SessionId(1),
        site: SiteId::new(),
        key: "h:22".into(),
        kind: PromptKind::HostKey {
            host: "h".into(),
            port: 22,
            key_type: "ssh-ed25519".into(),
            fingerprint: "SHA256:new".into(),
            other_key_types: Vec::new(),
        },
    }))
}

#[test]
fn menu_opens_at_the_last_press() {
    let mut app = App::new();
    assert!(app.menu.is_none());
    assert!(
        app.update(Message::PointerPressed(Point::new(300.0, 40.0)))
            .is_empty()
    );
    assert_eq!(app.last_press, Point::new(300.0, 40.0));
    assert!(app.update(open()).is_empty());
    let menu = app.menu.as_ref().expect("the menu is open");
    assert_eq!(menu.at, Point::new(300.0, 40.0));
    assert_eq!(menu.target, MenuTarget::SidebarOptions);
    assert_eq!(menu.cursor, None);
}

#[test]
fn escape_and_outside_click_close() {
    let mut app = App::new();
    app.update(open());
    assert!(app.menu.is_some());
    // Escape closes the menu, and nothing else sees it.
    assert!(
        app.update(Message::KeyPressed {
            key: FileKey::Escape,
            shift: false,
        })
        .is_empty()
    );
    assert!(app.menu.is_none());
    // A click outside (the overlay's backdrop) closes it too.
    app.update(open());
    assert!(app.menu.is_some());
    assert!(app.update(Message::Menu(MenuMsg::Close)).is_empty());
    assert!(app.menu.is_none());
}

#[test]
fn resize_closes_the_menu() {
    let mut app = App::new();
    assert_eq!(app.window_height, super::INITIAL_WINDOW_HEIGHT);
    app.update(open());
    assert!(
        app.update(Message::WindowResized(Size::new(1000.0, 700.0)))
            .is_empty()
    );
    assert!(app.menu.is_none());
    assert_eq!(app.window_width, 1000.0);
    assert_eq!(app.window_height, 700.0);
}

#[test]
fn choose_disabled_or_missing_item_does_nothing() {
    let mut app = App::new();
    app.update(open());
    assert!(app.update(Message::Menu(MenuMsg::Choose(99))).is_empty());
    assert!(app.menu.is_none());
}

#[test]
fn menu_blocked_behind_dialog() {
    let mut app = App::new();
    app.update(host_key_prompt(1));
    assert!(app.dialogs.is_open());
    // Neither the press nor the menu gets through.
    app.update(Message::PointerPressed(Point::new(50.0, 60.0)));
    assert_eq!(app.last_press, Point::ORIGIN);
    assert!(app.update(open()).is_empty());
    assert!(app.menu.is_none());
}

#[test]
fn a_dialog_closes_an_open_menu() {
    let mut app = App::new();
    app.update(open());
    assert!(app.menu.is_some());
    app.update(host_key_prompt(1));
    assert!(app.dialogs.is_open());
    assert!(app.menu.is_none());
}

#[test]
fn menu_ignored_while_closing() {
    let mut app = App::new();
    app.closing = true;
    assert!(app.update(open()).is_empty());
    assert!(app.menu.is_none());
    app.update(Message::PointerPressed(Point::new(5.0, 5.0)));
    assert_eq!(app.last_press, Point::ORIGIN);
}

// ---- Plan 13 Task 13: the pane menus ----

use crate::agent::paths::local_os_path;
use crate::app::actions::{Busy, BusyKind, PaneAction};
use crate::app::menu::{MenuAction, MenuItem};
use crate::backend::ConnectError;

fn right_click(tab: TabId, side: Side, row: Option<usize>) -> Message {
    pane_msg(tab, side, PaneMsg::ContextMenu(row))
}

fn pane_target(tab: TabId, side: Side, row: Option<usize>) -> MenuTarget {
    MenuTarget::Pane { tab, side, row }
}

fn selected(app: &App, side: Side) -> Vec<usize> {
    app.tabs[0]
        .pane(side)
        .unwrap()
        .selection
        .rows
        .iter()
        .copied()
        .collect()
}

/// The open menu's items.
fn open_items(app: &App) -> Vec<MenuItem> {
    app.menu_items(&app.menu.as_ref().expect("a menu is open").target)
}

fn actions(items: &[MenuItem]) -> Vec<MenuAction> {
    items.iter().map(|item| item.action).collect()
}

fn enabled(items: &[MenuItem], action: MenuAction) -> bool {
    items
        .iter()
        .find(|item| item.action == action)
        .unwrap_or_else(|| panic!("no {action:?}"))
        .enabled
}

/// The index of `action` in the open menu.
fn index_of(app: &App, action: MenuAction) -> usize {
    open_items(app)
        .iter()
        .position(|item| item.action == action)
        .unwrap_or_else(|| panic!("no {action:?}"))
}

fn key_pressed(key: FileKey) -> Message {
    Message::KeyPressed { key, shift: false }
}

#[tokio::test]
async fn right_click_unselected_row_selects_it_alone() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    let a = row_of(&app, Side::Local, "a.txt");
    let b = row_of(&app, Side::Local, "b.txt");
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(a)));
    app.tabs[0].set_focus(Side::Remote);
    app.update(Message::PointerPressed(Point::new(120.0, 300.0)));
    assert!(
        app.update(right_click(tab, Side::Local, Some(b)))
            .is_empty()
    );
    assert_eq!(selected(&app, Side::Local), [b]);
    assert_eq!(
        app.tabs[0].focus(),
        Some(Side::Local),
        "the pane gets the focus"
    );
    let menu = app.menu.as_ref().expect("the menu is open");
    assert_eq!(menu.at, Point::new(120.0, 300.0));
    assert_eq!(menu.target, pane_target(tab, Side::Local, Some(b)));
}

#[tokio::test]
async fn right_click_selected_row_keeps_selection() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    let a = row_of(&app, Side::Local, "a.txt");
    let b = row_of(&app, Side::Local, "b.txt");
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(a)));
    app.update(Message::Modifiers(Mods {
        ctrl: true,
        shift: false,
    }));
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(b)));
    app.update(Message::Modifiers(Mods::default()));
    app.update(right_click(tab, Side::Local, Some(a)));
    assert_eq!(selected(&app, Side::Local), [a, b]);
    // The menu acts on both: Delete asks about two items.
    let delete = index_of(&app, MenuAction::Delete);
    app.update(Message::Menu(MenuMsg::Choose(delete)));
    assert!(app.menu.is_none());
    let Some(PaneAction::ConfirmDelete(targets)) = &app.tabs[0].local().action else {
        panic!("{:?}", app.tabs[0].local().action);
    };
    assert_eq!(targets.len(), 2);
}

#[tokio::test]
async fn right_click_selected_row_renames_the_clicked_row() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    let a = row_of(&app, Side::Local, "a.txt");
    let b = row_of(&app, Side::Local, "b.txt");
    // Click a, Shift+click b: the cursor sits on b.
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(a)));
    app.update(Message::Modifiers(Mods {
        ctrl: false,
        shift: true,
    }));
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(b)));
    app.update(Message::Modifiers(Mods::default()));
    // Right-click a: both stay selected, Rename edits a.
    app.update(right_click(tab, Side::Local, Some(a)));
    assert!(selected(&app, Side::Local).contains(&a));
    assert!(selected(&app, Side::Local).contains(&b));
    let rename = index_of(&app, MenuAction::Rename);
    app.update(Message::Menu(MenuMsg::Choose(rename)));
    let Some(PaneAction::Edit(edit)) = &app.tabs[0].local().action else {
        panic!("{:?}", app.tabs[0].local().action);
    };
    assert_eq!(
        edit.kind,
        crate::app::actions::EditKind::Rename {
            from: "a.txt".into()
        }
    );
}

#[tokio::test]
async fn local_and_remote_items_and_enabled_state() {
    let (_fake, mut app, a, tab) = transfer_tab().await;
    // A local file.
    let file = row_of(&app, Side::Local, "a.txt");
    app.update(right_click(tab, Side::Local, Some(file)));
    let items = open_items(&app);
    assert_eq!(
        actions(&items),
        [
            MenuAction::Transfer(Side::Local),
            MenuAction::Rename,
            MenuAction::CopyPath,
            MenuAction::Refresh,
            MenuAction::NewFolder,
            MenuAction::Delete,
        ]
    );
    assert!(items.iter().all(|item| item.enabled), "{items:?}");
    let delete = items.last().unwrap();
    assert!(delete.destructive && delete.rule_before);
    assert_eq!(items.iter().filter(|item| item.rule_before).count(), 1);
    assert!(items.iter().all(|item| item.checked.is_none()));
    // A local folder: Open too.
    let dir = row_of(&app, Side::Local, "dir");
    app.update(right_click(tab, Side::Local, Some(dir)));
    assert_eq!(
        actions(&open_items(&app))[..2],
        [MenuAction::Transfer(Side::Local), MenuAction::Open]
    );

    // A remote file: Download, Edit, Rename, Permissions…
    let file = row_of(&app, Side::Remote, "r.txt");
    app.update(right_click(tab, Side::Remote, Some(file)));
    assert_eq!(
        actions(&open_items(&app)),
        [
            MenuAction::Transfer(Side::Remote),
            MenuAction::Edit,
            MenuAction::Rename,
            MenuAction::Permissions,
            MenuAction::CopyPath,
            MenuAction::Refresh,
            MenuAction::NewFolder,
            MenuAction::Delete,
        ]
    );
    assert!(open_items(&app).iter().all(|item| item.enabled));
    // A remote folder: Open, no Edit.
    let sub = row_of(&app, Side::Remote, "sub");
    app.update(right_click(tab, Side::Remote, Some(sub)));
    let items = open_items(&app);
    assert_eq!(
        actions(&items),
        [
            MenuAction::Transfer(Side::Remote),
            MenuAction::Open,
            MenuAction::Rename,
            MenuAction::Permissions,
            MenuAction::CopyPath,
            MenuAction::Refresh,
            MenuAction::NewFolder,
            MenuAction::Delete,
        ]
    );
    assert!(enabled(&items, MenuAction::Open));

    // Not connected: all but Copy path are off (shown, not hidden); Refresh would list
    // nothing without a session.
    app.conn_mut(&a.id).state = TabState::Failed(ConnectError {
        session: None,
        error: Error::new(ErrorKind::Network, "down"),
        keyring_unavailable: false,
    });
    let items = open_items(&app);
    assert_eq!(items.len(), 8);
    for item in &items {
        let on = item.action == MenuAction::CopyPath;
        assert_eq!(item.enabled, on, "{item:?}");
    }

    // A pane busy with an operation: every action is off.
    app.update(right_click(tab, Side::Local, Some(file - 1)));
    app.tabs[0].local_mut().busy = Some(Busy {
        op: RequestId(999),
        kind: BusyKind::Delete,
        done: 0,
        folder: rpath("/home/me"),
    });
    let items = open_items(&app);
    assert!(!items.is_empty());
    assert!(items.iter().all(|item| !item.enabled), "{items:?}");
}

#[tokio::test]
async fn empty_area_menu() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    let a = row_of(&app, Side::Local, "a.txt");
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(a)));
    app.update(right_click(tab, Side::Local, None));
    // The selection stays as it was.
    assert_eq!(selected(&app, Side::Local), [a]);
    let items = open_items(&app);
    assert_eq!(
        actions(&items),
        [
            MenuAction::NewFolder,
            MenuAction::Refresh,
            MenuAction::Compare
        ]
    );
    assert!(items.iter().all(|item| item.enabled && !item.destructive));
    assert!(items.iter().all(|item| !item.rule_before));
}

/// Each item is the message its toolbar button or key sends. (`App` is not `Clone`: the
/// chosen message itself is compared, then run.)
#[tokio::test]
async fn choosing_sends_the_toolbar_message() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    let pane = |side, msg| format!("{:?}", pane_msg(tab, side, msg));
    let cases = [
        (
            Side::Local,
            "a.txt",
            MenuAction::Transfer(Side::Local),
            pane(Side::Local, PaneMsg::Transfer),
        ),
        (
            Side::Local,
            "a.txt",
            MenuAction::Rename,
            pane(Side::Local, PaneMsg::Rename),
        ),
        (
            Side::Local,
            "a.txt",
            MenuAction::NewFolder,
            pane(Side::Local, PaneMsg::NewFolder),
        ),
        (
            Side::Local,
            "a.txt",
            MenuAction::Delete,
            pane(Side::Local, PaneMsg::Delete),
        ),
        (
            Side::Local,
            "a.txt",
            MenuAction::Refresh,
            pane(Side::Local, PaneMsg::Refresh),
        ),
        (
            Side::Local,
            "a.txt",
            MenuAction::CopyPath,
            pane(Side::Local, PaneMsg::CopyPath),
        ),
        (
            Side::Remote,
            "r.txt",
            MenuAction::Transfer(Side::Remote),
            pane(Side::Remote, PaneMsg::Transfer),
        ),
        (
            Side::Remote,
            "r.txt",
            MenuAction::Edit,
            pane(Side::Remote, PaneMsg::Edit),
        ),
        (
            Side::Remote,
            "r.txt",
            MenuAction::Permissions,
            pane(Side::Remote, PaneMsg::Permissions),
        ),
    ];
    for (side, name, action, expected) in cases {
        let row = row_of(&app, side, name);
        app.update(right_click(tab, side, Some(row)));
        let index = index_of(&app, action);
        let chosen = app.menu_command(Message::Menu(MenuMsg::Choose(index)));
        assert_eq!(format!("{chosen:?}"), expected, "{action:?}");
        assert!(app.menu.is_none());
    }
    // Open names the row it was asked on.
    let sub = row_of(&app, Side::Remote, "sub");
    app.update(right_click(tab, Side::Remote, Some(sub)));
    let index = index_of(&app, MenuAction::Open);
    let chosen = app.menu_command(Message::Menu(MenuMsg::Choose(index)));
    assert_eq!(
        format!("{chosen:?}"),
        pane(Side::Remote, PaneMsg::Open(sub))
    );
    // Compare is the toolbar's button.
    app.update(right_click(tab, Side::Remote, None));
    let index = index_of(&app, MenuAction::Compare);
    let chosen = app.menu_command(Message::Menu(MenuMsg::Choose(index)));
    assert_eq!(
        format!("{chosen:?}"),
        format!("{:?}", Message::Compare(CompareMsg::Start { tab }))
    );

    // Run through `update`: the same effects as the button (here Rename's editor).
    let a = row_of(&app, Side::Local, "a.txt");
    app.update(right_click(tab, Side::Local, Some(a)));
    let rename = index_of(&app, MenuAction::Rename);
    let effects = app.update(Message::Menu(MenuMsg::Choose(rename)));
    assert!(
        matches!(app.tabs[0].local().action, Some(PaneAction::Edit(_))),
        "{effects:?}"
    );
}

#[tokio::test]
async fn choose_disabled_pane_item_does_nothing() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    // Nothing selected: the `..` row's menu has Rename and Delete off.
    app.update(right_click(tab, Side::Local, Some(0)));
    let items = open_items(&app);
    assert!(!enabled(&items, MenuAction::Delete));
    let delete = index_of(&app, MenuAction::Delete);
    assert!(
        app.update(Message::Menu(MenuMsg::Choose(delete)))
            .is_empty()
    );
    assert!(app.menu.is_none(), "the menu closes");
    assert_eq!(app.tabs[0].local().action, None, "nothing ran");
}

#[tokio::test]
async fn copy_path_one_and_several() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    assert_eq!(app.copy_paths(0, Side::Local), None, "nothing selected");
    let a = row_of(&app, Side::Local, "a.txt");
    app.update(right_click(tab, Side::Local, Some(a)));
    let expected = local_os_path(&rpath("/home/me/a.txt"))
        .display()
        .to_string();
    assert_eq!(app.copy_paths(0, Side::Local).as_deref(), Some(&*expected));
    let copy = index_of(&app, MenuAction::CopyPath);
    let effects = app.update(Message::Menu(MenuMsg::Choose(copy)));
    let [Effect::WriteClipboard(text)] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(text.0, expected);

    // Several remote entries: one full path per line, in row order.
    let sub = row_of(&app, Side::Remote, "sub");
    let r = row_of(&app, Side::Remote, "r.txt");
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(sub)));
    app.update(Message::Modifiers(Mods {
        ctrl: true,
        shift: false,
    }));
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(r)));
    app.update(Message::Modifiers(Mods::default()));
    app.update(right_click(tab, Side::Remote, Some(r)));
    let copy = index_of(&app, MenuAction::CopyPath);
    let effects = app.update(Message::Menu(MenuMsg::Choose(copy)));
    let [Effect::WriteClipboard(text)] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(text.0, "/srv/sub\n/srv/r.txt");
}

#[tokio::test]
async fn refresh_relists_the_pane() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    let lists = |effects: &[Effect]| -> Vec<(Side, String)> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::List {
                    tab: t, side, path, ..
                } if *t == tab => Some((*side, path.as_str().to_owned())),
                _ => None,
            })
            .collect()
    };
    // F5 on the focused pane.
    app.tabs[0].set_focus(Side::Remote);
    let effects = app.update(Message::Key {
        key: FileKey::Refresh,
        shift: false,
    });
    assert_eq!(lists(&effects), [(Side::Remote, "/srv".to_owned())]);
    // The menu's item.
    app.update(right_click(tab, Side::Local, None));
    let refresh = index_of(&app, MenuAction::Refresh);
    let effects = app.update(Message::Menu(MenuMsg::Choose(refresh)));
    assert_eq!(lists(&effects), [(Side::Local, "/home/me".to_owned())]);
}

/// Shift+F10 / the Menu key: at the cursor's row of the focused pane, from where the list
/// is (reported whether it scrolls or not), minus how far it is scrolled.
#[tokio::test]
async fn keyboard_menu_opens_at_the_cursor_row() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    let placed = |side, x, y| {
        pane_msg(
            tab,
            side,
            PaneMsg::Placed(iced::Rectangle::new(
                Point::new(x, y),
                Size::new(400.0, 600.0),
            )),
        )
    };
    let menu_key = || Message::Key {
        key: FileKey::Menu,
        shift: true,
    };
    // The folder fits: the list never scrolled (no `Scrolled`, `viewport_h` 0 before it
    // was placed), only placed.
    assert_eq!(app.tabs[0].local().viewport_h, 0.0);
    app.update(placed(Side::Local, 300.0, 200.0));
    app.update(placed(Side::Remote, 720.0, 200.0));
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(2)));
    app.update(menu_key());
    let menu = app.menu.as_ref().expect("the menu is open");
    assert_eq!(menu.at, Point::new(324.0, 290.0));
    assert_eq!(menu.target, pane_target(tab, Side::Local, Some(2)));
    // The window changed (a bar above the list, say): the anchor follows.
    app.update(Message::Menu(MenuMsg::Close));
    app.update(placed(Side::Local, 280.0, 250.0));
    app.update(menu_key());
    assert_eq!(app.menu.as_ref().unwrap().at, Point::new(304.0, 340.0));
    // Scrolled down a row: one row higher.
    app.update(Message::Menu(MenuMsg::Close));
    app.update(pane_msg(
        tab,
        Side::Local,
        PaneMsg::Scrolled {
            offset_y: 30.0,
            viewport_h: 600.0,
        },
    ));
    app.update(menu_key());
    assert_eq!(app.menu.as_ref().unwrap().at, Point::new(304.0, 310.0));
    // Tab to the other pane: its list, its cursor (none yet: the first row, no row target).
    app.update(Message::Menu(MenuMsg::Close));
    app.update(Message::Key {
        key: FileKey::SwitchPane,
        shift: false,
    });
    app.update(menu_key());
    let menu = app.menu.as_ref().unwrap();
    assert_eq!(menu.at, Point::new(744.0, 230.0));
    assert_eq!(menu.target, pane_target(tab, Side::Remote, None));
}

#[tokio::test]
async fn arrows_enter_escape_in_menu() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    // `..` selected: Transfer, Rename, Copy path, Delete are off; Open, Refresh and New
    // folder are on.
    app.update(right_click(tab, Side::Local, Some(0)));
    let items = open_items(&app);
    let on: Vec<usize> = (0..items.len()).filter(|&i| items[i].enabled).collect();
    assert_eq!(
        on.iter().map(|&i| items[i].action).collect::<Vec<_>>(),
        [MenuAction::Open, MenuAction::Refresh, MenuAction::NewFolder]
    );
    let cursor = |app: &App| app.menu.as_ref().unwrap().cursor;
    // Down from nothing: the first enabled item; the keys reach nothing else.
    assert!(app.update(key_pressed(FileKey::Down)).is_empty());
    assert_eq!(cursor(&app), Some(on[0]));
    app.update(key_pressed(FileKey::Down));
    assert_eq!(cursor(&app), Some(on[1]));
    app.update(key_pressed(FileKey::Down));
    app.update(key_pressed(FileKey::Down));
    assert_eq!(cursor(&app), Some(on[0]), "wraps past the end");
    app.update(key_pressed(FileKey::Up));
    assert_eq!(cursor(&app), Some(on[2]), "wraps past the start");
    assert_eq!(selected(&app, Side::Local), [0], "the pane did not move");
    // Enter chooses it (New folder: its editor opens).
    app.update(key_pressed(FileKey::Enter));
    assert!(app.menu.is_none());
    assert!(matches!(
        app.tabs[0].local().action,
        Some(PaneAction::Edit(_))
    ));
    app.update(pane_msg(tab, Side::Local, PaneMsg::CancelAction));
    // Escape closes, nothing else.
    app.update(right_click(tab, Side::Local, Some(0)));
    assert!(app.update(key_pressed(FileKey::Escape)).is_empty());
    assert!(app.menu.is_none());
    // Up from nothing: the last enabled item.
    app.update(right_click(tab, Side::Local, Some(0)));
    app.update(key_pressed(FileKey::Up));
    assert_eq!(cursor(&app), Some(on[2]));
    // Enter with no item highlighted chooses nothing.
    app.update(Message::Menu(MenuMsg::Close));
    app.update(right_click(tab, Side::Local, Some(0)));
    assert!(app.update(key_pressed(FileKey::Enter)).is_empty());
    assert!(app.tabs[0].local().action.is_none());
    // Another key closes the menu, then acts as usual (F7: the pane's new folder).
    let effects = app.update(key_pressed(FileKey::NewFolder));
    assert!(app.menu.is_none());
    assert!(
        matches!(
            effects[..],
            [Effect::KeyUnlessTyping {
                key: FileKey::NewFolder,
                ..
            }]
        ),
        "{effects:?}"
    );
}

#[tokio::test]
async fn menu_closes_when_pane_relists() {
    let (fake, mut app, _a, tab) = transfer_tab().await;
    app.update(right_click(tab, Side::Local, Some(1)));
    // A listing of the other pane leaves it open.
    let effects = app.update(pane_msg(tab, Side::Remote, PaneMsg::Refresh));
    settle(&mut app, &fake, run_listings(&fake, &effects).await).await;
    assert!(app.menu.is_some());
    // Its own pane's rows change: it closes.
    app.tabs[0].set_focus(Side::Local);
    let effects = app.update(Message::Key {
        key: FileKey::Refresh,
        shift: false,
    });
    assert!(app.menu.is_some());
    settle(&mut app, &fake, run_listings(&fake, &effects).await).await;
    assert!(app.menu.is_none());
}

#[tokio::test]
async fn menu_on_closed_tab_does_nothing() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    let gone = TabId(tab.0 + 1000);
    // Opened for a tab that is not there: no items, a choice does nothing.
    app.update(right_click(gone, Side::Local, Some(1)));
    assert!(app.menu.is_none());
    app.menu = Some(crate::app::ContextMenu {
        at: Point::ORIGIN,
        target: pane_target(gone, Side::Local, Some(1)),
        cursor: None,
    });
    assert!(open_items(&app).is_empty());
    assert!(app.update(Message::Menu(MenuMsg::Choose(0))).is_empty());
    assert!(app.menu.is_none());
}

#[tokio::test]
async fn menu_closes_when_the_shown_tab_changes() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    // Ctrl+, opens Settings over it.
    app.update(right_click(tab, Side::Local, Some(1)));
    app.update(Message::OpenSettings);
    assert!(app.menu.is_none());
    // Back on the tab, then a form takes the center.
    app.update(Message::SelectTab(0));
    app.update(right_click(tab, Side::Local, Some(1)));
    app.update(Message::NewSite);
    assert!(app.menu.is_none());
    // The tab closes.
    app.update(Message::SelectTab(0));
    app.update(right_click(tab, Side::Local, Some(1)));
    assert!(app.menu.is_some());
    app.update(Message::CloseTab(tab));
    assert!(app.menu.is_none());
}

#[tokio::test]
async fn right_click_behind_dialog_is_ignored() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    app.update(host_key_prompt(7));
    assert!(app.dialogs.is_open());
    assert!(
        app.update(right_click(tab, Side::Local, Some(2)))
            .is_empty()
    );
    assert!(app.menu.is_none());
    assert!(selected(&app, Side::Local).is_empty());
    // Nor behind the palette.
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    app.update(Message::Palette(crate::app::PaletteMsg::Open));
    assert!(app.palette.is_some());
    app.update(right_click(tab, Side::Local, Some(2)));
    assert!(app.menu.is_none());
}

/// Refresh needs a pane that can list: not busy, and on the remote side a session (a
/// failed connection has none; one lost to the network reconnects instead, see
/// `status::menu_action_on_dropped_session_reconnects_like_its_button`). The palette
/// offers it only when the pane can act, like its other entries.
#[tokio::test]
async fn refresh_only_when_the_pane_can_list() {
    let (_fake, mut app, a, tab) = transfer_tab().await;
    let palette_refresh = |app: &mut App| {
        app.update(Message::Palette(crate::app::PaletteMsg::Open));
        let found = app
            .palette_results()
            .iter()
            .any(|e| e.label == strings::menu_refresh());
        app.update(Message::Palette(crate::app::PaletteMsg::Close));
        found
    };
    app.tabs[0].set_focus(Side::Local);
    assert!(palette_refresh(&mut app));
    // Busy: neither the key, nor the menu item, nor the palette.
    app.tabs[0].local_mut().busy = Some(Busy {
        op: RequestId(999),
        kind: BusyKind::Delete,
        done: 0,
        folder: rpath("/home/me"),
    });
    assert!(!palette_refresh(&mut app));
    let f5 = Message::Key {
        key: FileKey::Refresh,
        shift: false,
    };
    assert!(app.update(f5.clone()).is_empty());
    app.update(right_click(tab, Side::Local, None));
    assert!(!enabled(&open_items(&app), MenuAction::Refresh));
    app.update(Message::Menu(MenuMsg::Close));
    // Remote, not connected: nothing listed, nothing reset.
    app.conn_mut(&a.id).state = TabState::Failed(ConnectError {
        session: None,
        error: Error::new(ErrorKind::Network, "down"),
        keyring_unavailable: false,
    });
    app.tabs[0].set_focus(Side::Remote);
    assert!(app.update(f5).is_empty());
    assert_eq!(
        app.tabs[0].remote().listing,
        crate::app::files::Listing::Loaded
    );
    assert!(!palette_refresh(&mut app));
}

// ---- Plan 13 Task 14: menus on compare rows, queue rows, sidebar items and tabs ----

use hoprs_core::{EntryKind, OpenAs};
use hoprs_remote::{Difference, Progress, Status};
use hoprs_transfer::{TransferId, TransferState};

use super::compare::{entry, running};
use super::transfers::{conflict, failed, item, synced};
use crate::app::menu::TabRef;

fn compare_state(app: &App) -> &crate::app::compare::CompareState {
    app.tabs[0].compare().expect("a files tab")
}

/// A difference found by `run`, not selected.
fn found_diff(
    app: &mut App,
    tab: TabId,
    run: crate::app::CompareRunId,
    path: &str,
    status: Status,
    sides: (bool, bool),
) {
    app.update(Message::Compare(CompareMsg::Progress {
        tab,
        run,
        progress: Progress::Found(Difference {
            path: path.to_owned(),
            kind: EntryKind::File,
            status,
            local: sides.0.then(|| entry(path, EntryKind::File, 1)),
            remote: sides.1.then(|| entry(path, EntryKind::File, 2)),
            entries_inside: None,
        }),
    }));
}

/// A running comparison with three rows: 0 only here, 1 only on the server, 2 on both.
async fn compare_rows() -> (Arc<FakeBackend>, App, TabId) {
    let (mut app, fake, _a, tab, run) = running().await;
    found_diff(&mut app, tab, run, "up", Status::OnlyLocal, (true, false));
    found_diff(
        &mut app,
        tab,
        run,
        "down",
        Status::OnlyRemote,
        (false, true),
    );
    found_diff(
        &mut app,
        tab,
        run,
        "both",
        Status::SizeDiffers,
        (true, true),
    );
    (fake, app, tab)
}

fn compare_selected(app: &App) -> Vec<usize> {
    let state = compare_state(app);
    (0..state.rows.len())
        .filter(|&i| state.rows[i].selected)
        .collect()
}

fn compare_right_click(tab: TabId, index: usize) -> Message {
    Message::Compare(CompareMsg::ContextMenu { tab, index })
}

#[tokio::test]
async fn compare_row_menu_items_and_selection_rule() {
    let (_fake, mut app, tab) = compare_rows().await;
    app.update(Message::Compare(CompareMsg::Select {
        tab,
        index: 0,
        on: true,
    }));
    app.update(Message::PointerPressed(Point::new(400.0, 500.0)));
    // A row not selected: selected alone, the list gets the keyboard on it.
    assert!(app.update(compare_right_click(tab, 1)).is_empty());
    assert_eq!(compare_selected(&app), [1]);
    assert!(compare_state(&app).focused);
    assert_eq!(compare_state(&app).cursor, Some(1));
    let menu = app.menu.as_ref().expect("the menu is open");
    assert_eq!(menu.at, Point::new(400.0, 500.0));
    assert_eq!(menu.target, MenuTarget::CompareRow { tab, index: 1 });
    let items = open_items(&app);
    assert_eq!(
        actions(&items),
        [
            MenuAction::CompareUpload,
            MenuAction::CompareDownload,
            MenuAction::ShowInPanes
        ]
    );
    // Only on the server: nothing to upload.
    assert!(!enabled(&items, MenuAction::CompareUpload));
    assert!(enabled(&items, MenuAction::CompareDownload));
    assert!(enabled(&items, MenuAction::ShowInPanes));
    assert!(items.iter().all(|i| !i.destructive && !i.rule_before));

    // A selected row keeps the selection: the menu acts on all of it.
    app.update(Message::Compare(CompareMsg::Select {
        tab,
        index: 2,
        on: true,
    }));
    app.update(compare_right_click(tab, 2));
    assert_eq!(compare_selected(&app), [1, 2]);
    let items = open_items(&app);
    assert!(enabled(&items, MenuAction::CompareUpload));
    assert!(enabled(&items, MenuAction::CompareDownload));

    // A row that is not there opens nothing.
    app.update(Message::Menu(MenuMsg::Close));
    app.update(compare_right_click(tab, 99));
    assert!(app.menu.is_none());
    assert_eq!(compare_selected(&app), [1, 2]);

    // Behind a dialog: nothing.
    app.update(host_key_prompt(3));
    app.update(compare_right_click(tab, 0));
    assert!(app.menu.is_none());
    assert_eq!(compare_selected(&app), [1, 2]);
}

/// Upload and Download are on exactly when the panel's buttons are, and send their
/// messages; Show in panes is the double-click.
#[tokio::test]
async fn compare_menu_choose_matches_panel_buttons() {
    let (_fake, mut app, tab) = compare_rows().await;
    for index in 0..3 {
        app.update(compare_right_click(tab, index));
        let footer = crate::ui::compare::footer(compare_state(&app));
        let items = open_items(&app);
        assert_eq!(enabled(&items, MenuAction::CompareUpload), footer.upload);
        assert_eq!(
            enabled(&items, MenuAction::CompareDownload),
            footer.download
        );
        app.update(Message::Menu(MenuMsg::Close));
    }
    let cases = [
        (
            0,
            MenuAction::CompareUpload,
            Message::Compare(CompareMsg::UploadSelected { tab }),
        ),
        (
            1,
            MenuAction::CompareDownload,
            Message::Compare(CompareMsg::DownloadSelected { tab }),
        ),
        (
            2,
            MenuAction::ShowInPanes,
            Message::Compare(CompareMsg::Open { tab, index: 2 }),
        ),
    ];
    for (row, action, expected) in cases {
        app.update(compare_right_click(tab, row));
        let index = index_of(&app, action);
        let chosen = app.menu_command(Message::Menu(MenuMsg::Choose(index)));
        assert_eq!(format!("{chosen:?}"), format!("{expected:?}"), "{action:?}");
        assert!(app.menu.is_none());
    }
    // Through `update`: the upload queues the selected row, as the button does.
    app.update(compare_right_click(tab, 0));
    let upload = index_of(&app, MenuAction::CompareUpload);
    let effects = app.update(Message::Menu(MenuMsg::Choose(upload)));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::EnqueueCompare { items, .. } if items.len() == 1)),
        "{effects:?}"
    );
}

/// Shift+F10 in the compare list: at the cursor's row, from where the list is (reported
/// whether it scrolls or not), minus how far it is scrolled.
#[tokio::test]
async fn keyboard_menu_in_compare_list() {
    let (_fake, mut app, tab) = compare_rows().await;
    let menu_key = || Message::Key {
        key: FileKey::Menu,
        shift: true,
    };
    app.update(Message::Compare(CompareMsg::Placed {
        tab,
        bounds: iced::Rectangle::new(Point::new(260.0, 400.0), Size::new(800.0, 120.0)),
    }));
    app.update(Message::Compare(CompareMsg::Focus { tab, cursor: None }));
    // No cursor: the first row shown (line 1, under its group's header).
    app.update(menu_key());
    let menu = app.menu.as_ref().expect("the menu is open");
    assert_eq!(menu.target, MenuTarget::CompareRow { tab, index: 0 });
    assert_eq!(menu.at, Point::new(284.0, 460.0));
    assert_eq!(compare_selected(&app), [0], "the selection rule applies");
    assert_eq!(compare_state(&app).cursor, Some(0));
    // Down to the next row (the second group: line 3), scrolled by one line.
    app.update(Message::Menu(MenuMsg::Close));
    app.update(Message::Key {
        key: FileKey::Down,
        shift: false,
    });
    app.update(Message::Compare(CompareMsg::Scrolled {
        tab,
        offset_y: 30.0,
    }));
    app.update(menu_key());
    let menu = app.menu.as_ref().unwrap();
    assert_eq!(menu.target, MenuTarget::CompareRow { tab, index: 1 });
    assert_eq!(menu.at, Point::new(284.0, 490.0));
    // Arrows now drive the menu, not the list.
    app.update(key_pressed(FileKey::Down));
    assert_eq!(app.menu.as_ref().unwrap().cursor, Some(1));
    assert_eq!(compare_state(&app).cursor, Some(1));
    // The panes' keyboard is untouched: not focused, the key goes to the pane.
    app.update(Message::Menu(MenuMsg::Close));
    app.update(Message::Key {
        key: FileKey::Escape,
        shift: false,
    });
    assert!(!compare_state(&app).focused);
    app.update(menu_key());
    assert!(matches!(
        app.menu.as_ref().unwrap().target,
        MenuTarget::Pane { .. }
    ));
}

#[tokio::test]
async fn compare_menu_closes_when_its_rows_go() {
    let (_fake, mut app, tab) = compare_rows().await;
    app.update(compare_right_click(tab, 1));
    assert!(app.menu.is_some());
    app.update(Message::Compare(CompareMsg::Hide { tab }));
    assert!(app.menu.is_none());
}

#[tokio::test]
async fn compare_menu_closes_on_a_new_run() {
    let (_fake, mut app, tab) = compare_rows().await;
    app.update(compare_right_click(tab, 1));
    assert!(app.menu.is_some());
    app.update(Message::Compare(CompareMsg::Start { tab }));
    assert!(app.menu.is_none());
}

#[tokio::test]
async fn compare_menu_closes_on_a_folder_change() {
    let (_fake, mut app, tab) = compare_rows().await;
    app.update(compare_right_click(tab, 1));
    assert!(app.menu.is_some());
    app.update(pane_msg(
        tab,
        Side::Local,
        PaneMsg::Navigate(rpath("/home/me/sub")),
    ));
    assert!(compare_state(&app).rows.is_empty());
    assert!(app.menu.is_none());
}

#[tokio::test]
async fn queue_menu_matches_row_actions() {
    let fake = Arc::new(FakeBackend::new());
    let s = SiteId::new();
    let mut app = loaded(&fake).await;
    synced(
        &mut app,
        3,
        vec![
            item(1, &s, TransferState::Active),
            item(2, &s, failed("boom")),
            item(3, &s, conflict()),
            item(4, &s, TransferState::Paused),
        ],
    );
    let queue_menu = |app: &mut App, id: i64| {
        app.update(Message::Menu(MenuMsg::Open(MenuTarget::QueueRow(
            TransferId(id),
        ))));
        actions(&open_items(app))
    };
    assert_eq!(
        queue_menu(&mut app, 1),
        [
            MenuAction::Queue(RowAction::Pause),
            MenuAction::Queue(RowAction::Cancel)
        ]
    );
    assert_eq!(
        queue_menu(&mut app, 2),
        [
            MenuAction::Queue(RowAction::Retry),
            MenuAction::Queue(RowAction::Dismiss)
        ]
    );
    assert_eq!(
        queue_menu(&mut app, 4),
        [
            MenuAction::Queue(RowAction::Resume),
            MenuAction::Queue(RowAction::Cancel)
        ]
    );
    // A conflict's answers stay in its row: no menu (not an empty box).
    app.update(Message::Menu(MenuMsg::Open(MenuTarget::QueueRow(
        TransferId(3),
    ))));
    assert!(app.menu.is_none());
    // Each item is the row's button.
    queue_menu(&mut app, 1);
    let effects = app.update(Message::Menu(MenuMsg::Choose(0)));
    assert!(
        matches!(
            effects[..],
            [Effect::Control {
                id: TransferId(1),
                op: crate::backend::ControlOp::Pause
            }]
        ),
        "{effects:?}"
    );
    queue_menu(&mut app, 2);
    let chosen = app.menu_command(Message::Menu(MenuMsg::Choose(0)));
    assert_eq!(
        format!("{chosen:?}"),
        format!(
            "{:?}",
            Message::Queue(QueueMsg::Action(TransferId(2), RowAction::Retry))
        )
    );
}

#[tokio::test]
async fn queue_menu_on_removed_row_does_nothing() {
    let fake = Arc::new(FakeBackend::new());
    let s = SiteId::new();
    let mut app = loaded(&fake).await;
    synced(&mut app, 3, vec![item(1, &s, TransferState::Active)]);
    app.update(Message::Menu(MenuMsg::Open(MenuTarget::QueueRow(
        TransferId(1),
    ))));
    assert_eq!(open_items(&app).len(), 2);
    // The row goes (cleared, or a new snapshot without it) while the menu is open: the
    // menu, empty now, closes.
    synced(&mut app, 4, Vec::new());
    assert!(app.menu.is_none());
    let effects = app.update(Message::Menu(MenuMsg::Choose(0)));
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Control { .. })),
        "{effects:?}"
    );
    assert!(app.menu.is_none());
}

#[tokio::test]
async fn site_and_workspace_menus() {
    let a = site("a", "h", "u", Protocol::Ftp);
    let ws = workspace("w", &[&a.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_workspace(ws.clone()),
    );
    let mut app = loaded(&fake).await;
    let chosen = |app: &mut App, target: MenuTarget, action: MenuAction| {
        app.update(Message::Menu(MenuMsg::Open(target)));
        let index = index_of(app, action);
        format!(
            "{:?}",
            app.menu_command(Message::Menu(MenuMsg::Choose(index)))
        )
    };
    // A site: Open files and Edit, nothing else (no Disconnect, no Delete).
    let target = MenuTarget::Site(a.id.clone());
    app.update(Message::Menu(MenuMsg::Open(target.clone())));
    let items = open_items(&app);
    assert_eq!(
        actions(&items),
        [
            MenuAction::OpenSite,
            MenuAction::EditSite,
            MenuAction::Export
        ]
    );
    assert!(items.iter().all(|i| i.enabled && !i.destructive));
    assert_eq!(
        chosen(&mut app, target.clone(), MenuAction::OpenSite),
        format!("{:?}", Message::ConnectSite(a.id.clone()))
    );
    assert_eq!(
        chosen(&mut app, target.clone(), MenuAction::EditSite),
        format!("{:?}", Message::EditSite(a.id.clone()))
    );
    assert_eq!(
        chosen(&mut app, target, MenuAction::Export),
        format!(
            "{:?}",
            Message::Export(ExportMsg::Open(ExportScope::Site(a.id.clone())))
        )
    );
    // A workspace: Open all and Edit (Close all too, only for the open one).
    let target = MenuTarget::Workspace(ws.id.clone());
    app.update(Message::Menu(MenuMsg::Open(target.clone())));
    let items = open_items(&app);
    assert_eq!(
        actions(&items),
        [
            MenuAction::OpenAll,
            MenuAction::EditWorkspace,
            MenuAction::Export
        ]
    );
    assert!(items.iter().all(|i| i.enabled && !i.destructive));
    assert_eq!(
        chosen(&mut app, target.clone(), MenuAction::OpenAll),
        format!("{:?}", Message::OpenWorkspace(ws.id.clone()))
    );
    app.open_workspace = Some(ws.id.clone());
    app.update(Message::Menu(MenuMsg::Open(target.clone())));
    assert_eq!(
        actions(&open_items(&app)),
        [
            MenuAction::OpenAll,
            MenuAction::CloseAll,
            MenuAction::EditWorkspace,
            MenuAction::Export
        ]
    );
    assert_eq!(
        chosen(&mut app, target.clone(), MenuAction::CloseAll),
        format!("{:?}", Message::CloseWorkspace(ws.id.clone()))
    );
    assert_eq!(
        chosen(&mut app, target.clone(), MenuAction::EditWorkspace),
        format!("{:?}", Message::EditWorkspace(ws.id.clone()))
    );
    assert_eq!(
        chosen(&mut app, target, MenuAction::Export),
        format!(
            "{:?}",
            Message::Export(ExportMsg::Open(ExportScope::Workspace(ws.id.clone())))
        )
    );
    // Gone since: no items, so no menu; a choice does nothing.
    let gone = MenuTarget::Site(SiteId::new());
    assert!(app.menu_items(&gone).is_empty());
    app.update(Message::Menu(MenuMsg::Open(gone)));
    assert!(app.menu.is_none());
    assert!(app.update(Message::Menu(MenuMsg::Choose(0))).is_empty());
}

#[test]
fn sidebar_options_offer_export_everything() {
    let mut app = App::new();
    let target = MenuTarget::SidebarOptions;
    app.update(Message::Menu(MenuMsg::Open(target)));
    let items = open_items(&app);
    assert_eq!(
        actions(&items),
        [
            MenuAction::ShowWorkspaces,
            MenuAction::ShowSites,
            MenuAction::ExportAll
        ]
    );
    let last = &items[2];
    assert!(last.rule_before && last.enabled);
    assert_eq!(last.checked, None);
    app.update(Message::Menu(MenuMsg::Choose(2)));
    assert!(app.menu.is_none());
    assert!(matches!(app.form, Some(Form::Export)));
    let flow = app.export.as_ref().expect("the export flow");
    assert!(matches!(
        flow.title,
        crate::app::export::ExportTitle::All { .. }
    ));
}

#[test]
fn export_labels_and_icon() {
    use crate::ui::icons::Icon;
    use crate::ui::menu::{icon, label, shortcut};
    assert_eq!(label(MenuAction::Export), "Export…");
    assert_eq!(label(MenuAction::ExportAll), "Export everything…");
    for action in [MenuAction::Export, MenuAction::ExportAll] {
        assert_eq!(icon(action), Icon::ShieldCheck);
        assert_eq!(shortcut(action), None);
    }
}

#[tokio::test]
async fn menu_closes_when_its_tab_closes() {
    let (mut app, _sa, _sb) = three_tabs().await;
    app.update(Message::SelectTab(0));
    let other = app.tabs[2].id;
    // The menu of a tab the center does not show.
    app.update(Message::Menu(MenuMsg::Open(MenuTarget::Tab(
        TabRef::Session(other),
    ))));
    assert!(app.menu.is_some());
    app.update(Message::CloseTab(other));
    assert!(app.tab_index(other).is_none());
    assert_eq!(app.active_tab, Some(0), "the shown tab did not change");
    assert!(app.menu.is_none(), "an empty menu does not stay open");
}

/// Two sites: `a` (SSH, terminal and files tabs on one session) and `b` (FTP), connected,
/// and the settings tab open.
async fn three_tabs() -> (App, SessionId, SessionId) {
    let mut a = site("a", "a.example", "u", Protocol::Ssh);
    a.ssh.open_as = OpenAs::Both;
    let b = site("b", "b.example", "u", Protocol::Ftp);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()).with_site(b.clone()));
    let mut app = loaded(&fake).await;
    let mut sites = connect_effect(app.update(Message::ConnectSite(a.id.clone())));
    sites.extend(connect_effect(
        app.update(Message::ConnectSite(b.id.clone())),
    ));
    drive(&mut app, &fake, sites).await;
    assert_eq!(app.tabs.len(), 3);
    let sa = app.tab_conn(0).session().expect("a connected");
    let sb = app.tab_conn(2).session().expect("b connected");
    app.update(Message::OpenSettings);
    (app, sa, sb)
}

fn closed_sessions(effects: &[Effect]) -> Vec<SessionId> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::CloseSession(s) => Some(*s),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn close_other_tabs_keeps_one() {
    let (mut app, _sa, sb) = three_tabs().await;
    let keep = app.tabs[1].id;
    let target = MenuTarget::Tab(TabRef::Session(keep));
    app.update(Message::Menu(MenuMsg::Open(target.clone())));
    let items = open_items(&app);
    assert_eq!(
        actions(&items),
        [MenuAction::CloseTab, MenuAction::CloseOtherTabs]
    );
    assert!(items.iter().all(|i| i.enabled));
    let chosen = app.menu_command(Message::Menu(MenuMsg::Choose(1)));
    assert_eq!(
        format!("{chosen:?}"),
        format!("{:?}", Message::CloseOtherTabs(TabRef::Session(keep)))
    );
    // Close tab is the ×.
    app.update(Message::Menu(MenuMsg::Open(target)));
    let chosen = app.menu_command(Message::Menu(MenuMsg::Choose(0)));
    assert_eq!(
        format!("{chosen:?}"),
        format!("{:?}", Message::CloseTab(keep))
    );

    let effects = app.update(Message::CloseOtherTabs(TabRef::Session(keep)));
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.tabs[0].id, keep);
    assert_eq!(app.active_tab, Some(0));
    assert!(app.settings_tab.is_none());
    // `a` keeps its session for the kept tab; only `b`'s goes.
    assert_eq!(closed_sessions(&effects), [sb]);
    // Alone now: Close other tabs is off.
    app.update(Message::Menu(MenuMsg::Open(MenuTarget::Tab(
        TabRef::Session(keep),
    ))));
    assert!(!enabled(&open_items(&app), MenuAction::CloseOtherTabs));
}

#[tokio::test]
async fn close_other_tabs_from_settings() {
    let (mut app, sa, sb) = three_tabs().await;
    app.update(Message::SelectTab(0));
    assert!(!app.settings_shown());
    let target = MenuTarget::Tab(TabRef::Settings);
    app.update(Message::Menu(MenuMsg::Open(target.clone())));
    assert_eq!(
        actions(&open_items(&app)),
        [MenuAction::CloseTab, MenuAction::CloseOtherTabs]
    );
    let chosen = app.menu_command(Message::Menu(MenuMsg::Choose(0)));
    assert_eq!(
        format!("{chosen:?}"),
        format!("{:?}", Message::CloseSettings)
    );
    app.update(Message::Menu(MenuMsg::Open(target)));
    let effects = app.update(Message::Menu(MenuMsg::Choose(1)));
    assert!(app.tabs.is_empty());
    assert!(app.settings_shown(), "the settings tab stays, selected");
    let mut closed = closed_sessions(&effects);
    closed.sort_by_key(|s| s.0);
    let mut expected = vec![sa, sb];
    expected.sort_by_key(|s| s.0);
    assert_eq!(closed, expected);
}

#[test]
fn close_other_tabs_ignored_while_closing() {
    let mut app = App::new();
    app.update(Message::OpenSettings);
    app.closing = true;
    assert!(
        app.update(Message::CloseOtherTabs(TabRef::Settings))
            .is_empty()
    );
    assert!(app.settings_tab.is_some());
}

/// While the window closes, where the lists are and how far they scrolled still get
/// stored (the bounds reporter would otherwise publish them again every event); the
/// user's input stays ignored.
#[tokio::test]
async fn lists_placed_and_scrolled_while_closing() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    app.closing = true;
    let bounds = iced::Rectangle::new(Point::new(300.0, 200.0), Size::new(400.0, 600.0));
    assert!(
        app.update(pane_msg(tab, Side::Local, PaneMsg::Placed(bounds)))
            .is_empty()
    );
    assert_eq!(app.tabs[0].local().bounds, bounds);
    app.update(pane_msg(
        tab,
        Side::Local,
        PaneMsg::Scrolled {
            offset_y: 30.0,
            viewport_h: 500.0,
        },
    ));
    assert_eq!(app.tabs[0].local().offset_y, 30.0);
    app.update(Message::Queue(QueueMsg::Scrolled { offset_y: 12.0 }));
    assert_eq!(app.queue.offset_y, 12.0);
    // A click is still dropped.
    let before = app.tabs[0].local().selection.clone();
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(1)));
    assert_eq!(app.tabs[0].local().selection, before);
}
