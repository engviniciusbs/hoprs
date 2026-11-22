//! Task 8: transfers from the panes (buttons, Ctrl+Enter, drag, system drop) and the queue
//! panel driven by the engine's events (Review Focus 2: lag and stale events).

use std::path::PathBuf;

use super::*;

use hoprs_core::Error;
use hoprs_transfer::{
    ConflictInfo, ConflictPolicy, Direction, Event, EventKind, Origin, ProgressUpdate, Resolution,
    ResolveAction, TransferId, TransferItem, TransferSpec, TransferState,
};

use hoprs_core::ConnectionState;
use iced::Point;

use crate::app::queue::{QueueRow, RowStatus};
use crate::app::transfers::other;
use crate::backend::{ControlOp, TransferSnapshot};

fn spec(site: &SiteId, direction: Direction, local: &str, remote: &str) -> TransferSpec {
    TransferSpec {
        site: site.clone(),
        direction,
        local: rpath(local),
        remote: rpath(remote),
    }
}

pub(super) fn item(id: i64, site: &SiteId, state: TransferState) -> TransferItem {
    TransferItem {
        id: TransferId(id),
        parent: None,
        spec: spec(
            site,
            Direction::Upload,
            &format!("/home/me/f{id}"),
            &format!("/srv/f{id}"),
        ),
        is_dir: Some(false),
        size: Some(1000),
        bytes_done: 0,
        state,
        origin: Origin::User,
        tree_policy: None,
    }
}

fn event(seq: u64, kind: EventKind) -> Event {
    Event { seq, kind }
}

fn changed(seq: u64, id: i64, state: TransferState) -> Event {
    event(
        seq,
        EventKind::StateChanged {
            id: TransferId(id),
            state,
        },
    )
}

fn snapshot(seq: u64, items: Vec<TransferItem>) -> TransferSnapshot {
    TransferSnapshot {
        seq,
        items,
        workers: 4,
    }
}

pub(super) fn failed(message: &str) -> TransferState {
    TransferState::Failed(Error::new(ErrorKind::Permission, message))
}

pub(super) fn conflict() -> TransferState {
    TransferState::AwaitingDecision(ConflictInfo {
        src_size: 10,
        dst_size: 4,
        src_modified: None,
        dst_modified: None,
    })
}

/// The single snapshot request among `effects`.
fn snapshot_request(effects: &[Effect]) -> RequestId {
    match effects {
        [Effect::SnapshotQueue(request)] => *request,
        other => panic!("expected one SnapshotQueue, got {other:?}"),
    }
}

/// Starts the queue the way the subscription does (a resync), on `items` at `seq`.
pub(super) fn synced(app: &mut App, seq: u64, items: Vec<TransferItem>) {
    let request = snapshot_request(&app.update(Message::QueueResync));
    assert!(
        app.update(Message::QueueSnapshot {
            request,
            snapshot: snapshot(seq, items),
        })
        .is_empty()
    );
    assert!(!app.queue.resyncing());
}

fn state_of(app: &App, id: i64) -> &TransferState {
    &app.queue
        .row(TransferId(id))
        .expect("the row exists")
        .item
        .state
}

fn ids(rows: &[&QueueRow]) -> Vec<i64> {
    rows.iter().map(|r| r.item.id.0).collect()
}

/// The specs of the single `Enqueue` among `effects`.
fn enqueued(effects: &[Effect]) -> Vec<TransferSpec> {
    let specs: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Enqueue(specs) => Some(specs.clone()),
            _ => None,
        })
        .collect();
    match <[Vec<TransferSpec>; 1]>::try_from(specs) {
        Ok([specs]) => specs,
        Err(all) => panic!("expected one Enqueue, got {all:?}"),
    }
}

fn no_enqueue(effects: &[Effect]) -> bool {
    !effects.iter().any(|e| matches!(e, Effect::Enqueue(_)))
}

/// Runs the queue effects against `fake` as `effects::run` does and returns their
/// messages.
async fn run_queue(fake: &Arc<FakeBackend>, effects: Vec<Effect>) -> Vec<Message> {
    let backend: Arc<dyn BackendApi> = fake.clone();
    let mut messages = Vec::new();
    for effect in effects {
        messages.push(match effect {
            Effect::Enqueue(specs) => {
                Message::Enqueued(effects::enqueue_all(backend.clone(), specs).await)
            }
            Effect::Control { id, op } => Message::ControlDone {
                id,
                result: backend.control(id, op).await,
            },
            Effect::SnapshotQueue(request) => Message::QueueSnapshot {
                request,
                snapshot: backend.transfer_snapshot().await,
            },
            Effect::ClearFinished => Message::Cleared(backend.clear_finished().await),
            other => panic!("not a queue effect: {other:?}"),
        });
    }
    messages
}

/// Upload / Download send the selection to the other pane's folder, folders as they are;
/// Ctrl+Enter does the same from the focused pane. The backend gets each spec.
#[tokio::test]
async fn enqueue_selection_to_other_pane() {
    let (fake, mut app, a, tab) = transfer_tab().await;

    // Nothing selected: the button does nothing.
    assert!(!app.tabs[0].can_transfer(app.tab_conn(0), Side::Local));
    assert!(
        app.update(pane_msg(tab, Side::Local, PaneMsg::Transfer))
            .is_empty()
    );

    // a.txt and dir/ (Ctrl+click), then Upload: into the remote folder.
    let a_row = row_of(&app, Side::Local, "a.txt");
    let dir_row = row_of(&app, Side::Local, "dir");
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(a_row)));
    app.update(Message::Modifiers(Mods {
        ctrl: true,
        shift: false,
    }));
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(dir_row)));
    app.update(Message::Modifiers(Mods::default()));
    assert!(app.tabs[0].can_transfer(app.tab_conn(0), Side::Local));
    let effects = app.update(pane_msg(tab, Side::Local, PaneMsg::Transfer));
    let specs = enqueued(&effects);
    assert_eq!(
        specs,
        vec![
            spec(&a.id, Direction::Upload, "/home/me/dir", "/srv/dir"),
            spec(&a.id, Direction::Upload, "/home/me/a.txt", "/srv/a.txt"),
        ]
    );
    for message in run_queue(&fake, effects).await {
        assert!(app.update(message).is_empty());
    }
    assert_eq!(fake.enqueued(), specs);
    assert!(app.queue.error.is_none());

    // The remote pane: r.txt, Ctrl+Enter: a download into the local folder.
    app.update(Message::Key {
        key: FileKey::SwitchPane,
        shift: false,
    });
    let r_row = row_of(&app, Side::Remote, "r.txt");
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(r_row)));
    let effects = app.update(Message::Key {
        key: FileKey::Transfer,
        shift: false,
    });
    assert_eq!(
        enqueued(&effects),
        vec![spec(
            &a.id,
            Direction::Download,
            "/home/me/r.txt",
            "/srv/r.txt"
        )]
    );

    // The `..` row alone is no selection.
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(0)));
    assert!(
        app.update(pane_msg(tab, Side::Remote, PaneMsg::Transfer))
            .is_empty()
    );

    // A tab whose remote side is not connected sends nothing.
    app.conns.get_mut(&app.tabs[0].site).unwrap().state = TabState::Connecting;
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(a_row)));
    assert!(!app.tabs[0].can_transfer(app.tab_conn(0), Side::Local));
    assert!(
        app.update(pane_msg(tab, Side::Local, PaneMsg::Transfer))
            .is_empty()
    );
}

/// A file dropped from the system file manager goes to the remote folder of the tab in
/// view; with no connected tab in view the queue says why, inline.
#[tokio::test]
async fn drop_from_os_targets_remote_dir() {
    let fake = Arc::new(FakeBackend::new());

    // No tab yet. The queue opens to show why.
    let mut app = loaded(&fake).await;
    app.update(Message::ToggleQueue);
    assert!(!app.queue_open);
    let effects = app.update(Message::FileDropped(PathBuf::from("/tmp/x/photo.jpg")));
    assert!(effects.is_empty());
    assert_eq!(
        app.queue.error.as_ref().map(|e| e.message.as_str()),
        Some("Open a connected site to drop files into it")
    );
    assert!(app.queue_open);
    app.update(Message::Queue(QueueMsg::DismissError));
    assert!(app.queue.error.is_none());

    let (fake, mut app, a, tab) = transfer_tab().await;
    assert!(app.update(Message::FilesHovered(true)).is_empty());
    assert!(app.os_drag);
    let effects = app.update(Message::FileDropped(PathBuf::from(
        "/home/me/Downloads/photo.jpg",
    )));
    assert!(!app.os_drag);
    assert_eq!(
        enqueued(&effects),
        vec![spec(
            &a.id,
            Direction::Upload,
            "/home/me/Downloads/photo.jpg",
            "/srv/photo.jpg"
        )]
    );

    // The remote pane moved to /srv/sub: drops follow it. A folder goes as it is.
    let effects = app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::Navigate(rpath("/srv/sub")),
    ));
    settle(&mut app, &fake, run_listings(&fake, &effects).await).await;
    let effects = app.update(Message::FileDropped(PathBuf::from("/home/me/dir")));
    assert_eq!(
        enqueued(&effects),
        vec![spec(
            &a.id,
            Direction::Upload,
            "/home/me/dir",
            "/srv/sub/dir"
        )]
    );

    // The remote folder is busy (still listing): a different message than "no site".
    let effects = app.update(pane_msg(tab, Side::Remote, PaneMsg::Retry));
    assert!(!effects.is_empty(), "a listing is in flight");
    let effects = app.update(Message::FileDropped(PathBuf::from("/home/me/a.txt")));
    assert!(no_enqueue(&effects));
    assert_eq!(
        app.queue.error.as_ref().map(|e| e.message.as_str()),
        Some("The remote folder is busy or not listed yet; drop again in a moment")
    );
    app.update(Message::Queue(QueueMsg::DismissError));

    // A form in the center: no tab in view.
    app.update(Message::NewSite);
    let effects = app.update(Message::FileDropped(PathBuf::from("/home/me/a.txt")));
    assert!(no_enqueue(&effects));
    assert!(app.queue.error.is_some());
}

/// Where the local pane and the remote pane are, for the pointer.
const IN_LOCAL: Point = Point::new(100.0, 200.0);
const IN_REMOTE: Point = Point::new(600.0, 200.0);

/// A press on `row` of `side` at `at`: the root tracker publishes the position, then the
/// row takes the click.
fn press(app: &mut App, tab: TabId, side: Side, row: usize, at: Point) {
    app.update(Message::PointerPressed(at));
    app.update(pane_msg(tab, side, PaneMsg::Click(row)));
}

/// The pointer moves to `at` and is over `side`'s pane.
fn move_over(app: &mut App, tab: TabId, side: Side, at: Point) {
    app.update(Message::PointerMoved(at));
    app.update(pane_msg(tab, side, PaneMsg::PointerIn));
    app.update(pane_msg(tab, other(side), PaneMsg::PointerOut));
}

