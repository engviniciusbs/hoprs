//! A file tab's two panes, local and remote: listing, sorting, selection and navigation.
//! Pure state and its update; `ui::files` draws it and `effects` runs the listings.
//!
//! Every listing request carries a [`RequestId`]. A pane remembers only its latest one,
//! and a [`Message::Listed`] with any other id is dropped: the user left that folder (or
//! asked again) while the old listing was in flight. The old listing is never cancelled;
//! it finishes and its result is discarded (Review Focus 1).

use std::collections::{BTreeSet, HashSet};

use hoprs_core::{EntryKind, Error, ErrorKind, RemoteEntry, RemotePath};
use iced::{Point, Rectangle};

use super::actions::{Busy, PaneAction};
use super::menu::{ContextMenu, MenuTarget};
use super::{App, Drag, Effect, TabId};
use crate::agent::paths::local_os_path;
use crate::backend::Side;
use crate::ui::virtual_list::ROW_HEIGHT;

/// How far into a row a menu opened from the keyboard starts (past the kind icon).
pub(super) const MENU_INSET: f32 = 24.0;

/// One listing request; only a pane's latest one is applied.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(pub u64);

/// Where a pane's listing is.
#[derive(Clone, Debug, PartialEq)]
pub enum Listing {
    /// Nothing asked yet: the remote side before its session is connected.
    Idle,
    Loading,
    Loaded,
    Failed(Error),
}

/// The column a pane is sorted by.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Modified,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Sort {
    pub key: SortKey,
    pub descending: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Self {
            key: SortKey::Name,
            descending: false,
        }
    }
}

/// The keyboard modifiers held, as the selection reads them (`ctrl` is Cmd on macOS).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
}

/// Selected rows (row 0 is `..` when the folder has a parent), the anchor a Shift range
/// starts from and the cursor the arrows move.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub rows: BTreeSet<usize>,
    pub anchor: Option<usize>,
    pub cursor: Option<usize>,
}

impl Selection {
    /// A click on `row`: alone; Ctrl toggles it; Shift selects from the anchor to it
    /// (added to the selection with Ctrl too).
    pub fn click(&mut self, row: usize, mods: Mods) {
        match (mods.shift, mods.ctrl) {
            (true, ctrl) => {
                let anchor = *self.anchor.get_or_insert(row);
                if !ctrl {
                    self.rows.clear();
                }
                self.rows.extend(anchor.min(row)..=anchor.max(row));
            }
            (false, true) => {
                if !self.rows.remove(&row) {
                    self.rows.insert(row);
                }
                self.anchor = Some(row);
            }
            (false, false) => {
                self.rows = BTreeSet::from([row]);
                self.anchor = Some(row);
            }
        }
        self.cursor = Some(row);
    }

    /// The arrows: the cursor moves to `row`, alone or, with Shift, extending the range
    /// from the anchor.
    pub fn move_to(&mut self, row: usize, shift: bool) {
        self.click(row, Mods { ctrl: false, shift });
    }

    pub fn contains(&self, row: usize) -> bool {
        self.rows.contains(&row)
    }
}

/// One row of a pane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Row<'a> {
    /// `..`, to the parent folder.
    Up,
    Entry(&'a RemoteEntry),
}

/// One side of a file tab.
#[derive(Clone, Debug, PartialEq)]
pub struct Pane {
    pub path: RemotePath,
    pub listing: Listing,
    /// Sorted by `sort`, once per listing or header click.
    pub entries: Vec<RemoteEntry>,
    pub sort: Sort,
    pub selection: Selection,
    /// The row under the mouse.
    pub hovered: Option<usize>,
    /// The list's scroll offset and visible height, as `on_scroll` last reported them.
    pub offset_y: f32,
    pub viewport_h: f32,
    /// Where the list is in the window, as [`crate::ui::bounds`] last reported it (where
    /// a menu opened from the keyboard goes).
    pub bounds: Rectangle,
    /// The request whose result this pane waits for.
    pub request: Option<RequestId>,
    /// The inline editor, delete confirmation or permissions dialog open, if any.
    pub action: Option<PaneAction>,
    /// The file operation in flight, if any.
    pub busy: Option<Busy>,
    /// The last operation's failure, inline above the table until dismissed.
    pub action_error: Option<Error>,
    /// The entry to select once the folder is listed again (a new or renamed one).
    pub select_after: Option<String>,
    /// The folder came from a workspace's saved tab and has not listed yet: if it is
    /// gone, the pane falls back (parent, then home or `/`) instead of failing.
    pub restore: Option<Restore>,
    /// An inline note above the table (a saved folder that is gone), until dismissed.
    pub note: Option<String>,
    /// A transfer finished into this folder: listed again quietly when the re-list delay
    /// runs out and the pane is free ([`super::transfers`]).
    pub relist: bool,
    /// The listing in flight is such a quiet re-list: the rows stay on screen meanwhile,
    /// and the selection and the scroll survive it.
    pub soft: bool,
    /// When the quiet re-list in flight started.
    pub soft_since: Option<std::time::Instant>,
    /// How long the last quiet re-list took: the next one waits at least as long (a slow
    /// FTP listing does not run back to back during a burst of transfers).
    pub soft_took: std::time::Duration,
}

/// Where a pane is in its fallback from a saved folder that is gone.
#[derive(Clone, Debug, PartialEq)]
pub enum Restore {
    /// Listing the saved folder itself.
    Saved,
    /// Listing the saved folder's parent; `saved` is the folder that is gone.
    Parent { saved: RemotePath },
}

