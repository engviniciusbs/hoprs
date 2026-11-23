//! Plan 6 Task 1: tabs per session. A site's tabs (terminal and files) share its one
//! session: closing one keeps it for the others, its state reaches all of them, and
//! "Open as" decides which tabs a site opens with (Review Focus 5).

use std::path::PathBuf;

use hoprs_core::{ConnectionState, OpenAs, SavedTab, TabKind};
use hoprs_ssh::AuthMethod;

use super::*;
use crate::backend::broker::{PromptKind, PromptRequest};
use crate::backend::fake::Call;
use crate::backend::{BrokerEvent, PromptId};

fn ssh_site(name: &str, open_as: OpenAs) -> Site {
    let mut site = site(name, &format!("{name}.example"), "u", Protocol::Ssh);
    site.ssh.open_as = open_as;
    site
}

fn kinds(app: &App) -> Vec<TabKind> {
    app.tabs.iter().map(|t| t.kind()).collect()
}

/// The indices of the site's tabs, in strip order.
fn tabs_of(app: &App, site: &SiteId) -> Vec<usize> {
    (0..app.tabs.len())
        .filter(|&i| &app.tabs[i].site == site)
        .collect()
}

fn closes(effects: &[Effect]) -> Vec<SessionId> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::CloseSession(s) => Some(*s),
            _ => None,
        })
        .collect()
}

/// The tabs that `effects` list a pane of, with the side.
fn listings(effects: &[Effect]) -> Vec<(TabId, Side)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::List { tab, side, .. } => Some((*tab, *side)),
            _ => None,
        })
        .collect()
}

/// An app with `site` (a Both SSH site unless the caller says otherwise) open and
/// connected through the fake.
async fn connected(site: &Site) -> (App, Arc<FakeBackend>, SessionId) {
    let fake = Arc::new(FakeBackend::new().with_site(site.clone()));
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(site.id.clone())));
    assert!(drive(&mut app, &fake, sites).await.is_empty());
    let session = app.tab_conn(0).session().expect("connected");
    (app, fake, session)
}

#[tokio::test]
async fn open_as_both_opens_terminal_then_files_on_one_session() {
    let s = ssh_site("both", OpenAs::Both);
    let fake = Arc::new(FakeBackend::new().with_site(s.clone()));
    let mut app = loaded(&fake).await;

    let effects = app.update(Message::ConnectSite(s.id.clone()));
    assert_eq!(kinds(&app), [TabKind::Terminal, TabKind::Files]);
    assert_eq!(
        app.active_tab,
        Some(0),
        "the terminal comes first and has the focus"
    );
    // Only the files tab lists a folder (its local side, until connected).
    let files = app.tabs[1].id;
    assert_eq!(listings(&effects), [(files, Side::Local)]);
    // One connect for the site, whatever its tabs.
    let sites = connect_effect(effects);
    assert_eq!(sites, std::slice::from_ref(&s.id));

    let backend: Arc<dyn BackendApi> = fake.clone();
    let messages: Vec<Message> = effects::connect_sites(backend, sites).collect().await;
    assert_eq!(messages.len(), 1, "one connect result");
    let effects: Vec<Effect> = messages.into_iter().flat_map(|m| app.update(m)).collect();
    let (t0, t1) = (app.tab_conn(0).session(), app.tab_conn(1).session());
    assert!(t0.is_some());
    assert_eq!(t0, t1, "both tabs share the session");
    assert_eq!(
        app.watched_sessions(),
        vec![t0.unwrap()],
        "one feed per session"
    );
    // Connected, only the files tab lists its remote folder.
    assert_eq!(listings(&effects), [(files, Side::Remote)]);

    // Open as Files / Terminal open just that tab.
    let f = ssh_site("files", OpenAs::Files);
    let t = ssh_site("term", OpenAs::Terminal);
    let fake = FakeBackend::new().with_site(f.clone()).with_site(t.clone());
    let mut app = loaded(&fake).await;
    app.update(Message::ConnectSite(f.id.clone()));
    app.update(Message::ConnectSite(t.id.clone()));
    assert_eq!(kinds(&app), [TabKind::Files, TabKind::Terminal]);
    // Clicking a site with tabs again opens no more tabs: it focuses its first one.
    assert!(without_panes(app.update(Message::ConnectSite(f.id.clone()))).is_empty());
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.active_tab, Some(0));
}

