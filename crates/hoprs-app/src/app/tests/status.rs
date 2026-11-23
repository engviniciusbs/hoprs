//! Task 10: the status bar, connection states and the reconnect banner, saved tabs per
//! workspace, and the orderly shutdown on window close (Review Focus 5).

use std::path::PathBuf;

use hoprs_core::{ConnectionState, SavedTab, TabKind};
use hoprs_ssh::AuthMethod;

use super::*;
use crate::backend::fake::Call;
use crate::ui::status;

fn path(s: &str) -> RemotePath {
    RemotePath::new(s).unwrap()
}

fn files_tab(local: Option<&str>, remote: Option<&str>) -> SavedTab {
    SavedTab {
        kind: TabKind::Files,
        remote_dir: remote.map(str::to_owned),
        local_dir: local.map(PathBuf::from),
    }
}

/// The session of the site's tab, once connected.
fn session_of(app: &App, site: &SiteId) -> SessionId {
    app.tab_conn(app.tab_of(site).unwrap())
        .session()
        .expect("the tab is connected")
}

fn state(app: &mut App, session: SessionId, state: ConnectionState) -> Vec<Effect> {
    app.update(Message::SessionState {
        session,
        state,
        auth: None,
    })
}

fn select(app: &mut App, site: &SiteId) {
    let index = app.tab_of(site).unwrap();
    app.update(Message::SelectTab(index));
}

type Feed = std::pin::Pin<Box<dyn futures_util::Stream<Item = Message> + Send>>;

/// The feed's next message (`None` once it ended); never waits more than 5 s.
async fn next(feed: &mut Feed) -> Option<Message> {
    tokio::time::timeout(Duration::from_secs(5), feed.next())
        .await
        .expect("the feed answers")
}

/// The pane's folder and the path of the listing effect for it, if any.
fn listed_path(effects: &[Effect], tab: TabId, side: Side) -> Option<RemotePath> {
    effects.iter().find_map(|e| match e {
        Effect::List {
            tab: t,
            side: s,
            path,
            ..
        } if *t == tab && *s == side => Some(path.clone()),
        _ => None,
    })
}

