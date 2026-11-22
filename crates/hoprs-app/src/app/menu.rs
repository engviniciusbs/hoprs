//! Context menus: the model of the one menu that may be open ([`App::menu`]).
//!
//! - **Where:** a menu opens at [`App::last_press`], the window position of the press that
//!   opened it (published by the root [`crate::ui::pointer`] widget before any button sees
//!   the press); [`crate::ui::menu::origin`] keeps it inside the window.
//! - **What:** [`App::menu_items`] lists the items for the target as the app is now; an
//!   item maps to the message the same action sends elsewhere (a button, a key, the
//!   palette), so a menu never does anything the app cannot do without it.
//! - **Choosing:** [`App::menu_command`] turns a choice into the item's own message at the
//!   top of `update`, as the palette does with a run row, so it is handled exactly as if
//!   its button or key had sent it (the reconnection of a lost session included).
//! - **Opening from the keyboard:** Shift+F10 / the Menu key, only where the keyboard
//!   already is: a file pane and the compare list (at the cursor's row). The queue,
//!   sidebar and tab menus are mouse-only; their items have their buttons, keys and the
//!   palette.
//! - **Keyboard:** while a menu is open ↑/↓ move its highlight over the enabled items
//!   (wrapping), Enter chooses the highlighted one, Escape closes it; any other key closes
//!   it and then does what it always does ([`App::menu_key`]).
//! - **Closing:** Escape, a press outside, a scroll, a window resize, choosing an item; a
//!   security dialog or the palette closes it too, and so does the center showing another
//!   tab (or Settings, or a form), a new listing of the pane it is about, and the compare
//!   rows it is about going (a new run, a folder change, the panel's cross). Like other
//!   window input it is blocked behind a dialog or the palette, and ignored while the
//!   window closes.

use hoprs_core::{SiteId, WorkspaceId};
use hoprs_transfer::TransferId;
use hoprs_vault::ExportScope;
use iced::Point;

use super::files::Row;
use super::{
    App, CompareMsg, Effect, ExportMsg, FileKey, Message, PaneMsg, QueueMsg, RowAction, SidebarMsg,
    SidebarSection, TabId,
};
use crate::backend::Side;

/// The open menu.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextMenu {
    /// Where it was asked for, in window logical pixels.
    pub at: Point,
    pub target: MenuTarget,
    /// The item the keyboard highlights, if any.
    pub cursor: Option<usize>,
}

/// What a menu is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuTarget {
    /// The sidebar's ⋯ button: which sections show.
    SidebarOptions,
    /// A file pane of a tab: on `row` (and the selection it is part of), or on the pane's
    /// empty area (`None`).
    Pane {
        tab: TabId,
        side: Side,
        row: Option<usize>,
    },
    /// Row `index` (into the panel's rows) of a tab's folder compare, and the selection
    /// it is part of.
    CompareRow { tab: TabId, index: usize },
    /// A row of the transfer queue.
    QueueRow(TransferId),
    /// A site in the sidebar.
    Site(SiteId),
    /// A workspace's head in the sidebar.
    Workspace(WorkspaceId),
    /// A tab of the strip.
    Tab(TabRef),
}

/// A tab of the strip: a session tab, or the settings tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabRef {
    Session(TabId),
    Settings,
}

/// What an item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// Show or hide the workspaces section.
    ShowWorkspaces,
    /// Show or hide the sites section.
    ShowSites,
    /// Upload (from the local pane) / Download (from the remote one), Ctrl+Enter.
    Transfer(Side),
    /// Edit the remote file in the editor, F4.
    Edit,
    /// Enter the folder (`..` too), as a double click does.
    Open,
    /// F2.
    Rename,
    /// Permissions… (remote).
    Permissions,
    /// The selected entries' full paths to the clipboard.
    CopyPath,
    /// F5.
    Refresh,
    /// F7.
    NewFolder,
    /// Del.
    Delete,
    /// The tab's folder compare (the local toolbar's button).
    Compare,
    /// The compare panel's "Upload selected".
    CompareUpload,
    /// The compare panel's "Download selected".
    CompareDownload,
    /// The panes go to the compare row's item (its double-click, Enter).
    ShowInPanes,
    /// A queue row's button (never a conflict's answer: those stay in the row).
    Queue(RowAction),
    /// Connects the site, or shows its tab (a click on it).
    OpenSite,
    /// The site's form (its pencil).
    EditSite,
    /// The workspace's "Open all": its sites connect, its tabs come back.
    OpenAll,
    /// The open workspace's "Close all": its tabs are saved, then closed.
    CloseAll,
    /// The workspace's form (its pencil).
    EditWorkspace,
    /// Opens the export screen for the site or the workspace the menu is about.
    Export,
    /// Opens the export screen for everything (the sidebar's ⋯ menu).
    ExportAll,
    /// The tab's ×.
    CloseTab,
    /// Every other tab of the strip closes (each as its × would).
    CloseOtherTabs,
}

