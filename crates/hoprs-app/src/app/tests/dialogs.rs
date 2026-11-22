//! Task 9: the security and credential dialogs, and the changed host key in the tab
//! (Review Focus 4).

use std::path::PathBuf;

use hoprs_remote::{CertProblem, CertReason};
use hoprs_vault::{MemoryStore, Vault};

use super::*;
use crate::backend::broker::{KbdField, PromptBroker, PromptKind, PromptRequest};
use crate::backend::{BrokerEvent, KnownHostEntry, PromptAnswer, PromptId, VaultHandle};
use crate::ui::dialogs;

fn request(id: u64, session: u64, key: &str, kind: PromptKind) -> PromptRequest {
    PromptRequest {
        by_agent: false,
        id: PromptId(id),
        session: SessionId(session),
        site: SiteId::new(),
        key: key.into(),
        kind,
    }
}

fn host_key(other_key_types: &[&str]) -> PromptKind {
    PromptKind::HostKey {
        host: "h".into(),
        port: 22,
        key_type: "ssh-ed25519".into(),
        fingerprint: "SHA256:new".into(),
        other_key_types: other_key_types.iter().map(|t| (*t).to_owned()).collect(),
    }
}

fn password() -> PromptKind {
    PromptKind::Password {
        user: "u".into(),
        host: "h".into(),
    }
}

fn certificate(problem: CertProblem) -> PromptKind {
    PromptKind::Certificate {
        host: "ftp.example.com".into(),
        port: 21,
        fingerprint: "SHA256:BBBB".into(),
        problem,
    }
}

fn show(app: &mut App, request: PromptRequest) -> Vec<Effect> {
    app.update(Message::Prompt(BrokerEvent::Request(request)))
}

fn dialog_msg(app: &mut App, msg: DialogMsg) -> Vec<Effect> {
    app.update(Message::Dialog(msg))
}

/// The id of the dialog shown.
fn shown(app: &App) -> Option<PromptId> {
    app.dialogs.current.as_ref().map(|d| d.request.id)
}

/// The single answer among `effects`.
fn answer(effects: &[Effect]) -> (PromptId, &PromptAnswer) {
    let answers: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::AnswerPrompt { id, answer } => Some((*id, answer)),
            _ => None,
        })
        .collect();
    match answers[..] {
        [one] => one,
        _ => panic!("expected one answer, got {effects:?}"),
    }
}

/// The broker's next event, if one comes within 100 ms.
async fn next(
    events: &mut tokio::sync::mpsc::UnboundedReceiver<BrokerEvent>,
) -> Result<Option<BrokerEvent>, tokio::time::error::Elapsed> {
    tokio::time::timeout(Duration::from_millis(100), events.recv()).await
}

/// Puts the thread's language back to English when a test ends, however it ends.
struct RestoreEnglish;

impl Drop for RestoreEnglish {
    fn drop(&mut self) {
        crate::i18n::set_language(crate::i18n::Language::En);
    }
}

/// The content a dialog builds is read from the active language each time the view is
/// built, so an open dialog follows a switch at once. A security dialog is exclusive:
/// the settings page behind it takes no clicks, so the switch is set directly here.
#[test]
fn open_dialog_follows_a_language_switch() {
    use crate::app::settings::SettingsMsg;
    use crate::i18n::Language;
    use crate::settings::LanguageChoice;

    let _restore = RestoreEnglish;
    let mut app = App::new();
    show(
        &mut app,
        request(1, 1, "h:22", host_key(&["ssh-rsa", "ssh-ed25519"])),
    );
    let en = dialogs::content(app.dialogs.current.as_ref().expect("dialog open"));
    assert_eq!(en.title, "Unknown host key");
    assert_eq!(en.action, "Trust and connect");
    assert_eq!(en.rows[0].0, "Host");
    assert_eq!(en.rows[1].0, "Key type");

    // Under the dialog the settings message is not handled: the language stays.
    app.update(Message::Settings(SettingsMsg::Language(
        LanguageChoice::Fixed(Language::PtBr),
    )));
    assert_eq!(crate::i18n::language(), Language::En);

    crate::i18n::set_language(Language::PtBr);
    let pt = dialogs::content(app.dialogs.current.as_ref().expect("still open"));
    assert_eq!(pt.title, "Chave de host desconhecida");
    assert_eq!(pt.action, "Confiar e conectar");
    assert_eq!(pt.rows[1].0, "Tipo de chave");
    assert_eq!(
        pt.warning.as_deref(),
        Some(
            "Este host é conhecido com outro tipo de chave (ssh-rsa, ssh-ed25519). Alguém pode estar se passando por ele."
        )
    );
    assert_eq!(
        pt.rows[0].1, "h:22",
        "values the user did not write stay as they are"
    );

    crate::i18n::set_language(Language::En);
    let back = dialogs::content(app.dialogs.current.as_ref().expect("still open"));
    assert_eq!(back.title, "Unknown host key");
}