/// Review Focus 5: a terminal and a files tab of one site share its session; closing
/// either keeps the session for the other.
#[tokio::test]
async fn closing_one_tab_keeps_the_session_for_the_other() {
    let s = ssh_site("both", OpenAs::Both);
    let (mut app, fake, session) = connected(&s).await;
    let (terminal, files) = (app.tabs[0].id, app.tabs[1].id);

    let effects = app.update(Message::CloseTab(terminal));
    assert!(closes(&effects).is_empty(), "{effects:?}");
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.tabs[0].id, files);
    assert_eq!(app.tab_conn(0).session(), Some(session));
    assert_eq!(app.watched_sessions(), vec![session]);
    assert!(!fake.calls().contains(&Call::Close(session)));

    // The other way round: the files tab closes, the terminal keeps the session.
    let (mut app, _fake, session) = connected(&s).await;
    let files = app.tabs[1].id;
    assert!(closes(&app.update(Message::CloseTab(files))).is_empty());
    assert_eq!(kinds(&app), [TabKind::Terminal]);
    assert_eq!(app.tab_conn(0).session(), Some(session));

    // While connecting: closing one tab abandons nothing; the dialog stays and the
    // result lands in the tab left.
    let fake = Arc::new(FakeBackend::new().with_site(s.clone()));
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(s.id.clone())));
    let request = PromptRequest {
        by_agent: false,
        id: PromptId(1),
        session: SessionId(1),
        site: s.id.clone(),
        key: "both.example:22".into(),
        kind: PromptKind::Password {
            user: "u".into(),
            host: "both.example".into(),
        },
    };
    app.update(Message::Prompt(BrokerEvent::Request(request)));
    let terminal = app.tabs[0].id;
    // (A dialog blocks the ×: the user answers it first, then closes.)
    app.update(Message::Dialog(DialogMsg::Cancel));
    assert!(closes(&app.update(Message::CloseTab(terminal))).is_empty());
    let again = PromptRequest {
        by_agent: false,
        id: PromptId(2),
        session: SessionId(1),
        site: s.id.clone(),
        key: "both.example:22".into(),
        kind: PromptKind::Password {
            user: "u".into(),
            host: "both.example".into(),
        },
    };
    let effects = app.update(Message::Prompt(BrokerEvent::Request(again)));
    assert!(closes(&effects).is_empty(), "the site is not abandoned");
    assert!(app.dialogs.is_open());
    app.update(Message::Dialog(DialogMsg::Cancel));
    drive(&mut app, &fake, sites).await;
    assert_eq!(kinds(&app), [TabKind::Files]);
    assert!(matches!(app.tab_conn(0).state, TabState::Connected(_)));
}

#[tokio::test]
async fn closing_the_last_tab_closes_the_session() {
    let s = ssh_site("both", OpenAs::Both);
    let (mut app, _fake, session) = connected(&s).await;
    let (terminal, files) = (app.tabs[0].id, app.tabs[1].id);
    assert!(closes(&app.update(Message::CloseTab(files))).is_empty());
    let effects = app.update(Message::CloseTab(terminal));
    assert_eq!(closes(&effects), [session]);
    assert!(app.tabs.is_empty());
    assert!(app.watched_sessions().is_empty());
    assert_eq!(app.active_tab, None);

    // The last tab of a site still connecting takes its dialogs with it.
    let t = ssh_site("term", OpenAs::Terminal);
    let fake = FakeBackend::new().with_site(t.clone());
    let mut app = loaded(&fake).await;
    app.update(Message::ConnectSite(t.id.clone()));
    let request = PromptRequest {
        by_agent: false,
        id: PromptId(7),
        session: SessionId(9),
        site: t.id.clone(),
        key: "term.example:22".into(),
        kind: PromptKind::Password {
            user: "u".into(),
            host: "term.example".into(),
        },
    };
    app.update(Message::Prompt(BrokerEvent::Request(request)));
    app.update(Message::Dialog(DialogMsg::Cancel));
    let effects = app.update(Message::CloseTab(app.tabs[0].id));
    assert!(app.tabs.is_empty());
    let late = PromptRequest {
        by_agent: false,
        id: PromptId(8),
        session: SessionId(9),
        site: t.id.clone(),
        key: "term.example:22".into(),
        kind: PromptKind::Password {
            user: "u".into(),
            host: "term.example".into(),
        },
    };
    let mut effects = effects;
    effects.extend(app.update(Message::Prompt(BrokerEvent::Request(late))));
    assert_eq!(closes(&effects), [SessionId(9)]);
    assert!(!app.dialogs.is_open());
}