#[tokio::test]
async fn status_texts_for_each_state() {
    let mut a = site("Shop prod", "203.0.113.4", "deploy", Protocol::Ssh);
    a.port = 22;
    let b = site("Blog", "ftp.blog.net", "writer", Protocol::Ftp);
    let c = site(
        "Shop assets",
        "files.shop.net",
        "up",
        Protocol::FtpsExplicit,
    );
    let ws = workspace("Shop", &[&a.id, &b.id, &c.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_site(b.clone())
            .with_site(c.clone())
            .with_workspace(ws.clone()),
    );
    let mut app = loaded(&fake).await;

    // Nothing open: only the state.
    let line = status::line(&app);
    assert_eq!(line.state.as_deref(), Some(strings::not_connected()));
    assert_eq!(
        (line.endpoint, line.auth, line.workspace),
        (None, None, None)
    );

    let sites = connect_effect(app.update(Message::OpenWorkspace(ws.id.clone())));
    let line = status::line(&app);
    assert_eq!(line.state.as_deref(), Some("Connecting…"));
    assert_eq!(line.endpoint.as_deref(), Some("deploy@203.0.113.4:22"));
    assert_eq!(line.auth, None, "nothing authenticated yet");
    assert_eq!(line.workspace.as_deref(), Some("0 of 3 sites in Shop"));

    drive(&mut app, &fake, sites).await;
    let session = session_of(&app, &a.id);
    let line = status::line(&app);
    assert_eq!(line.state.as_deref(), Some("Connected"));
    assert_eq!(line.workspace.as_deref(), Some("3 of 3 sites in Shop"));

    // The session's feed: the method that logged in, then each state.
    app.update(Message::SessionState {
        session,
        state: ConnectionState::Connected,
        auth: Some(AuthMethod::Agent {
            key_type: "ed25519".into(),
        }),
    });
    assert_eq!(
        status::line(&app).auth.as_deref(),
        Some("ed25519 via ssh-agent")
    );
    state(
        &mut app,
        session,
        ConnectionState::Reconnecting { attempt: 2 },
    );
    let line = status::line(&app);
    assert_eq!(line.state.as_deref(), Some("Reconnecting (2/5)"));
    assert_eq!(line.workspace.as_deref(), Some("2 of 3 sites in Shop"));
    assert_eq!(
        line.auth.as_deref(),
        Some("ed25519 via ssh-agent"),
        "kept while reconnecting"
    );
    state(&mut app, session, ConnectionState::WaitingForUser);
    assert_eq!(status::line(&app).state.as_deref(), Some("Waiting for you"));
    let lost = Error::new(ErrorKind::Network, "Connection lost");
    state(&mut app, session, ConnectionState::Disconnected(lost));
    assert_eq!(
        status::line(&app).state.as_deref(),
        Some("Disconnected — Connection lost")
    );
    // A state for a session no tab has (closed meanwhile) changes nothing.
    assert!(state(&mut app, SessionId(999), ConnectionState::Connected).is_empty());
    assert_eq!(
        status::line(&app).state.as_deref(),
        Some("Disconnected — Connection lost")
    );

    // The other methods.
    for (method, label) in [
        (AuthMethod::Password, "password"),
        (
            AuthMethod::KeyFile {
                key_type: "rsa".into(),
            },
            "rsa key",
        ),
        (AuthMethod::KeyboardInteractive, "keyboard-interactive"),
    ] {
        app.update(Message::SessionState {
            session,
            state: ConnectionState::Connected,
            auth: Some(method),
        });
        assert_eq!(status::line(&app).auth.as_deref(), Some(label));
    }

    // FTP: the protocol is the method.
    select(&mut app, &b.id);
    let line = status::line(&app);
    assert_eq!(line.auth.as_deref(), Some("FTP (unencrypted)"));
    assert_eq!(line.endpoint.as_deref(), Some("writer@ftp.blog.net:21"));
    select(&mut app, &c.id);
    assert_eq!(status::line(&app).auth.as_deref(), Some("FTPS explicit"));

    // A first connect that failed reads as disconnected, with its reason.
    let d = site("d", "d.example", "u", Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(d.clone()));
    let mut app = loaded(&fake).await;
    fake.fail_next_connect(&d.id, Error::new(ErrorKind::Auth, "Authentication failed"));
    let sites = connect_effect(app.update(Message::ConnectSite(d.id.clone())));
    drive(&mut app, &fake, sites).await;
    let line = status::line(&app);
    assert_eq!(
        line.state.as_deref(),
        Some("Disconnected — Authentication failed")
    );
    assert_eq!(line.workspace, None, "no workspace open");
}

#[tokio::test]
async fn reconnect_banner_on_disconnected() {
    let a = site("a", "a.example", "u", Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(a.id.clone())));
    drive(&mut app, &fake, sites).await;
    let session = session_of(&app, &a.id);
    let tab = app.tabs[0].id;
    assert_eq!(app.watched_sessions(), vec![session]);

    // The session's feed reports what the backend's watch says, as it changes.
    let backend: Arc<dyn BackendApi> = fake.clone();
    fake.set_auth(session, AuthMethod::Password);
    let mut feed: Feed = Box::pin(effects::session_feed(backend.clone(), session));
    let first = next(&mut feed).await.unwrap();
    assert!(matches!(
        &first,
        Message::SessionState {
            session: s,
            state: ConnectionState::Connected,
            auth: Some(AuthMethod::Password),
        } if *s == session
    ));
    app.update(first);
    assert_eq!(
        app.tab_conn(0).lost_connection(),
        None,
        "no banner while connected"
    );
    assert_eq!(status::line(&app).auth.as_deref(), Some("password"));

    // The connection drops for good: the banner shows why, with Reconnect.
    let lost = Error::new(ErrorKind::Network, "Connection lost");
    fake.drop_connection(session, lost.clone());
    let dropped = next(&mut feed).await.unwrap();
    app.update(dropped);
    assert_eq!(app.tab_conn(0).lost_connection(), Some(&lost));

    // Reconnect goes through the backend's reconnect (one at a time).
    let effects = app.update(Message::ReconnectSession(tab));
    let [Effect::Reconnect { session: s }] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(*s, session);
    assert!(app.tab_conn(0).reconnecting);
    assert!(app.update(Message::ReconnectSession(tab)).is_empty());
    let done = effects::reconnect(backend.clone(), session).await;
    assert!(fake.calls().contains(&Call::Reconnect(session)));
    let effects = app.update(done);
    assert!(!app.tab_conn(0).reconnecting);
    assert_eq!(app.tab_conn(0).lost_connection(), None);
    // The remote folder is listed again.
    assert_eq!(
        listed_path(&effects, tab, Side::Remote),
        Some(app.tabs[0].remote().path.clone())
    );
    let up = next(&mut feed).await.unwrap();
    assert!(matches!(
        up,
        Message::SessionState {
            state: ConnectionState::Connected,
            ..
        }
    ));

    // A failed reconnect keeps the banner, with the new reason.
    fake.drop_connection(session, lost.clone());
    let refused = Error::new(ErrorKind::Auth, "Authentication failed");
    fake.fail_next_connect(&a.id, refused.clone());
    app.update(next(&mut feed).await.unwrap());
    app.update(Message::ReconnectSession(tab));
    let failed = effects::reconnect(backend.clone(), session).await;
    app.update(failed);
    assert_eq!(app.tab_conn(0).lost_connection(), Some(&refused));
    assert!(!app.tab_conn(0).reconnecting);

    // Closing the tab ends the watch: the app stops asking for it, and the backend's
    // stream ends with the session.
    app.update(Message::CloseTab(tab));
    assert!(app.watched_sessions().is_empty());
    fake.close(session).await;
    loop {
        match next(&mut feed).await {
            Some(Message::SessionState { .. }) => continue,
            None => break,
            Some(other) => panic!("{other:?}"),
        }
    }
    // A reconnect result for a session no tab has any more is dropped.
    assert!(
        app.update(Message::Reconnected {
            session,
            result: Ok(())
        })
        .is_empty()
    );
}

#[tokio::test]
async fn closing_workspace_saves_tabs() {
    let a = site("a", "a.example", "u", Protocol::Ssh);
    let b = site("b", "b.example", "u", Protocol::Ftp);
    let other = site("other", "o.example", "u", Protocol::Ssh);
    let mut ws = workspace("Shop", &[&a.id, &b.id]);
    // `b` had a saved tab: it has no tab now, so it keeps it.
    let kept = files_tab(Some("/srv/b-local"), Some("/b"));
    ws.sites[1].tabs = vec![kept.clone()];
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_site(b.clone())
            .with_site(other.clone())
            .with_workspace(ws.clone()),
    );
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::OpenWorkspace(ws.id.clone())));
    drive(&mut app, &fake, sites).await;
    assert_eq!(app.open_workspace, Some(ws.id.clone()));
    let sites = connect_effect(app.update(Message::ConnectSite(other.id.clone())));
    drive(&mut app, &fake, sites).await;
    // `b`'s tab is closed by hand; `a`'s panes move.
    let b_tab = app.tabs[app.tab_of(&b.id).unwrap()].id;
    app.update(Message::CloseTab(b_tab));
    let a_tab = app.tabs[app.tab_of(&a.id).unwrap()].id;
    app.update(Message::Pane {
        tab: a_tab,
        side: Side::Local,
        msg: PaneMsg::Navigate(path("/home/me/shop")),
    });
    app.update(Message::Pane {
        tab: a_tab,
        side: Side::Remote,
        msg: PaneMsg::Navigate(path("/var/www/shop")),
    });
    let a_session = session_of(&app, &a.id);
    let other_session = session_of(&app, &other.id);

    let effects = without_panes(app.update(Message::CloseWorkspace(ws.id.clone())));
    let saved = files_tab(Some("/home/me/shop"), Some("/var/www/shop"));
    let [
        Effect::SaveTabs { workspace, tabs },
        Effect::CloseSession(closed),
    ] = &effects[..]
    else {
        panic!("{effects:?}");
    };
    assert_eq!(workspace, &ws.id);
    assert_eq!(tabs, &vec![(a.id.clone(), vec![saved.clone()])]);
    assert_eq!(*closed, a_session);
    // The workspace's tabs are gone; the other site's tab stays.
    assert_eq!(app.open_workspace, None);
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.tab_conn(0).session(), Some(other_session));
    assert_eq!(status::line(&app).workspace, None);

    // The save goes to the vault; the profiles are read again after it.
    let done = effects::save_tabs(fake.clone(), ws.id.clone(), tabs.clone()).await;
    let stored = &fake.workspaces().await.unwrap()[0];
    assert_eq!(stored.sites[0].tabs, vec![saved]);
    assert_eq!(stored.sites[1].tabs, vec![kept]);
    assert!(matches!(app.update(done)[..], [Effect::LoadProfiles(_)]));

    // A failed save is logged only: nothing to reload, nothing shown.
    assert!(
        app.update(Message::TabsSaved(Err(Error::new(
            ErrorKind::Io,
            "Disk full"
        ))))
        .is_empty()
    );
    // Closing a workspace that is not open does nothing.
    assert!(
        app.update(Message::CloseWorkspace(ws.id.clone()))
            .is_empty()
    );
}