#[test]
fn prompts_are_queued_one_at_a_time() {
    let mut app = App::new();
    let effects = show(&mut app, request(1, 1, "h:22", host_key(&[])));
    // A decision has no field: the keyboard is taken from whatever had it behind.
    assert!(matches!(effects[..], [Effect::Unfocus]), "{effects:?}");
    // Two more arrive while the first is open: they wait, in order.
    let effects = show(&mut app, request(2, 2, "other:22", password()));
    assert!(effects.is_empty());
    show(
        &mut app,
        request(
            3,
            3,
            "ftp.example.com:21",
            certificate(CertProblem::Untrusted {
                reason: CertReason::Expired,
            }),
        ),
    );
    show(&mut app, request(4, 4, "x:22", password()));
    assert_eq!(shown(&app), Some(PromptId(1)));
    let queued: Vec<_> = app.dialogs.queued().map(|r| r.id).collect();
    assert_eq!(queued, [PromptId(2), PromptId(3), PromptId(4)]);

    // Answering the first shows the next one, and focuses its field.
    let effects = dialog_msg(&mut app, DialogMsg::Trust);
    assert!(matches!(
        answer(&effects),
        (PromptId(1), PromptAnswer::Decision(true))
    ));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::FocusDialogField(0)))
    );
    assert_eq!(shown(&app), Some(PromptId(2)));

    // A queued request that is withdrawn is gone without an answer.
    let effects = app.update(Message::Prompt(BrokerEvent::Withdrawn(PromptId(3))));
    assert!(effects.is_empty());
    assert_eq!(shown(&app), Some(PromptId(2)));
    let queued: Vec<_> = app.dialogs.queued().map(|r| r.id).collect();
    assert_eq!(queued, [PromptId(4)]);

    // The shown one withdrawn: it closes without an answer, and the next shows.
    let effects = app.update(Message::Prompt(BrokerEvent::Withdrawn(PromptId(2))));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::AnswerPrompt { .. }))
    );
    assert_eq!(shown(&app), Some(PromptId(4)));

    let effects = dialog_msg(&mut app, DialogMsg::Cancel);
    assert!(matches!(
        answer(&effects),
        (PromptId(4), PromptAnswer::Cancel)
    ));
    assert_eq!(shown(&app), None);
    assert_eq!(app.dialogs.queued().count(), 0);
    // Messages for no dialog are ignored.
    assert!(dialog_msg(&mut app, DialogMsg::Trust).is_empty());
}

/// Review Focus 4: two sessions ask about the same host key at once. Through the real
/// broker and the app, the user sees one dialog; its answer reaches both sessions.
#[tokio::test]
async fn same_host_prompt_answered_once() {
    use hoprs_ssh::Prompter;

    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path(), Arc::new(MemoryStore::new())).unwrap();
    let broker = PromptBroker::new(VaultHandle::new(vault));
    let mut events = broker.take_events().unwrap();
    let site = SiteId::new();
    let ask = |session: u64| {
        let prompter = broker.session(SessionId(session), site.clone(), "h", 22);
        tokio::spawn(async move {
            prompter
                .confirm_host_key("h", 22, "ssh-ed25519", "SHA256:new", &[])
                .await
        })
    };
    let first = ask(1);
    let second = ask(2);

    // Everything the broker sends reaches the app, as the subscription does.
    let mut app = App::new();
    while let Ok(Some(event)) = next(&mut events).await {
        app.update(Message::Prompt(event));
    }
    let id = shown(&app).expect("one dialog");
    assert_eq!(app.dialogs.queued().count(), 0, "not a second one");

    let effects = dialog_msg(&mut app, DialogMsg::Trust);
    let (answered, decision) = answer(&effects);
    assert_eq!(answered, id);
    assert!(broker.answer(answered, decision.clone()));
    for ask in [first, second] {
        let trusted = tokio::time::timeout(Duration::from_secs(1), ask)
            .await
            .expect("the session got the answer")
            .unwrap();
        assert!(trusted);
    }
    assert!(next(&mut events).await.is_err(), "no other dialog");
    assert_eq!(shown(&app), None);
}

#[test]
fn escape_rejects() {
    let mut app = App::new();
    let esc = Message::KeyPressed {
        key: FileKey::Escape,
        shift: false,
    };
    let enter = Message::KeyPressed {
        key: FileKey::Enter,
        shift: false,
    };

    show(&mut app, request(1, 1, "h:22", host_key(&[])));
    // While a dialog is open, the panes get no key.
    assert!(
        app.update(Message::Key {
            key: FileKey::Down,
            shift: false
        })
        .is_empty()
    );
    let effects = app.update(esc.clone());
    assert!(matches!(
        answer(&effects),
        (PromptId(1), PromptAnswer::Cancel)
    ));
    assert_eq!(shown(&app), None);

    // Enter in a trust decision is the safe button: Cancel, never Trust.
    show(
        &mut app,
        request(
            2,
            1,
            "h:21",
            certificate(CertProblem::Untrusted {
                reason: CertReason::UnknownIssuer,
            }),
        ),
    );
    let effects = app.update(enter);
    assert!(matches!(
        answer(&effects),
        (PromptId(2), PromptAnswer::Cancel)
    ));

    // A credential dialog too: Esc refuses, whatever was typed.
    show(&mut app, request(3, 1, "h:22", password()));
    dialog_msg(&mut app, DialogMsg::Input(0, Typed("typed".into())));
    let effects = app.update(esc);
    assert!(matches!(
        answer(&effects),
        (PromptId(3), PromptAnswer::Cancel)
    ));
    assert_eq!(shown(&app), None);
}

/// A connect refused because the host key changed (as the SSH session reports it).
async fn changed_host_key_tab(fake: &Arc<FakeBackend>, site: &Site) -> App {
    let mut app = loaded(fake).await;
    fake.fail_next_connect(
        &site.id,
        hoprs_ssh::testing::changed_host_key_error(&site.host, 3),
    );
    let sites = connect_effect(app.update(Message::ConnectSite(site.id.clone())));
    drive(&mut app, fake, sites).await;
    app
}

