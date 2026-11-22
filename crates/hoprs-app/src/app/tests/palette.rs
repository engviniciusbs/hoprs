//! Plan 6 Task 9: the Ctrl+K palette. Its fuzzy matching, the commands it lists for the
//! state the app is in, running and closing it, and its isolation: no key reaches the
//! panes or the terminal behind it, and a security dialog always wins over it.

use std::time::SystemTime;

use hoprs_core::{ConnectionState, OpenAs, TabKind};

use super::*;
use crate::app::palette::{Entry, Group, PaletteItem, Scope, fuzzy_score, rank};
use crate::backend::broker::{PromptKind, PromptRequest};
use crate::backend::{BrokerEvent, PromptId};
use crate::terminal::input::{Key, Modifiers};

const NONE: Modifiers = Modifiers {
    shift: false,
    ctrl: false,
    alt: false,
};
const CTRL_SHIFT: Modifiers = Modifiers {
    shift: true,
    ctrl: true,
    alt: false,
};

fn palette(msg: PaletteMsg) -> Message {
    Message::Palette(msg)
}

fn pressed(key: FileKey) -> Message {
    Message::KeyPressed { key, shift: false }
}

fn labels(entries: &[Entry]) -> Vec<String> {
    entries.iter().map(|e| e.label.clone()).collect()
}

fn entry<'a>(entries: &'a [Entry], label: &str) -> Option<&'a Entry> {
    entries.iter().find(|e| e.label == label)
}

/// Every command the palette lists now (no query), past the 12 rows it shows.
fn all(app: &App) -> Vec<Entry> {
    app.palette_entries()
}