#[tokio::test]
async fn session_state_reaches_every_tab_of_the_session() {
    let s = ssh_site("both", OpenAs::Both);
    let (mut app, _fake, session) = connected(&s).await;
    let files = app.tabs[1].id;

    app.update(Message::SessionState {
        session,
        state: ConnectionState::Reconnecting { attempt: 2 },
        auth: Some(AuthMethod::Password),
    });
    for tab in &app.tabs {
        assert!(
            matches!(
                app.conn(&tab.site).conn_status(),
                ConnStatus::Reconnecting { attempt: 2, .. }
            ),
            "{:?}",
            tab.kind()
        );
        assert_eq!(app.conn(&tab.site).auth, Some(AuthMethod::Password));
    }

    let lost = Error::new(ErrorKind::Network, "Connection reset");
    app.update(Message::SessionState {
        session,
        state: ConnectionState::Disconnected(lost.clone()),
        auth: None,
    });
    for tab in &app.tabs {
        assert_eq!(app.conn(&tab.site).lost_connection(), Some(&lost));
        assert_eq!(
            app.conn(&tab.site).failed_endpoint,
            Some(("both.example".into(), 22))
        );
    }

    // "Reconnect" from the terminal tab: one reconnect, shown in both tabs.
    let terminal = app.tabs[0].id;
    let effects = app.update(Message::ReconnectSession(terminal));
    assert!(matches!(
        effects[..],
        [Effect::Reconnect { session: s, .. }] if s == session
    ));
    assert!(app.tabs.iter().all(|t| app.conn(&t.site).reconnecting));
    // The files tab asks too: already in flight.
    assert!(app.update(Message::ReconnectSession(files)).is_empty());

    // The terminal tab closes meanwhile: the result still reaches the files tab.
    app.update(Message::CloseTab(terminal));
    let effects = app.update(Message::Reconnected {
        session,
        result: Ok(()),
    });
    let tab = &app.tabs[0];
    assert_eq!(tab.id, files);
    assert!(!app.conn(&tab.site).reconnecting);
    assert_eq!(app.conn(&tab.site).conn_status(), ConnStatus::Connected);
    assert_eq!(listings(&effects), [(files, Side::Remote)]);

    // A failed connect lands in both tabs, and "Close and reconnect" (or Reconnect) from
    // one of them starts every tab of the site over, with one connect.
    let fake = Arc::new(FakeBackend::new().with_site(s.clone()));
    fake.fail_next_connect(&s.id, Error::new(ErrorKind::Network, "Refused"));
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(s.id.clone())));
    drive(&mut app, &fake, sites).await;
    assert!(
        app.tabs
            .iter()
            .all(|t| matches!(app.conn(&t.site).state, TabState::Failed(_)))
    );
    assert!(
        app.tabs
            .iter()
            .all(|t| app.conn(&t.site).failed_endpoint.is_some())
    );
    let effects = app.update(Message::RestartSession(app.tabs[1].id));
    let [Effect::RestartSite { site, .. }] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(site, &s.id);
    assert!(
        app.tabs
            .iter()
            .all(|t| app.conn(&t.site).state == TabState::Connecting
                && app.conn(&t.site).failed_endpoint.is_none())
    );
    // The sidebar's retry on a failed site also retries it once for all its tabs.
    let fake = Arc::new(FakeBackend::new().with_site(s.clone()));
    fake.fail_next_connect(&s.id, Error::new(ErrorKind::Network, "Refused"));
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(s.id.clone())));
    drive(&mut app, &fake, sites).await;
    let sites = connect_effect(app.update(Message::ConnectSite(s.id.clone())));
    assert_eq!(sites, std::slice::from_ref(&s.id));
    assert!(
        app.tabs
            .iter()
            .all(|t| app.conn(&t.site).state == TabState::Connecting)
    );
    drive(&mut app, &fake, sites).await;
    let sessions: Vec<_> = app
        .tabs
        .iter()
        .map(|t| app.conn(&t.site).session())
        .collect();
    assert!(sessions[0].is_some());
    assert_eq!(sessions[0], sessions[1]);
}