/// Review Focus 4: a changed host key is explained in the tab, blocking: no dialog, no
/// way to accept the new key, only "Remove the old entry…" (and Edit site).
#[tokio::test]
async fn changed_host_key_has_no_accept() {
    let s = site("s", "h", "u", Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(s.clone()));
    let mut app = changed_host_key_tab(&fake, &s).await;

    let tab = &app.tabs[app.tab_of(&s.id).unwrap()];
    assert!(matches!(app.conn(&s.id).state, TabState::Failed(_)));
    assert_eq!(
        app.conn(&s.id).host_key_change(),
        Some(hoprs_ssh::HostKeyChange { line: Some(3) })
    );
    assert_eq!(
        app.failure_actions(tab),
        [FailureAction::RemoveOldEntry, FailureAction::EditSite],
        "no Reconnect, and nothing that accepts the key"
    );
    assert!(!app.dialogs.is_open(), "not a dialog");
    // "Trust" has nothing to act on.
    assert!(dialog_msg(&mut app, DialogMsg::Trust).is_empty());

    // Another failure keeps Reconnect.
    let other = site("o", "h2", "u", Protocol::Ssh);
    fake.save_site(other.clone()).await.unwrap();
    app.update(profiles_loaded(&app, fake.profiles().await));
    fake.fail_next_connect(&other.id, Error::new(ErrorKind::Network, "down"));
    let sites = connect_effect(app.update(Message::ConnectSite(other.id.clone())));
    drive(&mut app, &fake, sites).await;
    let tab = &app.tabs[app.tab_of(&other.id).unwrap()];
    assert_eq!(app.conn(&other.id).host_key_change(), None);
    assert_eq!(
        app.failure_actions(tab),
        [FailureAction::Reconnect, FailureAction::EditSite]
    );
}

fn entry(line: usize, covers_other_hosts: bool) -> KnownHostEntry {
    KnownHostEntry {
        line,
        key_type: "ssh-ed25519".into(),
        covers_other_hosts,
        hashed: false,
        text: format!("h{line} ssh-ed25519 AAAA"),
    }
}

/// Review Focus 4: "Remove the old entry…" shows the lines first, warns when one also
/// covers other hosts, removes only on confirmation, then connects again.
#[tokio::test]
async fn remove_entry_warns_on_multi_host_line() {
    let mut s = site("s", "h", "u", Protocol::Ssh);
    s.port = 2222;
    let fake = Arc::new(FakeBackend::new().with_site(s.clone()));
    let mut app = changed_host_key_tab(&fake, &s).await;
    let tab = app.tabs[app.tab_of(&s.id).unwrap()].id;
    let index = app.tab_of(&s.id).unwrap();
    let TabState::Failed(failure) = &app.tab_conn(index).state else {
        panic!("failed tab");
    };
    let old_session = failure.session.expect("the failed session is kept");
    fake.set_known_host_entries(vec![entry(3, true), entry(7, false)]);
    // The site is edited after the failure: its failed session is dropped (its endpoint
    // is no longer the site's), and the fix still works on the endpoint that failed.
    let mut edited = s.clone();
    edited.host = "elsewhere".into();
    edited.port = 22;
    let mut profiles = fake.profiles().await.unwrap();
    profiles.sites = vec![edited];
    let effects = app.update(profiles_loaded(&app, Ok(profiles)));
    assert!(
        matches!(effects[..], [Effect::CloseSession(closed)] if closed == old_session),
        "{effects:?}"
    );
    fake.close(old_session).await;
    let backend: Arc<dyn BackendApi> = fake.clone();

    let effects = app.update(Message::KnownHosts {
        tab,
        msg: KnownHostsMsg::Start,
    });
    let Ok([Effect::KnownHostEntries { host, port, .. }]) = <[Effect; 1]>::try_from(effects) else {
        panic!("one KnownHostEntries effect");
    };
    assert_eq!((host.as_str(), port), ("h", 2222));
    assert_eq!(app.tab_conn(index).known_hosts, KnownHostsFix::Checking);
    let message = effects::known_host_entries(backend.clone(), s.id.clone(), host, port).await;
    assert!(app.update(message).is_empty());
    let KnownHostsFix::Confirm(entries) = &app.tab_conn(index).known_hosts else {
        panic!("{:?}", app.tab_conn(index).known_hosts);
    };
    let warning = dialogs::known_hosts_warning(entries).expect("a warning");
    assert!(
        warning.starts_with("Line 3 also covers other hosts"),
        "{warning}"
    );
    assert!(!warning.contains('7'), "{warning}");
    assert!(dialogs::known_hosts_warning(&[entry(7, false)]).is_none());
    assert!(fake.known_host_removals().is_empty(), "nothing removed yet");

    // Keep: back to the explanation, nothing removed.
    app.update(Message::KnownHosts {
        tab,
        msg: KnownHostsMsg::Cancel,
    });
    assert_eq!(app.tab_conn(index).known_hosts, KnownHostsFix::Idle);

    // Again, and confirm: removed, then the tab connects again (the next connect asks
    // about the new key in a dialog).
    let effects = app.update(Message::KnownHosts {
        tab,
        msg: KnownHostsMsg::Start,
    });
    let Ok([Effect::KnownHostEntries { host, port, .. }]) = <[Effect; 1]>::try_from(effects) else {
        panic!("one KnownHostEntries effect");
    };
    let message = effects::known_host_entries(backend.clone(), s.id.clone(), host, port).await;
    app.update(message);
    let effects = app.update(Message::KnownHosts {
        tab,
        msg: KnownHostsMsg::Confirm,
    });
    let Ok(
        [
            Effect::RemoveKnownHost {
                host,
                port,
                expected,
                ..
            },
        ],
    ) = <[Effect; 1]>::try_from(effects)
    else {
        panic!("one RemoveKnownHost effect");
    };
    assert_eq!(
        expected,
        [entry(3, true), entry(7, false)],
        "the lines shown"
    );
    assert_eq!(app.tab_conn(index).known_hosts, KnownHostsFix::Removing);
    let message =
        effects::remove_known_host(backend.clone(), s.id.clone(), host, port, expected).await;
    let effects = app.update(message);
    assert_eq!(fake.known_host_removals(), [("h".to_owned(), 2222)]);
    // A new session connects (a reused session would not check known_hosts the same
    // way); the old one was already closed when the site changed.
    let Ok([Effect::RestartSite { close, site }]) = <[Effect; 1]>::try_from(effects) else {
        panic!("one RestartSite effect");
    };
    assert_eq!((close, &site), (None, &s.id));
    assert_eq!(app.tab_conn(index).state, TabState::Connecting);
    assert_eq!(app.tab_conn(index).known_hosts, KnownHostsFix::Idle);
    let messages: Vec<Message> = effects::restart_site(backend.clone(), close, site)
        .collect()
        .await;
    for message in messages {
        app.update(message);
    }
    assert_eq!(fake.closed(), [old_session]);
    let TabState::Connected(connected) = &app.tab_conn(index).state else {
        panic!("{:?}", app.tab_conn(index).state);
    };
    assert_ne!(connected.session, old_session, "a new session");

    // The file changed between the preview and the confirmation: refused inline, nothing
    // removed ("review again").
    fake.close(connected.session).await;
    let mut app = changed_host_key_tab(&fake, &s).await;
    let tab = app.tabs[0].id;
    fake.set_known_host_entries(vec![entry(3, false)]);
    app.update(Message::KnownHosts {
        tab,
        msg: KnownHostsMsg::Start,
    });
    let message =
        effects::known_host_entries(backend.clone(), s.id.clone(), "h".into(), 2222).await;
    app.update(message);
    fake.set_known_host_entries(vec![entry(3, false), entry(9, true)]);
    let effects = app.update(Message::KnownHosts {
        tab,
        msg: KnownHostsMsg::Confirm,
    });
    let Ok(
        [
            Effect::RemoveKnownHost {
                host,
                port,
                expected,
                ..
            },
        ],
    ) = <[Effect; 1]>::try_from(effects)
    else {
        panic!("one RemoveKnownHost effect");
    };
    let message = effects::remove_known_host(backend, s.id.clone(), host, port, expected).await;
    assert!(app.update(message).is_empty());
    let KnownHostsFix::Failed(error) = &app.tab_conn(0).known_hosts else {
        panic!("{:?}", app.tab_conn(0).known_hosts);
    };
    assert_eq!(error.kind, ErrorKind::Conflict);
    assert_eq!(fake.known_host_removals().len(), 1, "nothing more removed");

    // A failed removal shows inline and keeps the action.
    let mut app = changed_host_key_tab(&fake, &s).await;
    let tab = app.tabs[0].id;
    app.update(Message::KnownHosts {
        tab,
        msg: KnownHostsMsg::Start,
    });
    app.update(Message::KnownHostEntries {
        site: s.id.clone(),
        result: Ok(vec![entry(3, false)]),
    });
    app.update(Message::KnownHosts {
        tab,
        msg: KnownHostsMsg::Confirm,
    });
    let error = Error::new(ErrorKind::Permission, "Permission denied");
    assert!(
        app.update(Message::KnownHostRemoved {
            site: s.id.clone(),
            result: Err(error.clone()),
        })
        .is_empty()
    );
    assert_eq!(app.tab_conn(0).known_hosts, KnownHostsFix::Failed(error));
    assert!(matches!(app.tab_conn(0).state, TabState::Failed(_)));
}