fn host_key_prompt(id: u64) -> Message {
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

/// An SSH site opened as terminal and files (both tabs), connected, its shell open and
/// its panes listed.
async fn ssh_both(fake: &Arc<FakeBackend>, s: &Site) -> App {
    let mut app = loaded(fake).await;
    app.home = Some(rpath("/home/me"));
    let effects = app.update(Message::ConnectSite(s.id.clone()));
    let mut pending = run_listings(fake, &effects).await;
    let backend: Arc<dyn BackendApi> = fake.clone();
    pending.extend(
        effects::connect_sites(backend, vec![s.id.clone()])
            .collect::<Vec<_>>()
            .await,
    );
    while !pending.is_empty() {
        let mut next = Vec::new();
        for message in pending {
            let effects = app.update(message);
            next.extend(run_listings(fake, &effects).await);
            for effect in &effects {
                if let Effect::OpenShell {
                    tab,
                    request,
                    session,
                    cols,
                    rows,
                } = effect
                {
                    next.push(Message::ShellOpened {
                        tab: *tab,
                        request: *request,
                        result: fake.open_shell(*session, *cols, *rows).await,
                    });
                }
            }
        }
        pending = next;
    }
    assert_eq!(
        app.tabs.iter().map(|t| t.kind()).collect::<Vec<_>>(),
        [TabKind::Terminal, TabKind::Files]
    );
    app
}

fn ssh_site(name: &str) -> Site {
    let mut s = site(name, &format!("{name}.example"), "deploy", Protocol::Ssh);
    s.ssh.open_as = OpenAs::Both;
    s
}

fn fake_with(sites: &[&Site], workspaces: Vec<Workspace>) -> Arc<FakeBackend> {
    let mut fake = FakeBackend::new();
    for s in sites {
        fake = fake.with_site((*s).clone());
    }
    for w in workspaces {
        fake = fake.with_workspace(w);
    }
    fake.local_fs().add_dir("/home/me");
    Arc::new(fake)
}

#[test]
fn fuzzy_matches_in_order_and_ranks_word_starts() {
    // In order, case-insensitive; out of order or missing letters do not match.
    assert!(fuzzy_score("ns", "New site").is_some());
    assert!(fuzzy_score("NEW", "new site").is_some());
    assert!(fuzzy_score("nst", "New site").is_some());
    assert_eq!(fuzzy_score("sn", "New site"), None);
    assert_eq!(fuzzy_score("xyz", "New site"), None);
    assert_eq!(fuzzy_score("new sites", "New site"), None);
    // An empty query matches everything, equally.
    assert_eq!(fuzzy_score("", "New site"), Some(0));
    assert_eq!(fuzzy_score("", "Settings"), Some(0));
    // Word starts beat letters inside words.
    let word_starts = fuzzy_score("ns", "New site").unwrap();
    let inside = fuzzy_score("ns", "Options").unwrap();
    assert!(word_starts > inside, "{word_starts} > {inside}");
    // A contiguous run beats the same letters scattered over word starts.
    let run = fuzzy_score("set", "Settings").unwrap();
    let scattered = fuzzy_score("set", "Show eight items").unwrap();
    assert!(run > scattered, "{run} > {scattered}");
    // Separators count as word starts too (paths, user@host, dashes).
    let after_dash = fuzzy_score("w", "shop-web").unwrap();
    let inside_word = fuzzy_score("w", "shopweb").unwrap();
    assert!(after_dash > inside_word);
    // Ranking: best first; equal scores alphabetical (case-insensitive).
    let candidates = ["Options", "Zeta site", "New site", "alpha site", "Settings"];
    let order: Vec<&str> = rank("s", &candidates)
        .into_iter()
        .map(|i| candidates[i])
        .collect();
    assert_eq!(order[..3], ["alpha site", "New site", "Settings"]);
    assert_eq!(order.len(), 5, "every candidate holds an s");
    let order: Vec<&str> = rank("ns", &candidates)
        .into_iter()
        .map(|i| candidates[i])
        .collect();
    assert_eq!(order, ["New site", "Options", "Settings"]);
}

/// "Compare folders" is a file tab's action only, and sends the same message as the
/// Compare button.
#[tokio::test]
async fn compare_folders_is_listed_for_files_tabs_only() {
    let web = ssh_site("web");
    let fake = fake_with(&[&web], Vec::new());
    let mut app = ssh_both(&fake, &web).await;
    app.update(Message::SelectTab(0));
    assert!(entry(&all(&app), strings::palette_compare_folders()).is_none());
    app.update(Message::SelectTab(1));
    let entries = all(&app);
    let found = entry(&entries, strings::palette_compare_folders()).expect("a files tab");
    assert_eq!(found.group, Group::Action);
    let tab = app.tabs[1].id;
    app.update(palette(PaletteMsg::Open));
    app.update(palette(PaletteMsg::Query("compare folders".into())));
    assert!(matches!(
        app.palette_command(palette(PaletteMsg::Run)),
        Message::Compare(crate::app::CompareMsg::Start { tab: t }) if t == tab
    ));
}

#[tokio::test]
async fn palette_lists_context_actions() {
    // Nothing open: the global actions only.
    let app = App::new();
    let entries = all(&app);
    for label in [
        strings::new_site(),
        strings::import_filezilla(),
        strings::settings(),
        strings::palette_toggle_queue(),
        strings::palette_toggle_log(),
    ] {
        let found = entry(&entries, label).unwrap_or_else(|| panic!("{label}"));
        assert_eq!(found.group, Group::Action);
        assert!(!found.mono, "an action reads in Inter");
    }
    for label in [
        strings::new_terminal_tab(),
        strings::new_files_tab(),
        strings::reconnect(),
        strings::palette_close_tab(),
        strings::palette_open_workspace(),
        strings::new_folder(),
        strings::palette_paste(),
    ] {
        assert!(entry(&entries, label).is_none(), "{label}: {entries:?}");
    }

    // An SSH site, connected, its terminal active; a second site and a workspace saved.
    let web = ssh_site("web");
    let mut db = site("db", "db.example", "admin", Protocol::Ftp);
    db.remote_dir = Some("/srv".into());
    let ws = workspace("Shop", &[&web.id, &db.id]);
    let fake = fake_with(&[&web, &db], vec![ws.clone()]);
    fake.remote_fs(&web.id)
        .add_file("/notes.txt", b"n", SystemTime::UNIX_EPOCH);
    let mut app = ssh_both(&fake, &web).await;
    assert_eq!(app.active_tab, Some(0));
    let (terminal, files) = (app.tabs[0].id, app.tabs[1].id);
    let entries = all(&app);
    // Go to: the open tabs as "site · kind", the sites with user@host, the workspaces.
    let tab = entry(&entries, "web · terminal").expect("the terminal tab");
    assert_eq!(
        (tab.item.clone(), tab.group, tab.mono),
        (PaletteItem::Tab(terminal), Group::Tab, true)
    );
    assert!(entry(&entries, "web · files").is_some());
    let other = entry(&entries, "db").expect("a saved site");
    assert_eq!(other.item, PaletteItem::Site(db.id.clone()));
    assert_eq!(other.detail.as_deref(), Some("admin@db.example"));
    assert!(other.mono);
    let shop = entry(&entries, "Shop").expect("the workspace");
    assert_eq!(shop.item, PaletteItem::Workspace(ws.id.clone()));
    assert!(entry(&entries, strings::palette_open_workspace()).is_some());
    // Close all only while a workspace is open.
    let close_label = strings::palette_close_all("Shop");
    assert!(entry(&entries, &close_label).is_none());
    app.open_workspace = Some(ws.id.clone());
    let close = entry(&all(&app), &close_label).expect("close all").clone();
    assert_eq!(close.item, PaletteItem::CloseWorkspace(ws.id.clone()));
    assert_eq!(close.group, Group::Action);
    assert_eq!(
        format!("{:?}", app.item_message(&close.item)),
        format!("{:?}", Some(Message::CloseWorkspace(ws.id.clone())))
    );
    app.open_workspace = None;
    // The session's actions on the active (terminal) tab, with their shortcuts.
    let new_terminal = entry(&entries, strings::new_terminal_tab()).expect("new terminal tab");
    assert_eq!(new_terminal.item, PaletteItem::NewTerminalTab(terminal));
    assert!(entry(&entries, strings::new_files_tab()).is_some());
    assert_eq!(
        entry(&entries, strings::palette_close_tab()).map(|e| e.item.clone()),
        Some(PaletteItem::CloseTab(terminal))
    );
    let paste = entry(&entries, strings::palette_paste()).expect("paste");
    assert_eq!(paste.shortcut, Some(strings::palette_key_paste()));
    let find = entry(&entries, strings::palette_find()).expect("find");
    assert_eq!(find.shortcut, Some(strings::palette_key_find()));
    // Nothing selected in the terminal: no Copy. No file actions in a terminal tab.
    assert!(entry(&entries, strings::palette_copy()).is_none());
    assert!(entry(&entries, strings::new_folder()).is_none());
    assert!(entry(&entries, strings::reconnect()).is_none(), "connected");

    // The files tab: its focused pane's actions, each with its key; Rename and Delete
    // need a selection.
    app.update(Message::SelectTab(1));
    let entries = all(&app);
    let new_folder = entry(&entries, strings::new_folder()).expect("new folder");
    assert_eq!(new_folder.shortcut, Some(strings::key_new_folder()));
    assert!(entry(&entries, strings::rename()).is_none());
    assert!(entry(&entries, strings::palette_paste()).is_none());
    app.update(Message::Key {
        key: FileKey::SwitchPane,
        shift: false,
    });
    app.update(pane_msg(files, Side::Remote, PaneMsg::Click(0)));
    let entries = all(&app);
    let rename = entry(&entries, strings::rename()).expect("rename");
    assert_eq!(
        (rename.item.clone(), rename.shortcut),
        (
            PaletteItem::FileKey(FileKey::Rename, Side::Remote),
            Some(strings::key_rename())
        )
    );
    assert_eq!(
        entry(&entries, strings::delete()).and_then(|e| e.shortcut),
        Some(strings::key_delete())
    );
    assert_eq!(
        entry(&entries, strings::palette_edit()).and_then(|e| e.shortcut),
        Some(strings::key_edit())
    );
    assert_eq!(
        entry(&entries, strings::palette_download()).and_then(|e| e.shortcut),
        Some(strings::key_transfer())
    );
    // Refresh (F5), the focused pane's folder again.
    let refresh = entry(&entries, strings::menu_refresh()).expect("refresh");
    assert_eq!(
        (refresh.item.clone(), refresh.shortcut),
        (
            PaletteItem::FileKey(FileKey::Refresh, Side::Remote),
            Some(strings::key_refresh())
        )
    );

    // The connection is lost: Reconnect, and no new tab on a dead session.
    let session = app.tab_session(1).expect("a session");
    app.update(Message::SessionState {
        session,
        state: ConnectionState::Disconnected(Error::new(ErrorKind::Network, "reset")),
        auth: None,
    });
    let entries = all(&app);
    assert_eq!(
        entry(&entries, strings::reconnect()).map(|e| e.item.clone()),
        Some(PaletteItem::Reconnect(files))
    );
    assert!(entry(&entries, strings::new_terminal_tab()).is_none());

    // "Open workspace…" narrows the palette to the workspaces.
    app.update(palette(PaletteMsg::Open));
    app.update(palette(PaletteMsg::Query("open work".into())));
    assert_eq!(
        app.palette_results().first().map(|e| e.label.as_str()),
        Some(strings::palette_open_workspace())
    );
    assert!(app.update(pressed(FileKey::Enter)).is_empty());
    let open = app.palette.as_ref().expect("still open");
    assert_eq!((open.scope, open.query.as_str()), (Scope::Workspaces, ""));
    assert_eq!(labels(&app.palette_results()), ["Shop"]);
}

#[tokio::test]
async fn enter_runs_and_closes() {
    let mut app = App::new();
    // The top bar's button (or Ctrl+K) opens it, its field focused.
    let effects = app.update(palette(PaletteMsg::Open));
    assert!(matches!(effects[..], [Effect::FocusPalette]), "{effects:?}");
    assert_eq!(app.palette, Some(Default::default()));
    app.update(palette(PaletteMsg::Query("new si".into())));
    assert_eq!(
        app.palette_results().first().map(|e| e.label.as_str()),
        Some(strings::new_site())
    );
    // Enter runs the selected row through the action's own message, and closes.
    app.update(pressed(FileKey::Enter));
    assert!(app.palette.is_none());
    assert!(matches!(app.form, Some(Form::Site(_))));

    // ↓ moves the selection (wrapping), the field's submit runs it.
    app.update(palette(PaletteMsg::Open));
    app.update(palette(PaletteMsg::Query("toggle".into())));
    let results = labels(&app.palette_results());
    assert_eq!(
        results,
        [
            strings::palette_toggle_log(),
            strings::palette_toggle_queue()
        ]
    );
    app.update(pressed(FileKey::Down));
    assert_eq!(app.palette.as_ref().map(|p| p.selected), Some(1));
    app.update(pressed(FileKey::Down));
    assert_eq!(app.palette.as_ref().map(|p| p.selected), Some(0), "wraps");
    app.update(pressed(FileKey::Up));
    assert_eq!(app.palette.as_ref().map(|p| p.selected), Some(1), "wraps");
    assert!(app.queue_open);
    app.update(palette(PaletteMsg::Run));
    assert!(app.palette.is_none());
    assert!(!app.queue_open, "Toggle queue ran");
    // Typing again starts from the first row.
    app.update(palette(PaletteMsg::Open));
    app.update(pressed(FileKey::Down));
    app.update(palette(PaletteMsg::Query("set".into())));
    assert_eq!(app.palette.as_ref().map(|p| p.selected), Some(0));
    // A click on a row runs that row.
    let index = app
        .palette_results()
        .iter()
        .position(|e| e.label == strings::settings())
        .expect("settings listed");
    app.update(palette(PaletteMsg::Pick(index)));
    assert!(app.palette.is_none());
    assert!(app.settings_shown());
    // Enter with nothing matching does nothing, and keeps the palette open.
    app.update(palette(PaletteMsg::Open));
    app.update(palette(PaletteMsg::Query("zzzz".into())));
    assert!(app.palette_results().is_empty());
    assert!(app.update(pressed(FileKey::Enter)).is_empty());
    assert!(app.palette.is_some());
    app.update(palette(PaletteMsg::Close));

    // Session actions run with their effects: a new terminal tab opens its shell.
    let web = ssh_site("web");
    let fake = fake_with(&[&web], Vec::new());
    let mut app = ssh_both(&fake, &web).await;
    app.update(palette(PaletteMsg::Open));
    app.update(palette(PaletteMsg::Query("new term".into())));
    let effects = app.update(pressed(FileKey::Enter));
    assert!(app.palette.is_none());
    assert_eq!(app.tabs.len(), 3);
    assert_eq!(app.tabs[1].kind(), TabKind::Terminal);
    assert_eq!(app.active_tab, Some(1));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::OpenShell { .. })),
        "{effects:?}"
    );
    // Go to a tab: it becomes the active one (and its terminal takes the keyboard).
    app.update(palette(PaletteMsg::Open));
    app.update(palette(PaletteMsg::Query("web files".into())));
    app.update(palette(PaletteMsg::Run));
    assert_eq!(app.active_tab, Some(2));
    assert_eq!(app.tabs[2].kind(), TabKind::Files);
}