/// A press on a row, a move of more than 4 px, then the button up over the other pane
/// enqueues the selection there; released over its own pane (a plain click) or outside
/// the panes, nothing.
#[tokio::test]
async fn drag_between_panes_enqueues() {
    let (_fake, mut app, a, tab) = transfer_tab().await;
    let b_row = row_of(&app, Side::Local, "b.txt");

    // A plain click: press and release in the same pane.
    app.update(pane_msg(tab, Side::Local, PaneMsg::PointerIn));
    press(&mut app, tab, Side::Local, b_row, IN_LOCAL);
    assert_eq!(
        app.drag,
        Some(Drag {
            tab,
            from: Side::Local,
            origin: IN_LOCAL,
            at: IN_LOCAL,
            active: false
        })
    );
    assert!(app.tracks_pointer(), "a possible drag follows the pointer");
    assert_eq!(app.drop_target(), None);
    assert!(no_enqueue(&app.update(Message::DragEnd)));
    assert_eq!(app.drag, None);
    assert!(!app.tracks_pointer());

    // Press on b.txt, move to the remote pane (the enter may come before the exit), up.
    press(&mut app, tab, Side::Local, b_row, IN_LOCAL);
    move_over(&mut app, tab, Side::Remote, IN_REMOTE);
    assert!(app.drag.is_some_and(|d| d.active && d.at == IN_REMOTE));
    assert_eq!(
        app.drop_target(),
        Some((tab, Side::Remote)),
        "the remote pane shows it is the target"
    );
    let effects = app.update(Message::DragEnd);
    assert_eq!(
        enqueued(&effects),
        vec![spec(
            &a.id,
            Direction::Upload,
            "/home/me/b.txt",
            "/srv/b.txt"
        )]
    );
    assert_eq!((app.drag, app.drop_target()), (None, None));
    // A second release (no drag) does nothing.
    assert!(app.update(Message::DragEnd).is_empty());

    // From the remote pane to the local one: a download.
    let r_row = row_of(&app, Side::Remote, "r.txt");
    press(&mut app, tab, Side::Remote, r_row, IN_REMOTE);
    move_over(&mut app, tab, Side::Local, IN_LOCAL);
    assert_eq!(
        enqueued(&app.update(Message::DragEnd)),
        vec![spec(
            &a.id,
            Direction::Download,
            "/home/me/r.txt",
            "/srv/r.txt"
        )]
    );

    // The pointer leaves the window mid-drag (its release may never come back): the drag
    // ends, and a later release elsewhere drops nothing.
    press(&mut app, tab, Side::Local, b_row, IN_LOCAL);
    move_over(&mut app, tab, Side::Remote, IN_REMOTE);
    assert!(app.update(Message::DragCancel).is_empty());
    assert_eq!((app.drag, app.pointer), (None, None));
    app.update(pane_msg(tab, Side::Remote, PaneMsg::PointerIn));
    assert!(no_enqueue(&app.update(Message::DragEnd)));
    app.update(pane_msg(tab, Side::Remote, PaneMsg::PointerOut));

    // The window events behind it: any button up ends a drag, leaving the window cancels.
    use iced::{Event as IcedEvent, mouse};
    let id = iced::window::Id::unique();
    let status = iced::event::Status::Ignored;
    let map = |e| crate::ui::queue::pointer_message(e, status, id);
    assert!(matches!(
        map(IcedEvent::Mouse(mouse::Event::ButtonReleased(
            mouse::Button::Right
        ))),
        Some(Message::DragEnd)
    ));
    assert!(matches!(
        map(IcedEvent::Mouse(mouse::Event::ButtonReleased(
            mouse::Button::Left
        ))),
        Some(Message::DragEnd)
    ));
    assert!(matches!(
        map(IcedEvent::Mouse(mouse::Event::CursorLeft)),
        Some(Message::DragCancel)
    ));

    // Released outside both panes (over the queue): nothing.
    press(&mut app, tab, Side::Local, b_row, IN_LOCAL);
    app.update(Message::PointerMoved(Point::new(300.0, 700.0)));
    app.update(pane_msg(tab, Side::Local, PaneMsg::PointerOut));
    assert_eq!(app.pointer, None);
    assert!(no_enqueue(&app.update(Message::DragEnd)));
}

/// A press followed by a move of 4 px or less stays a click: released over the other
/// pane, it drops nothing.
#[tokio::test]
async fn press_without_4px_move_is_a_click() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    let b_row = row_of(&app, Side::Local, "b.txt");
    press(&mut app, tab, Side::Local, b_row, IN_LOCAL);
    app.update(Message::PointerMoved(Point::new(
        IN_LOCAL.x + 3.0,
        IN_LOCAL.y,
    )));
    app.update(pane_msg(tab, Side::Remote, PaneMsg::PointerIn));
    assert!(app.drag.is_some_and(|d| !d.active));
    assert_eq!(app.drop_target(), None);
    assert_eq!(
        (app.drop_label(), app.drag_invalid_over(tab, Side::Remote)),
        (None, false)
    );
    assert!(no_enqueue(&app.update(Message::DragEnd)));
    // Exactly 4 px is not more than the threshold either; 4.5 px is.
    press(&mut app, tab, Side::Local, b_row, IN_LOCAL);
    app.update(Message::PointerMoved(Point::new(
        IN_LOCAL.x,
        IN_LOCAL.y + DRAG_THRESHOLD,
    )));
    assert!(app.drag.is_some_and(|d| !d.active));
    app.update(Message::PointerMoved(Point::new(
        IN_LOCAL.x + 3.0,
        IN_LOCAL.y + 3.4,
    )));
    assert!(app.drag.is_some_and(|d| d.active));
    // Once active it stays active, even back at the press.
    app.update(Message::PointerMoved(IN_LOCAL));
    assert!(app.drag.is_some_and(|d| d.active));
    app.update(Message::DragCancel);
}

/// Over the other pane an active drag has a target and says what a drop does.
#[tokio::test]
async fn drag_over_other_pane_sets_target_and_label() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    let b_row = row_of(&app, Side::Local, "b.txt");
    press(&mut app, tab, Side::Local, b_row, IN_LOCAL);
    assert_eq!(app.drag_count(), 1);
    move_over(&mut app, tab, Side::Remote, IN_REMOTE);
    assert_eq!(app.drop_target(), Some((tab, Side::Remote)));
    assert_eq!(app.drop_label().as_deref(), Some("Upload 1 item to /srv"));
    assert!(!app.drag_invalid_over(tab, Side::Remote));
    app.update(Message::DragCancel);

    // Two selected (Ctrl+click adds a.txt).
    let a_row = row_of(&app, Side::Local, "a.txt");
    press(&mut app, tab, Side::Local, b_row, IN_LOCAL);
    app.update(Message::DragCancel);
    app.mods.ctrl = true;
    press(&mut app, tab, Side::Local, a_row, IN_LOCAL);
    app.mods.ctrl = false;
    assert_eq!(app.drag_count(), 2);
    move_over(&mut app, tab, Side::Remote, IN_REMOTE);
    assert_eq!(app.drop_label().as_deref(), Some("Upload 2 items to /srv"));
    app.update(Message::DragCancel);

    // A download names the local folder as the breadcrumb does (under home, with `~`).
    let r_row = row_of(&app, Side::Remote, "r.txt");
    press(&mut app, tab, Side::Remote, r_row, IN_REMOTE);
    move_over(&mut app, tab, Side::Local, IN_LOCAL);
    assert_eq!(app.drop_label().as_deref(), Some("Download 1 item to ~"));
    app.update(Message::DragCancel);
    assert_eq!(app.drop_label(), None);
}

/// Over its own pane a drag has no target: no label, the not-allowed cursor.
#[tokio::test]
async fn drag_over_source_pane_has_no_target() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    let b_row = row_of(&app, Side::Local, "b.txt");
    app.update(pane_msg(tab, Side::Local, PaneMsg::PointerIn));
    press(&mut app, tab, Side::Local, b_row, IN_LOCAL);
    app.update(Message::PointerMoved(Point::new(
        IN_LOCAL.x,
        IN_LOCAL.y + 40.0,
    )));
    assert!(app.drag.is_some_and(|d| d.active));
    assert_eq!((app.drop_target(), app.drop_label()), (None, None));
    assert!(app.drag_invalid_over(tab, Side::Local));
    assert!(
        !app.drag_invalid_over(tab, Side::Remote),
        "the pointer is not there"
    );
    assert!(no_enqueue(&app.update(Message::DragEnd)));
    assert!(
        !app.drag_invalid_over(tab, Side::Local),
        "no drag, no cursor"
    );
}

/// The remote pane of a site that lost its connection is no target: nothing drops.
#[tokio::test]
async fn disconnected_target_is_invalid() {
    let (_fake, mut app, a, tab) = transfer_tab().await;
    let b_row = row_of(&app, Side::Local, "b.txt");
    press(&mut app, tab, Side::Local, b_row, IN_LOCAL);
    // The site is closed under the drag: its pane can't act.
    let session = app.conn(&a.id).session().unwrap();
    app.update(Message::SessionState {
        session,
        state: ConnectionState::Disconnected(Error::new(ErrorKind::Network, "Connection reset")),
        auth: None,
    });
    // The drop of the session ended that drag; a new one over the dead pane is invalid.
    assert_eq!(app.drag, None);
    press(&mut app, tab, Side::Local, b_row, IN_LOCAL);
    move_over(&mut app, tab, Side::Remote, IN_REMOTE);
    assert!(app.drag.is_some_and(|d| d.active));
    assert!(app.conn(&a.id).lost_connection().is_some());
    assert_eq!((app.drop_target(), app.drop_label()), (None, None));
    assert!(app.drag_invalid_over(tab, Side::Remote));
    assert!(no_enqueue(&app.update(Message::DragEnd)));
}

/// Escape cancels an active drag (a later release drops nothing); before the move it is
/// the pane's Escape as always.
#[tokio::test]
async fn escape_cancels_drag() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    let b_row = row_of(&app, Side::Local, "b.txt");
    press(&mut app, tab, Side::Local, b_row, IN_LOCAL);
    move_over(&mut app, tab, Side::Remote, IN_REMOTE);
    let escape = Message::KeyPressed {
        key: FileKey::Escape,
        shift: false,
    };
    assert!(app.update(escape.clone()).is_empty());
    assert_eq!(app.drag, None);
    assert_eq!(
        app.tabs[0]
            .pane(Side::Local)
            .unwrap()
            .selected_entries()
            .len(),
        1,
        "the pane did not see that Escape"
    );
    assert!(no_enqueue(&app.update(Message::DragEnd)));

    // Not active yet: the drag stays (the release ends it), the pane takes Escape.
    press(&mut app, tab, Side::Local, b_row, IN_LOCAL);
    app.update(escape);
    assert!(app.drag.is_some());
    assert!(no_enqueue(&app.update(Message::DragEnd)));
}