impl Pane {
    pub fn new(path: RemotePath) -> Self {
        Self {
            path,
            listing: Listing::Idle,
            entries: Vec::new(),
            sort: Sort::default(),
            selection: Selection::default(),
            hovered: None,
            offset_y: 0.0,
            viewport_h: 0.0,
            bounds: Rectangle::default(),
            request: None,
            action: None,
            busy: None,
            action_error: None,
            select_after: None,
            restore: None,
            note: None,
            relist: false,
            soft: false,
            soft_since: None,
            soft_took: std::time::Duration::ZERO,
        }
    }

    /// Whether row 0 is `..`.
    pub fn has_up(&self) -> bool {
        self.path.parent().is_some()
    }

    pub fn row_count(&self) -> usize {
        usize::from(self.has_up()) + self.entries.len()
    }

    pub fn row(&self, index: usize) -> Option<Row<'_>> {
        match (self.has_up(), index) {
            (true, 0) => Some(Row::Up),
            (true, i) => self.entries.get(i - 1).map(Row::Entry),
            (false, i) => self.entries.get(i).map(Row::Entry),
        }
    }

    /// Re-sorts by `sort`, keeping the same entries selected.
    fn resort(&mut self, sort: Sort) {
        let up = usize::from(self.has_up());
        let name_of = |pane: &Pane, row: Option<usize>| {
            row.and_then(|r| r.checked_sub(up))
                .and_then(|i| pane.entries.get(i))
                .map(|e| e.name.clone())
        };
        let selected: HashSet<String> = self
            .selection
            .rows
            .iter()
            .filter_map(|&r| name_of(self, Some(r)))
            .collect();
        let anchor = name_of(self, self.selection.anchor);
        let cursor = name_of(self, self.selection.cursor);
        let up_selected = up == 1 && self.selection.contains(0);

        self.sort = sort;
        sort_entries(&mut self.entries, sort);

        let mut selection = Selection::default();
        if up_selected {
            selection.rows.insert(0);
        }
        for (i, entry) in self.entries.iter().enumerate() {
            if selected.contains(&entry.name) {
                selection.rows.insert(i + up);
            }
            if anchor.as_deref() == Some(&entry.name) {
                selection.anchor = Some(i + up);
            }
            if cursor.as_deref() == Some(&entry.name) {
                selection.cursor = Some(i + up);
            }
        }
        // `..` keeps its place at the top.
        selection.anchor = selection
            .anchor
            .or(self.selection.anchor.filter(|&r| r < up));
        selection.cursor = selection
            .cursor
            .or(self.selection.cursor.filter(|&r| r < up));
        self.selection = selection;
    }

    /// Where a menu opened from the keyboard on `row` goes: a little into the row, at its
    /// bottom edge, in window coordinates.
    pub fn menu_anchor(&self, row: usize) -> Point {
        Point::new(
            self.bounds.x + MENU_INSET,
            self.bounds.y + (row + 1) as f32 * ROW_HEIGHT - self.offset_y,
        )
    }

    /// The offset that brings `row` into view, if it is not.
    pub(super) fn scroll_to_show(&self, row: usize) -> Option<f32> {
        if self.viewport_h <= 0.0 {
            return None;
        }
        let top = row as f32 * ROW_HEIGHT;
        let bottom = top + ROW_HEIGHT;
        if top < self.offset_y {
            Some(top)
        } else if bottom > self.offset_y + self.viewport_h {
            Some(bottom - self.viewport_h)
        } else {
            None
        }
    }
}

/// Folders first, then the rest; within each group by `sort` (names compared without
/// case, then exactly, so the order is total and stable).
pub fn sort_entries(entries: &mut Vec<RemoteEntry>, sort: Sort) {
    let is_dir = |e: &RemoteEntry| e.kind == EntryKind::Dir;
    // Lower-cased once per entry, not once per comparison.
    let mut keyed: Vec<(String, RemoteEntry)> = entries
        .drain(..)
        .map(|e| (e.name.to_lowercase(), e))
        .collect();
    keyed.sort_by(|(ka, a), (kb, b)| {
        let by_name = ka.cmp(kb).then_with(|| a.name.cmp(&b.name));
        let order = match sort.key {
            SortKey::Name => by_name,
            // Folder sizes mean nothing: folders keep their name order.
            SortKey::Size if is_dir(a) && is_dir(b) => by_name,
            SortKey::Size => a.size.cmp(&b.size).then(by_name),
            SortKey::Modified => a.modified.cmp(&b.modified).then(by_name),
        };
        let order = if sort.descending {
            order.reverse()
        } else {
            order
        };
        is_dir(b).cmp(&is_dir(a)).then(order)
    });
    entries.extend(keyed.into_iter().map(|(_, e)| e));
}

/// What a pane shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaneStatus<'a> {
    /// The skeleton: the listing (or, on the remote side, the connection) is on its way.
    Loading,
    Empty,
    Failed(&'a Error),
    /// The remote side of a tab whose connection failed: "Not connected", Reconnect.
    NotConnected,
    Full,
}

/// What a pane's widgets send.
#[derive(Clone, Debug)]
pub enum PaneMsg {
    /// Focus the pane (a click anywhere in it).
    Focus,
    Click(usize),
    /// Double click on a row: enter the folder.
    Open(usize),
    /// A breadcrumb segment.
    Navigate(RemotePath),
    Sort(SortKey),
    Scrolled {
        offset_y: f32,
        viewport_h: f32,
    },
    /// Where the list is in the window (reported when it changes, scrolling or not).
    Placed(Rectangle),
    Hover(usize),
    Unhover(usize),
    Retry,
    /// The toolbar.
    NewFolder,
    Rename,
    Delete,
    Permissions,
    /// "Edit" (remote): opens the file in the editor ([`super::edit`]).
    Edit,
    /// The inline name editor.
    EditChanged(String),
    EditCommit,
    /// Cancel / Keep in the editor, confirmation or dialog.
    CancelAction,
    ConfirmDelete,
    /// A box of the rwx grid (its bit).
    PermissionToggled(u32),
    PermissionOctal(String),
    ApplyPermissions,
    /// "Dismiss" on the error bar.
    DismissError,
    /// Upload (local pane) / Download (remote pane): the selection to the other pane's
    /// folder.
    Transfer,
    /// The pointer entered or left the pane (a drag drops on the pane under it).
    PointerIn,
    PointerOut,
    /// A right press on a row, or on the pane's empty area (`None`): its menu.
    ContextMenu(Option<usize>),
    /// F5 / "Refresh": lists the folder again.
    Refresh,
    /// "Copy path": the selected entries' full paths to the clipboard.
    CopyPath,
}