#[test]
fn escape_closes_without_effects() {
    let mut app = App::new();
    app.update(palette(PaletteMsg::Open));
    app.update(palette(PaletteMsg::Query("settings".into())));
    assert_eq!(
        app.palette.as_ref().map(|p| p.query.as_str()),
        Some("settings")
    );
    let effects = app.update(pressed(FileKey::Escape));
    assert!(effects.is_empty(), "{effects:?}");
    assert!(app.palette.is_none());
    assert!(app.form.is_none(), "nothing ran");
    assert!(app.queue_open);
    // A click on the backdrop closes it the same way.
    app.update(palette(PaletteMsg::Open));
    assert!(app.update(palette(PaletteMsg::Close)).is_empty());
    assert!(app.palette.is_none());
    // Ctrl+K again (or the button) while it is open closes it too.
    app.update(palette(PaletteMsg::Open));
    assert!(app.update(palette(PaletteMsg::Open)).is_empty());
    assert!(app.palette.is_none());
    // Messages for a closed palette do nothing.
    assert!(app.update(palette(PaletteMsg::Run)).is_empty());
    assert!(app.update(palette(PaletteMsg::Pick(0))).is_empty());
    assert!(
        app.update(palette(PaletteMsg::Query("x".into())))
            .is_empty()
    );
    assert!(app.palette.is_none());
    assert!(app.form.is_none());
}

