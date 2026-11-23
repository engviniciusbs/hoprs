//! Plan 6 Task 4: the terminal tab. Shells open with the session, output arrives in
//! frame batches, keys and the clipboard go through the tab, the size follows with a
//! debounce, and an ended shell (lost connection, exit, overflow) says why and opens a new
//! one (Review Focus 1 and 5).

use std::time::Duration;

use alacritty_terminal::index::Side as GridSide;
use futures_util::stream;
use hoprs_core::{ConnectionState, OpenAs, TabKind};

use super::*;
use crate::app::terminal::{RESIZE_DEBOUNCE, SEARCH_DEBOUNCE, ShellEnd, ShellSignal, ShellState};
use crate::backend::fake::Call;
use crate::backend::{ShellId, ShellOutput};
use crate::effects::Batched;
use crate::terminal::input::{Key, Modifiers};
use crate::terminal::selection::SelectionKind;

fn ssh_site(name: &str, open_as: OpenAs) -> Site {
    let mut site = site(name, &format!("{name}.example"), "u", Protocol::Ssh);
    site.ssh.open_as = open_as;
    site
}

fn term(app: &App, index: usize) -> &crate::app::terminal::TerminalTab {
    app.tabs[index].terminal().expect("a terminal tab")
}

fn shell_of(app: &App, index: usize) -> ShellId {
    term(app, index).open_shell().expect("an open shell")
}

/// The text of each screen row, trailing blanks trimmed.
fn screen(app: &App, index: usize) -> Vec<String> {
    let model = &term(app, index).model;
    let mut rows = vec![String::new(); model.screen_lines()];
    for cell in model.cells() {
        let row = &mut rows[cell.line];
        while row.chars().count() < cell.column {
            row.push(' ');
        }
        row.push(cell.ch);
    }
    rows.into_iter().map(|r| r.trim_end().to_owned()).collect()
}

fn key(tab: TabId, key: Key, mods: Modifiers) -> Message {
    Message::Terminal {
        tab,
        msg: TerminalMsg::Key { key, mods },
    }
}

const NONE: Modifiers = Modifiers {
    shift: false,
    ctrl: false,
    alt: false,
};
const CTRL: Modifiers = Modifiers {
    shift: false,
    ctrl: true,
    alt: false,
};
const CTRL_SHIFT: Modifiers = Modifiers {
    shift: true,
    ctrl: true,
    alt: false,
};

/// The bytes of the `ShellInput` effects, in order.
fn inputs(effects: &[Effect]) -> Vec<Vec<u8>> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::ShellInput { data, .. } => Some(data.0.clone()),
            _ => None,
        })
        .collect()
}

/// Runs the effects that reach the fake's shells (open, input, resize, close) and feeds
/// their results back, until none is left; returns the others, unrun.
async fn run(app: &mut App, fake: &Arc<FakeBackend>, effects: Vec<Effect>) -> Vec<Effect> {
    let mut pending = effects;
    let mut rest = Vec::new();
    while !pending.is_empty() {
        let mut next = Vec::new();
        for effect in pending {
            let message = match effect {
                Effect::OpenShell {
                    tab,
                    request,
                    session,
                    cols,
                    rows,
                } => Message::ShellOpened {
                    tab,
                    request,
                    result: fake.open_shell(session, cols, rows).await,
                },
                Effect::ShellInput { tab, shell, data } => {
                    let _ = fake.shell_input(shell, data.0).await;
                    Message::Terminal {
                        tab,
                        msg: TerminalMsg::InputSent(shell),
                    }
                }
                Effect::ShellResize { shell, cols, rows } => {
                    fake.shell_resize(shell, cols, rows).await.unwrap();
                    continue;
                }
                Effect::CloseShell(shell) => {
                    fake.close_shell(shell);
                    continue;
                }
                other => {
                    rest.push(other);
                    continue;
                }
            };
            next.extend(app.update(message));
        }
        pending = next;
    }
    rest
}

/// A connected app with `site` open (as its "Open as" says), its shells open.
async fn connected(site: &Site) -> (App, Arc<FakeBackend>, SessionId) {
    let fake = Arc::new(FakeBackend::new().with_site(site.clone()));
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(site.id.clone())));
    let backend: Arc<dyn BackendApi> = fake.clone();
    let messages: Vec<Message> = effects::connect_sites(backend, sites).collect().await;
    let effects: Vec<Effect> = messages.into_iter().flat_map(|m| app.update(m)).collect();
    let terminals = app.tabs.iter().filter(|t| t.terminal().is_some()).count();
    let opens = effects
        .iter()
        .filter(|e| matches!(e, Effect::OpenShell { .. }))
        .count();
    assert_eq!(opens, terminals, "connected, each terminal opens its shell");
    run(&mut app, &fake, effects).await;
    let session = app.tab_conn(0).session().expect("connected");
    (app, fake, session)
}

/// Delivers the shell's output to the app through the real subscription stream until
/// `done` holds (or 5 s pass).
async fn pump(
    app: &mut App,
    fake: &Arc<FakeBackend>,
    feed: &mut (impl futures_util::Stream<Item = Message> + Unpin),
    done: impl Fn(&App) -> bool,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !done(app) {
        let message = tokio::time::timeout_at(deadline, feed.next())
            .await
            .expect("the shell's output arrives")
            .expect("the feed goes on");
        let next = app.update(message);
        effects.extend(run(app, fake, next).await);
    }
    effects
}

#[tokio::test]
async fn typing_sends_encoded_bytes() {
    let s = ssh_site("term", OpenAs::Terminal);
    let (mut app, fake, _) = connected(&s).await;
    let tab = app.tabs[0].id;
    let shell = shell_of(&app, 0);
    assert!(term(&app, 0).focused, "a new terminal has the keyboard");

    // A key goes out at once, encoded.
    let effects = app.update(key(tab, Key::Char('l'), NONE));
    assert_eq!(inputs(&effects), [b"l".to_vec()]);
    // While it is in flight the next keys wait, and then go together, in order.
    assert!(app.update(key(tab, Key::Char('s'), NONE)).is_empty());
    assert!(app.update(key(tab, Key::Enter, NONE)).is_empty());
    fake.shell_input(shell, b"l".to_vec()).await.unwrap();
    let effects = app.update(Message::Terminal {
        tab,
        msg: TerminalMsg::InputSent(shell),
    });
    assert_eq!(inputs(&effects), [b"s\r".to_vec()]);
    assert!(run(&mut app, &fake, effects).await.is_empty());
    // The fake echoes input: it reached the shell, in order.
    let mut feed = Box::pin(effects::shell_feed(fake.as_ref(), tab, shell));
    pump(&mut app, &fake, &mut feed, |app| screen(app, 0)[0] == "ls").await;

    // The modes the remote program set apply: application cursor keys (vim, less).
    let effects = app.update(key(tab, Key::Up, NONE));
    assert_eq!(inputs(&effects), [b"\x1b[A".to_vec()]);
    run(&mut app, &fake, effects).await;
    app.update(Message::TerminalOutput {
        tab,
        shell,
        bytes: b"\x1b[?1h".to_vec().into(),
        credit: Default::default(),
    });
    let effects = app.update(key(tab, Key::Up, NONE));
    assert_eq!(inputs(&effects), [b"\x1bOA".to_vec()]);
    run(&mut app, &fake, effects).await;
    // Ctrl and Alt combinations.
    let effects = app.update(key(tab, Key::Char('c'), CTRL));
    assert_eq!(inputs(&effects), [vec![0x03]]);
    run(&mut app, &fake, effects).await;
    let alt = Modifiers { alt: true, ..NONE };
    let effects = app.update(key(tab, Key::Char('b'), alt));
    assert_eq!(inputs(&effects), [b"\x1bb".to_vec()]);
    run(&mut app, &fake, effects).await;

    // A model reply (a DA query from the remote) is written back to the shell.
    let effects = app.update(Message::TerminalOutput {
        tab,
        shell,
        bytes: b"\x1b[c".to_vec().into(),
        credit: Default::default(),
    });
    assert_eq!(inputs(&effects), [b"\x1b[?6c".to_vec()]);
    run(&mut app, &fake, effects).await;

    // Without the focus (a click elsewhere), keys go nowhere.
    app.update(Message::Terminal {
        tab,
        msg: TerminalMsg::Focus(false),
    });
    assert!(app.update(key(tab, Key::Char('x'), NONE)).is_empty());
    // Showing the tab again gives it back.
    app.update(Message::SelectTab(0));
    assert!(term(&app, 0).focused);
    assert_eq!(
        inputs(&app.update(key(tab, Key::Char('y'), NONE))),
        [b"y".to_vec()]
    );
}

