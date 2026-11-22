# MCP server for agents — design

Status: approved by the author (2026-11-22) in conversation ("aprovada"). Decided there: what
the agent may do ("ler, criar, editar e transferir arquivos (se a checkbox estiver
selecionada) para auxiliar no dia a dia", then "também pode apagar arquivos, pastas e rodar
comandos no terminal"), the permission model ("deve ter duas opções, permitir leitura e a
outra seria permitir tudo", per site), approach A (the MCP talks to the open hoprs), and
the two behaviour playbacks ("tudo certo", twice). Ships in the same release as the partial
export and Google Drive upload (`2026-11-22-export-drive-design.md`).

## Intent

The author works with coding agents (Claude Code and others) every day. Today an agent
cannot use the servers saved in hoprs: it would need its own credentials and its own SSH or
FTP setup. The author wants agents to **work on the saved sites with autonomy** (read,
create, edit, transfer and delete files, run commands), **only on the sites the author
allowed, at the level the author chose**, with hoprs keeping the credentials and showing
what the agent does.

## Goals

- An MCP server any MCP client can start with one command: `hoprs mcp`.
- Per site, in the site form, one of three levels: **No access** (default), **Allow
  reading**, **Allow everything**.
- A master switch in Settings, off by default.
- Tools for: listing allowed sites, listing folders, reading text files, downloading,
  and (everything) writing files, creating folders, renaming, changing permissions,
  uploading, deleting, running a command over SSH.
- The agent acts without a confirmation per action, inside the level it was given.
- The user sees it: a status-bar indicator while an agent is connected, a log line per
  action, the agent's transfers in the queue.
- Security questions (unknown or changed host key, certificate, password, passphrase) are
  answered by the user in the hoprs window, never by the agent.

## Non-goals

- Working with hoprs closed, or on a machine without the window (a headless mode).
- An MCP endpoint over the network or over HTTP.
- An interactive terminal for the agent, folder compare for the agent (both: later).
- The agent managing hoprs itself: creating, editing or deleting saved sites and
  workspaces, changing settings, changing its own access, exporting backups.
- Restrictions finer than the site (folder allow-lists, command allow-lists).
- A confirmation per action, or per destructive action.
- A persistent audit file. The record is the log panel (in memory, as today).
- MCP features other than tools (resources, prompts, sampling).

## Anti-goals

- **An agent touching a site that is "No access"**, or learning that it exists.
- **A secret reaching the agent:** passwords, passphrases, key files and their paths, the
  Google Drive authorisation, the backup password. Never in a tool result, an error or the
  log.
- **The channel reachable by anyone else:** another user of the machine, or the network.
- **The agent raising its own access**, or answering a security prompt, **also through
  the local side of a transfer** (see "Protected local paths").