/// Review Focus 4: a changed certificate shows the fingerprint trusted before and the one
/// presented now.
#[test]
fn cert_changed_shows_both_fingerprints() {
    let mut app = App::new();
    show(
        &mut app,
        request(
            1,
            1,
            "ftp.example.com:21",
            certificate(CertProblem::Changed {
                pinned: "SHA256:AAAA".into(),
                reason: CertReason::UnknownIssuer,
            }),
        ),
    );
    let content = dialogs::content(app.dialogs.current.as_ref().unwrap());
    assert_eq!(content.title, strings::cert_changed_title());
    assert!(
        content
            .rows
            .contains(&(strings::label_trusted_before(), "SHA256:AAAA".into()))
    );
    assert!(
        content
            .rows
            .contains(&(strings::label_presented_now(), "SHA256:BBBB".into()))
    );
    assert!(
        content
            .rows
            .contains(&(strings::label_host(), "ftp.example.com:21".into()))
    );
    assert!(content.warning.is_some());
    assert_eq!(content.action, strings::trust_certificate());
    assert!(content.decision);

    // An untrusted one shows its fingerprint and reason; no "trusted before".
    let untrusted = PromptRequest {
        by_agent: false,
        kind: certificate(CertProblem::Untrusted {
            reason: CertReason::WrongName,
        }),
        ..request(2, 1, "ftp.example.com:21", password())
    };
    dialog_msg(&mut app, DialogMsg::Cancel);
    show(&mut app, untrusted);
    let content = dialogs::content(app.dialogs.current.as_ref().unwrap());
    assert_eq!(content.title, strings::cert_untrusted_title());
    assert!(
        content
            .rows
            .contains(&(strings::label_fingerprint(), "SHA256:BBBB".into()))
    );
    assert!(content.rows.contains(&(
        strings::label_reason(),
        "Issued for a different name".into()
    )));
    assert!(
        !content
            .rows
            .iter()
            .any(|(l, _)| *l == strings::label_trusted_before())
    );

    // A host key known with another type warns of impersonation.
    dialog_msg(&mut app, DialogMsg::Cancel);
    show(
        &mut app,
        request(3, 1, "h:22", host_key(&["ecdsa-sha2-nistp256"])),
    );
    let content = dialogs::content(app.dialogs.current.as_ref().unwrap());
    assert_eq!(
        content.warning.as_deref(),
        Some(
            "This host is known with a different key type (ecdsa-sha2-nistp256). \
             Someone may be impersonating it."
        )
    );
    assert!(
        content
            .rows
            .contains(&(strings::label_fingerprint(), "SHA256:new".into()))
    );
    assert_eq!(content.action, strings::trust_and_connect());
    dialog_msg(&mut app, DialogMsg::Cancel);
    show(&mut app, request(4, 1, "h:22", host_key(&[])));
    assert!(
        dialogs::content(app.dialogs.current.as_ref().unwrap())
            .warning
            .is_none()
    );
}