#[tokio::test]
async fn ctrl_k_goes_to_the_shell_when_focused() {
    let s = ssh_site("term", OpenAs::Terminal);
    let (mut app, _fake, _) = connected(&s).await;
    let tab = app.tabs[0].id;
    // Ctrl+K is bash's "kill line": the shell's, not the palette's.
    let effects = app.update(key(tab, Key::Char('k'), CTRL));
    assert_eq!(inputs(&effects), [vec![0x0b]]);
    let shell = shell_of(&app, 0);
    app.update(Message::Terminal {
        tab,
        msg: TerminalMsg::InputSent(shell),
    });
    // Ctrl+Shift+K opens the palette (Plan 6 Task 9): never sent to the shell.
    let effects = app.update(key(tab, Key::Char('K'), CTRL_SHIFT));
    assert!(matches!(effects[..], [Effect::FocusPalette]), "{effects:?}");
    assert!(app.palette.is_some());
    app.update(Message::Palette(crate::app::PaletteMsg::Close));
    let effects = app.update(key(tab, Key::Char('k'), CTRL_SHIFT));
    assert!(inputs(&effects).is_empty(), "{effects:?}");
    app.update(Message::Palette(crate::app::PaletteMsg::Close));
    // Without the focus, Ctrl+K goes nowhere.
    app.update(Message::Terminal {
        tab,
        msg: TerminalMsg::Focus(false),
    });
    assert!(app.update(key(tab, Key::Char('k'), CTRL)).is_empty());
}

#[tokio::test]
async fn copy_paste_shortcuts() {
    let s = ssh_site("term", OpenAs::Terminal);
    let (mut app, _fake, _) = connected(&s).await;
    let tab = app.tabs[0].id;
    let shell = shell_of(&app, 0);
    app.update(Message::TerminalOutput {
        tab,
        shell,
        bytes: b"hello world\r\n".to_vec().into(),
        credit: Default::default(),
    });

    // Nothing selected: Ctrl+Shift+C copies nothing, and never sends ^C.
    assert!(app.update(key(tab, Key::Char('C'), CTRL_SHIFT)).is_empty());

    // A drag selects; Ctrl+Shift+C copies it (whatever case the platform reports).
    let select = |msg| Message::Terminal { tab, msg };
    app.update(select(TerminalMsg::SelectStart {
        row: 0,
        col: 0,
        side: GridSide::Left,
        kind: SelectionKind::Cells,
    }));
    app.update(select(TerminalMsg::SelectMove {
        row: 0,
        col: 4,
        side: GridSide::Right,
    }));
    for letter in ['C', 'c'] {
        let effects = app.update(key(tab, Key::Char(letter), CTRL_SHIFT));
        assert!(
            matches!(&effects[..], [Effect::WriteClipboard(text)] if text.0 == "hello"),
            "{effects:?}"
        );
    }
    // A double click selects the word.
    app.update(select(TerminalMsg::SelectStart {
        row: 0,
        col: 8,
        side: GridSide::Left,
        kind: SelectionKind::Word,
    }));
    let effects = app.update(select(TerminalMsg::Copy));
    assert!(matches!(&effects[..], [Effect::WriteClipboard(text)] if text.0 == "world"));

    // Ctrl+Shift+V reads the clipboard; the text goes out as typed (line breaks as
    // Enter)...
    let effects = app.update(key(tab, Key::Char('V'), CTRL_SHIFT));
    assert!(matches!(&effects[..], [Effect::ReadClipboard(t)] if *t == tab));
    let effects = app.update(select(TerminalMsg::Pasted(Some("a\nb".to_owned()).into())));
    assert_eq!(inputs(&effects), [b"a\rb".to_vec()]);
    app.update(select(TerminalMsg::InputSent(shell)));
    // ...or bracketed, when the program asked for it, without what could end the paste.
    app.update(Message::TerminalOutput {
        tab,
        shell,
        bytes: b"\x1b[?2004h".to_vec().into(),
        credit: Default::default(),
    });
    let effects = app.update(select(TerminalMsg::Pasted(
        Some("rm -rf /\x1b[201~\x03".to_owned()).into(),
    )));
    assert_eq!(
        inputs(&effects),
        [b"\x1b[200~rm -rf /[201~\x1b[201~".to_vec()]
    );
    app.update(select(TerminalMsg::InputSent(shell)));
    // An empty clipboard sends nothing.
    assert!(
        app.update(select(TerminalMsg::Pasted(None.into())))
            .is_empty()
    );

    // Ctrl+Shift+F opens the search bar and focuses it; the shell gets nothing.
    let effects = app.update(key(tab, Key::Char('F'), CTRL_SHIFT));
    assert!(matches!(&effects[..], [Effect::FocusTerminalSearch(t)] if *t == tab));
    assert!(term(&app, 0).search.is_some());
    assert!(!term(&app, 0).focused, "the search field has the keyboard");
    // Escape closes it and gives the keyboard back to the terminal.
    app.update(Message::KeyPressed {
        key: FileKey::Escape,
        shift: false,
    });
    assert!(term(&app, 0).search.is_none());
    assert!(term(&app, 0).focused);
}