#[tokio::test]
async fn opening_workspace_restores_dirs() {
    let mut a = site("a", "a.example", "u", Protocol::Ssh);
    a.local_dir = Some(PathBuf::from("/site-default/local"));
    a.remote_dir = Some("/site-default/remote".into());
    let mut b = site("b", "b.example", "u", Protocol::Ftp);
    b.local_dir = Some(PathBuf::from("/b-local"));
    let c = site("c", "c.example", "u", Protocol::Ssh);
    let mut ws = workspace("Shop", &[&a.id, &b.id, &c.id]);
    ws.sites[0].tabs = vec![files_tab(Some("/home/me/shop"), Some("/var/www/shop"))];
    // Only a remote folder saved: the local side uses the site's.
    ws.sites[1].tabs = vec![files_tab(None, Some("/pub"))];
    // A saved terminal tab restores as a terminal tab (Plan 6), which takes no folder
    // from it.
    ws.sites[2].tabs = vec![SavedTab {
        kind: TabKind::Terminal,
        remote_dir: Some("/ignored".into()),
        local_dir: None,
    }];
    let other = workspace("Other", &[&a.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_site(b.clone())
            .with_site(c.clone())
            .with_workspace(ws.clone())
            .with_workspace(other.clone()),
    );
    let mut app = loaded(&fake).await;
    app.home = Some(path("/home/me"));

    let effects = app.update(Message::OpenWorkspace(ws.id.clone()));
    let tab = |app: &App, site: &SiteId| app.tab_of(site).unwrap();
    let (ia, ib, ic) = (tab(&app, &a.id), tab(&app, &b.id), tab(&app, &c.id));
    let (ta, tb, tc) = (&app.tabs[ia], &app.tabs[ib], &app.tabs[ic]);
    assert_eq!(ta.local().path, path("/home/me/shop"));
    assert_eq!(ta.remote().path, path("/var/www/shop"));
    assert_eq!(tb.local().path, path("/b-local"));
    assert_eq!(tb.remote().path, path("/pub"));
    assert_eq!(tc.kind(), TabKind::Terminal);
    // A terminal tab has no panes (and lists nothing).
    assert!(tc.files().is_none());
    assert_eq!(listed_path(&effects, tc.id, Side::Local), None);
    let ta = ta.id;
    // The local listings start at the restored folders.
    assert_eq!(
        listed_path(&effects, ta, Side::Local),
        Some(path("/home/me/shop"))
    );
    // Once connected, the remote side lists its restored folder.
    let sites = connect_effect(effects);
    let backend: Arc<dyn BackendApi> = fake.clone();
    let messages: Vec<Message> = effects::connect_sites(backend, sites).collect().await;
    let effects: Vec<Effect> = messages.into_iter().flat_map(|m| app.update(m)).collect();
    assert_eq!(
        listed_path(&effects, ta, Side::Remote),
        Some(path("/var/www/shop"))
    );

    // Opening another workspace saves the open one's tabs first; the tab already open
    // keeps its folders.
    let effects = app.update(Message::OpenWorkspace(other.id.clone()));
    let saves: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::SaveTabs { workspace, .. } => Some(workspace.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(saves, vec![ws.id.clone()]);
    assert_eq!(app.open_workspace, Some(other.id.clone()));
    assert_eq!(
        app.tabs[tab(&app, &a.id)].local().path,
        path("/home/me/shop")
    );
}

/// Review Focus 5: closing the window saves the open workspace's tabs, shuts the engine
/// down (its active items reopen paused), closes the sessions, and only then exits.
#[tokio::test]
async fn close_request_saves_then_shuts_down_in_order() {
    let a = site("a", "a.example", "u", Protocol::Ssh);
    let b = site("b", "b.example", "u", Protocol::Ftp);
    let ws = workspace("Shop", &[&a.id, &b.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_site(b.clone())
            .with_workspace(ws.clone()),
    );
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::OpenWorkspace(ws.id.clone())));
    drive(&mut app, &fake, sites).await;
    let a_tab = app.tabs[app.tab_of(&a.id).unwrap()].id;
    app.update(Message::Pane {
        tab: a_tab,
        side: Side::Remote,
        msg: PaneMsg::Navigate(path("/var/www")),
    });
    let mut sessions = [session_of(&app, &a.id), session_of(&app, &b.id)];
    sessions.sort();

    let window = iced::window::Id::unique();
    let effects = app.update(Message::CloseRequested(window));
    let [Effect::Shutdown { save, window: w }] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(*w, window);
    let (saved_ws, tabs) = save.clone().expect("the open workspace is saved");
    assert_eq!(saved_ws, ws.id);
    assert_eq!(tabs.len(), 2);
    assert_eq!(tabs[0].1[0].remote_dir.as_deref(), Some("/var/www"));
    assert!(app.closing);
    assert_eq!(
        status::line(&app).state.as_deref(),
        Some(strings::closing())
    );
    // (A second request while closing force-closes: see
    // `panicking_shutdown_and_second_request_still_exit`.)

    let backend: Arc<dyn BackendApi> = fake.clone();
    let outcome = effects::shutdown(backend.clone(), save.clone(), effects::SHUTDOWN_LIMIT).await;
    assert_eq!(outcome, effects::ShutdownOutcome::Done);
    // Exit comes last, once the backend is done.
    let exit = app.update(Message::ShutdownDone(window));
    assert!(matches!(exit[..], [Effect::ExitWindow(w)] if w == window));
    let mut expected = vec![Call::SaveTabs(ws.id.clone()), Call::Shutdown];
    expected.extend(sessions.iter().map(|s| Call::Close(*s)));
    assert_eq!(fake.calls(), expected);
    assert_eq!(
        fake.workspaces().await.unwrap()[0].sites[0].tabs[0]
            .remote_dir
            .as_deref(),
        Some("/var/www")
    );

    // A failed save never stops the shutdown; without a workspace nothing is saved.
    let fake = Arc::new(FakeBackend::new().with_workspace(ws.clone()));
    fake.fail_next_save_tabs(Error::new(ErrorKind::Io, "Disk full"));
    let backend: Arc<dyn BackendApi> = fake.clone();
    let outcome = effects::shutdown(backend, save.clone(), effects::SHUTDOWN_LIMIT).await;
    assert_eq!(outcome, effects::ShutdownOutcome::Done);
    assert_eq!(
        fake.calls(),
        vec![Call::SaveTabs(ws.id.clone()), Call::Shutdown]
    );
    let mut app = loaded(&FakeBackend::new()).await;
    let effects = app.update(Message::CloseRequested(window));
    assert!(matches!(
        &effects[..],
        [Effect::Shutdown { save: None, .. }]
    ));
}

/// The backend gets at most 5 s: a shutdown that hangs never keeps the window open.
#[tokio::test(start_paused = true)]
async fn close_times_out_after_5s() {
    assert_eq!(effects::SHUTDOWN_LIMIT, Duration::from_secs(5));
    let ws = WorkspaceId::new();
    let fake = Arc::new(FakeBackend::new());
    fake.hang_shutdown();
    let backend: Arc<dyn BackendApi> = fake.clone();
    let started = tokio::time::Instant::now();
    let outcome = effects::shutdown(backend, Some((ws, Vec::new())), effects::SHUTDOWN_LIMIT).await;
    let waited = started.elapsed();
    assert_eq!(outcome, effects::ShutdownOutcome::TimedOut);
    assert!(
        waited >= Duration::from_secs(5) && waited < Duration::from_millis(5100),
        "{waited:?}"
    );
    assert_eq!(
        fake.calls().last(),
        Some(&Call::Shutdown),
        "it got that far"
    );

    // The app exits all the same.
    let mut app = App::new();
    let window = iced::window::Id::unique();
    app.update(Message::CloseRequested(window));
    assert!(matches!(
        app.update(Message::ShutdownDone(window))[..],
        [Effect::ExitWindow(w)] if w == window
    ));
}

/// The `List` effect for one pane: its request and folder.
fn list_effect(effects: &[Effect], tab: TabId, side: Side) -> (RequestId, RemotePath) {
    effects
        .iter()
        .find_map(|e| match e {
            Effect::List {
                tab: t,
                side: s,
                request_id,
                path,
                ..
            } if *t == tab && *s == side => Some((*request_id, path.clone())),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no listing for {side:?} in {effects:?}"))
}

fn listed(
    app: &mut App,
    tab: TabId,
    side: Side,
    request_id: RequestId,
    result: Result<Vec<RemoteEntry>, Error>,
) -> Vec<Effect> {
    app.update(Message::Listed {
        tab,
        side,
        request_id,
        result,
    })
}

fn gone() -> Error {
    Error::new(ErrorKind::NotFound, "No such folder")
}

/// A save that hangs gets 2 s; the engine shutdown and the session closes still run,
/// and the window still closes.
#[tokio::test(start_paused = true)]
async fn hanging_save_still_shuts_down() {
    let a = site("a", "a.example", "u", Protocol::Ssh);
    let ws = workspace("Shop", &[&a.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_workspace(ws.clone()),
    );
    let session = fake.connect(a.id.clone()).await.unwrap().session;
    fake.hang_save_tabs();
    let backend: Arc<dyn BackendApi> = fake.clone();
    let started = tokio::time::Instant::now();
    let save = Some((ws.id.clone(), vec![(a.id.clone(), Vec::new())]));
    let outcome = effects::shutdown(backend, save, effects::SHUTDOWN_LIMIT).await;
    assert_eq!(outcome, effects::ShutdownOutcome::Done);
    let waited = started.elapsed();
    assert_eq!(effects::SAVE_LIMIT, Duration::from_secs(2));
    assert!(
        waited >= effects::SAVE_LIMIT && waited < Duration::from_millis(2100),
        "{waited:?}"
    );
    assert_eq!(
        fake.calls(),
        vec![
            Call::SaveTabs(ws.id.clone()),
            Call::Shutdown,
            Call::Close(session)
        ]
    );
    let mut app = App::new();
    let window = iced::window::Id::unique();
    app.update(Message::CloseRequested(window));
    assert!(matches!(
        app.update(Message::ShutdownDone(window))[..],
        [Effect::ExitWindow(w)] if w == window
    ));
}

/// A shutdown that panics still ends in `ShutdownDone` (an outcome, not a lost
/// message), and a second close request closes at once.
#[tokio::test]
async fn panicking_shutdown_and_second_request_still_exit() {
    let fake = Arc::new(FakeBackend::new());
    fake.panic_shutdown();
    let backend: Arc<dyn BackendApi> = fake.clone();
    let outcome = effects::shutdown(backend, None, effects::SHUTDOWN_LIMIT).await;
    assert_eq!(outcome, effects::ShutdownOutcome::Failed);
    assert_eq!(fake.calls(), vec![Call::Shutdown]);

    let mut app = App::new();
    let window = iced::window::Id::unique();
    assert!(matches!(
        app.update(Message::CloseRequested(window))[..],
        [Effect::Shutdown { .. }]
    ));
    // The shutdown never reported back; the user closes again: force close.
    assert!(matches!(
        app.update(Message::CloseRequested(window))[..],
        [Effect::ExitWindow(w)] if w == window
    ));
}

/// Closing with a security dialog open: every dialog, shown or queued, is answered
/// Cancel; new ones are cancelled unseen; the user's input starts nothing new; results
/// of running work still land.
#[tokio::test]
async fn close_request_cancels_dialogs_and_ignores_input() {
    use crate::backend::broker::{PromptKind, PromptRequest};
    use crate::backend::{BrokerEvent, PromptAnswer, PromptId};

    let a = site("a", "a.example", "u", Protocol::Ssh);
    let b = site("b", "b.example", "u", Protocol::Ssh);
    let ws = workspace("Shop", &[&a.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_site(b.clone())
            .with_workspace(ws.clone()),
    );
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(a.id.clone())));
    drive(&mut app, &fake, sites).await;
    let session = session_of(&app, &a.id);
    let request = |id: u64| PromptRequest {
        by_agent: false,
        id: PromptId(id),
        session,
        site: a.id.clone(),
        key: "a.example:22".into(),
        kind: PromptKind::Password {
            user: "u".into(),
            host: "a.example".into(),
        },
    };
    app.update(Message::Prompt(BrokerEvent::Request(request(1))));
    app.update(Message::Prompt(BrokerEvent::Request(request(2))));
    assert!(app.dialogs.is_open());

    let window = iced::window::Id::unique();
    let effects = app.update(Message::CloseRequested(window));
    let cancelled: Vec<PromptId> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::AnswerPrompt {
                id,
                answer: PromptAnswer::Cancel,
            } => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(cancelled, vec![PromptId(1), PromptId(2)]);
    assert!(matches!(effects.last(), Some(Effect::Shutdown { .. })));
    assert!(!app.dialogs.is_open());
    assert_eq!(app.dialogs.queued().count(), 0);

    // A dialog arriving now is cancelled without being shown.
    let effects = app.update(Message::Prompt(BrokerEvent::Request(request(3))));
    assert!(matches!(
        effects[..],
        [Effect::AnswerPrompt {
            id: PromptId(3),
            answer: PromptAnswer::Cancel
        }]
    ));
    assert!(!app.dialogs.is_open());

    // Nothing new starts.
    let tab = app.tabs[0].id;
    for input in [
        Message::ConnectSite(b.id.clone()),
        Message::OpenWorkspace(ws.id.clone()),
        Message::NewSite,
        Message::Queue(QueueMsg::ClearFinished),
        Message::Pane {
            tab,
            side: Side::Remote,
            msg: PaneMsg::Transfer,
        },
        Message::Pane {
            tab,
            side: Side::Local,
            msg: PaneMsg::Navigate(path("/elsewhere")),
        },
        Message::CloseTab(tab),
        Message::FileDropped(PathBuf::from("/tmp/x")),
    ] {
        let shown = format!("{input:?}");
        assert!(app.update(input).is_empty(), "{shown}");
    }
    assert!(app.form.is_none());
    assert_eq!(app.tabs.len(), 1);
    assert_ne!(app.tabs[0].local().path, path("/elsewhere"));

    // Results still land.
    state(
        &mut app,
        session,
        ConnectionState::Reconnecting { attempt: 1 },
    );
    assert!(matches!(
        app.tab_conn(0).conn,
        Some(ConnectionState::Reconnecting { attempt: 1 })
    ));
    assert!(matches!(
        app.update(Message::ShutdownDone(window))[..],
        [Effect::ExitWindow(_)]
    ));
}

/// A restored folder that is gone falls back to its parent, then home (local) or `/`,
/// with a note; a pane whose listing failed keeps its previously saved folder.
#[tokio::test]
async fn restored_folder_gone_falls_back_and_failed_sides_keep_the_saved_folder() {
    let a = site("a", "a.example", "u", Protocol::Ssh);
    let mut ws = workspace("Shop", &[&a.id]);
    ws.sites[0].tabs = vec![files_tab(Some("/gone/deep"), Some("/srv/old"))];
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_workspace(ws.clone()),
    );
    let mut app = loaded(&fake).await;
    app.home = Some(path("/home/me"));
    let effects = app.update(Message::OpenWorkspace(ws.id.clone()));
    let tab = app.tabs[0].id;

    // Local: /gone/deep is gone, then /gone too: home, with the note.
    let (request, at) = list_effect(&effects, tab, Side::Local);
    assert_eq!(at, path("/gone/deep"));
    let effects = listed(&mut app, tab, Side::Local, request, Err(gone()));
    let (request, at) = list_effect(&effects, tab, Side::Local);
    assert_eq!(at, path("/gone"));
    let note = strings::saved_folder_gone("/gone/deep");
    assert_eq!(app.tabs[0].local().note.as_deref(), Some(note.as_str()));
    let effects = listed(&mut app, tab, Side::Local, request, Err(gone()));
    let (request, at) = list_effect(&effects, tab, Side::Local);
    assert_eq!(at, path("/home/me"));
    listed(&mut app, tab, Side::Local, request, Ok(Vec::new()));
    assert_eq!(app.tabs[0].local().listing, files::Listing::Loaded);
    assert_eq!(app.tabs[0].local().note.as_deref(), Some(note.as_str()));
    assert_eq!(app.tabs[0].local().restore, None);

    // Remote: /srv/old is gone, /srv is there.
    let session_effects = app.update(Message::SiteConnected {
        site: a.id.clone(),
        result: fake.connect(a.id.clone()).await,
    });
    let (request, at) = list_effect(&session_effects, tab, Side::Remote);
    assert_eq!(at, path("/srv/old"));
    let effects = listed(&mut app, tab, Side::Remote, request, Err(gone()));
    let (request, at) = list_effect(&effects, tab, Side::Remote);
    assert_eq!(at, path("/srv"));
    listed(&mut app, tab, Side::Remote, request, Ok(Vec::new()));
    assert!(app.tabs[0].remote().note.is_some());
    // Dismiss clears the note.
    app.update(Message::Pane {
        tab,
        side: Side::Remote,
        msg: PaneMsg::DismissError,
    });
    assert_eq!(app.tabs[0].remote().note, None);

    // A later failure (not a restore) is shown as usual and never saved.
    let effects = app.update(Message::Pane {
        tab,
        side: Side::Local,
        msg: PaneMsg::Navigate(path("/locked")),
    });
    let (request, _) = list_effect(&effects, tab, Side::Local);
    let denied = Error::new(ErrorKind::Permission, "Permission denied");
    listed(&mut app, tab, Side::Local, request, Err(denied.clone()));
    assert_eq!(app.tabs[0].local().listing, files::Listing::Failed(denied));
    let effects = without_panes(app.update(Message::CloseWorkspace(ws.id.clone())));
    let Some(Effect::SaveTabs { tabs, .. }) = effects.first() else {
        panic!("{effects:?}");
    };
    assert_eq!(
        tabs[0].1,
        vec![files_tab(Some("/gone/deep"), Some("/srv"))],
        "the failed local side keeps its saved folder; the remote side saves /srv"
    );
}

/// The saved local folder round-trips through `local_fs_path` / `local_path`.
#[test]
fn local_dir_round_trips() {
    for dir in ["/home/me/shop", "/", "/with space/ünï"] {
        let remote = path(dir);
        let local = files::local_fs_path(&remote);
        assert_eq!(files::local_path(&local), Some(remote), "{dir}");
    }
}

/// A connect result for the session the tab already watches (same id) keeps what its
/// feed reported: the feed sends only changes from then on.
#[tokio::test]
async fn same_session_connect_keeps_the_feed_state() {
    let a = site("a", "a.example", "u", Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(a.id.clone())));
    drive(&mut app, &fake, sites).await;
    let session = session_of(&app, &a.id);
    app.update(Message::SessionState {
        session,
        state: ConnectionState::Connected,
        auth: Some(AuthMethod::Password),
    });
    app.update(Message::SiteConnected {
        site: a.id.clone(),
        result: Ok(Connected {
            session,
            keyring_unavailable: false,
        }),
    });
    assert_eq!(app.tab_conn(0).auth, Some(AuthMethod::Password));
    assert_eq!(app.tab_conn(0).conn, Some(ConnectionState::Connected));
    // A new session starts over.
    app.update(Message::SiteConnected {
        site: a.id.clone(),
        result: Ok(Connected {
            session: SessionId(99),
            keyring_unavailable: false,
        }),
    });
    assert_eq!(app.tab_conn(0).auth, None);
    assert_eq!(app.tab_conn(0).conn, None);
}

/// The remote listings among `effects`, for the tab's remote pane.
fn remote_lists(effects: &[Effect], tab: TabId) -> Vec<RemotePath> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::List {
                tab: t,
                side: Side::Remote,
                path,
                ..
            } if *t == tab => Some(path.clone()),
            _ => None,
        })
        .collect()
}