#[test]
fn cert_reason_text() {
    for (reason, text) in [
        (CertReason::Expired, "Expired"),
        (CertReason::NotYetValid, "Not valid yet"),
        (CertReason::WrongName, "Issued for a different name"),
        (
            CertReason::UnknownIssuer,
            "Unknown authority (self-signed or untrusted issuer)",
        ),
        (CertReason::Other("BadSignature".into()), "BadSignature"),
    ] {
        assert_eq!(strings::cert_reason(&reason), text, "{reason:?}");
    }
}

#[test]
fn password_cleared_after_submit() {
    let mut app = App::new();
    let effects = show(&mut app, request(1, 1, "h:22", password()));
    assert!(
        matches!(effects[..], [Effect::FocusDialogField(0)]),
        "{effects:?}"
    );
    show(&mut app, request(2, 2, "h2:22", password()));
    dialog_msg(&mut app, DialogMsg::Input(0, Typed("hunter2".into())));
    dialog_msg(&mut app, DialogMsg::Save(true));
    assert!(
        !format!("{app:?}").contains("hunter2"),
        "Debug never shows it"
    );

    let effects = dialog_msg(&mut app, DialogMsg::Submit);
    assert!(!format!("{effects:?}").contains("hunter2"), "{effects:?}");
    let (id, PromptAnswer::Secret { secret, save }) = answer(&effects) else {
        panic!("{effects:?}");
    };
    assert_eq!(
        (id, secret.expose_secret(), *save),
        (PromptId(1), "hunter2", true)
    );
    // The next dialog starts empty, unticked; nothing typed is left in the state.
    let next = app.dialogs.current.as_ref().unwrap();
    assert_eq!(next.request.id, PromptId(2));
    assert_eq!(next.fields, [Typed::default()]);
    assert!(!next.save);
    assert!(app.dialogs.queued().next().is_none());

    // Keyboard-interactive: one answer per prompt, in order; echo shown in clear.
    dialog_msg(&mut app, DialogMsg::Cancel);
    let kind = PromptKind::KeyboardInteractive {
        name: String::new(),
        instructions: "Two steps".into(),
        prompts: vec![
            KbdField {
                text: "Code: ".into(),
                echo: true,
            },
            KbdField {
                text: "Password: ".into(),
                echo: false,
            },
        ],
    };
    show(&mut app, request(3, 1, "h:22", kind));
    let content = dialogs::content(app.dialogs.current.as_ref().unwrap());
    assert_eq!(
        content.fields,
        [("Code:".to_owned(), true), ("Password:".to_owned(), false)]
    );
    assert!(
        !content.save,
        "keyboard-interactive answers are never saved"
    );
    assert_eq!(content.title, strings::kbd_title());
    dialog_msg(&mut app, DialogMsg::Input(0, Typed("123456".into())));
    dialog_msg(&mut app, DialogMsg::Input(1, Typed("s3cret".into())));
    let effects = dialog_msg(&mut app, DialogMsg::Submit);
    let debug = format!("{effects:?}");
    assert!(
        !debug.contains("123456") && !debug.contains("s3cret"),
        "{debug}"
    );
    let (_, PromptAnswer::Responses(responses)) = answer(&effects) else {
        panic!("{effects:?}");
    };
    let got: Vec<_> = responses.iter().map(|r| r.expose_secret()).collect();
    assert_eq!(got, ["123456", "s3cret"]);
    assert_eq!(shown(&app), None);

    // A passphrase offers saving too; its key file is shown.
    show(
        &mut app,
        request(
            4,
            1,
            "h:22",
            PromptKind::Passphrase {
                path: PathBuf::from("/home/u/.ssh/id_ed25519"),
            },
        ),
    );
    let content = dialogs::content(app.dialogs.current.as_ref().unwrap());
    assert!(content.save);
    assert!(
        content
            .rows
            .contains(&(strings::label_key_file(), "/home/u/.ssh/id_ed25519".into()))
    );
}