/// The size follows the widget at once in the model, and reaches the shell 100 ms after
/// the last change only (tokio's paused clock: no real waiting).
#[tokio::test(start_paused = true)]
async fn resize_is_debounced_100ms() {
    let s = ssh_site("term", OpenAs::Terminal);
    let (mut app, fake, _) = connected(&s).await;
    let tab = app.tabs[0].id;
    let shell = shell_of(&app, 0);
    assert_eq!(
        fake.shell_sizes(shell),
        [(80, 24)],
        "opened at the model's size"
    );

    // A drag of the window edge: three sizes, 30 ms apart.
    let t0 = tokio::time::Instant::now();
    let mut timers = Vec::new();
    for (cols, rows) in [(90, 30), (100, 30), (120, 40)] {
        let effects = app.update(Message::TerminalResized { tab, cols, rows });
        let [Effect::Later { after, message }] = <[Effect; 1]>::try_from(effects).unwrap() else {
            panic!("one timer per change");
        };
        assert_eq!(after, RESIZE_DEBOUNCE);
        assert_eq!(after, Duration::from_millis(100));
        assert_eq!(
            (
                term(&app, 0).model.columns(),
                term(&app, 0).model.screen_lines()
            ),
            (usize::from(cols), usize::from(rows)),
            "the model follows at once"
        );
        timers.push(tokio::spawn(async move {
            let message = effects::later(after, *message).await;
            (message, tokio::time::Instant::now())
        }));
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    // The same size again starts nothing.
    assert!(
        app.update(Message::TerminalResized {
            tab,
            cols: 120,
            rows: 40
        })
        .is_empty()
    );
    assert_eq!(
        fake.shell_sizes(shell),
        [(80, 24)],
        "nothing reached the shell yet"
    );

    // Each timer fires 100 ms after its change; only the last one resizes the shell.
    let mut resizes = Vec::new();
    for (n, timer) in timers.into_iter().enumerate() {
        let (message, fired) = timer.await.unwrap();
        assert_eq!(fired - t0, Duration::from_millis(100 + 30 * n as u64));
        let effects = app.update(message);
        if n < 2 {
            assert!(
                effects.is_empty(),
                "a stale timer does nothing: {effects:?}"
            );
        }
        resizes.extend(effects);
    }
    assert!(
        matches!(
            &resizes[..],
            [Effect::ShellResize {
                cols: 120,
                rows: 40,
                ..
            }]
        ),
        "{resizes:?}"
    );
    run(&mut app, &fake, resizes).await;
    assert_eq!(fake.shell_sizes(shell), [(80, 24), (120, 40)]);
}

/// Credits for one shell, as `shell_feed` makes them.
fn credits() -> Arc<tokio::sync::Semaphore> {
    Arc::new(tokio::sync::Semaphore::new(effects::MAX_IN_FLIGHT))
}

/// A batch as the shell output it stands for (its credit dropped: handled).
fn plain(item: Batched) -> ShellOutput {
    match item {
        Batched::Data(bytes, _) => ShellOutput::Data(bytes.0),
        Batched::Signal(ShellSignal::Exit(code)) => ShellOutput::Exit(code),
        Batched::Signal(ShellSignal::Overflowed) => ShellOutput::Overflowed,
        Batched::Signal(ShellSignal::Closed) => ShellOutput::Closed,
    }
}

/// Review Focus 1: a consumer that falls behind holds at most `MAX_IN_FLIGHT` batches
/// (bounded memory): the feed stops pulling, the shell's own pump fills up, and the
/// overflow is reported inline instead of output piling up in the app.
#[tokio::test]
async fn slow_consumer_is_bounded_and_overflow_is_reported() {
    let (handle, output) = hoprs_ssh::testing::flooding_shell().into_output();
    let mut batches = Box::pin(effects::batch_output(
        Box::pin(output),
        effects::FRAME,
        credits(),
    ));
    // The consumer takes the batches in flight and then stalls (it holds their credits).
    let mut held = Vec::new();
    for _ in 0..effects::MAX_IN_FLIGHT {
        match batches.next().await {
            Some(Batched::Data(bytes, credit)) => {
                assert!(bytes.len() <= effects::MAX_FRAME_BYTES + 1024 * 1024);
                held.push(credit);
            }
            other => panic!("expected data, got {other:?}"),
        }
    }
    // Nothing more comes while they are held: the feed does not pull.
    let more = tokio::time::timeout(Duration::from_millis(300), batches.next()).await;
    assert!(more.is_err(), "a third batch while two wait: {more:?}");
    // Meanwhile the shell's pump gives up on the stalled consumer.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !handle.overflowed() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the pump never overflowed"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Handled at last: what the pump kept drains (bounded), then the overflow, then the end.
    drop(held);
    let mut drained = 0usize;
    let mut tail = Vec::new();
    while let Some(item) = tokio::time::timeout(Duration::from_secs(10), batches.next())
        .await
        .expect("the feed drains")
    {
        match plain(item) {
            ShellOutput::Data(bytes) => {
                assert!(tail.is_empty(), "data after the overflow");
                drained += bytes.len();
            }
            other => tail.push(other),
        }
    }
    assert_eq!(tail, [ShellOutput::Overflowed, ShellOutput::Closed]);
    // The pump's queue (256 x 64 KiB) plus its pending limit (16 MiB), and a batch.
    assert!(drained <= 34 * 1024 * 1024, "drained {drained} bytes");
}

/// Review Focus 1: a flood of output reaches the app as few big messages, at most one
/// per 16 ms frame, every byte kept and in order, the end events after the data.
#[tokio::test(start_paused = true)]
async fn output_batches_one_message_per_frame() {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<ShellOutput>();
    let output = Box::pin(stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    }));
    // 10 000 chunks over one simulated frame: 10 bursts of 1 000, 1.5 ms apart.
    let producer = tokio::spawn(async move {
        for burst in 0..10u32 {
            for i in 0..1_000u32 {
                let n = burst * 1_000 + i;
                tx.send(ShellOutput::Data(n.to_le_bytes().to_vec()))
                    .unwrap();
            }
            tokio::time::sleep(Duration::from_micros(1_500)).await;
        }
        tx.send(ShellOutput::Exit(Some(0))).unwrap();
        tx.send(ShellOutput::Closed).unwrap();
    });
    let start = tokio::time::Instant::now();
    let mut batches = Box::pin(effects::batch_output(output, effects::FRAME, credits()));
    let mut received: Vec<(Duration, ShellOutput)> = Vec::new();
    // A consumer that keeps up: each batch's credit is dropped as it is handled.
    while let Some(item) = batches.next().await {
        received.push((tokio::time::Instant::now() - start, plain(item)));
    }
    producer.await.unwrap();

    let data: Vec<&(Duration, ShellOutput)> = received
        .iter()
        .filter(|(_, item)| matches!(item, ShellOutput::Data(_)))
        .collect();
    let in_first_frame = data
        .iter()
        .filter(|(at, _)| *at < Duration::from_millis(16))
        .count();
    assert!(
        in_first_frame <= 2,
        "{in_first_frame} messages within 16 ms: {:?}",
        data.iter().map(|(at, _)| *at).collect::<Vec<_>>()
    );
    assert!(data.len() <= 3, "{} messages for 10 000 chunks", data.len());
    // Consecutive messages are a frame apart at least.
    for pair in data.windows(2) {
        assert!(
            pair[1].0 - pair[0].0 >= Duration::from_millis(16),
            "{:?}",
            data.iter().map(|(at, _)| *at).collect::<Vec<_>>()
        );
    }
    // Every byte, in order.
    let bytes: Vec<u8> = data
        .iter()
        .flat_map(|(_, item)| match item {
            ShellOutput::Data(d) => d.clone(),
            _ => unreachable!(),
        })
        .collect();
    let expected: Vec<u8> = (0..10_000u32).flat_map(u32::to_le_bytes).collect();
    assert_eq!(bytes, expected);
    // The end comes after the data, in its order.
    let tail: Vec<&ShellOutput> = received
        .iter()
        .map(|(_, item)| item)
        .rev()
        .take(2)
        .collect();
    assert_eq!(tail, [&ShellOutput::Closed, &ShellOutput::Exit(Some(0))]);

    // A keystroke's echo after a quiet moment goes out at once (no frame of delay).
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<ShellOutput>();
    let output = Box::pin(stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    }));
    let mut batches = Box::pin(effects::batch_output(output, effects::FRAME, credits()));
    tx.send(ShellOutput::Data(b"a".to_vec())).unwrap();
    let start = tokio::time::Instant::now();
    assert_eq!(
        batches.next().await.map(plain),
        Some(ShellOutput::Data(b"a".to_vec()))
    );
    assert_eq!(tokio::time::Instant::now(), start);
}