fn reconnects(effects: &[Effect]) -> Vec<SessionId> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Reconnect { session } => Some(*session),
            _ => None,
        })
        .collect()
}

fn navigate(app: &mut App, tab: TabId, to: &str) -> Vec<Effect> {
    app.update(Message::Pane {
        tab,
        side: Side::Remote,
        msg: PaneMsg::Navigate(path(to)),
    })
}

/// Review Focus 3: a session lost for a network reason (the transport gave up while the
/// user was away) comes back by itself when the user acts on its tab. The action waits for
/// the reconnection ("Reconnecting…" in the banner instead of the error), then runs once;
/// a reconnection that fails fails the action with its reason, never replays it later.
#[tokio::test]
async fn action_on_dropped_session_reconnects_and_retries() {
    let a = site("a", "a.example", "u", Protocol::Ftp);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(a.id.clone())));
    drive(&mut app, &fake, sites).await;
    let session = session_of(&app, &a.id);
    let tab = app.tabs[0].id;
    let lost = Error::new(ErrorKind::Network, "Connection lost")
        .with_detail("no reply to the keepalive NOOP");
    state(
        &mut app,
        session,
        ConnectionState::Disconnected(lost.clone()),
    );
    assert_eq!(
        crate::ui::files::banner(app.tab_conn(0)),
        Some(crate::ui::files::Banner::Lost(&lost))
    );

    // Going to another folder reconnects first; the listing waits.
    let effects = navigate(&mut app, tab, "/srv");
    assert_eq!(reconnects(&effects), vec![session], "{effects:?}");
    assert!(remote_lists(&effects, tab).is_empty(), "{effects:?}");
    assert!(app.tab_conn(0).reconnecting);
    assert_eq!(
        crate::ui::files::banner(app.tab_conn(0)),
        Some(crate::ui::files::Banner::Reconnecting),
        "\"Reconnecting…\" instead of the error"
    );
    assert_eq!(app.tabs[0].remote().listing, Listing::Loading);
    // Another action meanwhile: no second reconnection; the latest folder wins.
    let effects = navigate(&mut app, tab, "/srv/www");
    assert!(reconnects(&effects).is_empty(), "{effects:?}");
    assert!(remote_lists(&effects, tab).is_empty(), "{effects:?}");
    // The transport reports its attempts: still waiting.
    let effects = state(
        &mut app,
        session,
        ConnectionState::Reconnecting { attempt: 1 },
    );
    assert!(remote_lists(&effects, tab).is_empty(), "{effects:?}");

    // Back: the action runs, once, for the latest folder.
    let effects = state(&mut app, session, ConnectionState::Connected);
    assert_eq!(remote_lists(&effects, tab), vec![path("/srv/www")]);
    let effects = app.update(Message::Reconnected {
        session,
        result: Ok(()),
    });
    assert!(
        remote_lists(&effects, tab).is_empty(),
        "never replayed twice: {effects:?}"
    );
    assert!(!app.tab_conn(0).reconnecting);
    assert_eq!(crate::ui::files::banner(app.tab_conn(0)), None);

    // Lost again; this time the reconnection fails: the action fails with its reason.
    state(
        &mut app,
        session,
        ConnectionState::Disconnected(lost.clone()),
    );
    let effects = navigate(&mut app, tab, "/tmp");
    assert_eq!(reconnects(&effects), vec![session]);
    let refused = Error::new(ErrorKind::Network, "Connection refused");
    let effects = app.update(Message::Reconnected {
        session,
        result: Err(refused.clone()),
    });
    assert!(remote_lists(&effects, tab).is_empty(), "{effects:?}");
    assert_eq!(
        app.tabs[0].remote().listing,
        Listing::Failed(refused.clone())
    );
    assert_eq!(app.tab_conn(0).lost_connection(), Some(&refused));
    // Back some other way: the failed listing is retried once, as any failed listing is
    // when its session comes back; the held action is not replayed on top of it.
    let effects = state(&mut app, session, ConnectionState::Connected);
    assert_eq!(remote_lists(&effects, tab), vec![path("/tmp")]);
    assert!(reconnects(&effects).is_empty());
}