/// The keys the focused pane handles.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FileKey {
    Up,
    Down,
    Home,
    End,
    /// Enter the folder under the cursor.
    Enter,
    /// Backspace: up one folder.
    Back,
    /// Tab: focus the other pane.
    SwitchPane,
    /// F7.
    NewFolder,
    /// F2.
    Rename,
    /// Del.
    Delete,
    /// F4: edit the remote file in the editor.
    Edit,
    /// Space: toggles the differences panel's row (the panes ignore it).
    Space,
    /// Closes the inline editor, confirmation or dialog (or the error bar).
    Escape,
    /// Ctrl+Enter: the selection to the other pane's folder.
    Transfer,
    /// F5: list the folder again.
    Refresh,
    /// Shift+F10 or the Menu key: the focused pane's menu, at the cursor's row.
    Menu,
}

/// `path` as a [`RemotePath`], if it is absolute and POSIX-shaped (the local side lists
/// through `LocalFs::new("/")`).
pub fn local_path(path: &std::path::Path) -> Option<RemotePath> {
    RemotePath::new(path.to_str()?).ok()
}

/// The inverse of [`local_path`]: a local pane's folder as a filesystem path (what a
/// workspace saves as `local_dir`).
pub fn local_fs_path(path: &RemotePath) -> std::path::PathBuf {
    std::path::PathBuf::from(path.as_str())
}

/// A pane's selection by entry name (`None`: the `..` row), to find it again after the
/// rows are listed anew.
struct SelectedNames {
    rows: Vec<Option<String>>,
    anchor: Option<Option<String>>,
    cursor: Option<Option<String>>,
}

impl SelectedNames {
    /// The name at `row` (`Some(None)`: the `..` row; `None`: past the rows).
    fn name_of(pane: &Pane, row: usize) -> Option<Option<String>> {
        match pane.row(row)? {
            Row::Up => Some(None),
            Row::Entry(entry) => Some(Some(entry.name.clone())),
        }
    }

    /// The row of `name` (`None`: the `..` row) among the pane's rows.
    fn row_of(pane: &Pane, name: &Option<String>) -> Option<usize> {
        match name {
            None => pane.has_up().then_some(0),
            Some(name) => pane
                .entries
                .iter()
                .position(|e| &e.name == name)
                .map(|i| usize::from(pane.has_up()) + i),
        }
    }

    fn of(pane: &Pane) -> Self {
        let name = |row: usize| Self::name_of(pane, row);
        Self {
            rows: pane
                .selection
                .rows
                .iter()
                .filter_map(|&r| name(r))
                .collect(),
            anchor: pane.selection.anchor.and_then(name),
            cursor: pane.selection.cursor.and_then(name),
        }
    }

    /// The selection on `pane`'s new rows; names that are gone drop out.
    fn restore(self, pane: &Pane) -> Selection {
        if self.rows.is_empty() && self.anchor.is_none() && self.cursor.is_none() {
            return Selection::default();
        }
        let up = usize::from(pane.has_up());
        let at: std::collections::HashMap<&str, usize> = pane
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.name.as_str(), up + i))
            .collect();
        let row = |name: &Option<String>| match name {
            None => pane.has_up().then_some(0),
            Some(name) => at.get(name.as_str()).copied(),
        };
        Selection {
            rows: self.rows.iter().filter_map(row).collect(),
            anchor: self.anchor.as_ref().and_then(row),
            cursor: self.cursor.as_ref().and_then(row),
        }
    }
}

impl App {
    pub(super) fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Lists the pane's folder again, superseding any listing in flight. The remote side
    /// waits for its session. The list starts again from the top: the skeleton replaces
    /// it meanwhile, so the scrollable that comes back is at 0 too. A terminal tab lists
    /// nothing.
    pub(super) fn request_listing(&mut self, index: usize, side: Side) -> Vec<Effect> {
        if !self.tabs[index].is_files() {
            return Vec::new();
        }
        // The folders are about to be read again: a comparison of the old ones stops.
        let mut effects = self.compare_folders_changing(index);
        effects.extend(self.list_pane(index, side));
        effects
    }

    /// The listing effects of [`request_listing`](Self::request_listing).
    pub(super) fn list_pane(&mut self, index: usize, side: Side) -> Vec<Effect> {
        let request_id = RequestId(self.next_id());
        let session = self.tab_session(index);
        let tab = &mut self.tabs[index];
        let tab_id = tab.id;
        let Some(pane) = tab.pane_mut(side) else {
            return Vec::new();
        };
        // A full listing shows whatever a finished transfer brought.
        pane.relist = false;
        pane.soft = false;
        pane.soft_since = None;
        pane.offset_y = 0.0;
        let scroll = Effect::ScrollPane {
            tab: tab_id,
            side,
            offset_y: 0.0,
        };
        let session = match side {
            Side::Local => None,
            Side::Remote if session.is_none() => {
                pane.listing = Listing::Idle;
                pane.request = None;
                return vec![scroll];
            }
            Side::Remote => session,
        };
        pane.listing = Listing::Loading;
        pane.request = Some(request_id);
        vec![
            Effect::List {
                tab: tab_id,
                side,
                request_id,
                session,
                path: pane.path.clone(),
            },
            scroll,
        ]
    }