/// One item, as the menu shows it now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub action: MenuAction,
    /// Shown dimmed and not clickable when `false`.
    pub enabled: bool,
    /// A check item's state; `None` for a plain item.
    pub checked: Option<bool>,
    /// Drawn in the error colour (and last, after a rule).
    pub destructive: bool,
    /// A rule separates it from the item above.
    pub rule_before: bool,
}

impl MenuItem {
    fn new(action: MenuAction) -> Self {
        Self {
            action,
            enabled: true,
            checked: None,
            destructive: false,
            rule_before: false,
        }
    }

    fn enabled(action: MenuAction, enabled: bool) -> Self {
        Self {
            enabled,
            ..Self::new(action)
        }
    }
}

/// Something in the menu.
#[derive(Debug, Clone)]
pub enum MenuMsg {
    /// Opens the menu of `target` at the last press (replacing an open one).
    Open(MenuTarget),
    /// Chooses item `n` of the open menu: it closes, then the item runs if it is enabled.
    Choose(usize),
    /// The pointer is over item `n`: it is highlighted (the keyboard goes on from there).
    Hover(usize),
    /// A press outside, a scroll: closes it.
    Close,
}

impl App {
    /// The items of `target`'s menu, as the app is now.
    pub fn menu_items(&self, target: &MenuTarget) -> Vec<MenuItem> {
        match target {
            MenuTarget::SidebarOptions => [
                (MenuAction::ShowWorkspaces, SidebarSection::Workspaces),
                (MenuAction::ShowSites, SidebarSection::Sites),
            ]
            .into_iter()
            .map(|(action, section)| MenuItem {
                checked: Some(!self.section(section).hidden),
                ..MenuItem::new(action)
            })
            .chain(std::iter::once(MenuItem {
                rule_before: true,
                ..MenuItem::new(MenuAction::ExportAll)
            }))
            .collect(),
            MenuTarget::Pane { tab, side, row } => self.pane_menu(*tab, *side, *row),
            MenuTarget::CompareRow { tab, index } => self.compare_menu(*tab, *index),
            MenuTarget::QueueRow(id) => self
                .queue
                .row(*id)
                .map(|row| queue_menu_actions(&self.queue, row))
                .unwrap_or_default()
                .into_iter()
                .map(|action| MenuItem::new(MenuAction::Queue(action)))
                .collect(),
            MenuTarget::Site(id) => match self.site(id) {
                Some(_) => vec![
                    MenuItem::new(MenuAction::OpenSite),
                    MenuItem::new(MenuAction::EditSite),
                    MenuItem::new(MenuAction::Export),
                ],
                None => Vec::new(),
            },
            MenuTarget::Workspace(id) => match self.workspaces.iter().any(|w| &w.id == id) {
                true => {
                    let mut items = vec![MenuItem::new(MenuAction::OpenAll)];
                    if self.open_workspace.as_ref() == Some(id) {
                        items.push(MenuItem::new(MenuAction::CloseAll));
                    }
                    items.push(MenuItem::new(MenuAction::EditWorkspace));
                    items.push(MenuItem::new(MenuAction::Export));
                    items
                }
                false => Vec::new(),
            },
            MenuTarget::Tab(tab) => {
                let open = match tab {
                    TabRef::Session(id) => self.tab_index(*id).is_some(),
                    TabRef::Settings => self.settings_tab.is_some(),
                };
                if !open {
                    return Vec::new();
                }
                let count = self.tabs.len() + usize::from(self.settings_tab.is_some());
                vec![
                    MenuItem::new(MenuAction::CloseTab),
                    MenuItem::enabled(MenuAction::CloseOtherTabs, count > 1),
                ]
            }
        }
    }

    /// A compare row's items: Upload and Download on exactly when the panel's buttons
    /// are (some selected row can go that way), Show in panes while the row is there.
    fn compare_menu(&self, tab: TabId, index: usize) -> Vec<MenuItem> {
        let Some(state) = self
            .tab_index(tab)
            .and_then(|i| self.tabs[i].compare())
            .filter(|state| index < state.rows.len())
        else {
            return Vec::new();
        };
        vec![
            MenuItem::enabled(
                MenuAction::CompareUpload,
                state.selected_uploads().next().is_some(),
            ),
            MenuItem::enabled(
                MenuAction::CompareDownload,
                state.selected_downloads().next().is_some(),
            ),
            MenuItem::new(MenuAction::ShowInPanes),
        ]
    }