/// The feed the subscription runs: data as `TerminalOutput`, the end as `ShellEvent`s,
/// exactly one `Closed` last, even for an output already taken.
#[tokio::test]
async fn shell_feed_maps_output_to_messages() {
    let s = ssh_site("term", OpenAs::Terminal);
    let (_app, fake, _) = connected(&s).await;
    let shell = ShellId(1);
    let tab = TabId(77);
    let mut feed = Box::pin(effects::shell_feed(fake.as_ref(), tab, shell));
    fake.push_shell_output(shell, b"hi");
    fake.push_shell_event(shell, ShellOutput::Exit(Some(2)));
    fake.end_shell(shell);
    let messages: Vec<Message> = feed.by_ref().collect().await;
    assert!(matches!(
        &messages[..],
        [
            Message::TerminalOutput { bytes, .. },
            Message::ShellEvent { event: ShellSignal::Exit(Some(2)), .. },
            Message::ShellEvent { event: ShellSignal::Closed, .. },
        ] if bytes.0 == b"hi"
    ));
    // Taken already: just `Closed`.
    let messages: Vec<Message> = effects::shell_feed(fake.as_ref(), tab, shell)
        .collect()
        .await;
    assert!(matches!(
        &messages[..],
        [Message::ShellEvent {
            event: ShellSignal::Closed,
            ..
        }]
    ));
}

/// Review Focus 1: an overflowing shell is never closed silently: the tab says so inline
/// and "Reopen" opens a new shell.
#[tokio::test]
async fn overflow_shows_inline_and_reopen() {
    let s = ssh_site("term", OpenAs::Terminal);
    let (mut app, fake, _) = connected(&s).await;
    let tab = app.tabs[0].id;
    let shell = shell_of(&app, 0);
    assert_eq!(term(&app, 0).end_banner(), None);

    // The pump's order: the data it kept, `Overflowed`, `Closed`.
    let mut feed = Box::pin(effects::shell_feed(fake.as_ref(), tab, shell));
    fake.push_shell_output(shell, b"yyyyyyyy");
    fake.push_shell_event(shell, ShellOutput::Overflowed);
    fake.end_shell(shell);
    let effects = pump(&mut app, &fake, &mut feed, |app| {
        matches!(term(app, 0).shell, ShellState::Ended(_))
    })
    .await;
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(term(&app, 0).shell, ShellState::Ended(ShellEnd::Overflowed));
    assert!(
        fake.calls().contains(&Call::CloseShell(shell)),
        "forgotten in the backend"
    );
    let banner = term(&app, 0).end_banner().expect("shown inline");
    assert_eq!(
        banner.message,
        "Output was too fast and the shell was closed"
    );
    assert_eq!(banner.action, "Reopen");
    assert!(
        screen(&app, 0)[0].starts_with("yyyyyyyy"),
        "the output kept stays"
    );
    assert!(
        app.terminal_feeds().is_empty(),
        "no feed for an ended shell"
    );

    // Reopen: a new shell on the same session, the scrollback kept.
    let effects = app.update(Message::Terminal {
        tab,
        msg: TerminalMsg::Restart,
    });
    assert!(
        matches!(&effects[..], [Effect::OpenShell { .. }]),
        "{effects:?}"
    );
    run(&mut app, &fake, effects).await;
    let new = shell_of(&app, 0);
    assert_ne!(new, shell);
    assert_eq!(app.terminal_feeds(), [(tab, new)]);
    assert_eq!(term(&app, 0).end_banner(), None);
    assert!(screen(&app, 0)[0].starts_with("yyyyyyyy"));
}

#[tokio::test]
async fn exit_shows_restart() {
    let s = ssh_site("term", OpenAs::Terminal);
    let (mut app, fake, _) = connected(&s).await;
    let tab = app.tabs[0].id;
    let shell = shell_of(&app, 0);

    // Late or foreign output (another shell) is ignored.
    assert!(
        app.update(Message::ShellEvent {
            tab,
            shell: ShellId(999),
            event: ShellSignal::Closed,
        })
        .is_empty()
    );
    assert_eq!(term(&app, 0).shell, ShellState::Open(shell));

    let mut feed = Box::pin(effects::shell_feed(fake.as_ref(), tab, shell));
    fake.push_shell_event(shell, ShellOutput::Exit(Some(3)));
    fake.end_shell(shell);
    pump(&mut app, &fake, &mut feed, |app| {
        matches!(term(app, 0).shell, ShellState::Ended(_))
    })
    .await;
    assert_eq!(
        term(&app, 0).shell,
        ShellState::Ended(ShellEnd::Exited(Some(3)))
    );
    let banner = term(&app, 0).end_banner().unwrap();
    assert_eq!(banner.message, "Shell exited (code 3)");
    assert_eq!(banner.action, "Restart shell");
    // Keys go nowhere meanwhile.
    assert!(app.update(key(tab, Key::Char('x'), NONE)).is_empty());
    // The connection is fine: no "Connection lost" banner, the session stays.
    assert!(app.conn(&s.id).lost_connection().is_none());
    assert!(!fake.calls().iter().any(|c| matches!(c, Call::Close(_))));

    // Restart shell: a new one.
    let effects = app.update(Message::Terminal {
        tab,
        msg: TerminalMsg::Restart,
    });
    run(&mut app, &fake, effects).await;
    assert_ne!(shell_of(&app, 0), shell);
    assert_eq!(term(&app, 0).end_banner(), None);
    // Restart does nothing on a running shell.
    assert!(
        app.update(Message::Terminal {
            tab,
            msg: TerminalMsg::Restart,
        })
        .is_empty()
    );

    // A shell that ends without an exit status while connected: "Shell closed".
    let shell = shell_of(&app, 0);
    app.update(Message::ShellEvent {
        tab,
        shell,
        event: ShellSignal::Closed,
    });
    assert_eq!(term(&app, 0).end_banner().unwrap().message, "Shell closed");

    // A shell that can't open (the channel budget): why, inline, and Retry.
    fake.fail_next_open_shell(Error::new(
        hoprs_core::ErrorKind::Protocol,
        "Too many channels on this connection (limit 10)",
    ));
    let effects = app.update(Message::Terminal {
        tab,
        msg: TerminalMsg::Restart,
    });
    run(&mut app, &fake, effects).await;
    let banner = term(&app, 0).end_banner().unwrap();
    assert_eq!(
        banner.message,
        "Can't open a shell: Too many channels on this connection (limit 10)"
    );
    assert_eq!(banner.action, "Retry");
}