    /// Lists the pane's folder again in the background, after finished transfers: the
    /// rows stay on screen and the list where it is; [`listed`](Self::listed) keeps the
    /// selection (by name) and starts no comparison. The remote side needs its session.
    pub(super) fn list_pane_soft(&mut self, index: usize, side: Side) -> Vec<Effect> {
        let request_id = RequestId(self.next_id());
        let session = match side {
            Side::Local => None,
            Side::Remote => match self.tab_session(index) {
                Some(session) => Some(session),
                None => return Vec::new(),
            },
        };
        let tab = &mut self.tabs[index];
        let tab_id = tab.id;
        let Some(pane) = tab.pane_mut(side) else {
            return Vec::new();
        };
        pane.relist = false;
        pane.soft = true;
        pane.soft_since = Some(std::time::Instant::now());
        pane.request = Some(request_id);
        vec![Effect::List {
            tab: tab_id,
            side,
            request_id,
            session,
            path: pane.path.clone(),
        }]
    }

    /// Goes to `path`: the old entries go, with any editor or dialog about them, and the
    /// list scrolls to the top.
    pub(super) fn navigate(&mut self, index: usize, side: Side, path: RemotePath) -> Vec<Effect> {
        let Some(pane) = self.tabs[index].pane_mut(side) else {
            return Vec::new();
        };
        pane.path = path;
        pane.entries.clear();
        pane.selection = Selection::default();
        pane.hovered = None;
        pane.action = None;
        pane.action_error = None;
        pane.select_after = None;
        pane.restore = None;
        pane.note = None;
        pane.relist = false;
        self.request_listing(index, side)
    }

    /// Enters the folder of `row` (`..` goes up); files stay where they are.
    fn open_row(&mut self, index: usize, side: Side, row: usize) -> Vec<Effect> {
        let Some(pane) = self.tabs[index].pane(side) else {
            return Vec::new();
        };
        let target = match pane.row(row) {
            Some(Row::Up) => pane.path.parent(),
            // A link may point at a folder: try it; a failure shows in the pane.
            Some(Row::Entry(entry)) if entry.kind != EntryKind::File => {
                Some(pane.path.join(&entry.name))
            }
            Some(Row::Entry(_)) | None => None,
        };
        match target {
            Some(path) => self.navigate(index, side, path),
            None => Vec::new(),
        }
    }

    pub(super) fn pane_msg(&mut self, tab: TabId, side: Side, msg: PaneMsg) -> Vec<Effect> {
        let Some(index) = self.tab_index(tab).filter(|&i| self.tabs[i].is_files()) else {
            return Vec::new();
        };
        let mods = self.mods;
        // A press or action in a pane gives it the keyboard back from the differences panel.
        if !matches!(
            msg,
            PaneMsg::Scrolled { .. }
                | PaneMsg::Placed(_)
                | PaneMsg::PointerIn
                | PaneMsg::PointerOut
        ) && let Some(state) = self.tabs[index].compare_mut()
        {
            state.focused = false;
        }
        let Some(pane) = self.tabs[index].pane_mut(side) else {
            return Vec::new();
        };
        match msg {
            PaneMsg::Focus => {}
            PaneMsg::Click(row) => {
                if row < pane.row_count() {
                    pane.selection.click(row, mods);
                    // A press on a row may start a drag to the other pane (once the
                    // pointer moves far enough from the press).
                    let at = self.last_press;
                    self.drag = Some(Drag {
                        tab,
                        from: side,
                        origin: at,
                        at,
                        active: false,
                    });
                }
            }
            PaneMsg::Open(row) => {
                self.tabs[index].set_focus(side);
                return self.open_row(index, side, row);
            }
            PaneMsg::Navigate(path) => {
                self.tabs[index].set_focus(side);
                return self.navigate(index, side, path);
            }
            PaneMsg::Sort(key) => {
                let descending = pane.sort.key == key && !pane.sort.descending;
                pane.resort(Sort { key, descending });
            }
            PaneMsg::Scrolled {
                offset_y,
                viewport_h,
            } => {
                pane.offset_y = offset_y;
                pane.viewport_h = viewport_h;
                return Vec::new();
            }
            PaneMsg::Placed(bounds) => {
                pane.bounds = bounds;
                pane.viewport_h = bounds.height;
                return Vec::new();
            }
            PaneMsg::Hover(row) => {
                pane.hovered = Some(row);
                return Vec::new();
            }
            PaneMsg::Unhover(row) => {
                if pane.hovered == Some(row) {
                    pane.hovered = None;
                }
                return Vec::new();
            }
            PaneMsg::Retry => return self.request_listing(index, side),
            PaneMsg::NewFolder => {
                self.tabs[index].set_focus(side);
                return self.start_new_folder(index, side);
            }
            PaneMsg::Rename => {
                self.tabs[index].set_focus(side);
                return self.start_rename(index, side);
            }
            PaneMsg::Delete => {
                self.tabs[index].set_focus(side);
                return self.start_delete(index, side);
            }
            PaneMsg::Permissions => {
                self.tabs[index].set_focus(side);
                return self.start_permissions(index, side);
            }
            PaneMsg::Edit => {
                self.tabs[index].set_focus(side);
                return self.start_edit(index, side);
            }
            PaneMsg::EditChanged(value) => {
                if let Some(PaneAction::Edit(edit)) = &mut pane.action {
                    edit.value = value;
                    edit.error = None;
                }
                return Vec::new();
            }
            PaneMsg::EditCommit => return self.commit_edit(index, side),
            PaneMsg::CancelAction => self.cancel_action(index, side),
            PaneMsg::ConfirmDelete => return self.confirm_delete(index, side),
            PaneMsg::PermissionToggled(bit) => {
                if let Some(PaneAction::Permissions(dialog)) = &mut pane.action {
                    dialog.toggle(bit);
                }
                return Vec::new();
            }
            PaneMsg::PermissionOctal(text) => {
                if let Some(PaneAction::Permissions(dialog)) = &mut pane.action {
                    dialog.set_octal(text);
                }
                return Vec::new();
            }
            PaneMsg::ApplyPermissions => return self.apply_permissions(index, side),
            PaneMsg::DismissError => {
                pane.action_error = None;
                pane.note = None;
            }
            PaneMsg::Transfer => {
                self.tabs[index].set_focus(side);
                return self.transfer_selection(index, side);
            }
            PaneMsg::PointerIn | PaneMsg::PointerOut => {
                self.pointer(tab, side, matches!(msg, PaneMsg::PointerIn));
                return Vec::new();
            }
            // A row's `mouse_area` captures its right press, so the pane's own (`None`)
            // comes only from outside the rows.
            PaneMsg::ContextMenu(row) => {
                let at = self.last_press;
                self.open_pane_menu(index, side, row, at);
                return Vec::new();
            }
            PaneMsg::Refresh => {
                self.tabs[index].set_focus(side);
                return self.refresh(index, side);
            }
            PaneMsg::CopyPath => {
                self.tabs[index].set_focus(side);
                return self
                    .copy_paths(index, side)
                    .map(|text| vec![Effect::WriteClipboard(text.into())])
                    .unwrap_or_default();
            }
        }
        self.tabs[index].set_focus(side);
        Vec::new()
    }