#[tokio::test]
async fn workspace_restores_terminal_tabs() {
    // Saved as Files only, the site's own "Open as" doesn't matter: the workspace
    // restores what it saved.
    let s = ssh_site("prod", OpenAs::Files);
    let mut ws = workspace("Shop", &[&s.id]);
    ws.sites[0].tabs = vec![
        SavedTab {
            kind: TabKind::Terminal,
            remote_dir: None,
            local_dir: None,
        },
        SavedTab {
            kind: TabKind::Files,
            remote_dir: Some("/var/www".into()),
            local_dir: Some(PathBuf::from("/home/me/shop")),
        },
        SavedTab {
            kind: TabKind::Terminal,
            remote_dir: None,
            local_dir: None,
        },
    ];
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(s.clone())
            .with_workspace(ws.clone()),
    );
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::OpenWorkspace(ws.id.clone())));
    assert_eq!(
        sites,
        std::slice::from_ref(&s.id),
        "one connect for the three tabs"
    );
    assert_eq!(
        kinds(&app),
        [TabKind::Terminal, TabKind::Files, TabKind::Terminal]
    );
    assert_eq!(app.tabs[1].remote().path.as_str(), "/var/www");
    assert_eq!(app.tabs[1].local().path.as_str(), "/home/me/shop");
    drive(&mut app, &fake, sites).await;
    let session = app.tab_conn(0).session().expect("connected");
    assert!(
        app.tabs
            .iter()
            .all(|t| app.conn(&t.site).session() == Some(session))
    );

    // Closing it saves every tab, terminals included, in strip order.
    let effects = without_panes(app.update(Message::CloseWorkspace(ws.id.clone())));
    let [Effect::SaveTabs { tabs, .. }, Effect::CloseSession(closed)] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(*closed, session, "one close for the session");
    assert_eq!(tabs, &vec![(s.id.clone(), ws.sites[0].tabs.clone())]);
    assert!(app.tabs.is_empty());

    // A workspace site that saved no tabs opens per its "Open as".
    let both = ssh_site("both", OpenAs::Both);
    let ws = workspace("New", &[&both.id]);
    let fake = FakeBackend::new()
        .with_site(both.clone())
        .with_workspace(ws.clone());
    let mut app = loaded(&fake).await;
    app.update(Message::OpenWorkspace(ws.id.clone()));
    assert_eq!(kinds(&app), [TabKind::Terminal, TabKind::Files]);
}

#[tokio::test]
async fn ftp_site_ignores_open_as() {
    let mut ftp = site("ftp", "ftp.example", "u", Protocol::Ftp);
    ftp.ssh.open_as = OpenAs::Terminal;
    let mut ws = workspace("w", &[&ftp.id]);
    ws.sites[0].tabs = vec![SavedTab {
        kind: TabKind::Terminal,
        remote_dir: None,
        local_dir: None,
    }];
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(ftp.clone())
            .with_workspace(ws.clone()),
    );
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(ftp.id.clone())));
    assert_eq!(kinds(&app), [TabKind::Files]);
    drive(&mut app, &fake, sites).await;
    // No terminal for an FTP site, even asked for.
    let tab = app.tabs[0].id;
    assert!(
        app.update(Message::NewTab {
            tab,
            kind: TabKind::Terminal
        })
        .is_empty()
    );
    assert_eq!(app.tabs.len(), 1);
    app.update(Message::CloseTab(tab));

    // A saved terminal tab of an FTP site restores as its files tab.
    app.update(Message::OpenWorkspace(ws.id.clone()));
    assert_eq!(kinds(&app), [TabKind::Files]);
}

#[tokio::test]
async fn delete_site_closes_all_its_tabs() {
    let s = ssh_site("both", OpenAs::Both);
    let other = ssh_site("other", OpenAs::Files);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(s.clone())
            .with_site(other.clone()),
    );
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(s.id.clone())));
    drive(&mut app, &fake, sites).await;
    let sites = connect_effect(app.update(Message::ConnectSite(other.id.clone())));
    drive(&mut app, &fake, sites).await;
    let session = app.tab_conn(0).session().unwrap();
    assert_eq!(app.tabs.len(), 3);

    let effects = app.update(Message::FormDone {
        op: ProfileOp::DeleteSite(s.id.clone()),
        result: Ok(()),
    });
    assert_eq!(closes(&effects), [session], "one close for the session");
    assert!(tabs_of(&app, &s.id).is_empty());
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.tabs[0].site, other.id);
    assert_eq!(app.active_tab, Some(0));
}