/// An action chosen in a context menu is the user acting, like the same action sent by its
/// button or key: on a site whose session was lost it reconnects first and is held, never
/// run on the dead session (here the remote pane's Refresh).
#[tokio::test]
async fn menu_action_on_dropped_session_reconnects_like_its_button() {
    use crate::app::menu::{MenuAction, MenuMsg, MenuTarget};

    let a = site("a", "a.example", "u", Protocol::Ftp);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(a.id.clone())));
    drive(&mut app, &fake, sites).await;
    let session = session_of(&app, &a.id);
    let tab = app.tabs[0].id;
    let lost = Error::new(ErrorKind::Network, "Connection lost");
    state(&mut app, session, ConnectionState::Disconnected(lost));

    let target = MenuTarget::Pane {
        tab,
        side: Side::Remote,
        row: None,
    };
    app.update(Message::Menu(MenuMsg::Open(target.clone())));
    assert!(app.menu.is_some());
    let refresh = app
        .menu_items(&target)
        .iter()
        .position(|item| item.action == MenuAction::Refresh && item.enabled)
        .expect("Refresh is offered");
    let here = app.tabs[0].remote().path.clone();
    let effects = app.update(Message::Menu(MenuMsg::Choose(refresh)));
    assert!(app.menu.is_none());
    // The same as the key: a reconnection, the listing held.
    assert_eq!(reconnects(&effects), vec![session], "{effects:?}");
    assert!(remote_lists(&effects, tab).is_empty(), "{effects:?}");
    assert!(app.tab_conn(0).reconnecting);
    // Back: the held action runs, once.
    let effects = state(&mut app, session, ConnectionState::Connected);
    assert_eq!(remote_lists(&effects, tab), vec![here]);
}