/// Review Focus 5: the connection drops under a terminal and a files tab of one session:
/// the banner says so with "Reconnect"; once reconnected the terminal has a **new** shell,
/// with "— reconnected —" in its scrollback, and the files tab lists again.
#[tokio::test]
async fn connection_lost_banner_then_new_shell() {
    let s = ssh_site("both", OpenAs::Both);
    let (mut app, fake, session) = connected(&s).await;
    assert_eq!(
        app.tabs.iter().map(|t| t.kind()).collect::<Vec<_>>(),
        [TabKind::Terminal, TabKind::Files]
    );
    let tab = app.tabs[0].id;
    let old = shell_of(&app, 0);
    let mut feed = Box::pin(effects::shell_feed(fake.as_ref(), tab, old));
    fake.push_shell_output(old, b"$ before\r\n");

    // The connection is lost: the shell ends with it, the feed says so.
    let lost = Error::new(hoprs_core::ErrorKind::Network, "Connection reset");
    fake.drop_connection(session, lost.clone());
    app.update(Message::SessionState {
        session,
        state: ConnectionState::Disconnected(lost.clone()),
        auth: None,
    });
    pump(&mut app, &fake, &mut feed, |app| {
        matches!(term(app, 0).shell, ShellState::Ended(_))
    })
    .await;
    assert_eq!(term(&app, 0).shell, ShellState::Ended(ShellEnd::Lost));
    // The shared "Connection lost" banner with Reconnect, not a shell banner.
    assert_eq!(app.conn(&s.id).lost_connection(), Some(&lost));
    assert_eq!(
        crate::ui::files::banner(app.conn(&s.id)),
        Some(crate::ui::files::Banner::Lost(&lost))
    );
    assert_eq!(term(&app, 0).end_banner(), None);
    assert!(app.terminal_feeds().is_empty());

    // Reconnect: the backend's reconnect, then a new shell.
    let effects = app.update(Message::ReconnectSession(tab));
    let [Effect::Reconnect { session: again }] = <[Effect; 1]>::try_from(effects).unwrap() else {
        panic!("one reconnect");
    };
    assert_eq!(again, session);
    let backend: Arc<dyn BackendApi> = fake.clone();
    let effects = app.update(effects::reconnect(backend, session).await);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::OpenShell { .. })),
        "{effects:?}"
    );
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::List {
                side: Side::Remote,
                ..
            }
        )),
        "the files tab lists again"
    );
    run(&mut app, &fake, effects).await;
    let new = shell_of(&app, 0);
    assert_ne!(new, old, "a new shell, not the old one");
    assert!(fake.calls().contains(&Call::OpenShell(session)));
    assert_eq!(app.terminal_feeds(), [(tab, new)]);
    let rows = screen(&app, 0);
    assert_eq!(rows[0], "$ before");
    assert!(
        rows.iter().any(|r| r == "— reconnected —"),
        "the scrollback marks the reconnect: {rows:?}"
    );
    // The feed saying `Connected` afterwards opens no second shell.
    let effects = app.update(Message::SessionState {
        session,
        state: ConnectionState::Connected,
        auth: None,
    });
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::OpenShell { .. })),
        "{effects:?}"
    );

    // The new shell works.
    let mut feed = Box::pin(effects::shell_feed(fake.as_ref(), tab, new));
    let effects = app.update(key(tab, Key::Char('z'), NONE));
    run(&mut app, &fake, effects).await;
    pump(&mut app, &fake, &mut feed, |app| {
        screen(app, 0).iter().any(|r| r.starts_with('z'))
    })
    .await;

    // A connection that comes back by itself (the session's own reconnect) also gives
    // the terminal a new shell, once: the old shell may still look open then.
    let current = shell_of(&app, 0);
    app.update(Message::SessionState {
        session,
        state: ConnectionState::Reconnecting { attempt: 1 },
        auth: None,
    });
    let effects = app.update(Message::SessionState {
        session,
        state: ConnectionState::Connected,
        auth: None,
    });
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::CloseShell(s) if *s == current)),
        "{effects:?}"
    );
    run(&mut app, &fake, effects).await;
    assert_ne!(shell_of(&app, 0), current);
    // The old shell's late `Closed` changes nothing.
    assert!(
        app.update(Message::ShellEvent {
            tab,
            shell: current,
            event: ShellSignal::Closed,
        })
        .is_empty()
    );
    assert!(term(&app, 0).open_shell().is_some());

    // Closing the terminal tab closes its shell; the files tab keeps the session.
    let shell = shell_of(&app, 0);
    let effects = app.update(Message::CloseTab(tab));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::CloseShell(s) if *s == shell)),
        "{effects:?}"
    );
    assert!(!effects.iter().any(|e| matches!(e, Effect::CloseSession(_))));
    assert_eq!(app.tab_conn(0).session(), Some(session));
}

/// A shell result nobody waits for (the tab closed meanwhile) is closed at once.
#[tokio::test]
async fn late_shell_is_closed() {
    let s = ssh_site("term", OpenAs::Terminal);
    let fake = Arc::new(FakeBackend::new().with_site(s.clone()));
    let mut app = loaded(&fake).await;
    let sites = connect_effect(app.update(Message::ConnectSite(s.id.clone())));
    let backend: Arc<dyn BackendApi> = fake.clone();
    let messages: Vec<Message> = effects::connect_sites(backend, sites).collect().await;
    let effects: Vec<Effect> = messages.into_iter().flat_map(|m| app.update(m)).collect();
    let open = effects
        .into_iter()
        .find(|e| matches!(e, Effect::OpenShell { .. }))
        .unwrap();
    let Effect::OpenShell { tab, request, .. } = open else {
        unreachable!()
    };
    app.update(Message::CloseTab(tab));
    let effects = app.update(Message::ShellOpened {
        tab,
        request,
        result: Ok(ShellId(42)),
    });
    assert!(matches!(&effects[..], [Effect::CloseShell(ShellId(42))]));
}

/// Output is fed to the model: the remote title is a hint (empty = reset), the scrollback
/// scrolls, and search-as-you-type waits for a pause.
#[tokio::test]
async fn output_title_scroll_and_search() {
    let s = ssh_site("term", OpenAs::Terminal);
    let (mut app, _fake, _) = connected(&s).await;
    let tab = app.tabs[0].id;
    let shell = shell_of(&app, 0);
    let out = |bytes: &[u8]| Message::TerminalOutput {
        tab,
        shell,
        bytes: bytes.to_vec().into(),
        credit: Default::default(),
    };
    app.update(out(b"\x1b]0;vim notes.txt\x07"));
    assert_eq!(term(&app, 0).title.as_deref(), Some("vim notes.txt"));
    app.update(out(b"\x1b]0;\x07"));
    assert_eq!(
        term(&app, 0).title,
        None,
        "an empty title resets to the site's name"
    );
    // Output of another shell (an old one) is dropped.
    app.update(Message::TerminalOutput {
        tab,
        shell: ShellId(999),
        bytes: b"stale".to_vec().into(),
        credit: Default::default(),
    });
    assert!(!screen(&app, 0).iter().any(|r| r.contains("stale")));

    for i in 0..60 {
        app.update(out(format!("line {i}\r\n").as_bytes()));
    }
    // The wheel scrolls back; typing goes back to the prompt.
    app.update(Message::Terminal {
        tab,
        msg: TerminalMsg::Scroll(5),
    });
    assert_eq!(term(&app, 0).model.display_offset(), 5);
    app.update(key(tab, Key::Char('q'), NONE));
    assert_eq!(term(&app, 0).model.display_offset(), 0);
    // Shift+PageUp pages back.
    let shift = Modifiers {
        shift: true,
        ..NONE
    };
    assert!(app.update(key(tab, Key::PageUp, shift)).is_empty());
    assert_eq!(term(&app, 0).model.display_offset(), 24);

    // Search: typing restarts a 150 ms timer; only the last one searches.
    let search = |msg| Message::Terminal { tab, msg };
    app.update(search(TerminalMsg::OpenSearch));
    let mut seqs = Vec::new();
    for q in ["l", "li", "line 1"] {
        let effects = app.update(search(TerminalMsg::SearchInput(q.into())));
        let [Effect::Later { after, message }] = <[Effect; 1]>::try_from(effects).unwrap() else {
            panic!("one timer");
        };
        assert_eq!(after, SEARCH_DEBOUNCE);
        seqs.push(*message);
    }
    // A stale timer does nothing.
    let last = seqs.pop().unwrap();
    app.update(seqs.remove(0));
    assert!(term(&app, 0).search.as_ref().unwrap().current.is_none());
    app.update(last);
    let bar = term(&app, 0).search.as_ref().unwrap();
    // The newest match first ("line 19" is the last "line 1…" written), scrolled into view.
    let first = bar.current.clone().expect("found");
    assert!(!bar.no_match);
    app.update(search(TerminalMsg::SearchPrevious));
    let second = term(&app, 0)
        .search
        .as_ref()
        .unwrap()
        .current
        .clone()
        .unwrap();
    assert!(second.start() < first.start(), "previous goes up");
    app.update(search(TerminalMsg::SearchNext));
    assert_eq!(term(&app, 0).search.as_ref().unwrap().current, Some(first));
    app.update(search(TerminalMsg::SearchInput("absent".into())));
    let effects = app.update(search(TerminalMsg::SearchInput("absent!".into())));
    let [Effect::Later { message, .. }] = <[Effect; 1]>::try_from(effects).unwrap() else {
        panic!()
    };
    app.update(*message);
    assert!(term(&app, 0).search.as_ref().unwrap().no_match);
    app.update(search(TerminalMsg::CloseSearch));
    assert!(term(&app, 0).search.is_none());
    assert!(term(&app, 0).focused);
}