- **Interrupting the user:** no tab opened, no focus taken, no sound when an agent works.
  (The security dialogs are the exception: they are the app's only modals already.)
- **The agent stuck on a question meant for a person** ("file exists: overwrite?").
- **Anything on stdout in `hoprs mcp` except the protocol.**
- Slowing the app: agent work never runs on the UI thread and never delays startup.

## Approaches considered

**Where the permission lives** (author chose per site, two levels):
- **Per site, "Allow reading" and "Allow everything" (chosen).**
- One switch per site (all or nothing); one global switch (every saved site at once).

**Where the server runs** (author chose A):
- **A. `hoprs mcp` is a small relay that talks to the open hoprs (chosen).** The app does
  the work with its live backend, so the prompts, the log, the queue and the existing
  connections all apply. Cost: hoprs must be open.
- B. A headless hoprs with its own backend. Works with the app closed, but nobody sees
  what the agent does, there is nowhere to ask about a new host key, and it would fight
  the open app for the transfer queue and the single-instance rule.
- C. An HTTP server inside the app. No relay, but an open local port, a token to manage
  and an HTTP server in the app, for nothing A lacks.

## Design

### The two halves

```
agent (Claude Code, …) ──stdio, MCP──▶ hoprs mcp (relay) ──local channel──▶ hoprs (the open app)
```

**`hoprs mcp` (the relay).** A mode of the same executable, parsed in `cli.rs` like
`--version`: no instance lock, no window, no log on stdout (the log goes to stderr).
- It speaks MCP over stdio and owns the protocol: `initialize`, the tool list, turning a
  tool call into a request to the app and the answer into a tool result.
- **The tool list is fixed** and does not depend on the app, so an agent started before
  hoprs still sees every tool. Permissions are enforced by the app on every call.
- It keeps one connection to the app while it lives (so the app can show "agent
  connected") and reconnects when a call finds none.
- When it cannot reach the app (hoprs closed, or the master switch off) every tool call
  returns a tool error that says exactly that and what to do ("Open hoprs and turn on
  Settings › Agents"). It never starts hoprs itself.
- Relay and app are one binary, but an updated relay can meet an older running app: the
  channel starts with a version handshake, and a mismatch is a tool error asking to
  restart hoprs.

**The app (the agent service).** A backend service in `hoprs-app` (no iced in it), started
with the backend when the master switch is on:
- listens on the local channel, checks every request (master switch, the site's level,
  the protocol), runs it against the real backend, and reports activity to the UI;
- its request/response format on the channel is private to hoprs (for example one JSON
  object per line, with a request id so calls can overlap).

### The local channel

- **Linux and macOS:** a Unix domain socket named `agent.sock` in the first usable
  directory of `instance_lock_paths()` (the directory of the single-instance lock), the
  directory `0700` and the socket `0600`, plus a check that the peer is the same user.
- **Windows:** a named pipe whose name carries the user (for example
  `\\.\pipe\hoprs-agent-<user SID>`), created with a security descriptor that admits only
  that user.
- It exists only while the master switch is on and hoprs runs. Turning the switch off
  closes it and drops the connected relays; a stale socket file from a crash is replaced.
- Several relays (several agents) may be connected at once.

### Permissions

- `hoprs-core`: `Site` gains the access level (for example `AgentAccess::{None, Read,
  All}`), stored in `sites.toml` as `agent_access = "read" | "all"`, absent meaning none.
  Every existing site is therefore "No access". (Older hoprs versions keep unknown fields
  on save, so opening the file with 1.5.0 does not lose it.)
- **Site form:** a section "Agents (MCP)" with one choice of three: No access / Allow
  reading / Allow everything, and a one-line hint under each of the last two saying what
  it allows ("everything" on an FTP site: files only, no commands).
- **Never restored or imported** (the controller's call): a backup, a partial export and
  the FileZilla import never carry or set the level; a restored or imported site is "No
  access". Access is only ever given by hand, in the form.
- **Settings › Agents (MCP)**, a new section:
  - "Allow agents to connect to hoprs" (off by default; `settings.toml`);
  - the command to register hoprs in Claude Code, with **Copy**, built from the running
    executable's full path (on macOS and Windows `hoprs` is not on `PATH`):
    `claude mcp add hoprs -- "<path to hoprs>" mcp`, and the same as a JSON snippet for
    other clients (`{"command": "<path>", "args": ["mcp"]}`);
  - a line with how many sites have each level.
- A change takes effect at once: every call re-checks. Calls already running finish;
  transfers the agent already queued stay in the queue (the user can cancel them there).

| | No access | Allow reading | Allow everything |
|---|---|---|---|
| appears in `list_sites` | no | yes | yes |
| list, stat, read, download | no | yes | yes |
| write, mkdir, rename, chmod, upload, delete | no | no | yes |
| run a command | no | no | SSH sites only |

A call on a site without the needed level fails with one error that does not tell "no
access" from "no such site".

### Tools

Names and fields are for the builder to finalise against the current MCP specification;
this is the set and the behaviour. Sites are addressed by the id `list_sites` returns.
Local paths are absolute paths on the user's machine; everything is allowed except the
protected paths below. Tool descriptions and errors for the agent are in English
only and are not part of the Fluent catalogs.

**Reading level**
- `list_sites`: the allowed sites: id, name, protocol, host, port, user, level, whether
  connected, the start folder. No secret, no key-file path.
- `list_directory(site, path)`: name, kind, size, modified, permissions; capped (for
  example 5 000 entries) with a "truncated" flag.
- `stat(site, path)`.
- `read_file(site, path, offset?, length?)`: UTF-8 text, at most 1 MiB per call. A larger
  or non-text file is an error that points to `download`.
- `download(site, remote_path, local_path, overwrite = false)`: a file or a folder,
  through the transfer queue.
- `transfer_status(transfer)`: state and counts of a transfer this agent started.

**Everything level**
- `write_file(site, path, content, overwrite = false)`: creates or replaces a text file
  (at most 1 MiB), written straight through the remote filesystem like a remote edit.
- `make_directory(site, path)`, `rename(site, from, to)`,
  `set_permissions(site, path, mode)`.
- `upload(site, local_path, remote_path, overwrite = false)`: a file or a folder, through
  the transfer queue.
- `delete(site, path, recursive = false)`: a folder with contents needs `recursive`.
- `run_command(site, command, timeout_seconds = 60)`: SSH only. A new exec channel on the
  site's connection (no PTY, no stdin); returns stdout, stderr and the exit code; each
  stream capped at 256 KiB with a "truncated" flag; the time limit is at most 600 s, after
  which the channel is closed and the result says so.

**Protected local paths** (the author's choice after the builder check). `download` and
`upload` refuse a local path that is, or is inside, or (for a folder transfer) contains:
- hoprs's own configuration and data directories (`sites.toml`, `workspaces.toml`,
  `settings.toml`, the queue, the edit and recovered copies, the channel's socket);
- the user's `~/.ssh` directory (`known_hosts`, keys, `config`), and hoprs's own
  `known_hosts` if it lives elsewhere;
- every key file a saved site names, wherever it is.

Without this, the tools themselves would defeat the anti-goals even from a site that is
only "Allow reading": a download could overwrite `known_hosts` (a changed host key
accepted with no dialog) or `sites.toml` and `settings.toml` (the agent's own access), and
an upload followed by a read could hand a private key to the agent. The check runs on the
canonical path (symbolic links resolved, for a file that does not exist yet through its
nearest existing parent), compares case-insensitively on Windows and macOS, and is
repeated for every item of a folder transfer, so a link inside an uploaded folder cannot
lead into a protected path. The refusal is one tool error that names the path as
protected. The rest of the disk stays open.

**Overwriting is explicit.** `overwrite = false` on an existing target is an error for
`write_file` and a skipped item for transfers. Agent transfers run under their own policy
(Overwrite or Skip), never "Ask", whatever the user's default is. Today
`enqueue_with_policy` applies its policy only to the enqueued item and a folder's children
fall back to the global policy, so `hoprs-transfer` changes: the policy of an enqueued
folder is inherited by everything under it.

**Long transfers.** `download` and `upload` wait for the transfer up to about a minute.
If it finished, they return the summary (files, bytes, skipped, failed with reasons); if
not, they return its id and "still running", and the agent polls `transfer_status`.

**Annotations.** Each tool carries the MCP hints for read-only and destructive tools, so
clients can show them properly.

### Connections and prompts

- A call on a site that is not connected connects it (`SessionManager::connect`), with no
  tab and no focus change. The sidebar shows the site as connected, as for a tab.
- There is one session per site (existing rule), so the agent shares the session a tab
  already uses, shells and edits untouched.
- A session opened for an agent and used by no tab closes after 10 minutes without an
  agent call. A session a tab uses is never closed by the agent service.
- **Prompts** go through the existing prompt broker to the existing dialogs, which gain a
  line saying the connection was asked for by an agent. The tool call waits up to two
  minutes; then the attempt is cancelled, its prompt withdrawn, and the agent gets an
  error saying the user has to answer in the hoprs window. A changed host key is always
  an error for the agent (the user resolves it in the app, as today).
- Channel limits of the SSH server apply to exec channels as they do to shells and SFTP
  streams; a refused channel is an ordinary tool error.

### What the user sees

```
Status bar:  ● Agent connected
Log:         14:02 agent › read /var/www/app/config.php on "Acme produção"
             14:03 agent › ran `systemctl status nginx` on "Acme staging" (exit 0)
```

- **Status bar:** an indicator while at least one relay is connected, with a tooltip.
  Nothing is shown otherwise.
- **Log:** one INFO line per tool call (action, site, path or command, outcome), under its
  own target so the log filter finds it. Never file contents, never command output, never
  a secret. The command line itself is logged: it is the record the author asked for.
- **Queue:** agent transfers are ordinary rows, with a small "agent" tag.
- The README says plainly what "Allow everything" means: an agent can be misled by what it
  reads (a file, a command's output), hoprs does not review its actions, and the level is
  the safeguard; keep production on "Allow reading" unless the agent really must change it.

### Code placement

- `hoprs-core`: the access level on `Site`.
- `hoprs-vault`: `sites.toml` field; export and import leave it out.
- `hoprs-ssh`: the exec channel.
- `hoprs-transfer`: a folder's children inherit the policy the folder was enqueued with.
- `hoprs-app`: the relay (`cli.rs` plus an `agent` module), the agent service in the
  backend behind `BackendApi` (so the fake backend covers the UI tests), the site form
  section, the Settings section, the status-bar indicator, the queue tag, the strings in
  `en.ftl`, `pt-BR.ftl` and `tr.ftl` (the controller writes the Turkish; the translator
  reviews later).
- No new crate: the dependency graph of the main spec §4.1 stays.

### Docs

README (a section "Agents (MCP)": setup for Claude Code and the generic snippet, the
levels, what the agent can and cannot do, the warning above), `docs/ARCHITECTURE.md`,
`docs/ROADMAP.md`, `CHANGELOG.md` (entry without a version bump), the privacy page of the
export design (one line: the MCP channel is local, hoprs sends nothing anywhere because of
it), the site's feature list.

## Tests

- Access matrix: every tool × every level × SSH and FTP, including "no such site" and "no
  access" being indistinguishable, and a level change taking effect on the next call.
- Relay: `initialize`, the tool list and tool calls driven over in-memory streams; app
  unreachable, master switch off and version mismatch each give their tool error; nothing
  but protocol on stdout.
- Channel: socket permissions and same-user check (Unix), pipe ACL (Windows), stale socket
  replaced, closed when the switch goes off.
- Service (fake backend and the in-memory remote filesystem): each tool's behaviour, the
  caps and "truncated" flags, overwrite rules, transfers returning "still running", the
  idle close of agent sessions, prompts timing out.
- `hoprs-ssh`: exec against the Docker SSH server (the ignored integration tests): output,
  exit code, time limit, large output.
- Protected local paths: each protected location refused for download and for upload, as
  itself, as a parent and as a child; through a symbolic link; with another letter case
  on Windows and macOS; a link inside an uploaded folder; a key file outside `~/.ssh`
  named by a site; a neighbouring path still allowed.
- Vault: the field round-trips in `sites.toml`; export, restore and import never carry it.
- i18n parity over the three catalogs.
- QA checklist `docs/qa/mcp-agents-checklist.md`, NEEDS HUMAN: a real Claude Code session
  on Linux, macOS and Windows (register, list, read, edit, upload, delete, run a command,
  a host-key prompt answered in the window, the switch turned off mid-session), and that
  stdio works with the Windows release binary (it is a GUI-subsystem executable).

## Constraints

- **The MCP specification moved recently** (a 2026-07-28 revision changed how sessions and
  protocol versions work). The builder reads the current specification and the client
  documentation before writing the relay, and supports the protocol versions current
  Claude Code negotiates. The official Rust SDK (`rmcp`) is preferred if its dependency
  tree respects the project's rules (no OpenSSL, no ring, no second TLS stack; the relay
  needs only stdio); otherwise the stdio transport is small enough to write directly.
- Tests pass on Linux, macOS and Windows (`tempfile`, never a literal `/tmp`); fmt and
  clippy clean on every commit; only `hoprs-app` depends on iced.
- Visual rules of mockup v2; no new modal.

## Builder-check rulings (2026-11-22)

1. **Local paths (the author's answer):** open, except the protected paths: hoprs's own
   directories, `~/.ssh`, and the key files saved sites name.
2. **Transfer policy (the controller's call):** `hoprs-transfer` changes so a folder's
   children inherit the policy the folder was enqueued with.

## Release

Ships in **1.6.0** with the partial export and Google Drive upload. That plan lands first;
this one rebases on it (both touch the site form, `BackendApi` and Settings). No version
bump here: the release pull request owns it.

## Risks

- **Prompt injection.** Content the agent reads on a server can steer it. hoprs cannot
  prevent that; the levels, the master switch and the log are the mitigation, and the
  README says so.
- **Windows stdio** with a GUI-subsystem binary whose `main` attaches to the parent
  console for `--version`: `mcp` mode must leave redirected handles alone. Covered by QA.
- **Same-user processes.** Any program running as the user can connect to the channel when
  the switch is on. That is the accepted bar (such a program can already read
  `sites.toml` and ask the keyring), and the reason the switch is off by default.

## Left to the builder

- Tool and field names, descriptions and JSON schemas; error wording.
- The private format of the channel and the handshake.
- SDK or hand-written stdio transport (see Constraints).
- How the app tracks agent-opened sessions and shows them in the sidebar.
- The exact caps (entries, bytes, wait and time limits) where the design says "for example"
  or "about".
- Concurrency limits per relay.