/// The source pane listing again (its rows are gone) or the source tab closing ends a
/// drag.
#[tokio::test]
async fn drag_ends_when_source_relists_or_tab_closes() {
    let (fake, mut app, _a, tab) = transfer_tab().await;
    let b_row = row_of(&app, Side::Local, "b.txt");
    press(&mut app, tab, Side::Local, b_row, IN_LOCAL);
    move_over(&mut app, tab, Side::Remote, IN_REMOTE);
    // The other pane listing again leaves the drag alone.
    let effects = app.update(pane_msg(tab, Side::Remote, PaneMsg::Retry));
    settle(&mut app, &fake, run_listings(&fake, &effects).await).await;
    assert!(app.drag.is_some());
    let effects = app.update(pane_msg(tab, Side::Local, PaneMsg::Retry));
    settle(&mut app, &fake, run_listings(&fake, &effects).await).await;
    assert_eq!(app.drag, None);

    press(&mut app, tab, Side::Local, b_row, IN_LOCAL);
    move_over(&mut app, tab, Side::Remote, IN_REMOTE);
    app.update(Message::CloseTab(tab));
    assert!(app.tabs.is_empty());
    assert_eq!(app.drag, None);
}

/// Files from the system over the window: the label counts them when the system said
/// how many, and needs a connected tab whose remote pane can act.
#[tokio::test]
async fn os_drop_label_counts_files() {
    let fake = Arc::new(FakeBackend::new());
    let mut app = loaded(&fake).await;
    app.update(Message::FilesHovered(true));
    assert_eq!(app.os_drop_label(), None, "no connected tab");
    app.update(Message::FilesHovered(false));

    let (_fake, mut app, _a, _tab) = transfer_tab().await;
    assert_eq!(app.os_drop_label(), None, "nothing hovers");
    for _ in 0..3 {
        app.update(Message::FilesHovered(true));
    }
    assert_eq!(app.os_drag_count, 3);
    assert_eq!(
        app.os_drop_label().as_deref(),
        Some("Upload 3 files to /srv")
    );
    app.update(Message::FilesHovered(false));
    assert_eq!((app.os_drag, app.os_drag_count), (false, 0));
    assert_eq!(app.os_drop_label(), None);
    // Hovering, count not known yet.
    app.os_drag = true;
    assert_eq!(app.os_drop_label().as_deref(), Some("Upload files to /srv"));
    app.update(Message::FilesHovered(true));
    assert_eq!(
        app.os_drop_label().as_deref(),
        Some("Upload 1 file to /srv")
    );
    app.update(Message::FileDropped(PathBuf::from("/home/me/a.txt")));
    assert_eq!((app.os_drag, app.os_drag_count), (false, 0));
}

/// Active (running, paused, awaiting a decision), Queued, Failed, Done (done, skipped,
/// cancelled): each row in one tab, the counts in the tab labels.
#[test]
fn queue_tabs_partition_states() {
    let mut app = App::new();
    let s = SiteId::new();
    assert!(app.queue.resyncing(), "waits for its first snapshot");
    synced(
        &mut app,
        9,
        vec![
            item(1, &s, TransferState::Active),
            item(2, &s, TransferState::Queued),
            item(3, &s, TransferState::Paused),
            item(4, &s, conflict()),
            item(5, &s, failed("Permission denied")),
            item(6, &s, TransferState::Done),
            item(7, &s, TransferState::Skipped("exists".into())),
            item(8, &s, TransferState::Cancelled),
        ],
    );
    assert_eq!(app.queue.workers, 4);
    assert_eq!(app.queue.counts(), [3, 1, 1, 3]);
    assert_eq!(ids(&app.queue.rows_in(QueueTab::Active)), [1, 3, 4]);
    assert_eq!(ids(&app.queue.rows_in(QueueTab::Queued)), [2]);
    assert_eq!(ids(&app.queue.rows_in(QueueTab::Failed)), [5]);
    assert_eq!(ids(&app.queue.rows_in(QueueTab::Done)), [6, 7, 8]);
    assert_eq!(strings::queue_tab(strings::queue_done(), 3), "Done 3");

    // Each state's buttons.
    let actions = |id| app.queue.actions(app.queue.row(TransferId(id)).unwrap());
    assert_eq!(actions(1), [RowAction::Pause, RowAction::Cancel]);
    assert_eq!(actions(2), [RowAction::Pause, RowAction::Cancel]);
    assert_eq!(actions(3), [RowAction::Resume, RowAction::Cancel]);
    assert_eq!(actions(8), [RowAction::Retry]);
    assert!(actions(6).is_empty() && actions(7).is_empty());

    // An item moving on changes tab; the tab buttons pick what shows.
    assert!(
        app.update(Message::TransferEvent(changed(
            10,
            2,
            TransferState::Active
        )))
        .is_empty()
    );
    assert_eq!(app.queue.counts(), [4, 0, 1, 3]);
    assert_eq!(app.queue.tab, QueueTab::Active);
    app.queue.offset_y = 60.0;
    assert!(
        app.update(Message::Queue(QueueMsg::SelectTab(QueueTab::Done)))
            .is_empty()
    );
    assert_eq!((app.queue.tab, app.queue.offset_y), (QueueTab::Done, 0.0));

    // Pause on a running row asks the engine; a button the row doesn't have does nothing.
    assert!(matches!(
        &app.update(Message::Queue(QueueMsg::Action(
            TransferId(1),
            RowAction::Pause
        )))[..],
        [Effect::Control {
            id: TransferId(1),
            op: ControlOp::Pause
        }]
    ));
    assert!(
        app.update(Message::Queue(QueueMsg::Action(
            TransferId(6),
            RowAction::Retry
        )))
        .is_empty()
    );
    assert!(
        app.update(Message::Queue(QueueMsg::Action(
            TransferId(99),
            RowAction::Pause
        )))
        .is_empty()
    );
}

/// Review Focus 2. The receiver lags (events lost): the stream says so, the queue asks a
/// new snapshot, buffers what comes meanwhile and, once the snapshot is in, drops every
/// event at or below its `seq`. A stale `StateChanged` (older than the snapshot, or
/// delivered out of order later) never overwrites the newer state; an old snapshot
/// answering late is ignored; a gap in `seq` starts another resync.
#[tokio::test]
async fn lagged_triggers_resnapshot_and_drops_stale() {
    let fake = Arc::new(FakeBackend::new());
    let s = SiteId::new();
    let mut app = App::new();
    let mut feed = Box::pin(effects::transfer_feed(fake.as_ref()));
    let mut next = async || {
        tokio::time::timeout(Duration::from_secs(5), feed.next())
            .await
            .expect("a message")
            .expect("the stream is open")
    };

    // Start: the subscription asks for a snapshot after subscribing.
    let first = next().await;
    assert!(matches!(first, Message::QueueResync));
    let effects = app.update(first);
    let r1 = snapshot_request(&effects);
    for message in run_queue(&fake, effects).await {
        assert!(app.update(message).is_empty());
    }
    assert_eq!(app.queue.len(), 0);
    assert!(!app.queue.resyncing());

    // Live: added, then running.
    fake.send_transfer_event(event(
        1,
        EventKind::Added(item(1, &s, TransferState::Queued)),
    ));
    fake.send_transfer_event(changed(2, 1, TransferState::Active));
    for _ in 0..2 {
        let message = next().await;
        assert!(app.update(message).is_empty());
    }
    assert_eq!(state_of(&app, 1), &TransferState::Active);

    // 20 events while the UI reads none: the 16-deep receiver lags and loses 3..=6.
    // seq 21: paused; seq 22: running again.
    for seq in 3..=22 {
        let e = match seq {
            21 => changed(seq, 1, TransferState::Paused),
            22 => changed(seq, 1, TransferState::Active),
            _ => event(
                seq,
                EventKind::Progress(vec![ProgressUpdate {
                    id: TransferId(1),
                    bytes_done: seq * 10,
                    total: Some(1000),
                    bytes_per_sec: 100,
                }]),
            ),
        };
        fake.send_transfer_event(e);
    }
    // By the time the snapshot is taken the engine went on: 23 (last progress), 24 (done).
    let mut done = item(1, &s, TransferState::Done);
    done.bytes_done = 1000;
    fake.set_queue(24, vec![done]);

    let lagged = next().await;
    assert!(
        matches!(lagged, Message::QueueResync),
        "a lagging receiver asks for a resync, got {lagged:?}"
    );
    let effects = app.update(lagged);
    let r2 = snapshot_request(&effects);
    assert_ne!(r1, r2);
    assert!(app.queue.resyncing());

    // The 16 events left (7..=22) arrive before the snapshot: buffered, not applied.
    for seq in 7..=22 {
        let message = next().await;
        assert!(matches!(&message, Message::TransferEvent(e) if e.seq == seq));
        assert!(app.update(message).is_empty());
    }
    assert_eq!(
        state_of(&app, 1),
        &TransferState::Active,
        "nothing applied while resyncing"
    );

    // The snapshot (seq 24) lands: every buffered event is below it, so dropped; the
    // stale "Active" of seq 22 does not overwrite "Done".
    for message in run_queue(&fake, effects).await {
        assert!(app.update(message).is_empty());
    }
    assert!(!app.queue.resyncing());
    assert_eq!(state_of(&app, 1), &TransferState::Done);
    assert_eq!(app.queue.row(TransferId(1)).unwrap().item.bytes_done, 1000);

    // 23 and 24 reach the UI after the snapshot that holds them: dropped too.
    fake.send_transfer_event(event(
        23,
        EventKind::Progress(vec![ProgressUpdate {
            id: TransferId(1),
            bytes_done: 1000,
            total: Some(1000),
            bytes_per_sec: 100,
        }]),
    ));
    fake.send_transfer_event(changed(24, 1, TransferState::Done));
    for _ in 23..=24 {
        let message = next().await;
        assert!(app.update(message).is_empty());
    }
    assert_eq!(state_of(&app, 1), &TransferState::Done);
    assert_eq!(app.queue.total_speed(), 0);

    // An old snapshot answering late (its request superseded) changes nothing.
    assert!(
        app.update(Message::QueueSnapshot {
            request: r1,
            snapshot: snapshot(5, vec![item(1, &s, TransferState::Queued)]),
        })
        .is_empty()
    );
    assert_eq!(state_of(&app, 1), &TransferState::Done);

    // A stale StateChanged delivered out of order: dropped.
    assert!(
        app.update(Message::TransferEvent(changed(
            15,
            1,
            TransferState::Active
        )))
        .is_empty()
    );
    assert_eq!(state_of(&app, 1), &TransferState::Done);

    // The next event applies; one after a gap (26 missing) resyncs instead.
    assert!(
        app.update(Message::TransferEvent(event(
            25,
            EventKind::Added(item(2, &s, TransferState::Queued))
        )))
        .is_empty()
    );
    assert_eq!(state_of(&app, 2), &TransferState::Queued);
    let effects = app.update(Message::TransferEvent(changed(
        27,
        2,
        TransferState::Active,
    )));
    let r3 = snapshot_request(&effects);
    assert_eq!(state_of(&app, 2), &TransferState::Queued);

    // A snapshot older than what is applied (a stopped engine answers `(0, [])`) is
    // dropped, and not asked again in a loop.
    assert!(
        app.update(Message::QueueSnapshot {
            request: r3,
            snapshot: snapshot(0, Vec::new()),
        })
        .is_empty()
    );
    assert_eq!(state_of(&app, 2), &TransferState::Queued);
    assert!(app.queue.resyncing());
}