#[tokio::test]
async fn profile_reload_closes_all_tabs_of_a_vanished_site() {
    let s = ssh_site("both", OpenAs::Both);
    let other = ssh_site("other", OpenAs::Files);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(s.clone())
            .with_site(other.clone()),
    );
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(other.id.clone())));
    drive(&mut app, &fake, sites).await;
    let sites = connect_effect(app.update(Message::ConnectSite(s.id.clone())));
    drive(&mut app, &fake, sites).await;
    let session = app.tab_conn(1).session().unwrap();

    // The host changes: every tab of the site loses the session, closed once.
    let mut moved = s.clone();
    moved.host = "new.example".into();
    fake.save_site(moved).await.unwrap();
    let effects = app.update(profiles_loaded(&app, fake.profiles().await));
    assert_eq!(closes(&effects), [session]);
    for i in tabs_of(&app, &s.id) {
        assert!(matches!(app.tab_conn(i).state, TabState::Failed(_)));
        assert_eq!(app.tab_conn(i).conn, None);
    }
    assert_eq!(tabs_of(&app, &s.id).len(), 2);

    // Reconnected, then gone from the profiles (a "Replace" restore): all its tabs go.
    let sites = connect_effect(app.update(Message::ConnectSite(s.id.clone())));
    drive(&mut app, &fake, sites).await;
    let session = app.tab_conn(1).session().unwrap();
    fake.delete_site(s.id.clone()).await.unwrap();
    let reload = app.load_profiles();
    assert!(matches!(reload, Effect::LoadProfiles(_)));
    let effects = app.update(profiles_loaded(&app, fake.profiles().await));
    assert_eq!(closes(&effects), [session]);
    assert!(tabs_of(&app, &s.id).is_empty());
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.tabs[0].site, other.id);
}

#[tokio::test]
async fn new_terminal_and_files_tabs_join_the_session() {
    let s = ssh_site("files", OpenAs::Files);
    let (mut app, _fake, session) = connected(&s).await;
    let first = app.tabs[0].id;

    let effects = app.update(Message::NewTab {
        tab: first,
        kind: TabKind::Terminal,
    });
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::OpenShell { session: s, .. } if *s == session)),
        "the new terminal opens its shell on the session: {effects:?}"
    );
    assert!(
        without_panes(effects).is_empty(),
        "no connect: the session is reused"
    );
    assert_eq!(kinds(&app), [TabKind::Files, TabKind::Terminal]);
    assert_eq!(app.active_tab, Some(1));
    assert_eq!(app.tab_conn(1).session(), Some(session));
    assert_eq!(app.tab_conn(1).conn_status(), ConnStatus::Connected);

    // Another files tab on the same session starts at the folders of the tab it was
    // opened from, and lists both sides at once.
    let here = |p: &str| RemotePath::new(p).unwrap();
    for (side, path) in [(Side::Local, "/home/me/work"), (Side::Remote, "/var/www")] {
        app.update(Message::Pane {
            tab: first,
            side,
            msg: PaneMsg::Navigate(here(path)),
        });
    }
    let effects = app.update(Message::NewTab {
        tab: first,
        kind: TabKind::Files,
    });
    assert_eq!(
        kinds(&app),
        [TabKind::Files, TabKind::Files, TabKind::Terminal]
    );
    let new = app.tabs[1].id;
    assert_eq!(app.active_tab, Some(1));
    assert_eq!(app.tab_conn(1).session(), Some(session));
    let mut listed = listings(&effects);
    listed.sort_by_key(|(_, side)| *side == Side::Remote);
    assert_eq!(listed, [(new, Side::Local), (new, Side::Remote)]);
    assert_eq!(app.tabs[1].local().path, here("/home/me/work"));
    assert_eq!(app.tabs[1].remote().path, here("/var/www"));
    let paths: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::List { path, .. } => Some(path.as_str()),
            _ => None,
        })
        .collect();
    assert!(paths.contains(&"/home/me/work") && paths.contains(&"/var/www"));
    assert_eq!(app.watched_sessions(), vec![session]);

    // Not on a tab whose site isn't connected.
    let down = ssh_site("down", OpenAs::Files);
    let fake = Arc::new(FakeBackend::new().with_site(down.clone()));
    fake.fail_next_connect(&down.id, Error::new(ErrorKind::Network, "Refused"));
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(down.id.clone())));
    drive(&mut app, &fake, sites).await;
    let tab = app.tabs[0].id;
    assert!(
        app.update(Message::NewTab {
            tab,
            kind: TabKind::Terminal
        })
        .is_empty()
    );
    assert_eq!(app.tabs.len(), 1);
}