#[tokio::test]
async fn palette_blocked_while_security_dialog_open() {
    let mut app = App::new();
    // A dialog is shown: neither the button nor Ctrl+K opens the palette.
    app.update(host_key_prompt(1));
    assert!(app.dialogs.is_open());
    assert!(app.update(palette(PaletteMsg::Open)).is_empty());
    assert!(app.palette.is_none());
    // Its keys stay the dialog's: Escape cancels it.
    let effects = app.update(pressed(FileKey::Escape));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::AnswerPrompt { .. })),
        "{effects:?}"
    );
    assert!(!app.dialogs.is_open());

    // The palette is open when a security prompt arrives: the palette closes, the dialog
    // shows (and takes the keyboard).
    app.update(palette(PaletteMsg::Open));
    app.update(palette(PaletteMsg::Query("set".into())));
    let effects = app.update(host_key_prompt(2));
    assert!(app.palette.is_none());
    assert!(app.dialogs.is_open());
    assert!(matches!(effects[..], [Effect::Unfocus]), "{effects:?}");
    // Enter is the dialog's (a decision: Cancel), never the palette's Settings.
    let effects = app.update(pressed(FileKey::Enter));
    assert!(app.form.is_none());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::AnswerPrompt { .. })),
        "{effects:?}"
    );

    // In a focused terminal, Ctrl+Shift+K opens it, except under a dialog.
    let web = ssh_site("web");
    let fake = fake_with(&[&web], Vec::new());
    let mut app = ssh_both(&fake, &web).await;
    let tab = app.tabs[0].id;
    let ctrl_shift_k = Message::Terminal {
        tab,
        msg: TerminalMsg::Key {
            key: Key::Char('K'),
            mods: CTRL_SHIFT,
        },
    };
    app.update(host_key_prompt(3));
    assert!(app.update(ctrl_shift_k.clone()).is_empty());
    assert!(app.palette.is_none());
    app.update(pressed(FileKey::Escape));
    assert!(!app.dialogs.is_open());
    let effects = app.update(ctrl_shift_k);
    assert!(matches!(effects[..], [Effect::FocusPalette]), "{effects:?}");
    assert!(app.palette.is_some());
    // While the window closes, it does not open.
    app.update(palette(PaletteMsg::Close));
    app.update(Message::CloseRequested(iced::window::Id::unique()));
    assert!(app.closing);
    assert!(app.update(palette(PaletteMsg::Open)).is_empty());
    assert!(app.palette.is_none());
}