/// A conflict shows inline in its row (no modal): Overwrite / Resume / Skip and its own
/// "Apply to all" box, which goes with the answer.
#[tokio::test]
async fn conflict_inline_apply_to_all() {
    let fake = Arc::new(FakeBackend::new());
    let s = SiteId::new();
    let mut app = loaded(&fake).await;
    synced(
        &mut app,
        3,
        vec![item(1, &s, conflict()), item(2, &s, conflict())],
    );
    let row = app.queue.row(TransferId(1)).unwrap();
    assert_eq!(app.queue.tab_of(row), QueueTab::Active);
    assert_eq!(
        app.queue.actions(row),
        [
            RowAction::Resolve(ResolveAction::Overwrite),
            RowAction::Resolve(ResolveAction::Resume),
            RowAction::Resolve(ResolveAction::Skip),
        ]
    );

    // Tick "Apply to all" on row 1, then Overwrite.
    assert!(
        app.update(Message::Queue(QueueMsg::ToggleApplyAll(TransferId(1))))
            .is_empty()
    );
    assert!(app.queue.apply_all(TransferId(1)));
    assert!(
        !app.queue.apply_all(TransferId(2)),
        "each row has its own box"
    );
    let effects = app.update(Message::Queue(QueueMsg::Action(
        TransferId(1),
        RowAction::Resolve(ResolveAction::Overwrite),
    )));
    let overwrite_all = ControlOp::Resolve(Resolution {
        action: ResolveAction::Overwrite,
        apply_to_all: true,
    });
    assert!(matches!(
        &effects[..],
        [Effect::Control { id: TransferId(1), op }] if *op == overwrite_all
    ));
    assert!(app.form.is_none(), "no modal");
    // The label says the answer lasts for the session (the engine makes it its policy).
    assert_eq!(strings::apply_to_all(), "Apply to all (this session)");
    assert_eq!(
        app.queue.session_policy(),
        None,
        "not before the engine accepts it"
    );
    for message in run_queue(&fake, effects).await {
        assert!(app.update(message).is_empty());
    }
    assert_eq!(fake.controls(), vec![(TransferId(1), overwrite_all)]);
    // Accepted: the queue remembers the session's policy and its head says so.
    assert_eq!(app.queue.session_policy(), Some(ConflictPolicy::Overwrite));
    assert_eq!(
        strings::session_policy(ConflictPolicy::Overwrite),
        "Conflicts: overwrite (this session)"
    );

    // A refused "to all" answer leaves the session's policy as it was.
    app.update(Message::Queue(QueueMsg::ToggleApplyAll(TransferId(2))));
    let effects = app.update(Message::Queue(QueueMsg::Action(
        TransferId(2),
        RowAction::Resolve(ResolveAction::Resume),
    )));
    assert_eq!(effects.len(), 1);
    app.update(Message::ControlDone {
        id: TransferId(2),
        result: Err(Error::new(
            ErrorKind::Conflict,
            "Transfer is not awaiting a decision",
        )),
    });
    assert_eq!(app.queue.session_policy(), Some(ConflictPolicy::Overwrite));
    app.update(Message::Queue(QueueMsg::ToggleApplyAll(TransferId(2))));

    // Row 2 alone: Skip, not to all.
    let effects = app.update(Message::Queue(QueueMsg::Action(
        TransferId(2),
        RowAction::Resolve(ResolveAction::Skip),
    )));
    assert!(matches!(
        &effects[..],
        [Effect::Control {
            id: TransferId(2),
            op: ControlOp::Resolve(Resolution {
                action: ResolveAction::Skip,
                apply_to_all: false,
            })
        }]
    ));

    // The engine answers: the rows move on and the box goes with the conflict.
    app.update(Message::TransferEvent(changed(4, 1, TransferState::Queued)));
    app.update(Message::TransferEvent(changed(
        5,
        2,
        TransferState::Skipped("skipped by user".into()),
    )));
    assert!(!app.queue.apply_all(TransferId(1)));
    assert_eq!(app.queue.counts(), [0, 1, 0, 1]);
}

/// A failed row shows its error and Retry / Dismiss. Retry asks the engine (its refusal
/// shows in the row); Dismiss moves the row to Done without touching the engine, and Retry
/// still works from there.
#[tokio::test]
async fn failed_row_retry_skip() {
    let fake = Arc::new(FakeBackend::new());
    let s = SiteId::new();
    let mut app = loaded(&fake).await;
    synced(
        &mut app,
        2,
        vec![
            item(1, &s, failed("Permission denied: /var/www")),
            item(2, &s, failed("Disk full")),
        ],
    );
    assert_eq!(ids(&app.queue.rows_in(QueueTab::Failed)), [1, 2]);
    let row = app.queue.row(TransferId(1)).unwrap();
    assert_eq!(
        app.queue.actions(row),
        [RowAction::Retry, RowAction::Dismiss]
    );

    // Retry: to the engine. Its refusal stays in the row until the row changes.
    let effects = app.update(Message::Queue(QueueMsg::Action(
        TransferId(1),
        RowAction::Retry,
    )));
    assert!(matches!(
        &effects[..],
        [Effect::Control {
            id: TransferId(1),
            op: ControlOp::Retry
        }]
    ));
    for message in run_queue(&fake, effects).await {
        assert!(app.update(message).is_empty());
    }
    assert_eq!(fake.controls(), vec![(TransferId(1), ControlOp::Retry)]);
    app.update(Message::ControlDone {
        id: TransferId(1),
        result: Err(Error::new(ErrorKind::Conflict, "Transfer is not failed")),
    });
    assert_eq!(
        app.queue
            .row_error(TransferId(1))
            .map(|e| e.message.as_str()),
        Some("Transfer is not failed")
    );
    app.update(Message::TransferEvent(changed(3, 1, TransferState::Queued)));
    assert!(app.queue.row_error(TransferId(1)).is_none());
    assert_eq!(
        app.queue.tab_of(app.queue.row(TransferId(1)).unwrap()),
        QueueTab::Queued
    );

    // Dismiss: out of Failed, into Done, only Retry left; the engine is not asked.
    assert!(
        app.update(Message::Queue(QueueMsg::Action(
            TransferId(2),
            RowAction::Dismiss
        )))
        .is_empty()
    );
    assert_eq!(fake.controls().len(), 1);
    assert_eq!(app.queue.counts(), [0, 1, 0, 1]);
    let row = app.queue.row(TransferId(2)).unwrap();
    assert_eq!(app.queue.tab_of(row), QueueTab::Done);
    assert_eq!(app.queue.actions(row), [RowAction::Retry]);
    assert_eq!(app.queue.status(row), RowStatus::Dismissed);
    // A dismissed row keeps its place across a resync.
    let request = snapshot_request(&app.update(Message::QueueResync));
    app.update(Message::QueueSnapshot {
        request,
        snapshot: snapshot(
            3,
            vec![
                item(1, &s, TransferState::Queued),
                item(2, &s, failed("Disk full")),
            ],
        ),
    });
    assert_eq!(app.queue.counts(), [0, 1, 0, 1]);

    // Retry from Done: to the engine, and the row is no longer dismissed.
    assert!(matches!(
        &app.update(Message::Queue(QueueMsg::Action(
            TransferId(2),
            RowAction::Retry
        )))[..],
        [Effect::Control {
            id: TransferId(2),
            op: ControlOp::Retry
        }]
    ));
    let row = app.queue.row(TransferId(2)).unwrap();
    assert_eq!(app.queue.tab_of(row), QueueTab::Failed);
}

/// The status column of a row: a running folder reads "Transferring…" (never "Starting…"
/// for its whole run); a file of unknown size shows its bytes and speed; "Starting…" only
/// before any byte or progress; a known size gets the bar.
#[test]
fn row_status_labels() {
    let mut app = App::new();
    let s = SiteId::new();
    let mut folder = item(1, &s, TransferState::Active);
    folder.is_dir = Some(true);
    folder.size = None;
    let mut unknown = item(2, &s, TransferState::Active);
    unknown.size = None;
    let fresh = item(3, &s, TransferState::Active);
    let mut resumed = item(4, &s, TransferState::Active);
    resumed.bytes_done = 250;
    synced(
        &mut app,
        1,
        vec![
            folder,
            unknown,
            fresh,
            resumed,
            item(5, &s, TransferState::Queued),
            item(6, &s, failed("boom")),
            item(7, &s, TransferState::Skipped("exists".into())),
        ],
    );
    fn status(app: &App, id: i64) -> RowStatus<'_> {
        app.queue.status(app.queue.row(TransferId(id)).unwrap())
    }
    assert_eq!(status(&app, 1), RowStatus::Transferring);
    assert_eq!(status(&app, 2), RowStatus::Starting);
    assert_eq!(status(&app, 3), RowStatus::Starting);
    // Bytes already there (a resumed copy): the bar at once.
    assert_eq!(
        status(&app, 4),
        RowStatus::Progress {
            fraction: 0.25,
            bytes_per_sec: 0
        }
    );
    assert_eq!(status(&app, 5), RowStatus::Waiting);
    assert!(matches!(status(&app, 6), RowStatus::Error(e) if e.message == "boom"));
    assert_eq!(status(&app, 7), RowStatus::Skipped("exists"));

    let update = |id, bytes_done, total, bytes_per_sec| ProgressUpdate {
        id: TransferId(id),
        bytes_done,
        total,
        bytes_per_sec,
    };
    app.update(Message::TransferEvent(event(
        2,
        EventKind::Progress(vec![
            update(1, 0, None, 0),
            update(2, 4096, None, 2048),
            update(3, 0, Some(1000), 0),
        ]),
    )));
    // The folder stays "Transferring…"; the unknown size shows bytes and speed.
    assert_eq!(status(&app, 1), RowStatus::Transferring);
    assert_eq!(
        status(&app, 2),
        RowStatus::Bytes {
            done: 4096,
            bytes_per_sec: 2048
        }
    );
    assert_eq!(strings::bytes_and_speed(4096, 2048), "4.0 KB, 2.0 KB/s");
    // A progress came (even at 0 bytes): the bar, not "Starting…".
    assert_eq!(
        status(&app, 3),
        RowStatus::Progress {
            fraction: 0.0,
            bytes_per_sec: 0
        }
    );
    // A refused control wins over the state.
    app.update(Message::ControlDone {
        id: TransferId(3),
        result: Err(Error::new(ErrorKind::Conflict, "nope")),
    });
    assert!(matches!(status(&app, 3), RowStatus::Error(e) if e.message == "nope"));
}