/// The scrollback setting applies to the terminals opened from then on.
#[tokio::test]
async fn new_scrollback_setting_applies_to_new_terminals() {
    let s = ssh_site("term", OpenAs::Files);
    let (mut app, fake, _) = connected(&s).await;
    app.settings.terminal_scrollback = 1_000;
    let first = app.tabs[0].id;
    let effects = app.update(Message::NewTab {
        tab: first,
        kind: TabKind::Terminal,
    });
    run(&mut app, &fake, effects).await;
    let tab = app.tabs[1].id;
    let shell = shell_of(&app, 1);
    for i in 0..3_000 {
        app.update(Message::TerminalOutput {
            tab,
            shell,
            bytes: format!("line {i}\r\n").into_bytes().into(),
            credit: Default::default(),
        });
    }
    assert_eq!(term(&app, 1).model.history_len(), 1_000);
}

/// End to end against the Docker SSH server (127.0.0.1:2222, alice / alice-pass): the
/// real backend, the site opened as Both, keys typed through the tab, the output through
/// the subscription's stream into the model; the size reaches the PTY.
#[tokio::test]
#[ignore = "needs docker: docker compose -f docker/compose.yml up -d --build --wait"]
async fn terminal_tab_end_to_end() {
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
        agent_dir: None,
        program: None,
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
                    let answer = if matches!(r.kind, PromptKind::HostKey { .. }) {
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
    site.ssh.open_as = OpenAs::Both;
    backend.save_site(site.clone()).await.unwrap();
    backend
        .set_password(site.id.clone(), Some("alice-pass".to_owned().into()))
        .await
        .unwrap();

    let (mut app, _) = App::boot();
    app.update(Message::ProfilesLoaded {
        request: app.profiles_requested(),
        result: backend.profiles().await,
    });
    let sites = connect_effect(app.update(Message::ConnectSite(site.id.clone())));
    let messages: Vec<Message> = effects::connect_sites(backend.clone(), sites)
        .collect()
        .await;
    let mut pending: Vec<Effect> = messages.into_iter().flat_map(|m| app.update(m)).collect();
    assert_eq!(
        app.tabs.iter().map(|t| t.kind()).collect::<Vec<_>>(),
        [TabKind::Terminal, TabKind::Files]
    );
    let tab = app.tabs[0].id;

    // Runs the shell effects against the real backend.
    async fn drive(app: &mut App, backend: &Arc<dyn BackendApi>, mut pending: Vec<Effect>) {
        while !pending.is_empty() {
            let mut next = Vec::new();
            for effect in pending {
                let message = match effect {
                    Effect::OpenShell {
                        tab,
                        request,
                        session,
                        cols,
                        rows,
                    } => Message::ShellOpened {
                        tab,
                        request,
                        result: backend.open_shell(session, cols, rows).await,
                    },
                    Effect::ShellInput { tab, shell, data } => {
                        backend.shell_input(shell, data.0).await.unwrap();
                        Message::Terminal {
                            tab,
                            msg: TerminalMsg::InputSent(shell),
                        }
                    }
                    Effect::ShellResize { shell, cols, rows } => {
                        backend.shell_resize(shell, cols, rows).await.unwrap();
                        continue;
                    }
                    Effect::Later { message, .. } => *message,
                    _ => continue,
                };
                next.extend(app.update(message));
            }
            pending = next;
        }
    }
    drive(&mut app, &backend, std::mem::take(&mut pending)).await;
    let shell = shell_of(&app, 0);
    let mut feed = Box::pin(effects::shell_feed(backend.as_ref(), tab, shell));

    // Types `echo hoprs` and Enter through the tab.
    let mut typed = Vec::new();
    for c in "echo hoprs".chars() {
        typed.extend(app.update(key(tab, Key::Char(c), NONE)));
        drive(&mut app, &backend, std::mem::take(&mut typed)).await;
    }
    let effects = app.update(key(tab, Key::Enter, NONE));
    drive(&mut app, &backend, effects).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !screen(&app, 0).iter().any(|r| r == "hoprs") {
        let message = tokio::time::timeout_at(deadline, feed.next())
            .await
            .unwrap_or_else(|_| panic!("no echo: {:?}", screen(&app, 0)))
            .expect("the shell runs");
        let effects = app.update(message);
        drive(&mut app, &backend, effects).await;
    }

    // The size reaches the PTY: `stty size` says so.
    let effects = app.update(Message::TerminalResized {
        tab,
        cols: 100,
        rows: 30,
    });
    drive(&mut app, &backend, effects).await;
    for c in "stty size".chars() {
        let effects = app.update(key(tab, Key::Char(c), NONE));
        drive(&mut app, &backend, effects).await;
    }
    let effects = app.update(key(tab, Key::Enter, NONE));
    drive(&mut app, &backend, effects).await;
    while !screen(&app, 0).iter().any(|r| r == "30 100") {
        let message = tokio::time::timeout_at(deadline, feed.next())
            .await
            .unwrap_or_else(|_| panic!("no size: {:?}", screen(&app, 0)))
            .expect("the shell runs");
        let effects = app.update(message);
        drive(&mut app, &backend, effects).await;
    }

    // Closing the tab closes the shell; the files tab keeps the session.
    let effects = app.update(Message::CloseTab(tab));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::CloseShell(s) if *s == shell))
    );
    backend.close_shell(shell);
    let rest: Vec<Message> = tokio::time::timeout(Duration::from_secs(5), feed.collect())
        .await
        .expect("the feed ends with the shell");
    assert!(matches!(
        rest.last(),
        Some(Message::ShellEvent {
            event: ShellSignal::Closed,
            ..
        })
    ));
    assert!(app.tab_conn(0).session().is_some());
    backend.shutdown().await;
    answers.abort();
}