/// A session refused for good (host key, authentication, certificate) never reconnects by
/// itself: an action fails as before and the banner keeps the reason and its fix.
#[tokio::test]
async fn host_key_or_auth_failure_does_not_auto_reconnect() {
    for refused in [
        Error::new(ErrorKind::Auth, "Authentication failed"),
        hoprs_ssh::testing::changed_host_key_error("a.example", 3),
        Error::new(ErrorKind::Tls, "Certificate not trusted"),
    ] {
        let a = site("a", "a.example", "u", Protocol::Ssh);
        let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
        let mut app = loaded(&fake).await;
        let sites = connect_effect(app.update(Message::ConnectSite(a.id.clone())));
        drive(&mut app, &fake, sites).await;
        let session = session_of(&app, &a.id);
        let tab = app.tabs[0].id;
        let effects = state(
            &mut app,
            session,
            ConnectionState::Disconnected(refused.clone()),
        );
        assert!(reconnects(&effects).is_empty(), "{refused:?}: {effects:?}");

        let effects = navigate(&mut app, tab, "/srv");
        assert!(reconnects(&effects).is_empty(), "{refused:?}: {effects:?}");
        assert_eq!(
            remote_lists(&effects, tab),
            vec![path("/srv")],
            "{refused:?}"
        );
        assert!(!app.tab_conn(0).reconnecting);
        assert_eq!(app.tab_conn(0).lost_connection(), Some(&refused));
        assert_ne!(
            crate::ui::files::banner(app.tab_conn(0)),
            Some(crate::ui::files::Banner::Reconnecting)
        );
    }
}