/// "Save in the system keyring" stores the typed password only once the session it was
/// typed for connects; a failed connect drops it.
#[tokio::test]
async fn save_in_keyring_only_after_success() {
    let ok = site("ok", "h", "u", Protocol::Ssh);
    let bad = site("bad", "h2", "u", Protocol::Ftp);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(ok.clone())
            .with_site(bad.clone()),
    );
    let mut app = loaded(&fake).await;
    let mut prompts = fake.prompt_events().unwrap();
    let backend: Arc<dyn BackendApi> = fake.clone();
    fake.fail_next_connect(&bad.id, Error::new(ErrorKind::Auth, "Denied"));

    for (s, connects) in [(&ok, true), (&bad, false)] {
        let release = fake.hold_connect(&s.id);
        let sites = connect_effect(app.update(Message::ConnectSite(s.id.clone())));
        let connecting =
            tokio::spawn(effects::connect_sites(backend.clone(), sites).collect::<Vec<_>>());
        // The session exists (it asks for its password while connecting).
        let mut session = None;
        for _ in 0..100 {
            session = (1..10)
                .map(SessionId)
                .find(|id| fake.session_site(*id).as_ref() == Some(&s.id));
            if session.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let session = session.expect("the session started");
        let id = PromptId(session.0 + 100);
        fake.push_prompt(BrokerEvent::Request(request(
            id.0,
            session.0,
            "h:22",
            password(),
        )));
        let event = prompts.recv().await.unwrap();
        app.update(Message::Prompt(event));
        dialog_msg(&mut app, DialogMsg::Input(0, Typed("hunter2".into())));
        dialog_msg(&mut app, DialogMsg::Save(true));
        let effects = dialog_msg(&mut app, DialogMsg::Submit);
        let (answered, reply) = answer(&effects);
        assert_eq!(answered, id);
        effects::answer_prompt(backend.clone(), answered, reply.clone()).await;
        assert!(
            fake.password(&s.id).is_none(),
            "not before the connect ends"
        );

        release.send(()).unwrap();
        let messages = connecting.await.unwrap();
        let effects: Vec<Effect> = messages.into_iter().flat_map(|m| app.update(m)).collect();
        let reloads = effects
            .iter()
            .filter(|e| matches!(e, Effect::LoadProfiles(_)))
            .count();
        if connects {
            assert_eq!(
                fake.password(&s.id).map(|p| p.expose_secret().to_owned()),
                Some("hunter2".into())
            );
            assert_eq!(reloads, 1, "the site now shows its saved password");
        } else {
            assert!(
                fake.password(&s.id).is_none(),
                "a failed connect saves nothing"
            );
            assert_eq!(reloads, 0);
        }
    }
    assert!(fake.sites().await.unwrap()[0].has_password);
    assert!(!fake.sites().await.unwrap()[1].has_password);
}

/// The dialogs' subscription: each broker event as `Message::Prompt`, in order; the
/// channel is handed out once, so a second feed is empty.
#[tokio::test]
async fn prompt_feed_delivers_broker_events() {
    let fake = FakeBackend::new();
    let mut feed = Box::pin(effects::prompt_feed(&fake));
    fake.push_prompt(BrokerEvent::Request(request(1, 1, "h:22", password())));
    fake.push_prompt(BrokerEvent::Withdrawn(PromptId(1)));
    let first = feed.next().await;
    assert!(
        matches!(first, Some(Message::Prompt(BrokerEvent::Request(ref r))) if r.id == PromptId(1)),
        "{first:?}"
    );
    let second = feed.next().await;
    assert!(
        matches!(
            second,
            Some(Message::Prompt(BrokerEvent::Withdrawn(PromptId(1))))
        ),
        "{second:?}"
    );
    let mut again = Box::pin(effects::prompt_feed(&fake));
    assert!(again.next().await.is_none());
}

/// A security dialog takes the keyboard from the window behind it: a rename editor that
/// had the focus can't commit while the dialog is open, and Enter means Cancel.
#[tokio::test]
async fn dialog_takes_the_keyboard_from_a_rename_editor() {
    let mut a = site("a", "h", "u", Protocol::Ftp);
    a.remote_dir = Some("/srv".into());
    let fake = Arc::new(FakeBackend::new().with_site(a.clone()));
    fake.remote_fs(&a.id)
        .add_file("/srv/report.pdf", b"r", std::time::SystemTime::UNIX_EPOCH);
    fake.local_fs().add_dir("/home/me");
    let mut app = connected_tab(&fake, &a).await;
    let tab = app.tabs[0].id;
    let key = |key| Message::Key { key, shift: false };
    app.update(key(FileKey::SwitchPane));
    app.update(pane_msg(tab, Side::Remote, PaneMsg::Click(1)));
    app.update(key(FileKey::Rename));
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::EditChanged("final.pdf".into()),
    ));
    let editing = app.tabs[0].remote().action.clone();
    assert!(editing.is_some(), "the rename editor is open");

    // The dialog opens: the editor loses the keyboard.
    let effects = show(&mut app, request(1, 1, "h:21", host_key(&[])));
    assert!(matches!(effects[..], [Effect::Unfocus]), "{effects:?}");
    // Enter is the dialog's: Cancel, and no rename.
    let effects = app.update(Message::KeyPressed {
        key: FileKey::Enter,
        shift: false,
    });
    assert!(matches!(
        answer(&effects),
        (PromptId(1), PromptAnswer::Cancel)
    ));
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::FileOp { .. })),
        "{effects:?}"
    );

    // While a dialog is open, what a focused field behind it could still send is dropped.
    show(&mut app, request(2, 1, "h:21", host_key(&[])));
    for blocked in [
        pane_msg(tab, Side::Remote, PaneMsg::EditCommit),
        pane_msg(tab, Side::Remote, PaneMsg::EditChanged("x".into())),
        key(FileKey::Enter),
        Message::FilterSites("typed".into()),
        Message::ConnectSite(a.id.clone()),
        Message::CloseTab(tab),
    ] {
        assert!(app.update(blocked.clone()).is_empty(), "{blocked:?}");
    }
    assert_eq!(
        app.tabs[0].remote().action,
        editing,
        "the editor is untouched"
    );
    assert_eq!(app.site_filter, "");
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(shown(&app), Some(PromptId(2)));
    // Scroll reports still land (the lists keep their offsets right).
    app.update(pane_msg(
        tab,
        Side::Remote,
        PaneMsg::Scrolled {
            offset_y: 30.0,
            viewport_h: 300.0,
        },
    ));
    assert_eq!(app.tabs[0].remote().offset_y, 30.0);

    // Closed: the editor works again.
    app.update(Message::KeyPressed {
        key: FileKey::Escape,
        shift: false,
    });
    let commit = app.update(pane_msg(tab, Side::Remote, PaneMsg::EditCommit));
    assert!(
        commit.iter().any(|e| matches!(e, Effect::FileOp { .. })),
        "{commit:?}"
    );
}