/// "Close and reconnect" (or any new session for the site): a shell of the old session
/// that still looks open is replaced at once, without waiting for its `Closed`.
#[tokio::test]
async fn a_new_session_replaces_the_old_shell() {
    let s = ssh_site("term", OpenAs::Terminal);
    let (mut app, fake, first) = connected(&s).await;
    let old = shell_of(&app, 0);
    // As `Effect::RestartSite` does: the old session closes, a new one connects, and its
    // result arrives before the old shell's `Closed`.
    fake.close(first).await;
    let backend: Arc<dyn BackendApi> = fake.clone();
    let messages: Vec<Message> = effects::connect_sites(backend, vec![s.id.clone()])
        .collect()
        .await;
    let effects: Vec<Effect> = messages.into_iter().flat_map(|m| app.update(m)).collect();
    let session = app.tab_conn(0).session().unwrap();
    assert_ne!(session, first);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::CloseShell(s) if *s == old)),
        "{effects:?}"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::OpenShell { session: on, .. } if *on == session))
    );
    run(&mut app, &fake, effects).await;
    assert_ne!(shell_of(&app, 0), old);
}

/// Escape in a focused terminal is the shell's alone: an open search bar stays open.
/// Only while the search field has the keyboard does Escape close the bar.
#[tokio::test]
async fn escape_in_a_focused_terminal_goes_only_to_the_shell() {
    let s = ssh_site("term", OpenAs::Terminal);
    let (mut app, _fake, _) = connected(&s).await;
    let tab = app.tabs[0].id;
    app.update(Message::Terminal {
        tab,
        msg: TerminalMsg::OpenSearch,
    });
    // A click back into the terminal: it has the keyboard, the bar stays open.
    app.update(Message::Terminal {
        tab,
        msg: TerminalMsg::Focus(true),
    });
    assert!(term(&app, 0).focused);
    // The widget sends Escape to the shell; the global subscription also reports it.
    let effects = app.update(key(tab, Key::Escape, NONE));
    assert_eq!(inputs(&effects), [b"\x1b".to_vec()]);
    let effects = app.update(Message::KeyPressed {
        key: FileKey::Escape,
        shift: false,
    });
    assert!(effects.is_empty(), "{effects:?}");
    assert!(
        term(&app, 0).search.is_some(),
        "Escape went to the shell only"
    );

    // With the search field focused (the terminal not), Escape closes the bar.
    app.update(Message::Terminal {
        tab,
        msg: TerminalMsg::Focus(false),
    });
    app.update(Message::KeyPressed {
        key: FileKey::Escape,
        shift: false,
    });
    assert!(term(&app, 0).search.is_none());
    assert!(term(&app, 0).focused);
}

/// Review Focus 5: a shell that died inside a full-screen program (alternate screen,
/// application keys, bracketed paste, mouse, hidden cursor, a sequence cut in half) does
/// not hand its modes to the new shell after a reconnect; the history stays.
#[tokio::test]
async fn reconnect_resets_the_dead_shells_modes() {
    use alacritty_terminal::term::TermMode;

    let s = ssh_site("term", OpenAs::Terminal);
    let (mut app, fake, session) = connected(&s).await;
    let tab = app.tabs[0].id;
    let old = shell_of(&app, 0);
    let out = |bytes: &[u8]| Message::TerminalOutput {
        tab,
        shell: old,
        bytes: bytes.to_vec().into(),
        credit: Default::default(),
    };
    for i in 0..40 {
        app.update(out(format!("history {i}\r\n").as_bytes()));
    }
    app.update(out(b"$ vim notes\r\n"));
    app.update(out(
        b"\x1b[?1049h\x1b[?1h\x1b[?2004h\x1b[?1000h\x1b[?1006h\x1b[?25l\x1b[31mvim\x1b]0;half",
    ));
    let mode = term(&app, 0).model.mode();
    assert!(mode.contains(TermMode::ALT_SCREEN | TermMode::APP_CURSOR | TermMode::BRACKETED_PASTE));
    assert!(!term(&app, 0).model.cursor().visible);
    let history = term(&app, 0).model.history_len();

    // The connection drops and comes back.
    let lost = Error::new(hoprs_core::ErrorKind::Network, "Connection reset");
    fake.drop_connection(session, lost.clone());
    app.update(Message::SessionState {
        session,
        state: ConnectionState::Disconnected(lost),
        auth: None,
    });
    app.update(Message::ShellEvent {
        tab,
        shell: old,
        event: ShellSignal::Closed,
    });
    let effects = app.update(Message::ReconnectSession(tab));
    assert!(matches!(&effects[..], [Effect::Reconnect { .. }]));
    let backend: Arc<dyn BackendApi> = fake.clone();
    let effects = app.update(effects::reconnect(backend, session).await);
    run(&mut app, &fake, effects).await;
    assert_ne!(shell_of(&app, 0), old);

    let model = &term(&app, 0).model;
    assert_eq!(
        model.mode(),
        TermMode::default(),
        "no mode of the dead program left"
    );
    assert!(model.cursor().visible);
    assert!(model.history_len() >= history, "the history stays");
    let rows = screen(&app, 0);
    assert!(
        rows.iter().any(|r| r == "$ vim notes"),
        "main screen back: {rows:?}"
    );
    assert!(rows.iter().any(|r| r == "— reconnected —"), "{rows:?}");
    assert!(!rows.iter().any(|r| r.contains("vim") && r != "$ vim notes"));
    // Keys are plain again: an arrow is CSI, a paste is not bracketed.
    let effects = app.update(key(tab, Key::Up, NONE));
    assert_eq!(inputs(&effects), [b"\x1b[A".to_vec()]);
}

/// "Restart shell" while the transport reconnects by itself (Task 5 fix round 1): the
/// click is taken, nothing is sent yet (no reconnect of the app's own, no shell on a dead
/// session), and the shell opens once the session is back.
#[tokio::test]
async fn restart_waits_for_the_connection() {
    let s = ssh_site("term", OpenAs::Terminal);
    let (mut app, fake, session) = connected(&s).await;
    let tab = app.tabs[0].id;
    let shell = shell_of(&app, 0);
    app.update(Message::ShellEvent {
        tab,
        shell,
        event: ShellSignal::Exit(Some(0)),
    });
    app.update(Message::ShellEvent {
        tab,
        shell,
        event: ShellSignal::Closed,
    });
    assert!(app.can_restart_terminal(0));
    // Then the connection drops: the transport reconnects.
    let lost = Error::new(hoprs_core::ErrorKind::Network, "Connection reset");
    fake.drop_connection(session, lost.clone());
    app.update(Message::SessionState {
        session,
        state: ConnectionState::Reconnecting { attempt: 1 },
        auth: None,
    });
    assert!(app.can_restart_terminal(0));
    assert!(
        app.update(Message::Terminal {
            tab,
            msg: TerminalMsg::Restart,
        })
        .is_empty()
    );
    assert_eq!(term(&app, 0).shell, ShellState::Idle, "waits for its shell");
    let effects = app.update(Message::SessionState {
        session,
        state: ConnectionState::Connected,
        auth: None,
    });
    let opens = effects
        .iter()
        .filter(|e| matches!(e, Effect::OpenShell { .. }))
        .count();
    assert_eq!(opens, 1, "{effects:?}");
}