/// The total is the sum of the running rows' speeds; a row that stops counts no more.
#[test]
fn total_speed_sums_active() {
    let mut app = App::new();
    let s = SiteId::new();
    synced(
        &mut app,
        1,
        vec![
            item(1, &s, TransferState::Active),
            item(2, &s, TransferState::Active),
            item(3, &s, TransferState::Paused),
        ],
    );
    assert_eq!(app.queue.total_speed(), 0);
    let update = |id, bytes_per_sec| ProgressUpdate {
        id: TransferId(id),
        bytes_done: 500,
        total: Some(1000),
        bytes_per_sec,
    };
    app.update(Message::TransferEvent(event(
        2,
        EventKind::Progress(vec![update(1, 1000), update(2, 2500), update(3, 400)]),
    )));
    assert_eq!(app.queue.total_speed(), 3500);
    let row = app.queue.row(TransferId(2)).unwrap();
    assert_eq!((row.bytes_per_sec, row.fraction()), (2500, Some(0.5)));
    assert_eq!(app.queue.row(TransferId(3)).unwrap().bytes_per_sec, 0);
    assert_eq!(strings::speed(app.queue.total_speed()), "3.4 KB/s");
    assert_eq!(
        strings::total_workers(app.queue.workers),
        " total, 4 workers"
    );
    assert_eq!(strings::percent(0.519), "51%");

    // The final progress, then Done: that row stops counting.
    app.update(Message::TransferEvent(event(
        3,
        EventKind::Progress(vec![ProgressUpdate {
            bytes_done: 1000,
            ..update(2, 3000)
        }]),
    )));
    assert_eq!(app.queue.total_speed(), 4000);
    app.update(Message::TransferEvent(changed(4, 2, TransferState::Done)));
    assert_eq!(app.queue.total_speed(), 1000);
    assert_eq!(app.queue.row(TransferId(2)).unwrap().fraction(), Some(1.0));
}

/// "Clear finished" calls the engine, which emits no event: a new snapshot (same `seq`,
/// accepted) makes the finished rows go. A failure shows inline.
#[tokio::test]
async fn clear_finished_removes_rows() {
    let fake = Arc::new(FakeBackend::new());
    let s = SiteId::new();
    let mut app = loaded(&fake).await;
    let items = vec![
        item(1, &s, TransferState::Done),
        item(2, &s, TransferState::Active),
        item(3, &s, failed("boom")),
        item(4, &s, TransferState::Cancelled),
    ];
    fake.set_queue(5, items.clone());
    synced(&mut app, 5, items);
    assert!(app.queue.has_finished());
    assert_eq!(app.queue.len(), 4);

    let effects = app.update(Message::Queue(QueueMsg::ClearFinished));
    assert!(matches!(effects[..], [Effect::ClearFinished]));
    let cleared = run_queue(&fake, effects).await;
    assert!(matches!(cleared[..], [Message::Cleared(Ok(3))]));
    assert_eq!(fake.clears(), 1);
    let mut effects = Vec::new();
    for message in cleared {
        effects.extend(app.update(message));
    }
    snapshot_request(&effects);
    for message in run_queue(&fake, effects).await {
        assert!(app.update(message).is_empty());
    }
    assert_eq!(app.queue.len(), 1);
    assert_eq!(ids(&app.queue.rows_in(QueueTab::Active)), [2]);
    assert!(!app.queue.has_finished());

    // The engine refused: the error shows above the rows, nothing else changes.
    assert!(
        app.update(Message::Cleared(Err(Error::new(
            ErrorKind::Io,
            "Transfer engine stopped"
        ))))
        .is_empty()
    );
    assert_eq!(
        app.queue.error.as_ref().map(|e| e.message.as_str()),
        Some("Transfer engine stopped")
    );
    assert_eq!(app.queue.len(), 1);
}

/// A lost connection pauses the site's transfers: its tab says so until one runs again.
#[test]
fn site_paused_until_a_transfer_runs() {
    let mut app = App::new();
    let s = SiteId::new();
    synced(&mut app, 0, vec![item(1, &s, TransferState::Queued)]);
    app.update(Message::TransferEvent(event(
        1,
        EventKind::SitePaused {
            site: s.clone(),
            error: Error::new(ErrorKind::Network, "Connection reset"),
        },
    )));
    assert!(app.queue.site_paused(&s).is_some());
    app.update(Message::TransferEvent(changed(2, 1, TransferState::Active)));
    assert!(app.queue.site_paused(&s).is_none());

    // Paused again; a resync whose snapshot shows the site running clears it, while a
    // site with nothing running stays paused (the snapshot doesn't list paused sites).
    let other = SiteId::new();
    for (seq, site) in [(3, &s), (4, &other)] {
        app.update(Message::TransferEvent(event(
            seq,
            EventKind::SitePaused {
                site: site.clone(),
                error: Error::new(ErrorKind::Network, "Connection reset"),
            },
        )));
    }
    let request = snapshot_request(&app.update(Message::QueueResync));
    app.update(Message::QueueSnapshot {
        request,
        snapshot: snapshot(
            4,
            vec![
                item(1, &s, TransferState::Active),
                item(2, &other, TransferState::Queued),
            ],
        ),
    });
    assert!(app.queue.site_paused(&s).is_none());
    assert!(app.queue.site_paused(&other).is_some());
}

/// End to end against the Docker SSH server: the real backend and engine, the
/// subscription's stream, the app's queue. A file uploaded from the local pane shows up
/// in the queue, runs and lands on the server; "Clear finished" removes it.
#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn upload_through_the_queue_end_to_end() {
    use crate::backend::{
        Backend, BackendConfig, BrokerEvent, PromptAnswer, SessionConfig, broker::PromptKind,
    };

    let dir = tempfile::tempdir().unwrap();
    let config = BackendConfig {
        profile_dir: dir.path().join("profiles"),
        queue_path: None,
        sessions: SessionConfig {
            known_hosts: Some(dir.path().join("known_hosts")),
            ..SessionConfig::default()
        },
        backup_params: hoprs_vault::ExportParams::default(),
        edit_dir: dir.path().join("edit"),
        recovered_dir: dir.path().join("recovered"),
    };
    let backend = Backend::open(config, Arc::new(hoprs_vault::MemoryStore::new())).unwrap();
    let backend: Arc<dyn BackendApi> = Arc::new(backend);
    // Trust the test server's host key.
    let mut prompts = backend.prompt_events().unwrap();
    let answers = tokio::spawn({
        let backend = backend.clone();
        async move {
            while let Some(event) = prompts.recv().await {
                if let BrokerEvent::Request(r) = event {
                    let yes = matches!(r.kind, PromptKind::HostKey { .. });
                    let answer = if yes {
                        PromptAnswer::Decision(true)
                    } else {
                        PromptAnswer::Cancel
                    };
                    backend.answer_prompt(r.id, answer).await;
                }
            }
        }
    });

    let mut site = Site::new("docker ssh", "127.0.0.1", "alice", Protocol::Ssh);
    site.port = hoprs_ssh::testing::SSH_PORT;
    site.ssh.use_agent = false;
    site.ssh.key_file = Some(hoprs_ssh::testing::fixture("id_ed25519"));
    backend.save_site(site.clone()).await.unwrap();
    let connected = backend.connect(site.id.clone()).await.unwrap();
    let work = format!(
        "/home/alice/t-queue-{}",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    backend
        .mkdir(Some(connected.session), Side::Remote, rpath(&work))
        .await
        .unwrap();
    let local = tempfile::tempdir().unwrap();
    std::fs::write(local.path().join("hello.txt"), b"hello queue").unwrap();
    let local_file = crate::app::files::local_path(&local.path().join("hello.txt")).unwrap();

    // The app follows the engine through the subscription's stream.
    let mut app = App::new();
    let mut feed = Box::pin(effects::transfer_feed(backend.as_ref()));
    let run = async |app: &mut App, effects: Vec<Effect>| {
        let mut messages = Vec::new();
        for effect in effects {
            messages.push(match effect {
                Effect::Enqueue(specs) => {
                    Message::Enqueued(effects::enqueue_all(backend.clone(), specs).await)
                }
                Effect::SnapshotQueue(request) => Message::QueueSnapshot {
                    request,
                    snapshot: backend.transfer_snapshot().await,
                },
                Effect::ClearFinished => Message::Cleared(backend.clear_finished().await),
                other => panic!("unexpected {other:?}"),
            });
        }
        let mut next = Vec::new();
        for message in messages {
            next.extend(app.update(message));
        }
        next
    };
    let first = feed.next().await.unwrap();
    let effects = app.update(first);
    assert!(run(&mut app, effects).await.is_empty());
    assert_eq!(app.queue.workers, 4);

    let effects = vec![Effect::Enqueue(vec![TransferSpec {
        site: site.id.clone(),
        direction: Direction::Upload,
        local: local_file,
        remote: rpath(&format!("{work}/hello.txt")),
    }])];
    assert!(run(&mut app, effects).await.is_empty());
    assert!(app.queue.error.is_none());

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while app.queue.rows_in(QueueTab::Done).is_empty() {
        let message = tokio::time::timeout_at(deadline, feed.next())
            .await
            .expect("the upload finishes")
            .unwrap();
        let effects = app.update(message);
        assert!(run(&mut app, effects).await.is_empty());
    }
    let done = app.queue.rows_in(QueueTab::Done);
    assert_eq!(done[0].item.state, TransferState::Done);
    assert_eq!(done[0].item.bytes_done, 11);
    let listed = backend
        .list(Some(connected.session), Side::Remote, rpath(&work))
        .await
        .unwrap();
    assert!(listed.iter().any(|e| e.name == "hello.txt" && e.size == 11));

    let effects = app.update(Message::Queue(QueueMsg::ClearFinished));
    let effects = run(&mut app, effects).await;
    assert!(run(&mut app, effects).await.is_empty());
    assert_eq!(app.queue.len(), 0);

    backend.close(connected.session).await;
    answers.abort();
}

/// A queue database that could not be opened: the backend runs on an in-memory queue,
/// and the queue panel says so (transfers of this run won't survive a restart).
#[tokio::test]
async fn unavailable_queue_is_shown_in_the_panel() {
    let cause = Error::new(hoprs_core::ErrorKind::Io, "Cannot open the queue database")
        .with_detail("file is not a database");
    let fake = Arc::new(FakeBackend::new().with_queue_error(cause.clone()));
    let (mut app, boot) = App::boot();
    assert!(
        boot.iter().any(|e| matches!(e, Effect::LoadQueueStatus)),
        "{boot:?}"
    );
    let backend: Arc<dyn BackendApi> = fake.clone();
    app.update(Message::QueueStatus(backend.queue_error()));
    let error = app.queue.error.as_ref().expect("the panel shows it");
    assert_eq!(error.message, strings::queue_unavailable(&cause.message));
    assert!(
        error.message.contains("won't survive a restart"),
        "{}",
        error.message
    );
    assert_eq!(error.detail, cause.detail);

    // A queue that opened: nothing to say.
    let mut app = App::new();
    let fine: Arc<dyn BackendApi> = Arc::new(FakeBackend::new());
    app.update(Message::QueueStatus(fine.queue_error()));
    assert_eq!(app.queue.error, None);
}