/// Tab and Shift+Tab cycle through the dialog's own fields, never into the window behind
/// it; a decision has no field, so Tab does nothing there.
#[test]
fn tab_cycles_within_dialog_fields() {
    let mut app = App::new();
    let tab = |shift| Message::KeyPressed {
        key: FileKey::SwitchPane,
        shift,
    };
    let prompts = (0..3)
        .map(|n| KbdField {
            text: format!("Q{n}"),
            echo: false,
        })
        .collect();
    let kind = PromptKind::KeyboardInteractive {
        name: String::new(),
        instructions: String::new(),
        prompts,
    };
    let focus = |effects: Vec<Effect>| match effects[..] {
        [Effect::FocusDialogField(n)] => n,
        _ => panic!("{effects:?}"),
    };
    assert_eq!(focus(show(&mut app, request(1, 1, "h:22", kind))), 0);
    assert_eq!(focus(app.update(tab(false))), 1);
    assert_eq!(focus(app.update(tab(false))), 2);
    assert_eq!(focus(app.update(tab(false))), 0, "wraps to the first");
    assert_eq!(focus(app.update(tab(true))), 2, "Shift+Tab goes back");
    // Typing in a field (after a click into it) moves the cycle there.
    dialog_msg(&mut app, DialogMsg::Input(1, Typed("x".into())));
    assert_eq!(focus(app.update(tab(false))), 2);

    dialog_msg(&mut app, DialogMsg::Cancel);
    show(&mut app, request(2, 1, "h:22", host_key(&[])));
    assert!(app.update(tab(false)).is_empty(), "no field to move to");
    assert!(app.update(tab(true)).is_empty());
}

/// Review Focus 4: the key changed from the one this session verified (not a known_hosts
/// line). SSH pins the key on reconnect, so this only happens to a session that already
/// connected: its feed reports `Disconnected` with the pinned-key error while the tab is
/// still `Connected`. The lost banner then explains it (no Reconnect, which would always
/// fail), and "Close and reconnect" closes the session and connects with a new one.
#[tokio::test]
async fn pinned_key_change_offers_close_and_reconnect() {
    let s = site("s", "h", "u", Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(s.clone()));
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(s.id.clone())));
    drive(&mut app, &fake, sites).await;
    let tab = app.tabs[0].id;
    let old = app.tab_conn(0).session().expect("connected");

    // The session reconnects on its own and finds another key than the one it pinned.
    app.update(Message::SessionState {
        session: old,
        state: hoprs_core::ConnectionState::Disconnected(
            hoprs_ssh::testing::pinned_key_changed_error("h"),
        ),
        auth: None,
    });
    assert!(matches!(app.tab_conn(0).state, TabState::Connected(_)));
    assert_eq!(
        app.tab_conn(0).host_key_change(),
        Some(hoprs_ssh::HostKeyChange { line: None })
    );
    assert_eq!(
        crate::ui::files::banner(app.tab_conn(0)),
        Some(crate::ui::files::Banner::HostKeyChanged),
        "the banner explains the changed key"
    );
    assert_eq!(
        app.failure_actions(&app.tabs[0]),
        [FailureAction::CloseAndReconnect, FailureAction::EditSite],
        "no Reconnect"
    );
    assert!(
        app.update(Message::ReconnectSession(tab)).is_empty(),
        "the session's reconnect can't help"
    );
    // No known_hosts edit for it.
    assert!(
        app.update(Message::KnownHosts {
            tab,
            msg: KnownHostsMsg::Start
        })
        .is_empty()
    );
    assert_eq!(app.tab_conn(0).known_hosts, KnownHostsFix::Idle);

    // A password typed with "save" for the old session is forgotten with it.
    show(&mut app, request(9, old.0, "h:22", password()));
    dialog_msg(&mut app, DialogMsg::Save(true));
    dialog_msg(&mut app, DialogMsg::Submit);
    assert!(app.dialogs.is_saving(old));

    let effects = app.update(Message::RestartSession(tab));
    let Ok([Effect::RestartSite { close, site }]) = <[Effect; 1]>::try_from(effects) else {
        panic!("one RestartSite effect");
    };
    assert_eq!((close, &site), (Some(old), &s.id));
    assert!(!app.dialogs.is_saving(old));
    assert_eq!(app.tab_conn(0).state, TabState::Connecting);
    assert_eq!(app.tab_conn(0).conn, None);
    assert_eq!(crate::ui::files::banner(app.tab_conn(0)), None);
    let backend: Arc<dyn BackendApi> = fake.clone();
    let messages: Vec<Message> = effects::restart_site(backend, close, site).collect().await;
    for message in messages {
        app.update(message);
    }
    assert_eq!(fake.closed(), [old], "closed before the new connect");
    let TabState::Connected(connected) = &app.tab_conn(0).state else {
        panic!("{:?}", app.tab_conn(0).state);
    };
    assert_ne!(connected.session, old, "a new session");
    assert_eq!(app.tab_conn(0).host_key_change(), None);

    // Closing a tab forgets its session's pending save too.
    let session = connected.session;
    show(&mut app, request(10, session.0, "h:22", password()));
    dialog_msg(&mut app, DialogMsg::Save(true));
    dialog_msg(&mut app, DialogMsg::Submit);
    assert!(app.dialogs.is_saving(session));
    app.update(Message::CloseTab(tab));
    assert!(!app.dialogs.is_saving(session));
}