/// The connection is one record per site: a change reaches every tab of the session,
/// also a tab opened after it, and the record goes with the site's last tab.
#[tokio::test]
async fn state_change_reaches_tabs_opened_after_it() {
    let s = ssh_site("files", OpenAs::Files);
    let (mut app, _fake, session) = connected(&s).await;
    app.update(Message::SessionState {
        session,
        state: ConnectionState::Reconnecting { attempt: 1 },
        auth: Some(AuthMethod::Password),
    });
    let first = app.tabs[0].id;
    app.update(Message::NewTab {
        tab: first,
        kind: TabKind::Terminal,
    });
    assert_eq!(app.tabs.len(), 2);
    for index in 0..2 {
        let conn = app.tab_conn(index);
        assert!(
            matches!(
                conn.conn_status(),
                ConnStatus::Reconnecting { attempt: 1, .. }
            ),
            "tab {index}"
        );
        assert_eq!(conn.auth, Some(AuthMethod::Password));
    }
    app.update(Message::SessionState {
        session,
        state: ConnectionState::Connected,
        auth: None,
    });
    assert!((0..2).all(|i| app.tab_conn(i).conn_status() == ConnStatus::Connected));
    assert_eq!(app.conns.len(), 1, "one record for the site");

    let terminal = app.tabs[1].id;
    app.update(Message::CloseTab(terminal));
    assert_eq!(app.conns.len(), 1, "the files tab still uses it");
    app.update(Message::CloseTab(first));
    assert!(app.conns.is_empty(), "gone with the last tab");
}

/// "Remove the old entry…" is the site's: started in one tab, its results land even if
/// that tab closed meanwhile, and the restart moves the remaining tab on.
#[tokio::test]
async fn known_hosts_fix_survives_closing_its_tab() {
    let s = ssh_site("both", OpenAs::Both);
    let fake = Arc::new(FakeBackend::new().with_site(s.clone()));
    fake.fail_next_connect(
        &s.id,
        hoprs_ssh::testing::changed_host_key_error(&s.host, 3),
    );
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(s.id.clone())));
    drive(&mut app, &fake, sites).await;
    let (terminal, files) = (app.tabs[0].id, app.tabs[1].id);

    let effects = app.update(Message::KnownHosts {
        tab: terminal,
        msg: KnownHostsMsg::Start,
    });
    let [Effect::KnownHostEntries { site, .. }] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(site, &s.id);
    assert_eq!(app.conn(&s.id).known_hosts, KnownHostsFix::Checking);
    app.update(Message::CloseTab(terminal));
    app.update(Message::KnownHostEntries {
        site: s.id.clone(),
        result: Ok(Vec::new()),
    });
    assert_eq!(
        app.conn(&s.id).known_hosts,
        KnownHostsFix::Confirm(Vec::new()),
        "the result landed for the site"
    );

    // Confirmed from the files tab: the removal restarts the site.
    let effects = app.update(Message::KnownHosts {
        tab: files,
        msg: KnownHostsMsg::Confirm,
    });
    assert!(matches!(effects[..], [Effect::RemoveKnownHost { .. }]));
    let effects = app.update(Message::KnownHostRemoved {
        site: s.id.clone(),
        result: Ok(0),
    });
    let [Effect::RestartSite { site, .. }] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(site, &s.id);
    assert_eq!(app.tab_conn(0).state, TabState::Connecting);
    assert_eq!(app.tab_conn(0).known_hosts, KnownHostsFix::Idle);
}

/// A site edited from SSH to FTP has no terminal: its terminal tabs close; the session
/// closes only if no files tab is left.
#[tokio::test]
async fn site_no_longer_ssh_closes_its_terminal_tabs() {
    let both = ssh_site("both", OpenAs::Both);
    let term = ssh_site("term", OpenAs::Terminal);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(both.clone())
            .with_site(term.clone()),
    );
    let mut app = loaded(&fake).await;
    for s in [&both, &term] {
        let sites = connect_effect(app.update(Message::ConnectSite(s.id.clone())));
        drive(&mut app, &fake, sites).await;
    }
    assert_eq!(app.tabs.len(), 3);
    let term_session = app.conn(&term.id).session().unwrap();

    for s in [&both, &term] {
        let mut ftp = s.clone();
        ftp.protocol = Protocol::Ftp;
        fake.save_site(ftp).await.unwrap();
    }
    let effects = app.update(profiles_loaded(&app, fake.profiles().await));
    assert_eq!(kinds(&app), [TabKind::Files]);
    assert_eq!(app.tabs[0].site, both.id);
    assert!(
        matches!(app.conn(&both.id).state, TabState::Connected(_)),
        "the files tab keeps its session"
    );
    assert_eq!(
        closes(&effects),
        [term_session],
        "the terminal-only site's session"
    );
}

