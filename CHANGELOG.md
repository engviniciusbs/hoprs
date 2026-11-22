# Changelog

## 1.6.0 — unreleased

You can export one site or one workspace, not only everything.

- Right-click a site or a workspace in the sidebar and choose **Export…**. The ⋯ menu and Ctrl+K have **Export everything…**, and the site and workspace forms have an **Export…** button (the keyboard way in). Settings → Backup keeps "Export backup…" and the restore panel.
- One export screen for all three: a title that names what is exported, a File field that starts as `hoprs-<name>-<date>.hoprs` (`hoprs-backup-<date>.hoprs` for everything), the password typed twice, and a question before overwriting a file you typed the path of.
- A file of one site or one workspace holds that site or workspace, the sites in it and their passwords. It restores with **Merge** only; Replace is refused with "This file holds only part of a backup. Restore it with Merge." hoprs 1.5.0 and older answer "Unsupported backup version" to it. A file of everything is unchanged and restores on every version.

Coding agents can work on your saved sites.

- `hoprs mcp` is an MCP server that Claude Code and any other MCP client can start. Settings › Agents (MCP) turns it on (off by default) and shows the command to register it, `claude mcp add hoprs -- "<path to hoprs>" mcp`, and a JSON snippet for other clients. hoprs must be open: the program talks to the running window, which keeps the passwords.
- Each site has a level in its form: **No access** (the default for every site, existing ones included), **Allow reading** (list, stat, read a text file, download) or **Allow everything** (also write, make folders, rename, change permissions, upload, delete, and run a command on SSH sites). An agent cannot see a site with no access, or tell it exists. A backup, a partial export and the FileZilla import never carry or set the level.
- Agents cannot see passwords, passphrases or key files, answer a host-key, certificate or password question (it appears in the hoprs window, marked "An agent asked for this connection."), change their own access, manage sites or settings, or transfer to or from hoprs's own files, `~/.ssh` or a site's key file.
- What you see: an "Agent connected" indicator in the status bar, an "agent" tag on its transfers in the queue, "Connected (by an agent)" on the site in the sidebar, and one line in the log per action (the command line included; never file content or command output). Turning the switch off closes agent sessions at once.
- The channel is local: a socket (`agent.sock`) or, on Windows, a named pipe, open to your user only and only while the switch is on. Nothing listens on the network.
- An agent can be misled by what it reads, and hoprs does not review its actions: keep production sites on "Allow reading" unless the agent really must change them.
- A queue file written by this version is refused by hoprs 1.5.0 and older, which then run on an empty in-memory queue.

## 1.5.0 — 2026-11-18

hoprs speaks Turkish.