/// A deleted site's transfers can never run again: its unfinished items are cancelled
/// through the usual control path (a folder takes its pending children with it), its
/// finished ones and other sites' are left alone.
#[tokio::test]
async fn deleting_a_site_cancels_its_transfers() {
    let s = srv_site();
    let t = site("t", "k", "u", Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(s.clone()).with_site(t.clone()));
    let mut app = loaded(&fake).await;
    let mut folder = item(5, &s.id, TransferState::Queued);
    folder.is_dir = Some(true);
    let mut child = item(6, &s.id, TransferState::Queued);
    child.parent = Some(TransferId(5));
    let mut done_folder = item(7, &s.id, TransferState::Done);
    done_folder.is_dir = Some(true);
    let mut orphan = item(8, &s.id, TransferState::Paused);
    orphan.parent = Some(TransferId(7));
    synced(
        &mut app,
        1,
        vec![
            item(1, &s.id, TransferState::Queued),
            item(2, &s.id, TransferState::Paused),
            item(3, &s.id, TransferState::Done),
            item(4, &t.id, TransferState::Active),
            folder,
            child,
            done_folder,
            orphan,
            item(9, &s.id, conflict()),
            item(10, &s.id, TransferState::Active),
        ],
    );

    // A failed delete cancels nothing.
    let effects = app.update(Message::FormDone {
        op: ProfileOp::DeleteSite(s.id.clone()),
        result: Err(Error::new(ErrorKind::Io, "disk full")),
    });
    assert!(effects.is_empty(), "{effects:?}");

    let effects = app.update(Message::FormDone {
        op: ProfileOp::DeleteSite(s.id.clone()),
        result: Ok(()),
    });
    let cancelled: Vec<i64> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Control {
                id,
                op: ControlOp::Cancel,
            } => Some(id.0),
            _ => None,
        })
        .collect();
    assert_eq!(cancelled, [1, 2, 5, 8, 9, 10], "{effects:?}");
}

/// Review Focus 3: a transfer started on a session lost for a network reason is queued at
/// once (the engine holds it until the site is back) and the session reconnects.
#[tokio::test]
async fn transfer_on_dropped_session_reconnects() {
    let (fake, mut app, _a, tab) = transfer_tab().await;
    let session = app.tab_conn(0).session().unwrap();
    let lost = Error::new(ErrorKind::Network, "Connection lost");
    fake.drop_connection(session, lost.clone());
    app.update(Message::SessionState {
        session,
        state: hoprs_core::ConnectionState::Disconnected(lost),
        auth: None,
    });
    let a_row = row_of(&app, Side::Local, "a.txt");
    app.update(pane_msg(tab, Side::Local, PaneMsg::Click(a_row)));
    let effects = app.update(pane_msg(tab, Side::Local, PaneMsg::Transfer));
    assert_eq!(enqueued(&effects).len(), 1, "{effects:?}");
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::Reconnect { session: s } if *s == session)),
        "{effects:?}"
    );
    assert!(app.tab_conn(0).reconnecting);
}

/// Task 5 fix round 1: a file operation held for a reconnection is dropped if its tab
/// closed meanwhile (never run behind the user's back).
#[tokio::test]
async fn held_file_op_of_a_closed_tab_is_dropped() {
    // An SSH site with a terminal and a files tab: the terminal keeps the site's
    // connection when the files tab closes.
    let mut a = site("a", "h", "u", Protocol::Ssh);
    a.ssh.open_as = hoprs_core::OpenAs::Both;
    a.remote_dir = Some("/srv".into());
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    fake.remote_fs(&a.id).add_dir("/srv/sub");
    fake.local_fs().add_dir("/home/me/dir");
    let mut app = connected_tab(&fake, &a).await;
    let index = app.tabs.iter().position(|t| t.is_files()).unwrap();
    assert_eq!(app.tabs.len(), 2);
    let tab = app.tabs[index].id;
    assert!(app.can_act(index, Side::Remote));
    let session = app.tab_conn(index).session().unwrap();
    let lost = Error::new(ErrorKind::Network, "Connection lost");
    fake.drop_connection(session, lost.clone());
    app.update(Message::SessionState {
        session,
        state: hoprs_core::ConnectionState::Disconnected(lost),
        auth: None,
    });
    app.update(pane_msg(tab, Side::Remote, PaneMsg::NewFolder));
    let effects = app.update(pane_msg(tab, Side::Remote, PaneMsg::EditCommit));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::Reconnect { session: s } if *s == session)),
        "{effects:?}"
    );
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::FileOp { .. })),
        "held: {effects:?}"
    );

    app.update(Message::CloseTab(tab));
    assert_eq!(app.tabs.len(), 1, "the terminal stays");
    let effects = app.update(Message::Reconnected {
        session,
        result: Ok(()),
    });
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::FileOp { .. })),
        "{effects:?}"
    );
}

/// Files dropped from the system on a site whose connection is down: nothing uploads (the
/// pane showed it takes no drop) and the queue says why.
#[tokio::test]
async fn os_drop_on_lost_session_does_nothing() {
    let (_fake, mut app, a, _tab) = transfer_tab().await;
    let session = app.conn(&a.id).session().unwrap();
    app.update(Message::SessionState {
        session,
        state: ConnectionState::Disconnected(Error::new(ErrorKind::Network, "Connection reset")),
        auth: None,
    });
    app.update(Message::FilesHovered(true));
    assert_eq!(app.os_drop_label(), None);
    let effects = app.update(Message::FileDropped(PathBuf::from("/home/me/a.txt")));
    assert!(no_enqueue(&effects));
    assert_eq!(
        app.queue.error.as_ref().map(|e| e.message.as_str()),
        Some("The site is not connected; reconnect it to drop files into it")
    );
}

/// A drag that carries nothing (the only selected row deselected by Ctrl+click) has no
/// target and drops nothing.
#[tokio::test]
async fn drag_with_nothing_selected_has_no_target() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    let b_row = row_of(&app, Side::Local, "b.txt");
    press(&mut app, tab, Side::Local, b_row, IN_LOCAL);
    app.update(Message::DragCancel);
    app.mods.ctrl = true;
    press(&mut app, tab, Side::Local, b_row, IN_LOCAL);
    app.mods.ctrl = false;
    assert_eq!(app.drag_count(), 0);
    move_over(&mut app, tab, Side::Remote, IN_REMOTE);
    assert!(app.drag.is_some_and(|d| d.active));
    assert_eq!((app.drop_target(), app.drop_label()), (None, None));
    assert!(app.drag_invalid_over(tab, Side::Remote));
    assert!(no_enqueue(&app.update(Message::DragEnd)));
}

/// A right press during a drag opens a menu: the first Escape closes the menu, the next
/// cancels the drag.
#[tokio::test]
async fn escape_closes_menu_before_drag() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    let b_row = row_of(&app, Side::Local, "b.txt");
    press(&mut app, tab, Side::Local, b_row, IN_LOCAL);
    move_over(&mut app, tab, Side::Remote, IN_REMOTE);
    app.update(Message::PointerPressed(IN_REMOTE));
    app.update(pane_msg(tab, Side::Remote, PaneMsg::ContextMenu(None)));
    assert!(app.menu.is_some());
    let escape = Message::KeyPressed {
        key: FileKey::Escape,
        shift: false,
    };
    app.update(escape.clone());
    assert!(app.menu.is_none());
    assert!(app.drag.is_some_and(|d| d.active));
    app.update(escape);
    assert_eq!(app.drag, None);
}

/// A dialog or the palette opened during a drag: Escape closes it first, the next one
/// cancels the drag.
#[tokio::test]
async fn escape_closes_dialog_or_palette_before_drag() {
    let (_fake, mut app, _a, tab) = transfer_tab().await;
    let b_row = row_of(&app, Side::Local, "b.txt");
    let escape = Message::KeyPressed {
        key: FileKey::Escape,
        shift: false,
    };
    // The palette.
    press(&mut app, tab, Side::Local, b_row, IN_LOCAL);
    move_over(&mut app, tab, Side::Remote, IN_REMOTE);
    app.update(Message::Palette(crate::app::palette::PaletteMsg::Open));
    assert!(app.palette.is_some());
    app.update(escape.clone());
    assert!(app.palette.is_none());
    assert!(app.drag.is_some_and(|d| d.active));
    app.update(escape.clone());
    assert_eq!(app.drag, None);
    // A security dialog.
    press(&mut app, tab, Side::Local, b_row, IN_LOCAL);
    move_over(&mut app, tab, Side::Remote, IN_REMOTE);
    app.update(super::menu::host_key_prompt(1));
    assert!(app.dialogs.is_open());
    app.update(escape.clone());
    assert!(!app.dialogs.is_open());
    assert!(app.drag.is_some_and(|d| d.active));
    app.update(escape);
    assert_eq!(app.drag, None);
}

// Plan 14, Task 2: a finished transfer re-lists its destination folder where it is shown.

/// A queued file transfer of `site` (`local` <-> `remote`).
fn moving(id: i64, site: &SiteId, direction: Direction, local: &str, remote: &str) -> TransferItem {
    TransferItem {
        id: TransferId(id),
        parent: None,
        spec: spec(site, direction, local, remote),
        is_dir: Some(false),
        size: Some(1),
        bytes_done: 0,
        state: TransferState::Queued,
        origin: Origin::User,
        tree_policy: None,
    }
}

/// How many re-list timers `effects` start.
fn relist_ticks(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| {
            matches!(e, Effect::Later { after, message }
                if *after == crate::app::transfers::RELIST_DELAY
                    && matches!(**message, Message::RelistPanes))
        })
        .count()
}

/// The `(tab, side, folder)` of each listing among `effects`.
fn listed_panes(effects: &[Effect]) -> Vec<(TabId, Side, String)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::List {
                tab, side, path, ..
            } => Some((*tab, *side, path.to_string())),
            _ => None,
        })
        .collect()
}

fn finish(app: &mut App, seq: u64, id: i64, state: TransferState) -> Vec<Effect> {
    app.update(Message::TransferEvent(changed(seq, id, state)))
}

/// Delivers the results of the listings among `effects`; returns what they caused.
async fn deliver(app: &mut App, fake: &Arc<FakeBackend>, effects: &[Effect]) -> Vec<Effect> {
    let mut after = Vec::new();
    for message in run_listings(fake, effects).await {
        after.extend(app.update(message));
    }
    after
}