    /// Whether F5 / "Refresh" would list pane `side` again: no operation runs in it (it
    /// lists the folder itself when done) and, remote, the site has a session (one lost
    /// to the network counts: the listing reconnects it).
    pub(super) fn can_refresh(&self, index: usize, side: Side) -> bool {
        self.tabs[index]
            .pane(side)
            .is_some_and(|p| p.busy.is_none())
            && (side == Side::Local || self.tab_conn(index).session().is_some())
    }

    /// F5 / "Refresh": the folder again, when [`can_refresh`](Self::can_refresh).
    fn refresh(&mut self, index: usize, side: Side) -> Vec<Effect> {
        if !self.can_refresh(index, side) {
            return Vec::new();
        }
        self.request_listing(index, side)
    }

    /// The menu of pane `side` (of the tab at `index`) on `row` (`None`: its empty area),
    /// at `at`. A row that is not selected is selected alone first; a selected one keeps
    /// the selection, so the menu acts on all of it. The pane gets the keyboard.
    fn open_pane_menu(&mut self, index: usize, side: Side, row: Option<usize>, at: Point) {
        let tab = &mut self.tabs[index];
        let tab_id = tab.id;
        let Some(pane) = tab.pane_mut(side) else {
            return;
        };
        let row = row.filter(|&r| r < pane.row_count());
        if let Some(row) = row {
            if pane.selection.contains(row) {
                // Keep the selection; the clicked row becomes the cursor, so an action
                // on one entry (Rename) acts on the row under the pointer.
                pane.selection.cursor = Some(row);
            } else {
                pane.selection.click(row, Mods::default());
            }
        }
        tab.set_focus(side);
        self.menu = Some(ContextMenu {
            at,
            target: MenuTarget::Pane {
                tab: tab_id,
                side,
                row,
            },
            cursor: None,
        });
    }

    /// What "Copy path" copies from pane `side` of the tab at `index`: the selected
    /// entries' full paths, one per line, in row order (local: the system's path, see
    /// [`local_os_path`]; remote: the remote one). `None` with nothing selected.
    pub fn copy_paths(&self, index: usize, side: Side) -> Option<String> {
        let pane = self.tabs.get(index)?.pane(side)?;
        let paths: Vec<String> = pane
            .selected_entries()
            .iter()
            .map(|entry| {
                let path = pane.path.join(&entry.name);
                match side {
                    Side::Local => local_os_path(&path).display().to_string(),
                    Side::Remote => path.as_str().to_owned(),
                }
            })
            .collect();
        (!paths.is_empty()).then(|| paths.join("\n"))
    }

    /// A listing finished. Applied only if it is the pane's latest request.
    pub(super) fn listed(
        &mut self,
        tab: TabId,
        side: Side,
        request_id: RequestId,
        result: Result<Vec<RemoteEntry>, Error>,
    ) -> Vec<Effect> {
        let Some(index) = self.tab_index(tab) else {
            return Vec::new();
        };
        let quiet = self.tabs[index]
            .compare()
            .is_some_and(|s| s.quiet_request == Some(request_id));
        if quiet && let Some(state) = self.tabs[index].compare_mut() {
            state.quiet_request = None;
        }
        let Some(pane) = self.tabs[index].pane_mut(side) else {
            return Vec::new();
        };
        if pane.request != Some(request_id) {
            // Superseded (or never ours): the pane shows another folder now.
            return Vec::new();
        }
        pane.request = None;
        let soft = std::mem::take(&mut pane.soft);
        if let Some(since) = pane.soft_since.take() {
            pane.soft_took = since.elapsed();
        }
        if soft {
            return self.soft_listed(index, side, result);
        }
        pane.selection = Selection::default();
        pane.hovered = None;
        // The rows a drag from this pane carried are gone.
        if self.drag.is_some_and(|d| d.tab == tab && d.from == side) {
            self.drag = None;
        }
        // The rows a menu on this pane was about are gone.
        if self.pane_menu_row(tab, side).is_some() {
            self.menu = None;
        }
        let Some(pane) = self.tabs[index].pane_mut(side) else {
            return Vec::new();
        };
        if let Err(error) = &result
            && error.kind == ErrorKind::NotFound
            && let Some(restore) = pane.restore.take()
        {
            return self.restore_fallback(index, side, restore);
        }
        pane.restore = None;
        let listed = result.is_ok();
        match result {
            Ok(mut entries) => {
                // FTP listings may carry the folder's own "." and "..".
                entries.retain(|e| {
                    !matches!(e.name.as_str(), "" | "." | "..") && !e.name.contains('/')
                });
                sort_entries(&mut entries, pane.sort);
                pane.entries = entries;
                pane.listing = Listing::Loaded;
            }
            Err(error) => {
                pane.entries.clear();
                pane.listing = Listing::Failed(error);
                pane.select_after = None;
            }
        }
        if listed {
            let mut effects = self.select_new_entry(index, side);
            if !quiet {
                effects.extend(self.compare_after_listing(index));
            }
            effects
        } else {
            self.failed_listing_ends_suppression(index);
            Vec::new()
        }
    }