    /// A pane's items: each on when its toolbar button (or key) would act now, off (not
    /// hidden) otherwise. Delete comes last, after a rule.
    fn pane_menu(&self, tab: TabId, side: Side, row: Option<usize>) -> Vec<MenuItem> {
        let Some(index) = self.tab_index(tab).filter(|&i| self.tabs[i].is_files()) else {
            return Vec::new();
        };
        let tab = &self.tabs[index];
        let conn = self.tab_conn(index);
        let Some(pane) = tab.pane(side) else {
            return Vec::new();
        };
        let idle = pane.busy.is_none();
        let can = tab.can_act(conn, side);
        let selected = !pane.selected_entries().is_empty();
        let refresh = MenuItem::enabled(MenuAction::Refresh, self.can_refresh(index, side));
        let new_folder = MenuItem::enabled(MenuAction::NewFolder, can);
        let Some(row) = row else {
            let compare = tab.can_act(conn, Side::Remote);
            return vec![
                new_folder,
                refresh,
                MenuItem::enabled(MenuAction::Compare, compare),
            ];
        };
        let (is_file, is_folder) = match pane.row(row) {
            Some(Row::Up) => (false, true),
            Some(Row::Entry(entry)) => (
                entry.kind == hoprs_core::EntryKind::File,
                entry.kind != hoprs_core::EntryKind::File,
            ),
            None => (false, false),
        };
        let mut items = vec![MenuItem::enabled(
            MenuAction::Transfer(side),
            tab.can_transfer(conn, side),
        )];
        if side == Side::Remote && is_file {
            let edit = matches!(tab.edit_target(conn, side), Some(Ok(_)));
            items.push(MenuItem::enabled(MenuAction::Edit, edit));
        }
        if is_folder {
            items.push(MenuItem::enabled(MenuAction::Open, can));
        }
        items.push(MenuItem::enabled(MenuAction::Rename, can && selected));
        if side == Side::Remote {
            let chmod = can && !pane.permission_targets().is_empty();
            items.push(MenuItem::enabled(MenuAction::Permissions, chmod));
        }
        items.extend([
            MenuItem::enabled(MenuAction::CopyPath, idle && selected),
            refresh,
            new_folder,
            MenuItem {
                destructive: true,
                rule_before: true,
                ..MenuItem::enabled(MenuAction::Delete, can && selected)
            },
        ]);
        items
    }

    /// The message that flips whether `section` is hidden.
    fn set_hidden(&self, section: SidebarSection) -> Message {
        Message::Sidebar(SidebarMsg::SetHidden {
            section,
            hidden: !self.section(section).hidden,
        })
    }

    /// The message `action` sends for `target`, if it does anything now.
    fn menu_message(&self, target: &MenuTarget, action: MenuAction) -> Option<Message> {
        Some(match (target, action) {
            (MenuTarget::SidebarOptions, MenuAction::ShowWorkspaces) => {
                self.set_hidden(SidebarSection::Workspaces)
            }
            (MenuTarget::SidebarOptions, MenuAction::ShowSites) => {
                self.set_hidden(SidebarSection::Sites)
            }
            (MenuTarget::SidebarOptions, MenuAction::ExportAll) => {
                Message::Export(ExportMsg::Open(ExportScope::All))
            }
            (&MenuTarget::Pane { tab, .. }, MenuAction::Compare) => {
                Message::Compare(CompareMsg::Start { tab })
            }
            (&MenuTarget::Pane { tab, side, row }, action) => {
                let msg = match action {
                    MenuAction::Transfer(_) => PaneMsg::Transfer,
                    MenuAction::Edit => PaneMsg::Edit,
                    MenuAction::Open => PaneMsg::Open(row?),
                    MenuAction::Rename => PaneMsg::Rename,
                    MenuAction::Permissions => PaneMsg::Permissions,
                    MenuAction::CopyPath => PaneMsg::CopyPath,
                    MenuAction::Refresh => PaneMsg::Refresh,
                    MenuAction::NewFolder => PaneMsg::NewFolder,
                    MenuAction::Delete => PaneMsg::Delete,
                    _ => return None,
                };
                Message::Pane { tab, side, msg }
            }
            (&MenuTarget::CompareRow { tab, index }, action) => Message::Compare(match action {
                MenuAction::CompareUpload => CompareMsg::UploadSelected { tab },
                MenuAction::CompareDownload => CompareMsg::DownloadSelected { tab },
                MenuAction::ShowInPanes => CompareMsg::Open { tab, index },
                _ => return None,
            }),
            (&MenuTarget::QueueRow(id), MenuAction::Queue(action)) => {
                Message::Queue(QueueMsg::Action(id, action))
            }
            (MenuTarget::Site(id), MenuAction::OpenSite) => Message::ConnectSite(id.clone()),
            (MenuTarget::Site(id), MenuAction::EditSite) => Message::EditSite(id.clone()),
            (MenuTarget::Site(id), MenuAction::Export) => {
                Message::Export(ExportMsg::Open(ExportScope::Site(id.clone())))
            }
            (MenuTarget::Workspace(id), MenuAction::Export) => {
                Message::Export(ExportMsg::Open(ExportScope::Workspace(id.clone())))
            }
            (MenuTarget::Workspace(id), MenuAction::OpenAll) => Message::OpenWorkspace(id.clone()),
            (MenuTarget::Workspace(id), MenuAction::CloseAll) => {
                Message::CloseWorkspace(id.clone())
            }
            (MenuTarget::Workspace(id), MenuAction::EditWorkspace) => {
                Message::EditWorkspace(id.clone())
            }
            (&MenuTarget::Tab(TabRef::Session(id)), MenuAction::CloseTab) => Message::CloseTab(id),
            (MenuTarget::Tab(TabRef::Settings), MenuAction::CloseTab) => Message::CloseSettings,
            (&MenuTarget::Tab(tab), MenuAction::CloseOtherTabs) => Message::CloseOtherTabs(tab),
            _ => return None,
        })
    }