/// Task 5 fix round 1: while the transport reconnects by itself (its own round after a
/// drop, or the session manager's quiet extra round), an action waits instead of failing
/// "Not connected", and runs once when the session is back, with no reconnect of the
/// app's own. If that round gives up, the app reconnects for the waiting action.
#[tokio::test]
async fn action_during_transport_reconnect_waits_and_runs_once() {
    let a = site("a", "a.example", "u", Protocol::Ftp);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(a.id.clone())));
    drive(&mut app, &fake, sites).await;
    let session = session_of(&app, &a.id);
    let tab = app.tabs[0].id;

    state(
        &mut app,
        session,
        ConnectionState::Reconnecting { attempt: 1 },
    );
    let effects = navigate(&mut app, tab, "/srv");
    assert!(reconnects(&effects).is_empty(), "{effects:?}");
    assert!(remote_lists(&effects, tab).is_empty(), "{effects:?}");
    assert!(!app.tab_conn(0).reconnecting);
    assert_eq!(app.tabs[0].remote().listing, Listing::Loading);
    let effects = state(
        &mut app,
        session,
        ConnectionState::Reconnecting { attempt: 2 },
    );
    assert!(remote_lists(&effects, tab).is_empty(), "{effects:?}");
    let effects = state(&mut app, session, ConnectionState::Connected);
    assert_eq!(remote_lists(&effects, tab), vec![path("/srv")]);
    assert!(reconnects(&effects).is_empty());
    assert!(app.held.is_empty());

    // The round gives up while an action waits: the app reconnects for it, then the
    // action runs once.
    state(
        &mut app,
        session,
        ConnectionState::Reconnecting { attempt: 1 },
    );
    navigate(&mut app, tab, "/tmp");
    let lost = Error::new(ErrorKind::Network, "Connection timed out");
    let effects = state(&mut app, session, ConnectionState::Disconnected(lost));
    assert_eq!(reconnects(&effects), vec![session], "{effects:?}");
    assert!(remote_lists(&effects, tab).is_empty(), "{effects:?}");
    let effects = app.update(Message::Reconnected {
        session,
        result: Ok(()),
    });
    assert_eq!(remote_lists(&effects, tab), vec![path("/tmp")]);
    assert!(
        app.replayed.is_empty(),
        "no stale replay ids: {:?}",
        app.replayed
    );
}

