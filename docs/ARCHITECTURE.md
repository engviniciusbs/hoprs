# How hoprs is built

A map for contributors: where things live, how a click becomes network traffic, and where to
put a change. The README's [Architecture](../README.md#architecture) section has the short
version; the full original design (in Portuguese) is
[docs/superpowers/specs/2026-11-11-hoprs-design.md](superpowers/specs/2026-11-11-hoprs-design.md).

## The crates

A Cargo workspace under `crates/`. Only `hoprs-app` knows about the GUI; everything else is
plain async Rust, tested without a window.

| Crate | What it owns |
|---|---|
| `hoprs-core` | Shared types: `Site`, `Protocol`, `Workspace`, `RemotePath`, `RemoteEntry`, `ConnectionState` and the one `Error` type (a kind, an English message, an optional detail, and a stable `code` + `args` for translation) |
| `hoprs-vault` | The profile file (`sites.toml`), secrets in the OS keyring, encrypted backup export (of everything, one workspace or one site) and import (Argon2id + XChaCha20-Poly1305) |
| `hoprs-ssh` | SSH sessions on russh: authentication (agent, keys, password, keyboard-interactive), `known_hosts`, keepalive, the PTY shells of the terminal, and `exec`: one command on its own channel (no PTY, no stdin), with a time limit and capped output, for agents |
| `hoprs-remote` | The `RemoteFs` trait and its implementations: SFTP (over a `hoprs-ssh` session), FTP / FTPS (suppaftp), the local disk, and an in-memory one for tests |
| `hoprs-transfer` | The transfer engine: a persistent queue (SQLite), workers per site, resume, conflict policies, one writer per destination, recovery after a crash. For agents: a policy (overwrite or skip) that a folder's children inherit, an origin (`User` or `Agent`) kept in the queue and passed to every child, and a guard hook the app installs, which every agent item passes before any file is touched |
| `hoprs-import` | Reads FileZilla's `sitemanager.xml` and turns it into sites and workspaces |
| `hoprs-app` | The `hoprs` binary: the iced application, its screens, the backend that wires the crates together, settings, the translation catalogs |

Dependencies (A → B means A depends on B). This graph is fixed; a change that needs a new
edge is a design discussion first.

```
every crate    → hoprs-core
hoprs-remote   → hoprs-ssh
hoprs-transfer → hoprs-remote
hoprs-import   → hoprs-vault
hoprs-app      → hoprs-transfer, hoprs-remote, hoprs-ssh, hoprs-vault, hoprs-import
```

One tokio runtime runs all I/O. The UI thread never blocks on it.

## `RemoteFs`: one interface for every protocol

`hoprs-remote` defines `RemoteFs` (`list`, `stat`, `open_read`, `open_write`, `mkdir`,
`rename`, `remove`, …). The file panes and the transfer engine only see this trait, so SFTP,
FTP, FTPS and the local disk behave the same to them. A new protocol is a new `RemoteFs`
implementation plus a `Protocol` variant; the panes and the queue need no change.

## Inside `hoprs-app`

The app follows iced's model: state, messages, a pure update, and views.

```
 user input ──► Message ──► App::update ──► state changes
                               │
                               └──► Effect ──► effects::run ──► BackendApi ──► crates
                                                                    │
 views ◄── App state ◄── Message (results, events) ◄────────────────┘
```