/// The remote title is capped; input waiting behind a slow write is capped too, and the
/// tab says when it dropped some.
#[tokio::test]
async fn title_and_pending_input_are_capped() {
    use crate::app::terminal::{MAX_PENDING_INPUT, MAX_TITLE_CHARS};

    let s = ssh_site("term", OpenAs::Terminal);
    let (mut app, _fake, _) = connected(&s).await;
    let tab = app.tabs[0].id;
    let shell = shell_of(&app, 0);
    let long = format!("\x1b]0;{}\x07", "t".repeat(10_000));
    app.update(Message::TerminalOutput {
        tab,
        shell,
        bytes: long.into_bytes().into(),
        credit: Default::default(),
    });
    assert_eq!(
        term(&app, 0).title.as_ref().map(|t| t.chars().count()),
        Some(MAX_TITLE_CHARS)
    );

    // One write in flight; pastes pile up behind it until the cap.
    assert_eq!(inputs(&app.update(key(tab, Key::Char('x'), NONE))).len(), 1);
    let chunk = "p".repeat(MAX_PENDING_INPUT / 2);
    for _ in 0..2 {
        app.update(Message::Terminal {
            tab,
            msg: TerminalMsg::Pasted(Some(chunk.clone()).into()),
        });
    }
    assert!(!term(&app, 0).input_dropped);
    app.update(Message::Terminal {
        tab,
        msg: TerminalMsg::Pasted(Some(chunk.clone()).into()),
    });
    assert!(
        term(&app, 0).input_dropped,
        "the third paste is past the cap"
    );
    // What fit goes out whole, once the write in flight ends.
    let effects = app.update(Message::Terminal {
        tab,
        msg: TerminalMsg::InputSent(shell),
    });
    assert_eq!(inputs(&effects)[0].len(), 2 * chunk.len());
    // The next key clears the note.
    app.update(key(tab, Key::Char('y'), NONE));
    assert!(!term(&app, 0).input_dropped);
}

/// Review Focus 3: "Restart shell" on a session lost for a network reason reconnects it,
/// then opens the shell once the session is back (once, however the app hears of it).
#[tokio::test]
async fn restart_on_lost_session_reconnects_then_opens() {
    let s = ssh_site("term", OpenAs::Terminal);
    let (mut app, fake, session) = connected(&s).await;
    let tab = app.tabs[0].id;
    let shell = shell_of(&app, 0);
    app.update(Message::ShellEvent {
        tab,
        shell,
        event: ShellSignal::Exit(Some(0)),
    });
    app.update(Message::ShellEvent {
        tab,
        shell,
        event: ShellSignal::Closed,
    });
    let lost = Error::new(hoprs_core::ErrorKind::Network, "Connection reset");
    fake.drop_connection(session, lost.clone());
    app.update(Message::SessionState {
        session,
        state: ConnectionState::Disconnected(lost),
        auth: None,
    });
    assert!(app.can_restart_terminal(0), "a click reconnects");
    let effects = app.update(Message::Terminal {
        tab,
        msg: TerminalMsg::Restart,
    });
    let [Effect::Reconnect { session: again }] = &effects[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(*again, session);
    assert!(app.conn(&s.id).reconnecting);

    let backend: Arc<dyn BackendApi> = fake.clone();
    let effects = app.update(effects::reconnect(backend, session).await);
    let opens = |effects: &[Effect]| {
        effects
            .iter()
            .filter(|e| matches!(e, Effect::OpenShell { .. }))
            .count()
    };
    assert_eq!(opens(&effects), 1, "{effects:?}");
    let effects = app.update(Message::SessionState {
        session,
        state: ConnectionState::Connected,
        auth: None,
    });
    assert_eq!(opens(&effects), 0, "{effects:?}");
}

/// Final review: only the user's own terminal actions (a key, a paste, "Restart") count
/// as acting on the site; the terminal's timers and write acknowledgements never do.
#[test]
fn terminal_timers_are_not_user_actions() {
    use crate::app::reconnect::is_user_action;
    let tab = TabId(1);
    let msg = |msg| Message::Terminal { tab, msg };
    for user in [
        TerminalMsg::Key {
            key: Key::Char('x'),
            mods: NONE,
        },
        TerminalMsg::Paste,
        TerminalMsg::Pasted(Some("x".to_owned()).into()),
        TerminalMsg::Restart,
    ] {
        assert!(is_user_action(&msg(user.clone())), "{user:?}");
    }
    for not_user in [
        TerminalMsg::ResizeDue(1),
        TerminalMsg::SearchDue(1),
        TerminalMsg::InputSent(ShellId(1)),
        TerminalMsg::Resized { cols: 80, rows: 24 },
        TerminalMsg::Focus(true),
        TerminalMsg::Scroll(1),
        TerminalMsg::Copy,
        TerminalMsg::SearchInput("x".into()),
    ] {
        assert!(!is_user_action(&msg(not_user.clone())), "{not_user:?}");
    }
}

/// Final review: what a shell prints and what the user types or pastes into it never
/// shows in the `Debug` of the messages and effects that carry it (a password typed at a
/// `sudo` prompt, a pasted token); only its size does.
#[test]
fn terminal_content_never_in_debug() {
    const OUTPUT: &str = "OUTPUT-hunter2-Zq9";
    const PASTE: &str = "PASTE-tok3n-Xw7";
    const TYPED: &str = "TYPED-s3cret-Kp4";
    let tab = TabId(1);
    let shell = ShellId(1);
    let shown = [
        format!(
            "{:?}",
            Message::TerminalOutput {
                tab,
                shell,
                bytes: OUTPUT.as_bytes().to_vec().into(),
                credit: crate::effects::Credit::default(),
            }
        ),
        format!(
            "{:?}",
            crate::effects::Batched::Data(
                OUTPUT.as_bytes().to_vec().into(),
                crate::effects::Credit::default()
            )
        ),
        format!(
            "{:?}",
            Message::Terminal {
                tab,
                msg: TerminalMsg::Pasted(Some(PASTE.to_owned()).into()),
            }
        ),
        format!(
            "{:?}",
            Message::Terminal {
                tab,
                msg: TerminalMsg::Key {
                    key: Key::Text(TYPED.to_owned()),
                    mods: NONE,
                },
            }
        ),
        format!(
            "{:?}",
            Message::Terminal {
                tab,
                msg: TerminalMsg::Key {
                    key: Key::Char('\u{2603}'),
                    mods: NONE,
                },
            }
        ),
        format!(
            "{:?}",
            Effect::ShellInput {
                tab,
                shell,
                data: TYPED.as_bytes().to_vec().into(),
            }
        ),
        format!("{:?}", Effect::WriteClipboard(PASTE.to_owned().into())),
    ];
    for debug in &shown {
        for secret in [OUTPUT, PASTE, TYPED, "\u{2603}"] {
            assert!(!debug.contains(secret), "{secret} in {debug}");
            // Nor as the bytes' numbers.
            let bytes = format!("{:?}", &secret.as_bytes()[..3]);
            let bytes = bytes.trim_matches(['[', ']']);
            assert!(!debug.contains(bytes), "{secret} as {bytes} in {debug}");
        }
    }
    // The size still shows (enough to debug a flow).
    assert!(shown[0].contains(&OUTPUT.len().to_string()), "{}", shown[0]);
}