#[tokio::test]
async fn keys_do_not_leak_to_panes_or_terminal() {
    let web = ssh_site("web");
    let fake = fake_with(&[&web], Vec::new());
    fake.remote_fs(&web.id)
        .add_file("/notes.txt", b"n", SystemTime::UNIX_EPOCH);
    let mut app = ssh_both(&fake, &web).await;
    let (terminal, files) = (app.tabs[0].id, app.tabs[1].id);

    // The terminal: with the palette open, it does not have the keyboard and what its
    // widget could still send is dropped; no byte reaches the shell.
    assert!(app.terminal_has_keyboard(&app.tabs[0]));
    app.update(palette(PaletteMsg::Open));
    assert!(app.overlay_open());
    assert!(!app.terminal_has_keyboard(&app.tabs[0]));
    for msg in [
        TerminalMsg::Key {
            key: Key::Char('x'),
            mods: NONE,
        },
        TerminalMsg::Key {
            key: Key::Enter,
            mods: NONE,
        },
        TerminalMsg::Paste,
        TerminalMsg::OpenSearch,
        TerminalMsg::Focus(false),
    ] {
        let effects = app.update(Message::Terminal {
            tab: terminal,
            msg: msg.clone(),
        });
        assert!(effects.is_empty(), "{msg:?}: {effects:?}");
    }
    assert!(app.tabs[0].terminal().unwrap().search.is_none());
    assert!(
        app.tabs[0].terminal().unwrap().focused,
        "focus kept for after"
    );
    app.update(pressed(FileKey::Escape));
    assert!(
        app.terminal_has_keyboard(&app.tabs[0]),
        "back after closing"
    );

    // The files tab: a row selected in the remote pane.
    app.update(Message::SelectTab(1));
    app.update(Message::Key {
        key: FileKey::SwitchPane,
        shift: false,
    });
    app.update(pane_msg(files, Side::Remote, PaneMsg::Click(0)));
    let before_cursor = app.tabs[1].remote().selection.cursor;
    let before_focus = app.tabs[1].focus();
    app.update(palette(PaletteMsg::Open));
    // The pane keys are the palette's now: F2, F7, Del, F4, Tab, Backspace, Ctrl+Enter do
    // nothing; ↑/↓ move the palette's selection, not the pane's cursor.
    for key in [
        FileKey::Rename,
        FileKey::NewFolder,
        FileKey::Delete,
        FileKey::Edit,
        FileKey::SwitchPane,
        FileKey::Back,
        FileKey::Transfer,
        FileKey::Home,
        FileKey::End,
        FileKey::Down,
        FileKey::Up,
    ] {
        let effects = app.update(pressed(key));
        assert!(effects.is_empty(), "{key:?}: {effects:?}");
    }
    // What the pane's widgets or a stray `KeyUnlessTyping` could send is dropped too.
    for msg in [
        Message::Key {
            key: FileKey::Rename,
            shift: false,
        },
        Message::Key {
            key: FileKey::Delete,
            shift: false,
        },
        pane_msg(files, Side::Remote, PaneMsg::Click(1)),
        Message::FilterSites("typed".into()),
        Message::CloseTab(files),
        Message::SelectTab(0),
        Message::ToggleQueue,
    ] {
        assert!(app.update(msg.clone()).is_empty(), "{msg:?}");
    }
    assert!(app.palette.is_some(), "still open");
    let pane = app.tabs[1].remote();
    assert!(pane.action.is_none(), "no editor or confirmation opened");
    assert_eq!(pane.selection.cursor, before_cursor);
    assert_eq!(app.tabs[1].focus(), before_focus);
    assert_eq!(app.active_tab, Some(1));
    assert_eq!(app.site_filter, "");
    assert_eq!(app.tabs.len(), 2);
    assert!(app.queue_open);
    // Closed: the pane's keys work again.
    app.update(pressed(FileKey::Escape));
    assert!(app.palette.is_none());
    app.update(Message::Key {
        key: FileKey::Delete,
        shift: false,
    });
    assert!(app.tabs[1].remote().action.is_some(), "Del asks again");
}