/// Review Focus 5: "Close and reconnect" on a lost session whose host key changed, from
/// either tab, moves both tabs to the new session.
#[tokio::test]
async fn close_and_reconnect_moves_every_tab_to_the_new_session() {
    let s = ssh_site("both", OpenAs::Both);
    let (mut app, fake, old) = connected(&s).await;
    app.update(Message::SessionState {
        session: old,
        state: ConnectionState::Disconnected(hoprs_ssh::testing::pinned_key_changed_error(&s.host)),
        auth: None,
    });
    for tab in &app.tabs {
        assert_eq!(
            crate::ui::files::banner(app.conn(&tab.site)),
            Some(crate::ui::files::Banner::HostKeyChanged)
        );
        assert_eq!(
            app.failure_actions(tab),
            [FailureAction::CloseAndReconnect, FailureAction::EditSite]
        );
    }

    let effects = app.update(Message::RestartSession(app.tabs[1].id));
    let Ok([Effect::RestartSite { close, site }]) = <[Effect; 1]>::try_from(effects) else {
        panic!("one RestartSite effect");
    };
    assert_eq!(close, Some(old));
    assert!((0..2).all(|i| app.tab_conn(i).state == TabState::Connecting));
    let backend: Arc<dyn BackendApi> = fake.clone();
    let messages: Vec<Message> = effects::restart_site(backend, close, site).collect().await;
    let effects: Vec<Effect> = messages.into_iter().flat_map(|m| app.update(m)).collect();
    let new = app.tab_conn(0).session().expect("connected again");
    assert_ne!(new, old);
    assert_eq!(app.tab_conn(1).session(), Some(new));
    assert_eq!(app.watched_sessions(), vec![new]);
    assert_eq!(fake.closed(), [old]);
    // The files tab lists its remote folder on the new session.
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::List { side: Side::Remote, session: Some(s), .. } if *s == new
    )));
}

/// Plan 13 Task 8: the settings tab. A separate slot in the strip: at most one, and while
/// it is selected no session tab is.
#[tokio::test]
async fn open_settings_twice_focuses_one_tab() {
    let s = ssh_site("prod", OpenAs::Files);
    let (mut app, _fake, _) = connected(&s).await;
    assert_eq!(app.active_tab, Some(0));
    app.update(Message::OpenSettings);
    app.update(Message::OpenSettings);
    let tab = app.settings_tab.as_ref().expect("one settings tab");
    assert!(tab.active);
    assert_eq!(tab.section, crate::app::settings::SettingsSection::General);
    assert_eq!(app.active_tab, None);
    assert!(app.settings_shown());
    assert_eq!(app.tabs.len(), 1, "no session tab added");
}

#[tokio::test]
async fn select_tab_leaves_settings_open_and_returns() {
    let s = ssh_site("prod", OpenAs::Files);
    let (mut app, _fake, _) = connected(&s).await;
    app.update(Message::OpenSettings);
    app.update(Message::SelectTab(0));
    assert_eq!(app.active_tab, Some(0));
    let tab = app.settings_tab.as_ref().expect("still open");
    assert!(!tab.active);
    assert!(!app.settings_shown());
    // Back to it from the strip.
    app.update(Message::SelectSettings);
    assert!(app.settings_shown());
    assert_eq!(app.active_tab, None);
    // A form covers it, the strip still shows it; selecting it again drops the form.
    app.update(Message::NewSite);
    assert!(!app.settings_shown());
    assert!(app.settings_tab.as_ref().is_some_and(|t| t.active));
    app.update(Message::SelectSettings);
    assert!(app.form.is_none());
    assert!(app.settings_shown());
}

#[tokio::test]
async fn close_settings_returns_to_previous_tab() {
    let a = ssh_site("a", OpenAs::Files);
    let b = ssh_site("b", OpenAs::Files);
    let c = ssh_site("c", OpenAs::Files);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(a.clone())
            .with_site(b.clone())
            .with_site(c.clone()),
    );
    let mut app = loaded(&fake).await;
    for site in [&a, &b, &c] {
        app.update(Message::ConnectSite(site.id.clone()));
    }
    assert_eq!(app.tabs.len(), 3);
    app.update(Message::SelectTab(1));
    let from = app.tabs[1].id;
    app.update(Message::OpenSettings);
    app.update(Message::CloseSettings);
    assert!(app.settings_tab.is_none());
    assert_eq!(app.active_tab, app.tab_index(from));

    // The tab it came from closed meanwhile: the last tab.
    app.update(Message::OpenSettings);
    app.update(Message::CloseTab(from));
    assert!(
        app.settings_shown(),
        "closing another tab keeps it selected"
    );
    app.update(Message::CloseSettings);
    assert_eq!(app.active_tab, Some(app.tabs.len() - 1));

    // Not selected: closing it changes no selection.
    app.update(Message::OpenSettings);
    app.update(Message::SelectTab(0));
    app.update(Message::CloseSettings);
    assert!(app.settings_tab.is_none());
    assert_eq!(app.active_tab, Some(0));

    // No session tab at all: nothing selected after.
    let mut app = App::new();
    app.update(Message::OpenSettings);
    assert!(app.settings_shown());
    app.update(Message::CloseSettings);
    assert!(app.settings_tab.is_none());
    assert_eq!(app.active_tab, None);
}