- The whole interface is available in Turkish (Türkçe), translated by İsmail Duran ([@P1LOTRX](https://github.com/P1LOTRX), #40): menus, dialogs, errors, the queue, dates and numbers (`1.000`, `4,1 KB`, `11 Kas 14:02`).
- Settings › Language offers Türkçe, and "System" picks it on a Turkish OS locale (`tr_TR.UTF-8`, `tr-TR`, …).
- The Linux desktop entry has Turkish names.

## 1.4.0 — 2026-11-17

hoprs tells you when a new version is out, and can install it.

- An update card in the sidebar appears once per new version (a footer line keeps the offer after you dismiss it). hoprs checks GitHub's latest release a few seconds after start and every 24 hours; the request carries only `User-Agent: hoprs/<version>` and honours the proxy environment variables (`ALL_PROXY`, `HTTPS_PROXY`, `HTTP_PROXY`).
- **Update** downloads the right package for your system, verifies it against the release's `SHA256SUMS` and opens the system installer: the `.msi` on Windows after a second click ("Close and install", with a warning when transfers are running), the `.dmg` on macOS, the `.deb` on Linux (App Center, plus the `sudo apt install` command to copy). The portable `.zip` and source builds open the release page.
- Settings → General: "Check for updates automatically" (on by default) and **Check now**.
- Ctrl+K: "Update hoprs to …" while a newer version is known.
- The checksum catches a broken download; it does not prove who published the release (see README, Updates).

## 1.3.1 — 2026-11-16

- Workspaces are plain groups in the sidebar: the big "Open workspace" button and the "Close tabs" link are gone. Right-click a workspace for "Open all", "Close all" (the open workspace) and "Edit". The open workspace is marked with a green icon, and Ctrl+K lists "Close all in (workspace name)" while one is open.
- A finished upload, download or drag shows up without Refresh: the destination folder is listed again, quietly (selection and scroll kept), in every pane that shows it; a burst of files gives one listing per pane every half second at most.

## 1.3.0 — 2026-11-16

hoprs gets a polished interface: icons, right-click menus, a drag that says what it will do, Settings as a tab, and a compare panel and queue you can resize.

- Icons (Phosphor, MIT) on actions, files, tabs and the sidebar.
- Disabled actions are readable and look disabled.
- Sidebar sections collapse and can be hidden (⋯ menu); connected sites are marked; Edit is a pencil on hover.
- Files and folders have kind icons; the selected row has a clear bar; long names end in … with the full name on hover.
- Tabs, the transfer queue and the status bar use icons: kind icons on tabs, icon-only close, new tab, queue and Log buttons with tooltips, a lock for encrypted connections.
- The hoprs logo at the top of the sidebar; the version moved to Settings.
- Settings open as a tab with a section list (Ctrl+, or ⌘,).
- The compare panel and the transfer queue resize by dragging their top edge (double-click resets); heights are remembered; collapse leaves one summary line.
- Right-click menus on files (with Copy path and Refresh, F5); Shift+F10 or the Menu key opens them from the keyboard.
- Right-click menus on the compare list, the queue, the sidebar and the tabs (with Close other tabs); Shift+F10 opens the compare list's from the keyboard.
- Dragging between panes starts after a small move and says what will happen: a chip follows the pointer, the dragged rows dim, and the target pane gets a dashed outline and says "Upload 2 items to …".

## 1.2.0 — 2026-11-16

hoprs speaks Brazilian Portuguese and compares folders.

- Folder compare: the Compare button (or Ctrl+K › Compare folders) opens a panel that lists what exists only locally, only on the server, or differs (size or time) between the two open folders. Upload or download the selected rows; they replace the file on the other side, and nothing is ever deleted.
- It re-runs by itself when you change folders (Settings › Folder compare turns that off), skips `.git`, `node_modules` and `.DS_Store` (an editable list, `*` allowed), pauses at 10 000 entries per side and asks before going on, and reports a subfolder it cannot read without stopping.
- It uses the tab's own connection. On FTP the walk lists one folder at a time, so the panes stay responsive. FTP listings without `MLSD` compare times to the minute.
- Files hoprs transferred stay "identical" for the session on servers that cannot set times.

- The whole interface is available in English and Brazilian Portuguese: menus, dialogs, errors, the queue, dates and numbers (`4,1 KB`, `11 nov 14:02`).
- New "Language" setting (Settings, Idioma): System (the default, follows the OS language), English, or Português (Brasil). It switches at once, no restart.
- Texts live in Fluent catalogs (`crates/hoprs-app/i18n/en.ftl` and `pt-BR.ftl`), so adding a language is a new file. Tests keep both catalogs in step.
- Errors carry a stable code, so their text follows the language. Messages from servers and the OS stay as they come.
- The queue file moves to schema v3 (it keeps error codes). Once 1.2 has opened it, 1.1 cannot open the file (it refuses a newer schema and runs on an in-memory queue, leaving the file untouched), so do not downgrade with transfers pending.
- The Linux desktop entry has Portuguese names.

## 1.1.0 — 2026-11-14

hoprs now ships for macOS and Windows as well as Linux.

- macOS: a universal `.dmg` (Apple Silicon and Intel, macOS 11 or newer), ad-hoc signed only. The first launch needs "Open Anyway" in System Settings, Privacy & Security (or `xattr -dr com.apple.quarantine /Applications/hoprs.app`). Passwords live in the Keychain; after each update macOS asks for access again, choose "Always Allow".
- Windows: an `.msi` (per machine, Start menu and desktop shortcut) and a portable `.zip` (x64). The C runtime is linked statically, so no Visual C++ redistributable is needed. On "Windows protected your PC" choose "More info", then "Run anyway". Passwords live in the Credential Manager.
- No console window on Windows; `hoprs --version` and `--help` still print into the cmd or PowerShell window that ran them.
- ssh-agent on Windows through the OpenSSH agent pipe (`\\.\pipe\openssh-ssh-agent`); Pageant is not supported.
- Remote editing: the default editor `system` opens files with `xdg-open` on Linux, `open -t` on macOS and the associated app on Windows (the Open With dialog if there is none). On Windows a custom editor must be a windowed program, for example `code.cmd --wait`.
- Command palette: Cmd+K on macOS (Ctrl+K elsewhere); in a focused terminal Ctrl+Shift+K everywhere.
- FileZilla import also reads `~/.config/filezilla` on macOS and `%APPDATA%\FileZilla` on Windows.
- Your data lives in `~/Library/Application Support/hoprs` on macOS and `%APPDATA%\hoprs` on Windows. Uninstalling never removes it or the saved passwords.
- Known issue, macOS: quit with the window's close button. Cmd+Q and Dock → Quit skip the unsent-edits prompt and do not save open tabs.
- Release workflow: builds on three runners and publishes all or nothing; a manual run is a dry run. The website's download buttons follow the visitor's system and go live only once the release is published.

## 1.0.0 — 2026-11-14

First release.

- Transfer engine: persistent queue (SQLite), parallel transfers, resume after a drop or a restart, conflict handling (ask, overwrite, overwrite if newer, skip, resume) per item or for the whole batch, size and time checks, throttled progress.
- SSH and SFTP: agent, key, password and keyboard-interactive authentication, `known_hosts` verification, keepalive and automatic reconnection with backoff, pipelined SFTP.
- FTP, FTPS explicit and FTPS implicit: connection pool, active and passive modes, PASV workaround for NAT, pinned server certificates, keepalive and automatic reconnection. Unencrypted FTP is labelled as such.
- Saved sites and workspaces: a workspace is a bundle of sites; opening it connects them all and restores its tabs. Profiles are plain TOML without secrets.
- Credentials in the system keyring (Secret Service); encrypted backup export and import (Argon2id and XChaCha20-Poly1305).
- Import of FileZilla `sitemanager.xml`, with a report of what was imported and what was skipped.
- Desktop app: sidebar, tabs, dual-pane file browser with virtualised lists (100 000 entries), transfer queue with progress, and inline error and security dialogs (host key and certificate changes) instead of modal cascades.
- Terminal for SSH sites: `xterm-256color` with truecolor, scrollback, copy and paste, configurable font size, reconnect bar. Terminal and files tabs share one connection.
- Remote editing: open a remote file in your editor, upload on save, with a conflict strip if the file changed on the server.
- Command palette (Ctrl+K) to jump to sites, workspaces and actions.
- Technical log view and `RUST_LOG` support.
- Idle-drop handling: connections that the server closes while idle are reconnected on next use.
- Two queue items never write the same destination file at once; the second waits and then goes through the normal conflict check.
- `hoprs --version` and `--help`; desktop entry, AppStream metadata and app icon for Linux.