fn touch_remote(fake: &FakeBackend, site: &SiteId, path: &str) {
    fake.remote_fs(site)
        .add_file(path, b"x", std::time::SystemTime::UNIX_EPOCH);
}

/// An upload's Done re-lists the remote pane showing its folder, a download's the local
/// one: once, after the batching delay, quietly (rows kept on screen meanwhile, no scroll
/// to the top, no comparison re-armed).
#[tokio::test]
async fn done_item_relists_the_pane_showing_its_folder() {
    let (fake, mut app, a, tab) = transfer_tab().await;
    synced(
        &mut app,
        1,
        vec![
            moving(1, &a.id, Direction::Upload, "/home/me/a.txt", "/srv/a.txt"),
            moving(
                2,
                &a.id,
                Direction::Download,
                "/home/me/r2.txt",
                "/srv/r.txt",
            ),
        ],
    );
    touch_remote(&fake, &a.id, "/srv/a.txt");
    let effects = finish(&mut app, 2, 1, TransferState::Done);
    assert_eq!(relist_ticks(&effects), 1, "{effects:?}");
    assert!(
        listings(&effects).is_empty(),
        "nothing lists before the delay"
    );

    let effects = app.update(Message::RelistPanes);
    assert_eq!(listed_panes(&effects), [(tab, Side::Remote, "/srv".into())]);
    assert_eq!(
        app.tabs[0].remote().listing,
        Listing::Loaded,
        "no loading flash"
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::ScrollPane { .. })),
        "{effects:?}"
    );
    let after = deliver(&mut app, &fake, &effects).await;
    assert!(after.is_empty(), "no comparison re-armed: {after:?}");
    assert!(entry_names(&app, 0, Side::Remote).contains(&"a.txt".to_string()));

    fake.local_fs()
        .add_file("/home/me/r2.txt", b"r", std::time::SystemTime::UNIX_EPOCH);
    let effects = finish(&mut app, 3, 2, TransferState::Done);
    assert_eq!(relist_ticks(&effects), 1);
    let effects = app.update(Message::RelistPanes);
    assert_eq!(
        listed_panes(&effects),
        [(tab, Side::Local, "/home/me".into())]
    );
    deliver(&mut app, &fake, &effects).await;
    assert!(entry_names(&app, 0, Side::Local).contains(&"r2.txt".to_string()));
}

/// Another folder, the other side, another site: nothing re-lists. Nor does a folder the
/// pane left before the delay ran out (Review Focus 2).
#[tokio::test]
async fn other_folder_other_side_other_site_not_relisted() {
    let (fake, mut app, a, tab) = transfer_tab().await;
    let b = site("b", "h2", "u", Protocol::Ftp);
    synced(
        &mut app,
        1,
        vec![
            moving(
                1,
                &a.id,
                Direction::Upload,
                "/home/me/a.txt",
                "/srv/sub/a.txt",
            ),
            // The remote side of `/home/me` (the local pane's folder).
            moving(2, &a.id, Direction::Upload, "/x/a.txt", "/home/me/a.txt"),
            // The local side of `/srv` (the remote pane's folder).
            moving(3, &a.id, Direction::Download, "/srv/r.txt", "/x/r.txt"),
            // Another site's `/srv`.
            moving(4, &b.id, Direction::Upload, "/home/me/a.txt", "/srv/a.txt"),
            moving(5, &a.id, Direction::Upload, "/home/me/b.txt", "/srv/b.txt"),
        ],
    );
    for (seq, id) in [(2, 1), (3, 2), (4, 3), (5, 4)] {
        let effects = finish(&mut app, seq, id, TransferState::Done);
        assert_eq!(relist_ticks(&effects), 0, "item {id}: {effects:?}");
    }
    assert!(listings(&app.update(Message::RelistPanes)).is_empty());

    // Into `/srv`, but the pane goes to `/srv/sub` before the delay ends.
    assert_eq!(
        relist_ticks(&finish(&mut app, 6, 5, TransferState::Done)),
        1
    );
    let effects = app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::Navigate(rpath("/srv/sub")),
    ));
    settle(&mut app, &fake, run_listings(&fake, &effects).await).await;
    assert_eq!(app.tabs[0].remote().path, rpath("/srv/sub"));
    let effects = app.update(Message::RelistPanes);
    assert!(listings(&effects).is_empty(), "{effects:?}");
    assert_eq!(relist_ticks(&effects), 0, "nothing left waiting");
}

#[tokio::test]
async fn failed_cancelled_skipped_do_not_relist() {
    let (_fake, mut app, a, _tab) = transfer_tab().await;
    synced(
        &mut app,
        1,
        (1..=3)
            .map(|i| moving(i, &a.id, Direction::Upload, "/home/me/a.txt", "/srv/a.txt"))
            .collect(),
    );
    let ends = [
        failed("denied"),
        TransferState::Cancelled,
        TransferState::Skipped("exists".into()),
    ];
    for (i, state) in ends.into_iter().enumerate() {
        let id = i as i64 + 1;
        let effects = finish(&mut app, id as u64 + 1, id, state);
        assert_eq!(relist_ticks(&effects), 0, "item {id}: {effects:?}");
    }
    assert!(listings(&app.update(Message::RelistPanes)).is_empty());
}

/// Review Focus 1: ten items finishing together give one timer and one listing; more
/// finishing while that listing is out wait for it, then give one more.
#[tokio::test]
async fn burst_of_done_items_gives_one_listing() {
    let (fake, mut app, a, tab) = transfer_tab().await;
    synced(
        &mut app,
        1,
        (1..=20)
            .map(|i| {
                moving(
                    i,
                    &a.id,
                    Direction::Upload,
                    &format!("/home/me/f{i}"),
                    &format!("/srv/f{i}"),
                )
            })
            .collect(),
    );
    let mut ticks = 0;
    for id in 1..=10 {
        touch_remote(&fake, &a.id, &format!("/srv/f{id}"));
        ticks += relist_ticks(&finish(&mut app, id as u64 + 1, id, TransferState::Done));
    }
    assert_eq!(ticks, 1);
    let first = app.update(Message::RelistPanes);
    assert_eq!(listed_panes(&first), [(tab, Side::Remote, "/srv".into())]);

    // The listing is still out.
    let mut ticks = 0;
    for id in 11..=20 {
        touch_remote(&fake, &a.id, &format!("/srv/f{id}"));
        ticks += relist_ticks(&finish(&mut app, id as u64 + 1, id, TransferState::Done));
    }
    assert_eq!(ticks, 1);
    let effects = app.update(Message::RelistPanes);
    assert!(listings(&effects).is_empty(), "never two listings at once");
    assert_eq!(relist_ticks(&effects), 1, "it waits for the listing out");
    deliver(&mut app, &fake, &first).await;
    let effects = app.update(Message::RelistPanes);
    assert_eq!(listed_panes(&effects), [(tab, Side::Remote, "/srv".into())]);
    deliver(&mut app, &fake, &effects).await;
    assert_eq!(app.tabs[0].remote().entries.len(), 22);
    assert_eq!(relist_ticks(&app.update(Message::RelistPanes)), 0);
}

/// A listing the user started (F5) is out when the item ends: the re-list follows it.
#[tokio::test]
async fn listing_in_flight_defers_the_relist() {
    let (fake, mut app, a, tab) = transfer_tab().await;
    synced(
        &mut app,
        1,
        vec![moving(
            1,
            &a.id,
            Direction::Upload,
            "/home/me/a.txt",
            "/srv/a.txt",
        )],
    );
    let refresh = app.update(pane_msg(tab, Side::Remote, PaneMsg::Refresh));
    assert_eq!(listings(&refresh).len(), 1);
    assert_eq!(
        relist_ticks(&finish(&mut app, 2, 1, TransferState::Done)),
        1
    );
    let effects = app.update(Message::RelistPanes);
    assert!(listings(&effects).is_empty(), "{effects:?}");
    assert_eq!(relist_ticks(&effects), 1);
    touch_remote(&fake, &a.id, "/srv/a.txt");
    settle(&mut app, &fake, run_listings(&fake, &refresh).await).await;
    let effects = app.update(Message::RelistPanes);
    assert_eq!(listed_panes(&effects), [(tab, Side::Remote, "/srv".into())]);
}

/// Review Focus 4: the selection and the scroll survive a re-list (by name: the rows
/// moved); an open rename editor holds the re-list back until it closes, its typed text
/// untouched.
#[tokio::test]
async fn relist_keeps_selection_and_rename_editor() {
    let (fake, mut app, a, tab) = transfer_tab().await;
    synced(
        &mut app,
        1,
        vec![
            moving(1, &a.id, Direction::Upload, "/home/me/a.txt", "/srv/a.txt"),
            moving(2, &a.id, Direction::Upload, "/home/me/b.txt", "/srv/b.txt"),
        ],
    );
    let r_row = row_of(&app, Side::Remote, "r.txt");
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(r_row)));
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Hover(r_row)));
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::Scrolled {
            offset_y: 12.0,
            viewport_h: 400.0,
        },
    ));
    touch_remote(&fake, &a.id, "/srv/a.txt");
    finish(&mut app, 2, 1, TransferState::Done);
    let effects = app.update(Message::RelistPanes);
    assert_eq!(listings(&effects).len(), 1);
    deliver(&mut app, &fake, &effects).await;
    let moved = row_of(&app, Side::Remote, "r.txt");
    assert_ne!(moved, r_row, "a.txt sorts before r.txt");
    let pane = app.tabs[0].remote();
    assert_eq!(
        pane.selection.rows.iter().copied().collect::<Vec<_>>(),
        [moved]
    );
    assert_eq!(pane.selection.cursor, Some(moved));
    assert_eq!(pane.offset_y, 12.0);
    // The hover is where the pointer is: the same row, now another entry.
    assert_eq!(pane.hovered, Some(r_row));

    // F2 on r.txt, a name typed; then b.txt lands.
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Rename));
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::EditChanged("typed".into()),
    ));
    touch_remote(&fake, &a.id, "/srv/b.txt");
    finish(&mut app, 3, 2, TransferState::Done);
    let effects = app.update(Message::RelistPanes);
    assert!(listings(&effects).is_empty(), "the editor holds it back");
    assert_eq!(relist_ticks(&effects), 1);
    let typed = |app: &App| match &app.tabs[0].remote().action {
        Some(crate::app::actions::PaneAction::Edit(edit)) => Some(edit.value.clone()),
        _ => None,
    };
    assert_eq!(typed(&app).as_deref(), Some("typed"));
    app.update(pane_msg(tab, Side::Remote, PaneMsg::CancelAction));
    let effects = app.update(Message::RelistPanes);
    assert_eq!(listings(&effects).len(), 1);
    deliver(&mut app, &fake, &effects).await;
    assert!(entry_names(&app, 0, Side::Remote).contains(&"b.txt".to_string()));
}