    /// A failed listing never reaches `compare_after_listing`: once nothing is pending any
    /// more, an Open's suppression ends here.
    fn failed_listing_ends_suppression(&mut self, index: usize) {
        let idle = [Side::Local, Side::Remote].iter().all(|s| {
            self.tabs[index]
                .pane(*s)
                .is_none_or(|p| p.request.is_none())
        });
        if idle && let Some(state) = self.tabs[index].compare_mut() {
            state.suppress_auto = false;
        }
    }

    /// The row of the menu open on pane `side` of `tab` (`Some(None)`: its empty area), if
    /// one is open there.
    fn pane_menu_row(&self, tab: TabId, side: Side) -> Option<Option<usize>> {
        match &self.menu {
            Some(ContextMenu {
                target:
                    MenuTarget::Pane {
                        tab: t,
                        side: s,
                        row,
                    },
                ..
            }) if *t == tab && *s == side => Some(*row),
            _ => None,
        }
    }

    /// A background re-list after transfers ([`list_pane_soft`](Self::list_pane_soft))
    /// came back. The user did not ask for it, so it disturbs nothing: the selection (which
    /// a drag from the pane carries) and an open menu follow their entries by name (a menu
    /// whose entry is gone closes), the hover stays on the row under the pointer, and no
    /// comparison starts. A failure leaves the rows as they were and goes to the technical
    /// log, unless the folder itself is gone.
    fn soft_listed(
        &mut self,
        index: usize,
        side: Side,
        result: Result<Vec<RemoteEntry>, Error>,
    ) -> Vec<Effect> {
        let tab = self.tabs[index].id;
        let menu_row = self.pane_menu_row(tab, side);
        let Some(pane) = self.tabs[index].pane_mut(side) else {
            return Vec::new();
        };
        let mut entries = match result {
            Ok(entries) => entries,
            Err(error) if error.kind != ErrorKind::NotFound => {
                tracing::warn!(
                    folder = %pane.path,
                    %error,
                    "background re-list after a transfer failed; the rows shown are kept"
                );
                return Vec::new();
            }
            Err(error) => {
                pane.entries.clear();
                pane.selection = Selection::default();
                pane.hovered = None;
                pane.listing = Listing::Failed(error);
                pane.select_after = None;
                if menu_row.is_some() {
                    self.menu = None;
                }
                if self.drag.is_some_and(|d| d.tab == tab && d.from == side) {
                    self.drag = None;
                }
                self.failed_listing_ends_suppression(index);
                return Vec::new();
            }
        };
        let kept = SelectedNames::of(pane);
        let menu_name = menu_row
            .flatten()
            .map(|row| SelectedNames::name_of(pane, row));
        // FTP listings may carry the folder's own "." and "..".
        entries.retain(|e| !matches!(e.name.as_str(), "" | "." | "..") && !e.name.contains('/'));
        sort_entries(&mut entries, pane.sort);
        pane.entries = entries;
        pane.listing = Listing::Loaded;
        pane.selection = kept.restore(pane);
        // The pointer has not moved: the hover stays on its row, if the list still has it.
        pane.hovered = pane.hovered.filter(|row| *row < pane.row_count());
        if let Some(name) = menu_name {
            let row = name.and_then(|name| SelectedNames::row_of(pane, &name));
            match (row, &mut self.menu) {
                (
                    Some(row),
                    Some(ContextMenu {
                        target: MenuTarget::Pane { row: at, .. },
                        ..
                    }),
                ) => *at = Some(row),
                _ => self.menu = None,
            }
        }
        self.select_new_entry(index, side)
    }

    /// A restored folder is gone: the pane goes to its parent, and if that is gone too,
    /// to home (local) or `/`, with a note naming the saved folder. Past that, a failure
    /// shows as usual.
    fn restore_fallback(&mut self, index: usize, side: Side, restore: Restore) -> Vec<Effect> {
        let Some(path) = self.tabs[index].pane(side).map(|pane| pane.path.clone()) else {
            return Vec::new();
        };
        let (saved, target, next) = match restore {
            Restore::Saved => match path.parent() {
                Some(parent) => (path.clone(), parent, Some(Restore::Parent { saved: path })),
                None => (path, self.fallback_dir(side), None),
            },
            Restore::Parent { saved } => (saved, self.fallback_dir(side), None),
        };
        let effects = self.navigate(index, side, target);
        if let Some(pane) = self.tabs[index].pane_mut(side) {
            pane.restore = next;
            pane.note = Some(crate::ui::strings::saved_folder_gone(saved.as_str()));
        }
        effects
    }

    /// Where a pane goes when nothing better is known: home on the local side, else `/`.
    fn fallback_dir(&self, side: Side) -> RemotePath {
        match side {
            Side::Local => self.home.clone().unwrap_or_else(RemotePath::root),
            Side::Remote => RemotePath::root(),
        }
    }