#[tokio::test]
async fn empty_query_reaches_sites_workspaces_and_settings() {
    // A files tab with a selection has many own actions; several sites and a workspace
    // are saved. Without typing, the palette still lists a few of each kind: at most 5
    // of the tab's own actions, at most 3 other tabs, then "Open workspace…", Settings
    // and the sites, then the other global actions.
    let web = ssh_site("web");
    let mut others: Vec<Site> = ["db", "nas", "blog", "mail", "wiki"]
        .iter()
        .map(|name| site(name, &format!("{name}.example"), "deploy", Protocol::Ftp))
        .collect();
    others.sort_by(|a, b| a.name.cmp(&b.name));
    let mut sites: Vec<&Site> = vec![&web];
    sites.extend(others.iter());
    let ws = workspace("Shop", &[&web.id]);
    let fake = fake_with(&sites, vec![ws]);
    fake.remote_fs(&web.id)
        .add_file("/notes.txt", b"n", SystemTime::UNIX_EPOCH);
    let mut app = ssh_both(&fake, &web).await;
    let (terminal, files) = (app.tabs[0].id, app.tabs[1].id);
    // More tabs on the session than the empty list shows.
    for _ in 0..3 {
        app.update(Message::NewTab {
            tab: terminal,
            kind: TabKind::Terminal,
        });
    }
    let index = app.tab_index(files).expect("the files tab");
    app.update(Message::SelectTab(index));
    app.update(Message::Key {
        key: FileKey::SwitchPane,
        shift: false,
    });
    app.update(pane_msg(files, Side::Remote, PaneMsg::Click(0)));
    let own = all(&app)
        .iter()
        .take_while(|e| e.group == Group::Action)
        .count();
    assert!(own > 5, "the test needs more than 5 own actions: {own}");

    app.update(palette(PaletteMsg::Open));
    let results = app.palette_results();
    assert!(results.len() <= crate::app::palette::MAX_RESULTS);
    let groups: Vec<Group> = results.iter().map(|e| e.group).collect();
    // The tab's own actions first, at most 5.
    let context = groups.iter().take_while(|g| **g == Group::Action).count();
    assert_eq!(context, 5, "{:?}", labels(&results));
    assert_eq!(results[0].label, strings::new_folder());
    // Then the other tabs (not the active one), at most 3.
    let tabs: Vec<&Entry> = results.iter().filter(|e| e.group == Group::Tab).collect();
    assert_eq!(tabs.len(), 3, "{:?}", labels(&results));
    assert!(tabs.iter().all(|e| e.item != PaletteItem::Tab(files)));
    // Go to: the workspaces, Settings and some sites are reachable without typing.
    for label in [strings::palette_open_workspace(), strings::settings()] {
        assert!(
            entry(&results, label).is_some(),
            "{label}: {:?}",
            labels(&results)
        );
    }
    assert!(
        results.iter().any(|e| e.group == Group::Site),
        "{:?}",
        labels(&results)
    );
    // Typing still searches every row.
    app.update(palette(PaletteMsg::Query("wiki".into())));
    assert_eq!(
        app.palette_results().first().map(|e| e.label.as_str()),
        Some("wiki")
    );
    app.update(palette(PaletteMsg::Query("close tab".into())));
    assert_eq!(
        app.palette_results().first().map(|e| e.label.as_str()),
        Some(strings::palette_close_tab())
    );
}