/// Plan 13 Task 8: the settings tab has no connection: no state, endpoint nor login; the
/// open workspace's count stays.
#[tokio::test]
async fn settings_tab_status_line_has_no_connection() {
    let a = site("Shop prod", "203.0.113.4", "deploy", Protocol::Ssh);
    let ws = workspace("Shop", &[&a.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_workspace(ws.clone()),
    );
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::OpenWorkspace(ws.id.clone())));
    drive(&mut app, &fake, sites).await;
    assert_eq!(status::line(&app).state.as_deref(), Some("Connected"));
    app.update(Message::OpenSettings);
    let line = status::line(&app);
    assert_eq!(line.state, None);
    assert_eq!((line.endpoint, line.auth), (None, None));
    assert_eq!(
        line.workspace,
        Some(strings::workspace_status(1, 1, "Shop"))
    );
    select(&mut app, &a.id);
    assert_eq!(status::line(&app).state.as_deref(), Some("Connected"));
}

/// The agent indicator: only while a relay is connected, also on the settings tab; the
/// tooltip has the count.
#[tokio::test]
async fn agent_indicator_follows_the_relays() {
    use crate::agent::AgentEvent;

    let a = site("Shop prod", "203.0.113.4", "deploy", Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let mut app = loaded(&fake).await;
    assert_eq!(status::line(&app).agents, None);
    app.update(Message::Agent(AgentEvent::Relays(1)));
    assert_eq!(
        status::line(&app).agents.as_deref(),
        Some("Agent connected")
    );
    app.update(Message::Agent(AgentEvent::Relays(3)));
    assert_eq!(
        status::line(&app).agents.as_deref(),
        Some("Agent connected")
    );
    assert!(strings::agent_connected_tip(3).contains('3'));
    app.update(Message::OpenSettings);
    assert!(app.settings_shown());
    assert_eq!(
        status::line(&app).agents.as_deref(),
        Some("Agent connected")
    );
    // The last relay leaves, or events arrive coalesced: the latest value wins.
    app.update(Message::Agent(AgentEvent::Relays(0)));
    assert_eq!(status::line(&app).agents, None);
}

fn channel_error() -> hoprs_core::Error {
    hoprs_core::Error::coded(
        ErrorKind::Io,
        "agent-channel-failed",
        "Could not open the channel for agents",
    )
}

/// An agent connecting, working or leaving changes nothing the user is doing.
#[tokio::test]
async fn agent_events_never_touch_tabs_or_focus() {
    use crate::agent::AgentEvent;

    let a = site("Shop prod", "203.0.113.4", "deploy", Protocol::Ftp);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    let mut app = loaded(&fake).await;
    app.update(Message::ConnectSite(a.id.clone()));
    let before = (app.tabs.len(), app.active_tab, app.dialogs.is_open());
    let form = format!("{:?}", app.form);
    let events = [
        AgentEvent::Relays(2),
        AgentEvent::Sites(vec![a.id.clone()]),
        AgentEvent::Connected(a.id.clone(), SessionId(40)),
        AgentEvent::Channel(Some(channel_error())),
        AgentEvent::Channel(None),
        AgentEvent::Sites(Vec::new()),
        AgentEvent::Relays(0),
    ];
    for event in events {
        let effects = app.update(Message::Agent(event.clone()));
        assert!(effects.is_empty(), "{event:?}: {effects:?}");
        assert_eq!(
            (app.tabs.len(), app.active_tab, app.dialogs.is_open()),
            before
        );
        assert_eq!(format!("{:?}", app.form), form);
    }
    assert!(app.agents.error.is_none());
    assert!(app.agents.sites.is_empty());
    // Only the channel's error is kept, and cleared by `None`.
    app.update(Message::Agent(AgentEvent::Channel(Some(channel_error()))));
    assert!(app.agents.error.is_some());
}

/// The window is closing: an agent event still lands (it is a result).
#[tokio::test]
async fn agent_events_land_while_closing() {
    use crate::agent::AgentEvent;

    let mut app = App::new();
    app.closing = true;
    let effects = app.update(Message::Agent(AgentEvent::Relays(2)));
    assert!(effects.is_empty());
    assert_eq!(app.agents.relays, 2);
}