    /// A key from the subscription. Escape goes straight to the pane (a text field that
    /// had it already let go of the focus); the others first make sure no text field has
    /// the focus. With the pane's editor or dialog open, only Enter may matter.
    pub(super) fn key_pressed(&mut self, key: FileKey, shift: bool) -> Vec<Effect> {
        if self.dialogs.is_open() {
            return self.dialog_key(key, shift);
        }
        // The palette has the keyboard: nothing reaches a pane or a form behind it.
        if self.palette.is_some() {
            return self.palette_key(key);
        }
        if self.form.is_some() {
            return Vec::new();
        }
        // Escape in a terminal tab closes its search bar.
        if key == FileKey::Escape && self.terminal_escape() {
            return Vec::new();
        }
        let Some(pane) = self
            .active_tab
            .and_then(|i| self.tabs.get(i))
            .and_then(|t| t.pane(t.focus()?))
        else {
            return Vec::new();
        };
        if key == FileKey::Escape {
            return self.file_key(key, shift);
        }
        let typing = matches!(
            pane.action,
            Some(PaneAction::Edit(_) | PaneAction::Permissions(_))
        );
        if typing && key != FileKey::Enter {
            return Vec::new();
        }
        vec![Effect::KeyUnlessTyping { key, shift }]
    }

    /// A key for the active tab's focused pane (ignored while a form is open, or in a
    /// terminal tab).
    pub(super) fn file_key(&mut self, key: FileKey, shift: bool) -> Vec<Effect> {
        if self.form.is_some() || self.dialogs.is_open() {
            return Vec::new();
        }
        let Some(index) = self
            .active_tab
            .filter(|&i| self.tabs.get(i).is_some_and(|t| t.is_files()))
        else {
            return Vec::new();
        };
        if let Some(effects) = self.compare_key(index, key) {
            return effects;
        }
        let tab = &mut self.tabs[index];
        let Some(files) = tab.files_mut() else {
            return Vec::new();
        };
        let side = files.focus;
        let pane = match side {
            Side::Local => &mut files.local,
            Side::Remote => &mut files.remote,
        };
        // An editor or dialog is open: only Escape (cancel) and Enter (commit a name) act;
        // every other key belongs to it.
        match (&pane.action, key) {
            (Some(_), FileKey::Escape) => {
                self.cancel_action(index, side);
                return Vec::new();
            }
            (Some(PaneAction::Edit(_)), FileKey::Enter) => return self.commit_edit(index, side),
            (Some(_), _) => return Vec::new(),
            (None, FileKey::Escape) => {
                pane.action_error = None;
                return Vec::new();
            }
            (None, FileKey::NewFolder) => return self.start_new_folder(index, side),
            (None, FileKey::Rename) => return self.start_rename(index, side),
            (None, FileKey::Delete) => return self.start_delete(index, side),
            (None, FileKey::Edit) => return self.start_edit(index, side),
            (None, FileKey::Transfer) => return self.transfer_selection(index, side),
            (None, FileKey::Refresh) => return self.refresh(index, side),
            (None, FileKey::Menu) => {
                // At the cursor's row (the first one without a cursor).
                let row = pane.selection.cursor;
                let at = pane.menu_anchor(row.unwrap_or(0));
                self.open_pane_menu(index, side, row, at);
                return Vec::new();
            }
            (None, _) => {}
        }
        if key == FileKey::SwitchPane {
            files.focus = match side {
                Side::Local => Side::Remote,
                Side::Remote => Side::Local,
            };
            return Vec::new();
        }
        if !matches!(
            self.pane_status(index, side),
            PaneStatus::Full | PaneStatus::Empty
        ) {
            return Vec::new();
        }
        let tab = &mut self.tabs[index];
        let tab_id = tab.id;
        let Some(pane) = tab.pane_mut(side) else {
            return Vec::new();
        };
        let count = pane.row_count();
        if count == 0 {
            return Vec::new();
        }
        let cursor = pane.selection.cursor;
        let target = match key {
            FileKey::Up => cursor.map_or(0, |c| c.saturating_sub(1)),
            FileKey::Down => cursor.map_or(0, |c| (c + 1).min(count - 1)),
            FileKey::Home => 0,
            FileKey::End => count - 1,
            FileKey::Enter => {
                return match cursor {
                    Some(row) => self.open_row(index, side, row),
                    None => Vec::new(),
                };
            }
            FileKey::Back => {
                return match pane.path.parent() {
                    Some(parent) => self.navigate(index, side, parent),
                    None => Vec::new(),
                };
            }
            // The differences panel's key; the panes have none for it.
            FileKey::Space => return Vec::new(),
            FileKey::SwitchPane
            | FileKey::NewFolder
            | FileKey::Rename
            | FileKey::Delete
            | FileKey::Edit
            | FileKey::Transfer
            | FileKey::Refresh
            | FileKey::Menu
            | FileKey::Escape => unreachable!("handled above"),
        };
        pane.selection.move_to(target, shift);
        match pane.scroll_to_show(target) {
            Some(offset_y) => {
                pane.offset_y = offset_y;
                vec![Effect::ScrollPane {
                    tab: tab_id,
                    side,
                    offset_y,
                }]
            }
            None => Vec::new(),
        }
    }