#[tokio::test]
async fn results_computed_only_when_needed() {
    use crate::app::palette::results_computed;
    let mut app = App::new();
    app.update(palette(PaletteMsg::Open));
    let before = results_computed();
    // Typing, closing the queue behind it (dropped), unrelated messages: no ranking.
    app.update(palette(PaletteMsg::Query("set".into())));
    app.update(Message::ToggleQueue);
    app.update(palette(PaletteMsg::Query("se".into())));
    assert_eq!(results_computed(), before, "nothing needed the results");
    // ↓ needs the count, Enter the row.
    app.update(pressed(FileKey::Down));
    assert_eq!(results_computed(), before + 1);
    app.update(pressed(FileKey::Enter));
    assert_eq!(results_computed(), before + 2);
    assert!(app.palette.is_none());
}

/// Plan 13 Task 8: Settings shows its key; while the settings tab is open the palette
/// goes to it like a tab, and closes it.
#[test]
fn palette_lists_the_settings_tab() {
    let mut app = App::new();
    let rows = all(&app);
    let settings = entry(&rows, strings::settings()).expect("settings listed");
    assert_eq!(settings.item, PaletteItem::Settings);
    assert_eq!(settings.shortcut, Some(strings::key_settings()));
    assert!(!rows.iter().any(|e| e.item == PaletteItem::SettingsTab));
    assert!(!rows.iter().any(|e| e.item == PaletteItem::CloseSettings));

    app.update(Message::OpenSettings);
    let rows = all(&app);
    let tab = rows
        .iter()
        .find(|e| e.item == PaletteItem::SettingsTab)
        .expect("listed among the tabs");
    assert_eq!(tab.group, Group::Tab);
    let close = entry(&rows, strings::palette_close_settings()).expect("close listed");
    assert_eq!(close.item, PaletteItem::CloseSettings);
    // An empty query offers closing it first.
    app.update(palette(PaletteMsg::Open));
    assert_eq!(
        app.palette_results().first().map(|e| e.item.clone()),
        Some(PaletteItem::CloseSettings)
    );
    app.update(palette(PaletteMsg::Run));
    assert!(app.settings_tab.is_none());
}

#[test]
fn palette_lists_export_everything() {
    use crate::app::ExportMsg;
    use hoprs_vault::ExportScope;
    let app = App::new();
    let entries = all(&app);
    let found = entry(&entries, "Export everything…").expect("the row");
    assert_eq!(found.group, Group::Action);
    assert_eq!(
        format!("{:?}", app.item_message(&found.item)),
        format!(
            "{:?}",
            Some(Message::Export(ExportMsg::Open(ExportScope::All)))
        )
    );
}