| Where | What |
|---|---|
| `src/app.rs`, `src/app/*.rs` | `App` (all UI state), `Message`, and the **pure** `update`: no I/O, it only changes state and returns `Effect`s. One file per area (`files`, `transfers`, `terminal`, `edit`, `dialogs`, `palette`, `settings`, …) |
| `src/app/export.rs`, `src/ui/export.rs` | The export flow, a centre form (`Form::Export`) with its state in `App::export`: scope, File field, password, overwrite question, running and result; the view draws it |
| `src/effects.rs` | Turns each `Effect` into an `iced::Task`: the only place that calls the backend |
| `src/backend/` | `BackendApi` and its real implementation: the vault, the transfer engine, the session manager (`sessions.rs`), the terminal shells, remote edits (`edit.rs`), and the prompt broker (`broker.rs`) that turns "trust this host key?" or "password?" requests from the connection crates into dialogs and the answers back |
| `src/backend/fake.rs` | `FakeBackend`, an in-memory `BackendApi` for UI tests (test builds only) |
| `src/ui/` | Views, one file per area (`sidebar`, `files`, `queue`, `terminal`, `settings`, `dialogs`, …), the theme, widgets, and `fit.rs` (what fits at which width) |
| `src/ui/strings.rs` | **Every user-visible text**, as functions that look up the translation catalogs |
| `src/i18n.rs`, `i18n/*.ftl` | The Fluent catalogs (English, Brazilian Portuguese), the active language, number formatting |
| `src/terminal/` | The terminal widget on top of `alacritty_terminal` |
| `src/update/`, `src/opener.rs` | The update notice. `update/` is pure logic (release JSON, version compare, `SHA256SUMS` parsing, picking the package for this system) plus a blocking HTTP client and the download sequence (download, verify, hand off), run on the blocking pool through `BackendApi` methods. `opener.rs` opens the package or page with the system. The state machine and its messages are `app/update.rs` (pure, like the rest of `update`); the card, footer line and Settings rows are `ui/update.rs` |
| `src/drive/` | The Google Drive destination of the export. `config.rs` is the build-time OAuth client and the endpoints; `oauth.rs` the PKCE, `state` and URL pieces and the callback parser; `loopback.rs` the listener on `127.0.0.1` that waits for the browser; `client.rs` the blocking HTTPS client with its rules (HTTPS only, no redirect, a token only to its own host, multipart upload of at most 5 MB); `mod.rs` `Drive`, which holds the connection (memory plus the keyring entry) and runs connect, upload and disconnect. It reaches the app through the `drive_*` methods of `BackendApi` (`drive_status`, `drive_connect`, `drive_cancel`, `drive_disconnect`, `drive_upload`, plus `seal_export` and `save_sealed`, which the Drive flow shares with the file export); the flow is in `app/export.rs`. Run on the blocking pool; nothing in it imports iced |
| `src/agent/` | The MCP server for coding agents, in two halves (below). `relay.rs` is `hoprs mcp`; `channel.rs` (`channel/unix.rs`, `channel/windows.rs`) is the local socket or pipe; `proto.rs` is what travels on it; `server.rs` is the app's side of the channel; `service.rs` (`service/files.rs`, `service/transfers.rs`) runs the tools against the `AgentHost` trait (`host.rs`); `tools.rs` is the table of the thirteen tools; `paths.rs` is local paths and the protected ones; `testing.rs` is `FakeHost` for tests. Nothing in it imports iced. The real host is `src/backend/agent.rs` |
| `src/settings.rs` | The settings file (`settings.toml`): load, validate, save, keeping keys a newer hoprs wrote |

### Agents (MCP)

An agent talks to a small relay; the relay talks to the open app. The app does the work with
its live backend, so the prompts, the log, the queue and the sessions all apply.

```
agent ──stdio, MCP──▶ hoprs mcp ──socket or pipe──▶ hoprs (the open app)
(Claude Code, …)      agent/relay.rs                 agent/server.rs ─▶ agent/service.rs ─▶ AgentHost
                                                                                          (sessions, engine, RemoteFs)
```

- **The relay** (`hoprs mcp`, parsed in `cli.rs`, never takes the instance lock) speaks MCP
  over stdio through the `rmcp` crate, in both protocol eras. Its tool list is the fixed table
  in `agent/tools.rs`, so it does not depend on the app. It turns a call into one line on the
  channel and the answer into a tool result. Stdout carries protocol messages only.
- **The channel** is `agent.sock` beside `instance.lock` (a named pipe on Windows), opened
  only while the Settings switch is on. It admits the same user only. A hello with the
  version comes first: a relay and a window of different versions are refused.