    /// The scroll offsets of the tab now in the center, so its lists show where their
    /// state says (the scrollable widgets are shared between tabs and forms).
    pub(super) fn sync_scroll(&self) -> Vec<Effect> {
        let Some((tab, files)) = self
            .active_tab
            .and_then(|i| self.tabs.get(i))
            .filter(|_| self.form.is_none())
            .and_then(|t| Some((t.id, t.files()?)))
        else {
            return Vec::new();
        };
        [(Side::Local, &files.local), (Side::Remote, &files.remote)]
            .into_iter()
            .map(|(side, pane)| Effect::ScrollPane {
                tab,
                side,
                offset_y: pane.offset_y,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;

    fn entry(name: &str, kind: EntryKind, size: u64, modified: u64) -> RemoteEntry {
        RemoteEntry {
            name: name.into(),
            path: RemotePath::root().join(name),
            kind,
            size,
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(modified)),
            mode: None,
        }
    }

    fn names(entries: &[RemoteEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn sort_folders_first() {
        use EntryKind::{Dir, File, Symlink};
        let mut entries = vec![
            entry("b.txt", File, 300, 3),
            entry("Zeta", Dir, 0, 1),
            entry("a.txt", File, 100, 5),
            entry("alpha", Dir, 0, 9),
            entry("B.txt", File, 200, 4),
            entry("link", Symlink, 10, 2),
            entry("Beta", Dir, 0, 7),
        ];
        let by = |key, descending| Sort { key, descending };

        // Name: folders first, names without case (ties broken exactly: "B.txt" < "b.txt").
        sort_entries(&mut entries, Sort::default());
        assert_eq!(
            names(&entries),
            ["alpha", "Beta", "Zeta", "a.txt", "B.txt", "b.txt", "link"]
        );
        // Descending flips each group; folders stay first.
        sort_entries(&mut entries, by(SortKey::Name, true));
        assert_eq!(
            names(&entries),
            ["Zeta", "Beta", "alpha", "link", "b.txt", "B.txt", "a.txt"]
        );
        // Size: files by size; folders (no meaningful size) by name.
        sort_entries(&mut entries, by(SortKey::Size, false));
        assert_eq!(
            names(&entries),
            ["alpha", "Beta", "Zeta", "link", "a.txt", "B.txt", "b.txt"]
        );
        // Modified, newest first.
        sort_entries(&mut entries, by(SortKey::Modified, true));
        assert_eq!(
            names(&entries),
            ["alpha", "Beta", "Zeta", "a.txt", "B.txt", "b.txt", "link"]
        );

        // A header click re-sorts once and keeps the selection on the same entries.
        let mut pane = Pane::new(RemotePath::new("/srv").unwrap());
        pane.entries = entries.clone();
        pane.listing = Listing::Loaded;
        pane.resort(Sort::default());
        // Rows: 0 "..", 1 alpha, 2 Beta, 3 Zeta, 4 a.txt, 5 B.txt, 6 b.txt, 7 link.
        pane.selection.click(4, Mods::default());
        pane.selection.click(
            7,
            Mods {
                ctrl: true,
                shift: false,
            },
        );
        pane.resort(by(SortKey::Name, true));
        // Rows: 0 "..", 1 Zeta, 2 Beta, 3 alpha, 4 link, 5 b.txt, 6 B.txt, 7 a.txt.
        assert_eq!(pane.selection.rows, BTreeSet::from([4, 7]));
        assert_eq!(pane.selection.cursor, Some(4), "the cursor follows link");
        assert_eq!(pane.row(7), Some(Row::Entry(&pane.entries[6])));
        assert_eq!(pane.row(0), Some(Row::Up));
        assert_eq!(pane.row(8), None);
    }

    #[test]
    fn selection_shift_ctrl() {
        let plain = Mods::default();
        let ctrl = Mods {
            ctrl: true,
            shift: false,
        };
        let shift = Mods {
            ctrl: false,
            shift: true,
        };
        let both = Mods {
            ctrl: true,
            shift: true,
        };
        let rows = |s: &Selection| s.rows.iter().copied().collect::<Vec<_>>();
        let mut s = Selection::default();

        // A click selects one row and anchors there.
        s.click(3, plain);
        assert_eq!(rows(&s), [3]);
        assert_eq!((s.anchor, s.cursor), (Some(3), Some(3)));
        // Shift+click: the range from the anchor, either direction.
        s.click(6, shift);
        assert_eq!(rows(&s), [3, 4, 5, 6]);
        s.click(1, shift);
        assert_eq!(
            rows(&s),
            [1, 2, 3],
            "the range is replaced, the anchor kept"
        );
        assert_eq!((s.anchor, s.cursor), (Some(3), Some(1)));
        // Ctrl+click toggles one row and moves the anchor to it.
        s.click(8, ctrl);
        assert_eq!(rows(&s), [1, 2, 3, 8]);
        s.click(2, ctrl);
        assert_eq!(rows(&s), [1, 3, 8]);
        assert_eq!(s.anchor, Some(2));
        // Ctrl+Shift+click adds a range to what is selected.
        s.click(10, both);
        assert_eq!(rows(&s), (1..=10).collect::<Vec<_>>());
        assert_eq!((s.anchor, s.cursor), (Some(2), Some(10)));
        // A plain click starts over.
        s.click(5, plain);
        assert_eq!(rows(&s), [5]);
        // Shift+click with no anchor yet selects just that row.
        let mut fresh = Selection::default();
        fresh.click(4, shift);
        assert_eq!(rows(&fresh), [4]);
        assert_eq!(fresh.anchor, Some(4));

        // Arrows: alone, or extending from the anchor with Shift.
        s.move_to(6, false);
        assert_eq!(rows(&s), [6]);
        assert_eq!((s.anchor, s.cursor), (Some(6), Some(6)));
        s.move_to(7, true);
        s.move_to(8, true);
        assert_eq!(rows(&s), [6, 7, 8]);
        s.move_to(4, true);
        assert_eq!(rows(&s), [4, 5, 6]);
        assert_eq!((s.anchor, s.cursor), (Some(6), Some(4)));
        assert!(s.contains(5) && !s.contains(7));
    }
}