    /// A key while a menu is open (`None`: the menu took it). ↑/↓ move the highlight over
    /// the enabled items, wrapping (from none: the first or the last); Enter becomes the
    /// choice of the highlighted item (nothing without one); Escape closes the menu. Any
    /// other key closes it and goes on to do what it always does. Anything else, or no
    /// open menu, passes unchanged.
    pub(super) fn menu_key(&mut self, msg: Message) -> Option<Message> {
        let Message::KeyPressed { key, .. } = msg else {
            return Some(msg);
        };
        let Some(menu) = &self.menu else {
            return Some(msg);
        };
        match key {
            FileKey::Up | FileKey::Down => {
                let enabled: Vec<usize> = self
                    .menu_items(&menu.target)
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| item.enabled)
                    .map(|(i, _)| i)
                    .collect();
                let at = menu
                    .cursor
                    .and_then(|c| enabled.iter().position(|&i| i == c));
                let count = enabled.len();
                let next = match (key, at) {
                    _ if count == 0 => None,
                    (FileKey::Down, Some(p)) => Some((p + 1) % count),
                    (FileKey::Down, None) => Some(0),
                    (_, Some(p)) => Some((p + count - 1) % count),
                    (_, None) => Some(count - 1),
                };
                if let (Some(p), Some(menu)) = (next, &mut self.menu) {
                    menu.cursor = Some(enabled[p]);
                }
                None
            }
            FileKey::Enter => menu.cursor.map(|c| Message::Menu(MenuMsg::Choose(c))),
            FileKey::Escape => {
                self.menu = None;
                None
            }
            _ => {
                self.menu = None;
                Some(msg)
            }
        }
    }

    /// A choice (`MenuMsg::Choose`) closes the menu and becomes the item's own message,
    /// which `update` then handles as if its button or key had sent it; a missing or
    /// disabled item, or no open menu, leaves the choice as it is (it does nothing).
    /// Anything else passes unchanged.
    pub(super) fn menu_command(&mut self, msg: Message) -> Message {
        let Message::Menu(MenuMsg::Choose(index)) = msg else {
            return msg;
        };
        let Some(menu) = self.menu.take() else {
            return msg;
        };
        self.menu_items(&menu.target)
            .get(index)
            .filter(|item| item.enabled)
            .and_then(|item| self.menu_message(&menu.target, item.action))
            .unwrap_or(msg)
    }

    pub(super) fn menu_msg(&mut self, msg: MenuMsg) -> Vec<Effect> {
        match msg {
            MenuMsg::Open(target) => {
                self.menu = Some(ContextMenu {
                    at: self.last_press,
                    target,
                    cursor: None,
                });
                Vec::new()
            }
            MenuMsg::Hover(index) => {
                if let Some(menu) = &mut self.menu {
                    menu.cursor = Some(index);
                }
                Vec::new()
            }
            // `update` resolved it already ([`App::menu_command`]): what reaches here
            // chose nothing, and the menu is closed.
            MenuMsg::Choose(_) | MenuMsg::Close => {
                self.menu = None;
                Vec::new()
            }
        }
    }
}

/// A queue row's menu: its buttons, but a conflict's answers (they stay in the row, next
/// to its "Apply to all").
pub fn queue_menu_actions(
    queue: &super::queue::Queue,
    row: &super::queue::QueueRow,
) -> Vec<RowAction> {
    queue
        .actions(row)
        .into_iter()
        .filter(|action| !matches!(action, RowAction::Resolve(_)))
        .collect()
}