#[tokio::test]
async fn settings_tab_not_restored_with_workspace() {
    let s = ssh_site("prod", OpenAs::Files);
    let ws = workspace("Shop", &[&s.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(s.clone())
            .with_workspace(ws.clone()),
    );
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::OpenWorkspace(ws.id.clone())));
    drive(&mut app, &fake, sites).await;
    app.update(Message::OpenSettings);
    let effects = without_panes(app.update(Message::CloseWorkspace(ws.id.clone())));
    let Some(Effect::SaveTabs { tabs, .. }) = effects.first() else {
        panic!("{effects:?}");
    };
    // Only the session tab is saved.
    assert_eq!(tabs[0].0, s.id);
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].1.len(), 1, "one saved tab: the files tab");
    assert!(app.settings_tab.is_some(), "the workspace's close keeps it");
    assert!(app.settings_shown());

    // Reopened: its tabs come back, the settings tab stays one, now not selected.
    app.update(Message::OpenWorkspace(ws.id.clone()));
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.active_tab, Some(0));
    let tab = app.settings_tab.as_ref().expect("still there");
    assert!(!tab.active);
}

#[tokio::test]
async fn ctrl_comma_blocked_by_dialog() {
    let mut app = App::new();
    let request = PromptRequest {
        by_agent: false,
        id: PromptId(1),
        session: SessionId(1),
        site: SiteId::new(),
        key: "h:22".into(),
        kind: PromptKind::Password {
            user: "u".into(),
            host: "h".into(),
        },
    };
    app.update(Message::Prompt(BrokerEvent::Request(request)));
    assert!(app.dialogs.is_open());
    app.update(Message::OpenSettings);
    assert!(app.settings_tab.is_none());
}

/// Review fix: the settings tab closes like any tab's neighbour: when the last session
/// tab goes (its ×, or the workspace's close), the settings tab left is selected.
#[tokio::test]
async fn settings_selected_when_last_session_tab_closes() {
    let s = ssh_site("prod", OpenAs::Files);
    let (mut app, _fake, _) = connected(&s).await;
    app.update(Message::OpenSettings);
    app.update(Message::SelectTab(0));
    assert!(!app.settings_shown());
    let id = app.tabs[0].id;
    app.update(Message::CloseTab(id));
    assert!(app.tabs.is_empty());
    assert!(app.settings_shown(), "the remaining tab is selected");

    // The same through closing the workspace.
    let ws = workspace("Shop", &[&s.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(s.clone())
            .with_workspace(ws.clone()),
    );
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::OpenWorkspace(ws.id.clone())));
    drive(&mut app, &fake, sites).await;
    app.update(Message::OpenSettings);
    app.update(Message::SelectTab(0));
    app.update(Message::CloseWorkspace(ws.id.clone()));
    assert!(app.tabs.is_empty());
    assert!(app.settings_shown());
}

/// Review fix: the strip's scroll counts the settings tab after the session tabs.
#[tokio::test]
async fn strip_scroll_counts_the_settings_tab() {
    let a = ssh_site("a", OpenAs::Files);
    let b = ssh_site("b", OpenAs::Files);
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()).with_site(b.clone()));
    let mut app = loaded(&fake).await;
    app.update(Message::ConnectSite(a.id.clone()));
    app.update(Message::ConnectSite(b.id.clone()));
    app.update(Message::OpenSettings);
    let effects = app.update(Message::SelectTab(1));
    let offsets: Vec<f32> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::ScrollTabs { x } => Some(*x),
            _ => None,
        })
        .collect();
    // Tab 1 of three (two session tabs and Settings): the middle.
    assert_eq!(offsets, [0.5]);
}

#[tokio::test]
async fn closing_settings_gives_a_terminal_the_keyboard_back() {
    let s = ssh_site("both", OpenAs::Both);
    let (mut app, _fake, _) = connected(&s).await;
    assert_eq!(app.active_tab, Some(0));
    app.update(Message::OpenSettings);
    // Whatever took the keyboard away while settings were up.
    app.tabs[0].terminal_mut().unwrap().focused = false;
    app.update(Message::CloseSettings);
    assert_eq!(app.active_tab, Some(0));
    assert!(app.tabs[0].terminal().unwrap().focused);
}