/// Review Focus 3: two tabs of the site on `/srv` both re-list; another site's tab on
/// `/srv` does not. Local folders are the same machine for every tab.
#[tokio::test]
async fn same_remote_folder_in_two_tabs_both_relist() {
    // Several tabs share a session on an SSH site.
    let mut a = site("a", "h", "u", Protocol::Ssh);
    a.remote_dir = Some("/srv".into());
    let mut b = site("b", "h2", "u", Protocol::Ftp);
    b.remote_dir = Some("/srv".into());
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()).with_site(b.clone()));
    fake.remote_fs(&a.id).add_dir("/srv");
    fake.remote_fs(&b.id).add_dir("/srv");
    let mut app = connected_tab(&fake, &a).await;
    let first = app.tabs[0].id;
    let effects = app.update(Message::NewTab {
        tab: first,
        kind: hoprs_core::TabKind::Files,
    });
    settle(&mut app, &fake, run_listings(&fake, &effects).await).await;
    let effects = app.update(Message::ConnectSite(b.id.clone()));
    settle(&mut app, &fake, run_listings(&fake, &effects).await).await;
    let backend: Arc<dyn BackendApi> = fake.clone();
    let messages: Vec<Message> = effects::connect_sites(backend, vec![b.id.clone()])
        .collect()
        .await;
    settle(&mut app, &fake, messages).await;
    assert_eq!(
        app.tabs.len(),
        3,
        "{:?}",
        app.tabs
            .iter()
            .map(|t| (t.site.clone(), t.kind()))
            .collect::<Vec<_>>()
    );
    let of_a: Vec<TabId> = app
        .tabs
        .iter()
        .filter(|t| t.site == a.id)
        .map(|t| t.id)
        .collect();
    assert_eq!(of_a.len(), 2);
    for t in &app.tabs {
        assert_eq!(t.remote().path, rpath("/srv"));
        assert_eq!(t.remote().listing, Listing::Loaded);
    }

    synced(
        &mut app,
        1,
        vec![
            moving(1, &a.id, Direction::Upload, "/home/me/a.txt", "/srv/a.txt"),
            moving(
                2,
                &b.id,
                Direction::Download,
                "/home/me/z.txt",
                "/srv/z.txt",
            ),
        ],
    );
    assert_eq!(
        relist_ticks(&finish(&mut app, 2, 1, TransferState::Done)),
        1
    );
    let mut got = listed_panes(&app.update(Message::RelistPanes));
    got.sort_by_key(|(t, _, _)| t.0);
    let want: Vec<_> = of_a
        .iter()
        .map(|t| (*t, Side::Remote, "/srv".to_string()))
        .collect();
    assert_eq!(got, want);

    // A download lands in `/home/me`, which every tab's local pane shows.
    assert_eq!(
        relist_ticks(&finish(&mut app, 3, 2, TransferState::Done)),
        1
    );
    let effects = app.update(Message::RelistPanes);
    let got = listed_panes(&effects);
    assert_eq!(got.len(), 3, "{got:?}");
    assert!(
        got.iter()
            .all(|(_, side, path)| *side == Side::Local && path == "/home/me")
    );
}

/// Endings only a snapshot shows (the events were lost to a lag) re-list too; the first
/// snapshot, the queue's state at start, does not.
#[tokio::test]
async fn done_seen_only_in_a_snapshot_relists() {
    let (_fake, mut app, a, _tab) = transfer_tab().await;
    let mut done = moving(1, &a.id, Direction::Upload, "/home/me/a.txt", "/srv/a.txt");
    done.state = TransferState::Done;
    // `synced` asserts the first snapshot causes nothing.
    synced(
        &mut app,
        1,
        vec![
            done,
            moving(2, &a.id, Direction::Upload, "/home/me/b.txt", "/srv/b.txt"),
        ],
    );
    let request = snapshot_request(&app.update(Message::QueueResync));
    let mut now = moving(2, &a.id, Direction::Upload, "/home/me/b.txt", "/srv/b.txt");
    now.state = TransferState::Done;
    let mut still = moving(1, &a.id, Direction::Upload, "/home/me/a.txt", "/srv/a.txt");
    still.state = TransferState::Done;
    let effects = app.update(Message::QueueSnapshot {
        request,
        snapshot: snapshot(5, vec![still, now]),
    });
    assert_eq!(relist_ticks(&effects), 1, "{effects:?}");
    assert_eq!(listed_panes(&app.update(Message::RelistPanes)).len(), 1);
}

/// The single listing among `effects`, as `(tab, side, request)`.
fn the_listing(effects: &[Effect]) -> (TabId, Side, RequestId) {
    match listings(effects)[..] {
        [(side, request, _, _)] => {
            let tab = effects
                .iter()
                .find_map(|e| match e {
                    Effect::List { tab, .. } => Some(*tab),
                    _ => None,
                })
                .unwrap();
            (tab, side, request)
        }
        _ => panic!("expected one listing: {effects:?}"),
    }
}

/// A background re-list that fails (a transient error) leaves the rows, the state and the
/// selection as they were; only a folder that is gone shows its failure.
#[tokio::test]
async fn failed_background_relist_keeps_the_rows_unless_the_folder_is_gone() {
    let (_fake, mut app, a, tab) = transfer_tab().await;
    synced(
        &mut app,
        1,
        vec![
            moving(1, &a.id, Direction::Upload, "/home/me/a.txt", "/srv/a.txt"),
            moving(2, &a.id, Direction::Upload, "/home/me/b.txt", "/srv/b.txt"),
        ],
    );
    let r_row = row_of(&app, Side::Remote, "r.txt");
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(r_row)));
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Hover(r_row)));
    let before = app.tabs[0].remote().entries.clone();

    finish(&mut app, 2, 1, TransferState::Done);
    let (t, side, request_id) = the_listing(&app.update(Message::RelistPanes));
    let effects = app.update(Message::Listed {
        tab: t,
        side,
        request_id,
        result: Err(Error::new(ErrorKind::Network, "connection reset")),
    });
    assert!(effects.is_empty(), "{effects:?}");
    let pane = app.tabs[0].remote();
    assert_eq!(pane.listing, Listing::Loaded);
    assert_eq!(pane.entries, before);
    assert_eq!(
        pane.selection.rows.iter().copied().collect::<Vec<_>>(),
        [r_row]
    );
    assert_eq!(pane.hovered, Some(r_row));
    assert_eq!(pane.request, None);

    finish(&mut app, 3, 2, TransferState::Done);
    let (t, side, request_id) = the_listing(&app.update(Message::RelistPanes));
    // As for any failed listing, an Open's suppression ends once nothing is pending.
    app.tabs[0].compare_mut().unwrap().suppress_auto = true;
    app.update(Message::Listed {
        tab: t,
        side,
        request_id,
        result: Err(Error::new(ErrorKind::NotFound, "no such folder")),
    });
    assert!(matches!(
        app.tabs[0].remote().listing,
        Listing::Failed(ref e) if e.kind == ErrorKind::NotFound
    ));
    assert!(!app.tabs[0].compare().unwrap().suppress_auto);
}

/// A menu opened on a row while a background re-list is out stays open on the same entry
/// (its row number follows the entry); one whose entry is gone closes.
#[tokio::test]
async fn background_relist_keeps_an_open_menu() {
    let (fake, mut app, a, tab) = transfer_tab().await;
    synced(
        &mut app,
        1,
        vec![
            moving(1, &a.id, Direction::Upload, "/home/me/a.txt", "/srv/a.txt"),
            moving(2, &a.id, Direction::Upload, "/home/me/b.txt", "/srv/b.txt"),
        ],
    );
    touch_remote(&fake, &a.id, "/srv/a.txt");
    finish(&mut app, 2, 1, TransferState::Done);
    let effects = app.update(Message::RelistPanes);
    assert_eq!(listings(&effects).len(), 1);
    let r_row = row_of(&app, Side::Remote, "r.txt");
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::ContextMenu(Some(r_row)),
    ));
    assert!(app.menu.is_some());
    deliver(&mut app, &fake, &effects).await;
    let moved = row_of(&app, Side::Remote, "r.txt");
    assert!(
        matches!(
            &app.menu,
            Some(crate::app::menu::ContextMenu {
                target: crate::app::menu::MenuTarget::Pane { row: Some(r), .. },
                ..
            }) if *r == moved
        ),
        "{:?}",
        app.menu
    );

    // The entry goes away meanwhile: the menu closes.
    app.menu = None;
    touch_remote(&fake, &a.id, "/srv/b.txt");
    finish(&mut app, 3, 2, TransferState::Done);
    let effects = app.update(Message::RelistPanes);
    assert_eq!(listings(&effects).len(), 1);
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::ContextMenu(Some(moved)),
    ));
    hoprs_remote::RemoteFs::remove_file(&fake.remote_fs(&a.id), &rpath("/srv/r.txt"))
        .await
        .unwrap();
    deliver(&mut app, &fake, &effects).await;
    assert!(app.menu.is_none());
}

/// The hover stays on the pointer's row through a background re-list; a row the shorter
/// list no longer has is hovered no more.
#[tokio::test]
async fn background_relist_drops_a_hover_past_the_last_row() {
    let (fake, mut app, a, tab) = transfer_tab().await;
    synced(
        &mut app,
        1,
        vec![moving(
            1,
            &a.id,
            Direction::Upload,
            "/home/me/a.txt",
            "/srv/a.txt",
        )],
    );
    finish(&mut app, 2, 1, TransferState::Done);
    let effects = app.update(Message::RelistPanes);
    assert_eq!(listings(&effects).len(), 1);
    let past = app.tabs[0].remote().row_count() + 3;
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Hover(past)));
    deliver(&mut app, &fake, &effects).await;
    assert_eq!(app.tabs[0].remote().hovered, None);
}

/// A pane whose last background re-list was slow waits at least that long before the
/// next one (FTP during a burst).
#[tokio::test]
async fn a_slow_relist_lengthens_the_next_delay() {
    let (_fake, mut app, a, _tab) = transfer_tab().await;
    synced(
        &mut app,
        1,
        vec![
            moving(1, &a.id, Direction::Upload, "/home/me/a.txt", "/srv/a.txt"),
            moving(2, &a.id, Direction::Upload, "/home/me/b.txt", "/srv/b.txt"),
        ],
    );
    let effects = finish(&mut app, 2, 1, TransferState::Done);
    assert_eq!(relist_ticks(&effects), 1);
    let (t, side, request_id) = the_listing(&app.update(Message::RelistPanes));
    app.update(Message::Listed {
        tab: t,
        side,
        request_id,
        result: Ok(Vec::new()),
    });
    let slow = std::time::Duration::from_secs(3);
    app.tabs[0].pane_mut(Side::Remote).unwrap().soft_took = slow;
    let effects = finish(&mut app, 3, 2, TransferState::Done);
    let delays: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Later { after, message } if matches!(**message, Message::RelistPanes) => {
                Some(*after)
            }
            _ => None,
        })
        .collect();
    assert_eq!(delays, [slow]);
}