- **The service** (`agent/service.rs`) is the single path to everything: `Service::call`
  checks the switch, the site's level and the protocol (`Service::admit`), then runs the tool.
  It reaches the app only through `AgentHost`, so it is tested against `FakeHost` (an
  in-memory host over `MemFs`), not through `BackendApi`. `BackendApi` gained only
  `set_agents_enabled` and `agent_events`.
- **Sessions:** a call on a site that is not connected connects it with no tab; the session
  is marked agent-only until a tab takes it over, and an idle reaper closes it (never while
  the site has an unfinished transfer).
- **Transfers** go through the engine with origin `Agent` and a policy that never asks. The
  guard installed in `backend/agent.rs` refuses any item whose local path is protected
  (`agent/paths.rs`: hoprs's own directories, `~/.ssh`, `known_hosts`, every saved key file).

### Where to put a change

- **A new text on screen:** add the key to `i18n/en.ftl`, `i18n/pt-BR.ftl` and `i18n/tr.ftl`, and a
  function in `ui/strings.rs` that looks it up with a literal key (`tr("my-key")` or
  `tr_args("my-key", &[…])`). Never put a user-visible string literal in a view. Tests fail
  if a key is missing in a language, unused, or uses different variables.
- **A new error hoprs reports:** `Error::coded(kind, "area-what-happened", "English text")`
  with `.arg("name", value)` for each value in the text, plus `error-area-what-happened` in
  both catalogs. Views show errors through `strings::error`, which translates them.
- **A new setting:** a field in `Settings` with its file key (unknown values are kept on
  save), a `SettingsMsg` variant handled in `app/settings.rs`, and a row in
  `ui/settings.rs`.
- **A new action on files:** the message and its pure handling in `app/files.rs` or
  `app/actions.rs`, an `Effect` if it needs the backend, and the backend call in
  `effects.rs` / `backend/`.
- **A new agent tool:** a row in `agent/tools.rs` (name, title, description, schema, level,
  hints), its handler in `agent/service/`, and its row in the `access_matrix` test, which
  pins every tool at every level on SSH and FTP. Tool texts are English only and live in
  `agent/`, not in the catalogs.
- **Anything protocol-level:** the protocol crate (`hoprs-ssh`, `hoprs-remote`), behind
  `RemoteFs` or the session API, with tests there.

## Testing

- **Unit tests** sit next to the code (`#[cfg(test)] mod tests`). `cargo test --workspace`
  runs them on every OS in CI.
- **App tests** (`crates/hoprs-app/src/app/tests/`) drive `App::update` with messages against
  `FakeBackend` and check the resulting state and effects. No window, no network. This is
  where most UI behaviour is tested.
- **Integration tests** (`crates/*/tests/`, marked `#[ignore]`) talk to real OpenSSH and
  vsftpd servers in Docker (`docker/compose.yml`), including FTPS and servers that drop idle
  connections. CI runs them on every pull request.
- **Benchmarks** (`crates/hoprs-app/benches/`) render the UI offscreen; the performance job
  and its targets are described in [docs/perf/README.md](perf/README.md).
- **Manual QA checklists** for what tests cannot see (real input devices, OS integration)
  are in [docs/qa/](qa/).

## Files hoprs keeps

| File | Owner | Notes |
|---|---|---|
| `sites.toml`, `workspaces.toml` (config dir) | `hoprs-vault` | Sites and workspaces; never secrets |
| OS keyring entries | `hoprs-vault` | Passwords and passphrases; the app's own entry (`hoprs/google-drive`, the Google Drive connection) |
| `settings.toml` (config dir) | `hoprs-app` | App settings |
| `queue.db` (data dir) | `hoprs-transfer` | The transfer queue (SQLite, versioned schema) |
| `agent.sock` (beside `instance.lock`; a named pipe on Windows) | `hoprs-app` | The channel for agents, present only while the switch is on and hoprs runs |
| `known_hosts` | `hoprs-ssh` | Shared with OpenSSH (`~/.ssh/known_hosts`) |

The README's [Where your data lives](../README.md#where-your-data-lives) section has the exact
paths per OS.