/// A connection lost for another reason keeps the plain banner with Reconnect.
#[tokio::test]
async fn plain_loss_keeps_reconnect() {
    let s = site("s", "h", "u", Protocol::Ssh);
    let fake = Arc::new(FakeBackend::new().with_site(s.clone()));
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(s.id.clone())));
    drive(&mut app, &fake, sites).await;
    let tab = app.tabs[0].id;
    let session = app.tab_conn(0).session().unwrap();
    let lost = Error::new(ErrorKind::Network, "Connection lost");
    app.update(Message::SessionState {
        session,
        state: hoprs_core::ConnectionState::Disconnected(lost.clone()),
        auth: None,
    });
    assert_eq!(app.tab_conn(0).host_key_change(), None);
    assert_eq!(
        crate::ui::files::banner(app.tab_conn(0)),
        Some(crate::ui::files::Banner::Lost(&lost))
    );
    assert!(matches!(
        app.update(Message::ReconnectSession(tab))[..],
        [Effect::Reconnect { .. }]
    ));
}

/// A tab closed while it connects takes its site's dialogs with it: the one shown and
/// those waiting are dropped (their sessions are closed, which withdraws them in the
/// broker and forgets a secret typed with "save"), and a dialog the connect still raises
/// afterwards is never shown. Another site's dialog stays. A dialog blocks the × of a
/// tab, but "Close tabs" of the open workspace (or a profile reload) still closes tabs.
#[tokio::test]
async fn closing_a_connecting_tab_drops_its_prompts() {
    let s = site("s", "h", "u", Protocol::Ssh);
    let t = site("t", "k", "u", Protocol::Ssh);
    let w = workspace("w", &[&s.id]);
    let fake = Arc::new(
        FakeBackend::new()
            .with_site(s.clone())
            .with_site(t.clone())
            .with_workspace(w.clone()),
    );
    let mut app = loaded(&fake).await;
    connect_effect(app.update(Message::OpenWorkspace(w.id.clone())));
    assert_eq!(app.tab_conn(0).state, TabState::Connecting);
    let of = |site: &Site, id: u64, session: u64, kind: PromptKind| PromptRequest {
        by_agent: false,
        site: site.id.clone(),
        ..request(id, session, "h:22", kind)
    };
    let closed = |effects: &[Effect]| -> Vec<SessionId> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::CloseSession(session) => Some(*session),
                _ => None,
            })
            .collect()
    };

    // A password for s, typed with "save"; then a host key for s and a password for t.
    show(&mut app, of(&s, 1, 5, password()));
    dialog_msg(&mut app, DialogMsg::Input(0, Typed("pw".into())));
    dialog_msg(&mut app, DialogMsg::Save(true));
    show(&mut app, of(&s, 2, 5, host_key(&[])));
    show(&mut app, of(&t, 3, 6, password()));
    dialog_msg(&mut app, DialogMsg::Submit);
    assert!(app.dialogs.is_saving(SessionId(5)));
    assert_eq!(shown(&app), Some(PromptId(2)));

    let effects = app.update(Message::CloseWorkspace(w.id.clone()));
    assert!(app.tabs.is_empty());
    assert_eq!(closed(&effects), [SessionId(5)], "{effects:?}");
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::AnswerPrompt { .. })),
        "dropped, not answered (a refusal would be shared with other sessions): {effects:?}"
    );
    assert!(
        !app.dialogs.is_saving(SessionId(5)),
        "the save is forgotten"
    );
    assert_eq!(shown(&app), Some(PromptId(3)), "t's dialog stays");
    assert_eq!(app.dialogs.queued().count(), 0);

    // The connect asks again (another session of s): never shown, its session closed.
    let effects = show(&mut app, of(&s, 4, 7, password()));
    assert_eq!(closed(&effects), [SessionId(7)], "{effects:?}");
    assert_eq!(shown(&app), Some(PromptId(3)));
    assert_eq!(app.dialogs.queued().count(), 0);
    // The broker withdraws what the closed sessions held: nothing changes.
    app.update(Message::Prompt(BrokerEvent::Withdrawn(PromptId(2))));
    assert_eq!(shown(&app), Some(PromptId(3)));

    // The connect's result lands with no tab: its session is closed, and s is a site like
    // any other again.
    let effects = app.update(Message::SiteConnected {
        site: s.id.clone(),
        result: Err(ConnectError {
            session: Some(SessionId(7)),
            error: Error::new(ErrorKind::NotFound, "Session closed"),
            keyring_unavailable: false,
        }),
    });
    assert_eq!(closed(&effects), [SessionId(7)]);
    show(&mut app, of(&s, 5, 8, password()));
    assert_eq!(app.dialogs.queued().count(), 1, "shown in turn again");
    dialog_msg(&mut app, DialogMsg::Cancel);
    dialog_msg(&mut app, DialogMsg::Cancel);
    assert!(!app.dialogs.is_open());

    // The × of a tab still connecting, after a password typed with "save" for it: its
    // session is closed before the connect can store the password.
    connect_effect(app.update(Message::ConnectSite(s.id.clone())));
    let tab = app.tabs[0].id;
    show(&mut app, of(&s, 6, 9, password()));
    dialog_msg(&mut app, DialogMsg::Input(0, Typed("pw".into())));
    dialog_msg(&mut app, DialogMsg::Save(true));
    dialog_msg(&mut app, DialogMsg::Submit);
    assert!(app.dialogs.is_saving(SessionId(9)));
    let effects = app.update(Message::CloseTab(tab));
    assert_eq!(closed(&effects), [SessionId(9)], "{effects:?}");
    assert!(!app.dialogs.is_saving(SessionId(9)));
}
