<p align="center">
  <img src="docs/images/banner.png" width="100%" alt="hoprs: SSH, SFTP and FTP in one window. A fast, native desktop client for Linux, macOS and Windows, written in Rust. The banner shows the logo and the app with a terminal tab and a files tab, two files being dragged to the server.">
</p>

<p align="center">
  A fast, native SSH, SFTP, FTP and FTPS desktop client for Linux, macOS and Windows, written in Rust.
</p>

<p align="center">
  <a href="https://github.com/engviniciusbs/hoprs/releases/latest"><img src="https://img.shields.io/github/v/release/engviniciusbs/hoprs?label=release&color=f5f5f5&labelColor=141414" alt="Latest release"></a>
  <a href="https://github.com/engviniciusbs/hoprs/actions/workflows/ci.yml"><img src="https://github.com/engviniciusbs/hoprs/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI"></a>
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-f5f5f5?labelColor=141414" alt="License: MIT OR Apache-2.0"></a>
  <img src="https://img.shields.io/badge/platforms-Linux%20%7C%20macOS%20%7C%20Windows-f5f5f5?labelColor=141414" alt="Platforms: Linux, macOS, Windows">
</p>

<p align="center">
  <a href="https://hoprs.online">Website</a> ·
  <a href="https://github.com/engviniciusbs/hoprs/releases/latest">Download</a> ·
  <a href="#first-steps">First steps</a> ·
  <a href="CHANGELOG.md">Changelog</a> ·
  <a href="CONTRIBUTING.md">Contributing</a>
</p>

hoprs keeps the idea that made FileZilla useful (a list of saved sites you open with one
click) and puts a terminal, a two-pane file browser, folder compare and a persistent
transfer queue behind it, in one quiet window that opens in under 100 ms and idles at about
33 MB.

- **Version:** 1.5.0 ([changelog](CHANGELOG.md))
- **Platforms:** Linux (Ubuntu 24.04 or newer, amd64), macOS 11 or newer (Apple Silicon and Intel), Windows 10 or newer (x64).
- **Languages:** English, Brazilian Portuguese and Turkish.
- **License:** MIT OR Apache-2.0

## Contents

- [Features](#features)
- [Screenshots](#screenshots)
- [Install](#install)
- [First steps](#first-steps)
- [Using hoprs](#using-hoprs)
- [Where your data lives](#where-your-data-lives)
- [Configuration](#configuration)
- [System requirements](#system-requirements)
- [Building from source](#building-from-source)
- [Architecture](#architecture)
- [Testing](#testing)
- [Performance](#performance)
- [Releasing](#releasing)
- [Troubleshooting](#troubleshooting)
- [Contributing](#contributing)
- [Security](#security)
- [License](#license)

## Features

- **Saved sites with a protocol each:** `ssh` (terminal, files over SFTP, or both, over one
  connection), `ftp`, `ftps-explicit` and `ftps-implicit`. Plain FTP is allowed and labelled
  `ftp (unencrypted)`.
- **Workspaces:** a workspace groups sites in the sidebar. Open its sites one by one, or
  all at once (right-click, Open all), which also restores their tabs. A site can belong to
  several workspaces.
- **Terminal:** `xterm-256color` with truecolor, 10 000 lines of scrollback by default,
  find in scrollback, copy and paste, and a reconnect bar when the connection drops.
- **Two-pane file browser:** local and remote side by side, virtualised lists that stay
  smooth at 100 000 entries, rename, new folder, delete, upload and download.
- **Right-click menus and a visible drag:** every file, tab, site and queue row has a menu
  with what can be done to it, and dragging files between the panes shows what is moving
  and where it will land.
- **Folder compare:** a panel lists what is new, changed or missing on each side of the two
  open folders, and uploads or downloads only the difference. Nothing is deleted.
- **Transfer queue:** persistent (SQLite), parallel, resumes after a dropped connection or
  a restart, checks size and time, and handles conflicts per item or for the whole batch
  (ask, overwrite, overwrite if newer, skip, resume). Two items never write the same
  destination at once.
- **Remote editing:** open a remote file in your editor; every save is uploaded, and if the
  file changed on the server you choose what happens instead of losing either side.
- **Credentials in the system keyring** (Secret Service, macOS Keychain, Windows Credential Manager). Site files hold no secrets.
  `ssh-agent` is supported and preferred.
- **Encrypted backup:** export everything, one workspace or one site, and restore it, in one
  encrypted file with the passwords (Argon2id and XChaCha20-Poly1305).
- **FileZilla import:** reads `sitemanager.xml` and reports what was imported and what was
  skipped.
- **Security prompts inline:** host key and certificate changes are shown in place, never as
  a cascade of modal dialogs. `known_hosts` is verified; FTPS certificates can be pinned.
- **Connections that survive:** keepalive, automatic reconnection with backoff, and recovery
  from connections that servers drop while idle.
- **Language:** English, Brazilian Portuguese or Turkish. hoprs follows the OS language by default;
  Settings › Language switches it at once.
- **Command palette:** Ctrl+K (Cmd+K on macOS) jumps to any tab, site, workspace or action.
- **Agents (MCP):** Claude Code and other coding agents can work on the sites you allow, one
  by one, at a level you choose (reading, or everything), through `hoprs mcp`. hoprs keeps the
  passwords and shows what the agent does. Off by default; see [Agents (MCP)](#agents-mcp).
- **Update notice:** hoprs tells you when a new version is out and, with one click,
  downloads it, checks it against the release's `SHA256SUMS` and opens the system
  installer. It can be turned off; see [Updates](#updates).
- **Technical log:** a log panel and `RUST_LOG` support, with no passwords, file contents or
  terminal bytes in it.

## Screenshots

![hoprs: a saved SSH site open with the local and remote folders side by side, file kind icons and a selected row, the folder compare panel listing the differences, and an upload running in the transfer queue](docs/images/files.png)

| | |
|---|---|
| ![The folder compare panel: files only on the server and files that differ, two of them selected, with Upload selected and Download selected enabled](docs/images/compare.png) | ![An SSH terminal tab after ls -la, sharing its connection with the files tab, while an upload keeps running in the queue](docs/images/terminal.png) |
| **Folder compare.** See what differs, pick, then upload or download. | **Terminal.** SSH shell and files over one connection. |
| ![The Ctrl+K palette open with "new" typed, listing actions such as New files tab and New folder](docs/images/palette.png) | ![The same files tab with the interface in Brazilian Portuguese](docs/images/pt-br.png) |
| **Go to anything.** Ctrl+K jumps to tabs, sites, workspaces and actions. | **Em português.** The whole interface in Brazilian Portuguese. |

These are real screenshots of hoprs 1.3.0 against the Docker test servers, taken by
[`scripts/readme-shots.sh`](scripts/readme-shots.sh).

## Install

Every release has these files on the
[latest release](https://github.com/engviniciusbs/hoprs/releases/latest) page, plus
`SHA256SUMS` for all of them:

| System | File |
|---|---|
| Linux (Ubuntu 24.04 or newer, amd64) | `hoprs_1.5.0-1_amd64.deb` |
| macOS 11 or newer, Apple Silicon and Intel | `hoprs_1.5.0_macos_universal.dmg` |
| Windows 10 or newer, x64 (installer, per machine) | `hoprs_1.5.0_windows_x64.msi` |
| Windows 10 or newer, x64 (portable) | `hoprs_1.5.0_windows_x64.zip` |

The macOS and Windows builds are **not signed** with a paid certificate (see the steps
below), so each system warns once on first launch.

### Linux: the `.deb` (recommended)

1. Download `hoprs_1.5.0-1_amd64.deb` and `SHA256SUMS`.
2. Check the download and install it:

   ```sh
   sha256sum -c SHA256SUMS --ignore-missing
   sudo apt install ./hoprs_1.5.0-1_amd64.deb
   ```

   Expected output of the first command: `hoprs_1.5.0-1_amd64.deb: OK`. `apt` installs the
   libraries hoprs needs (Wayland, X11, Vulkan/EGL loaders, D-Bus).

3. Start hoprs from the app menu, or run:

   ```sh
   hoprs             # opens the window
   hoprs --version   # prints "hoprs 1.5.0"
   hoprs --help
   ```

Only one instance runs at a time; starting a second one prints `hoprs is already running`
and exits.

### macOS: the `.dmg`

1. Download `hoprs_1.5.0_macos_universal.dmg` and `SHA256SUMS`, and check it:

   ```sh
   shasum -a 256 -c SHA256SUMS --ignore-missing
   ```

2. Open the `.dmg` and drag hoprs to Applications.
3. Open hoprs once. macOS refuses, because the app is ad-hoc signed only, not notarised.
   Go to System Settings, Privacy & Security, and click "Open Anyway". Or, in Terminal:

   ```sh
   xattr -dr com.apple.quarantine /Applications/hoprs.app
   ```

   You do this once per download.
4. **Keychain:** hoprs keeps passwords in the macOS Keychain. After each update macOS asks
   for Keychain access again (an ad-hoc signature changes with every build); choose
   "Always Allow".
5. **Known issue:** quit with the window's close button. Cmd+Q and Dock, Quit skip the
   unsent-edits prompt and do not save open tabs.

The `.dmg` is one universal build for Apple Silicon and Intel Macs.

### Windows: the `.msi` or the `.zip`

1. Download `hoprs_1.5.0_windows_x64.msi` (or the portable `hoprs_1.5.0_windows_x64.zip`)
   and `SHA256SUMS`. In PowerShell, compare the hash with the line in `SHA256SUMS`:

   ```powershell
   (Get-FileHash .\hoprs_1.5.0_windows_x64.msi -Algorithm SHA256).Hash
   ```

2. Run the `.msi`. It installs per machine (it asks for administrator rights) and adds a
   Start menu entry and a desktop shortcut. On "Windows protected your PC" (SmartScreen),
   choose "More info", then "Run anyway".
3. The `.zip` holds `hoprs.exe`, the licences and a `README.txt`; unzip it anywhere and run
   `hoprs.exe`. SmartScreen may warn the same way.

`hoprs.exe` links the C runtime statically, so the Visual C++ redistributable is not needed.
`hoprs --version` and `hoprs --help` print into the cmd or PowerShell window you run them in.

### Updating

hoprs tells you when a new version is out (see [Updates](#updates) below). You can always
update by hand: quit hoprs, download the new file from the
[latest release](https://github.com/engviniciusbs/hoprs/releases/latest) and install it over
the old one:

| System | How |
|---|---|
| Linux | `sudo apt install ./hoprs_<version>-1_amd64.deb` (replaces the installed version) |
| macOS | Open the new `.dmg` and drag hoprs to Applications; choose Replace |
| Windows, `.msi` | Run the new `.msi`; it replaces the installed version |
| Windows, `.zip` | Unzip the new `.zip` over the old folder |

### Updates

hoprs looks at GitHub's latest release once, a few seconds after it starts, and again every
24 hours. If a newer version exists, a card appears in the sidebar (once per version; a line
above "Import from FileZilla" keeps the offer after you dismiss it).

- **What is sent:** a normal HTTPS request to the GitHub API with `User-Agent: hoprs/<version>`.
  No identifier, no account, nothing about your sites. The proxy environment variables
  (`ALL_PROXY`, `HTTPS_PROXY`, `HTTP_PROXY`) are honoured; the desktop's proxy settings are not
  read.
- **Turn it off:** Settings → General → "Check for updates automatically". **Check now** (same
  place) and Ctrl+K → "Update hoprs to …" work either way.
- **What Update does:** it downloads the file for your system, checks it against the release's
  `SHA256SUMS` and hands it to the system installer.

  | System | Result |
  |---|---|
  | Linux (`.deb`) | Opens with the default app for `.deb` files (App Center); the card also shows the `sudo apt install` command to copy |
  | macOS (`.dmg`) | Opens the disk image; close the hoprs window, then drag hoprs to Applications and choose Replace |
  | Windows (`.msi`) | After a second click ("Close and install"), hoprs closes and the installer runs |
  | Windows `.zip`, source builds | Opens the release page |

- **What the check proves:** the checksum catches a corrupted or truncated download. It does
  not prove who published the release: `SHA256SUMS` comes from the same release, so the trust
  is the same as downloading from the site. Signing `SHA256SUMS` is on the
  [roadmap](docs/ROADMAP.md). A file saved by hoprs carries no browser download mark
  (Windows Mark-of-the-Web, macOS quarantine), so SmartScreen and Gatekeeper may not ask
  before it runs.

Your sites, workspaces, settings, transfer queue and saved passwords stay where they are:
they live in your user folders and the system keyring, not next to the program (see
[Where your data lives](#where-your-data-lives)). On macOS, steps 3 and 4 above apply to each
new download from the browser ("Open Anyway" once, then Keychain access again).

### Uninstall

| System | How | Your data |
|---|---|---|
| Linux | `sudo apt remove hoprs` (or `purge`) | Kept |
| macOS | Drag hoprs from Applications to the Bin | Kept |
| Windows | Settings, Apps, hoprs, Uninstall (the `.zip`: delete the folder) | Kept |

Uninstalling never touches your data or your saved passwords (see
[Where your data lives](#where-your-data-lives)). To remove them too, delete the folders
listed there, and the keyring entries for service `hoprs/<site-id>` (up to two per site, a
password and a key passphrase): in Seahorse ("Passwords and Keys") on Linux, Keychain Access
on macOS. On Windows, Credential Manager shows them as `password.hoprs/<site-id>` and
`key-passphrase.hoprs/<site-id>`.

## First steps

1. **Add a site.** Click "New site", pick the protocol, fill in host, port and user, and
   choose how to authenticate: `ssh-agent`, a key file (with an optional passphrase), a
   password, or keyboard-interactive. For SSH sites, "Open as" chooses files, terminal or
   both.
2. **Or import from FileZilla.** "Import from FileZilla" reads `sitemanager.xml` (usually
   `~/.config/filezilla/sitemanager.xml`; `~/.config/filezilla` on macOS too,
   `%APPDATA%\FileZilla\sitemanager.xml` on Windows) and shows a report of what came in and what was
   skipped and why.
3. **Group sites into a workspace.** A workspace is a group: click it to
   show its sites. Right-click it and choose "Open all" to connect every site and restore the
   tabs you had ("Close all" closes them again).
4. **Back everything up.** "Export backup…" writes one encrypted file with your sites,
   workspaces and passwords, protected by a password you choose (without it the backup
   cannot be opened). "Restore backup…" on another machine brings all three back. Restore
   offers two modes: Merge (the default) adds what is new and keeps what you have; Replace
   makes everything the backup's, removing sites, workspaces and passwords not in it, and
   hoprs asks before doing it. To export only one site or one workspace, see
   [Backup and export](#backup-and-export).

The first time hoprs meets an SSH host it shows the host key fingerprint inline and asks
before trusting it; a changed key is shown as a warning, never accepted silently.

## Using hoprs

### Terminal

An SSH site opens as files, terminal or both. Its terminal and files tabs share one
connection, and "New terminal tab" or "New files tab" adds more over the same connection.
Font size (10 to 24 px) and scrollback (1 000 to 100 000 lines) are in Settings. When the
connection drops, a bar offers "Reconnect" and a new shell opens.

The server cannot write to your clipboard (OSC 52 is off).

### Compare folders

The Compare button in a files tab (or "Compare folders" in the Ctrl+K palette) opens a
panel between the two panes that walks the local and the remote folder you have open and
lists what differs, grouped as "Only on this computer", "Only on the server", "Different"
(size or modification time; the tooltip shows both times) and "Could not read" (a
subfolder that could not be listed; the rest is still compared). A folder that exists on
one side only is one row and is not walked.

- **Automatic:** with "Compare automatically" on (the panel's checkbox, or Settings ›
  Folder compare), changing folders in either pane starts a new comparison after a short
  pause. Turn it off to compare only when you press Compare. File operations (rename, new
  folder, delete, permissions) do not re-run the comparison; a folder change or an explicit
  refresh does. The comparison uses the tab's connection; it never opens its own.
- **Ignore list:** `.git, node_modules, .DS_Store` by default, editable in Settings. Names
  are matched at any depth on both sides, and `*` matches any run of characters (`*.log`).
- **Limit:** a side with more than 10 000 entries pauses the walk and shows what was found
  so far, marked as partial; "Compare everything" continues without the limit.
- **Transfers:** select rows and press Upload selected or Download selected. Each file goes
  into the queue with the overwrite policy, so it replaces the file on the other side
  (you chose it after seeing the difference). Nothing is deleted, ever, and nothing moves
  without a click. Double-click a row to take both panes to its folder. The panel is
  keyboard-friendly: arrows move, Space toggles, Enter opens.
- **Panel size:** drag the top edge of the compare panel (or of the transfer queue) to
  resize it; a double-click on the edge resets it, and the heights are remembered. The
  caret collapses the compare panel to one line with the counts.
- **Servers that cannot set times:** a file hoprs just transferred is shown as identical
  for the rest of the tab's life when its sizes match, even if the server kept an old time.
  FTP listings without `MLSD` only have minute precision, so times within a minute count
  as equal.

### Transfers

![Two local files dragged over the remote pane: a chip with the first name and a "2" badge follows the pointer, the dragged rows are dimmed, and the remote pane has a dashed outline and the label "Upload 2 items to /home/alice/web-app"](docs/images/drag.png)

Select files in either pane and press Ctrl+Enter, or drag them across. Every transfer goes
into the queue at the bottom, which survives restarts: close hoprs in the middle of an
upload and it resumes where it stopped. When the destination already exists, hoprs asks
what to do (ask, overwrite, overwrite if newer, skip, resume), for that item or for the rest
of the batch.

A drag starts after the pointer moves a few pixels. A chip with the first item's name (and
"+N" for more) follows the pointer, the dragged rows dim, and the pane that will receive
them gets a dashed outline and says what a drop does ("Upload 2 items to /the/folder").
Escape or leaving the window cancels the drag.

### Right-click menus

![The right-click menu open on a remote file: Download, Edit, Rename, Permissions…, Copy path, Refresh, New folder and Delete, each with its icon and key](docs/images/menu.png)

Right-click a file or folder for its actions: Upload or Download, Edit (remote files), Open
(folders), Rename, Permissions… (remote), Copy path, Refresh (F5), New folder and Delete.
Items that do not apply are dimmed, not hidden. Right-clicking below the rows gives New
folder, Refresh and Compare. Shift+F10 or the Menu key opens the menu of the selected row
from the keyboard; arrows move, Enter runs, Escape closes. The compare list (Upload,
Download, Show in panes), the queue rows (Pause, Resume, Cancel, Retry, Dismiss), the
sidebar's sites and workspaces (Open, Edit, Export…) and the tabs (Close tab, Close other tabs) have
menus too.

### Backup and export

An export is one encrypted file, protected by a password you type twice (without it the
file cannot be opened). It holds one of three things:

- **Everything:** every site, every workspace and the saved passwords.
- **One workspace:** the workspace, with its saved tabs, and the sites in it, with their
  passwords.
- **One site:** that site and its password.

Where to start:

- Right-click a site or a workspace in the sidebar and choose "Export…".
- The sidebar's ⋯ menu and the Ctrl+K palette have "Export everything…".
- When you edit a site or a workspace, its form has an "Export…" button: this is the way in
  from the keyboard. It exports the site or workspace as saved, and leaves the form (edits
  you have not saved are dropped).
- Settings → Backup has "Export backup…" (everything); "Restore backup…" stays there.

The export screen is a form in the middle of the window. Its title says what is exported,
the File field starts as `hoprs-<name>-<date>.hoprs` in your Documents folder (or home), and
"Choose…" opens the save dialog. Everything is `hoprs-backup-<date>.hoprs`. If you type the
path of a file that exists, hoprs asks before overwriting it.

A file of one site or one workspace is a partial file. Restore it with Merge: Replace is
refused, because it would remove everything the file does not hold. Partial files need
hoprs 1.6.0 or newer; hoprs 1.5.0 and older answer "Unsupported backup version" and change
nothing. A file of everything restores on every version, as before.

### Agents (MCP)

Coding agents such as Claude Code can use the sites you saved in hoprs: list folders, read
and write files, transfer, and run commands, without their own credentials. hoprs does the
work, keeps the passwords, and shows what the agent does.

It works through `hoprs mcp`, a small program the MCP client starts (the same executable as
the app, started with the argument `mcp`). It talks to the **running** hoprs over a channel
that stays on your computer, so hoprs must be open. It is off until you turn it on, and
every site starts at "No access".

**Set it up**

1. In hoprs, turn on Settings › Agents (MCP) › "Allow agents to connect to hoprs".
2. Copy the command that section shows and run it in a terminal. For Claude Code it looks
   like this, with the full path of the hoprs you are running (on macOS and Windows hoprs is
   not on `PATH`, so the path matters; Windows paths look like
   `C:\Program Files\hoprs\hoprs.exe`):

   ```
   claude mcp add hoprs -- "/usr/bin/hoprs" mcp
   ```

   Other MCP clients take the JSON snippet in the same section: for example `{"command":"/usr/bin/hoprs","args":["mcp"]}` (the section shows yours, with your path).
3. Edit a site and choose its level under "Agents (MCP)".

**The levels** (per site, in the site's form):

| | No access | Allow reading | Allow everything |
|---|---|---|---|
| The agent sees the site and can tell it exists | no | yes | yes |
| List folders, describe a path, read a text file, download | no | yes | yes |
| Write a file, make a folder, rename, change permissions, upload, delete | no | no | yes |
| Run a command | no | no | SSH sites only |

`download` needs only "Allow reading", and it writes on this computer: it creates files, and
with `overwrite` it replaces an existing one. The level protects the server; it does not
confine the agent on this computer (see the protected paths below).

"No access" is the default for every site, existing ones included. A site with no access is
not listed, and an agent that asks for it gets the same answer as for a site that does not
exist. A change of level takes effect on the next call. A backup, a partial export or the
FileZilla import never carries a level: a restored or imported site is "No access", and you
give access only by hand, in the site form.

**What an agent can do.** Thirteen tools: `list_sites`, `list_directory`, `stat`,
`read_file`, `download`, `transfer_status` (reading level); `write_file`, `make_directory`,
`rename`, `set_permissions`, `upload`, `delete` and `run_command` (everything level).
Listings come in pages of up to 5 000 entries, `read_file` and `write_file` move at most
1 MiB a call (use `download` and `upload` for more, or for files that are not text), and a
command stops after 60 seconds unless the agent asks for up to 600. `download` and `upload`
take a file or a folder through the transfer queue and never ask what to do with an existing
file: the agent says `overwrite` or the file is skipped. A call that needs a connection
connects the site, with no tab, no focus change and no sound; it closes after 10 minutes
without a call (never while it has an unfinished transfer).

**What an agent cannot do**

- See, or touch, a site with "No access", or learn that it exists.
- See a password, a passphrase, a key file or the path of one.
- Answer a host-key, certificate or password question. These appear in the hoprs window,
  marked "An agent asked for this connection."; the agent waits up to two minutes, then
  hears that the user has to answer. A changed host key is always an error for the agent.
- Change its own access, manage sites or workspaces, change settings, or export backups.
- Transfer to or from hoprs's own files and folders (the program itself included), `~/.ssh`,
  the `known_hosts` file hoprs uses, or any key file a saved site names. This holds for a
  file, for a folder that contains one, and through symbolic links. Every path an agent
  gives must be absolute and contain no `..`.

  Nothing else on this computer is protected. A `download` (at "Allow reading") may write
  into any other place you can write, including files that run later: a shell start-up
  file, an autostart entry, the settings of the agent's own client. An agent that is misled
  could use that to run code as you. Give access only to agents you trust on this machine,
  and keep the MCP client's own permission prompts on for hoprs's tools. The protected paths
  cover only the local side of transfers: a saved site that points at this computer (SSH to
  `localhost`) gives an agent with "Allow everything" the remote-side tools on this
  machine's own files, `~/.ssh` and hoprs's files included.
- Open an interactive terminal.

**What you see.** "Agent connected" in the status bar while an agent is connected (the
tooltip counts them). It appears with the agent's first call, when `hoprs mcp` connects to
hoprs, not when the MCP client starts it. A small "agent" tag on its transfers in the queue
(you can pause or cancel them there); "Connected (by an agent)" on its sites in the
sidebar; and one line per action in the log, from `agent`, for example
``ran `systemctl status nginx` on "Acme" (exit 0)``. The log holds the command line and the paths, never file content or command
output. Turning the switch off closes the channel and the agents' sessions at once, except a
session an unfinished transfer still uses (it stays until the transfer ends; a paused
transfer keeps it), and the agent's next calls say so.

**Read this before you choose "Allow everything".** An agent can be misled by what it reads:
a file, a command's output. hoprs does not review what the agent does, and it does not ask
before each action. The level is the safeguard: keep production sites on "Allow reading"
unless the agent really must change them. Anything running as your user on this computer can
use the channel while the switch is on (see [SECURITY.md](SECURITY.md)); that is why the
switch starts off.

**Known limits**

- hoprs must be open. The agent's calls fail with a message that says so, and work again as
  soon as hoprs is open and the switch is on; the agent need not be restarted.
- `run_command` is for SSH sites. There is no terminal and no input. If a command passes its
  time limit, hoprs sends it a KILL signal and says so, but it may keep running on the server.
- `read_file` refuses a file whose last path component is a symbolic link; `download` it.
- On Windows the agent's local paths must be on the drive hoprs's local pane reaches (the
  current drive); other drives and network paths are refused with a message that says so.
- On some FTP servers a link to a folder is listed as a folder, so a recursive `delete` goes
  into it.
- A transfer queue written by this version is refused by hoprs 1.5.0 and older.

### Settings

![The Settings tab on its Transfers section: workers per site, what to do when a file already exists, and the keepalive interval](docs/images/settings.png)

Settings open as a tab: Ctrl+, (⌘, on macOS), the gear at the bottom of the sidebar, or
"Settings" in the Ctrl+K palette. There is only ever one; pressing the shortcut again
focuses it. A list on the left switches between General, Transfers, Folder compare, Remote
editing, Terminal, Agents (MCP) and Backup (export and restore). Closing the tab returns to the tab you came from.

### Remote editing

Select a remote file and press F4 (or "Edit"). hoprs downloads it to
`$XDG_RUNTIME_DIR/hoprs/edit/` (a temporary folder on macOS and Windows), opens it with the editor set in Settings › Remote editing
(default `system`: `xdg-open` on Linux, `open -t` on macOS, the file's associated app on
Windows, with the Open With dialog if it has none; or your own command such as `gedit` or
`code --wait`, with the file's path added last), and uploads it every time you save, after 500 ms of quiet. If the file changed on the server
since you opened it, nothing is overwritten: a strip offers Overwrite, Keep remote or Save a
copy. Closing a tab or the window with unsent changes asks first, and unsent changes are kept
in the `recovered` folder of your data directory.

On Windows a custom editor must be a program that opens a window and can wait: use
`code.cmd --wait` or a full path. Console editors such as `vim.exe` do not work.

### Keyboard shortcuts

| Key | Where | Does |
|---|---|---|
| Ctrl+K (Cmd+K on macOS) | anywhere (Ctrl+Shift+K in a focused terminal, on macOS too) | Go to anything: tabs, sites, workspaces, actions |
| Ctrl+Shift+C / Ctrl+Shift+V | terminal | Copy the selection / paste |
| Ctrl+Shift+F | terminal | Find in scrollback |
| F2 | file pane | Rename |
| F4 | remote pane | Edit in your editor |
| F7 | file pane | New folder |
| Del | file pane | Delete |
| Ctrl+Enter | file pane | Upload or download the selection |
| F5 | file pane | Refresh |
| Shift+F10 or the Menu key | file pane, compare list | Open the selected row's menu |
| Ctrl+, (Cmd+, on macOS) | anywhere, a focused terminal too | Settings |
| Tab | files tab | Switch pane |

Terminal shortcuts use Ctrl+Shift on every system, macOS included. Plain Ctrl+K in a focused terminal goes to the shell (in bash it kills the rest of the
line).

### Log

The Log button in the status bar opens the technical log: connections, retries, transfer
decisions. It never contains passwords, passphrases, file contents or terminal output.

### Language

The interface is in English, Brazilian Portuguese and Turkish. "System" (the default)
follows the OS language: any Portuguese locale gives Portuguese, any Turkish locale Turkish,
anything else English. Pick one in
Settings › Language (Idioma); it applies immediately. Messages that come from servers or
the OS (for example an SFTP server's own error text) are shown as they are.

Translators: the texts are Fluent files in `crates/hoprs-app/i18n/`. `en.ftl` is the
reference: copy it, translate the values, and keep the `{ $var }` names unchanged. The
tests check that every catalog has the same keys and variables as `en.ftl` (`cargo test -p hoprs-app`).

## Where your data lives

| What | Linux | macOS | Windows |
|---|---|---|---|
| Config: `sites.toml` (no secrets), `workspaces.toml`, `settings.toml` | `~/.config/hoprs/` | `~/Library/Application Support/hoprs/` | `%APPDATA%\hoprs\` |
| Transfer queue `queue.db` (SQLite), `recovered/` (remote edits that could not be uploaded) | `~/.local/share/hoprs/` | `~/Library/Application Support/hoprs/` | `%APPDATA%\hoprs\` |
| Working copies of files being edited | `$XDG_RUNTIME_DIR/hoprs/edit/` (cleared at logout) | a temporary folder | a temporary folder |
| `agent.sock`, the channel for agents (only while Settings › Agents (MCP) is on and hoprs runs; the pipe `\\.\pipe\hoprs-agent-<your SID>` on Windows) | beside `instance.lock` (normally `$XDG_RUNTIME_DIR/hoprs/`; if that is unusable, `~/.config/hoprs/`, then the temporary folder) | `~/Library/Application Support/hoprs/` | none: a named pipe |
| Passwords and key passphrases, service `hoprs/<site-id>` | Secret Service (GNOME Keyring, KWallet) | macOS Keychain | Windows Credential Manager |

The TOML files are plain text and safe to keep in a dotfiles repository: they never hold a
password or passphrase. Two keys belong to agents: `agents_enabled = true` in `settings.toml` (written only
when on) and `agent_access = "read"` or `"all"` on a site in `sites.toml` (absent means no access).

## Configuration

Most settings live in the Settings screen. A few environment variables change behaviour:

| Variable | Effect | Default |
|---|---|---|
| `RUST_LOG` | Log filter, in [`EnvFilter`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html) syntax (`info`, `hoprs_ssh=debug`, ...). Terminal crates are capped at `warn` so terminal bytes never reach the log | `info` (noisy dependencies at `warn`) |
| `ICED_BACKEND` | Renderer: `wgpu` (GPU) or `tiny-skia` (software) | chosen at start-up |
| `SSH_AUTH_SOCK` | The `ssh-agent` socket used by sites with "Use ssh-agent". On Windows, hoprs uses the OpenSSH agent pipe `\\.\pipe\openssh-ssh-agent` (an MSYS or Git for Windows `SSH_AUTH_SOCK` falls back to it); Pageant is not supported | set by your session |

**Renderer (Linux).** At start-up hoprs looks for a GPU render node under `/dev/dri`. With none, or
with only software drivers (as in many VMs), it draws with tiny-skia, which keeps memory at
about 32 MB instead of the 166 MB that llvmpipe takes. Set `ICED_BACKEND` to force either.

## System requirements

**Linux** at run time:

- **A Wayland or X11 session.** With a GPU, iced draws through wgpu (Vulkan or OpenGL; Mesa
  is enough); without one it falls back to the software renderer.
- **The Secret Service and a keyring daemon** (GNOME Keyring or KWallet) to save passwords
  and key passphrases. Without one, hoprs still connects but asks for the password every
  time.
- **The XDG desktop portal** (`xdg-desktop-portal` with a GNOME/GTK or KDE backend) for the
  "Choose…" file dialogs. Without a portal they fall back to `zenity` (untested).
- **`ssh-agent`** (optional).
- **A text editor** for remote editing; `xdg-open` (from `xdg-utils`) by default.

**macOS:** 11 (Big Sur) or newer, Apple Silicon or Intel. The Keychain stores passwords;
`ssh-agent` is read from `SSH_AUTH_SOCK` (launchd's agent works).

**Windows:** 10 or newer, x64 (ARM64 is not built). The Credential Manager stores passwords.
For `ssh-agent`, start the "OpenSSH Authentication Agent" service. No other runtime is
needed.

## Building from source

### Prerequisites

- **Rust 1.99.1.** `rust-toolchain.toml` pins it; with [rustup](https://rustup.rs) installed,
  the right toolchain (with `rustfmt` and `clippy`) is fetched on the first `cargo` command.
- **A C toolchain and CMake** for the crypto dependencies:
  - Ubuntu / Debian: `sudo apt install build-essential cmake pkg-config`
  - macOS: `xcode-select --install` and `brew install cmake`
  - Windows: Visual Studio Build Tools ("Desktop development with C++"),
    [CMake](https://cmake.org) and [NASM](https://www.nasm.us) on `PATH`

- **Docker** (optional) for the integration tests and for building the `.deb`.

The MCP server for agents uses [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk), the
official Rust SDK, for the protocol on stdio. It adds exactly ten packages to `Cargo.lock` (`rmcp`, `schemars`, `schemars_derive`,
`serde_derive_internals`, `dyn-clone`, `ref-cast`, `ref-cast-impl`, `pastey`,
`unicode-general-category`, `unicode-normalization`) and needs nothing to install: no TLS, no HTTP, no OpenSSL.

### Build and run

```sh
git clone https://github.com/engviniciusbs/hoprs.git
cd hoprs
cargo run --release        # builds and starts the `hoprs` binary (crate hoprs-app)
```

The binary ends up in `target/release/hoprs`.

### Build the `.deb`

```sh
scripts/package-deb.sh                                              # needs Docker
scripts/check-deb.sh target/deb-ubuntu-24.04/debian/hoprs_1.5.0-1_amd64.deb
```

`package-deb.sh` builds inside an Ubuntu 24.04 container pinned by digest, with
`cargo-deb`, `--locked` and the pinned toolchain, so the package only needs glibc 2.39.
`check-deb.sh` installs the result in a clean Ubuntu 24.04 container and checks the file
list and permissions, that every loaded library comes from `Depends`, that the app starts
and idles under 80 MB, and that removing and purging the package keep user data.

### Scripts

| Script | Does |
|---|---|
| `scripts/package-deb.sh` | Builds the `.deb` in a container (`--host` builds on this machine) |
| `scripts/check-deb.sh <deb>` | Installs and checks a `.deb` in a clean container |
| `scripts/icons.sh` | Renders the PNG icons (16 to 512 px) from the SVG |
| `scripts/perf.sh` | Runs every performance check (about 45 min) |
| `scripts/package-macos.sh`, `scripts/check-macos.sh` | Build and check the universal `.dmg` (on macOS) |
| `scripts/package-windows.sh`, `scripts/check-windows.ps1` | Build and check the `.msi` and `.zip` (on Windows) |
| `scripts/release-notes.sh` | Prints a version's section of the changelog |
| `scripts/workspace-version.sh` | Prints the workspace version |
| `scripts/readme-shots.sh` | Retakes the README screenshots from the released `.deb` against the Docker test servers |
| `scripts/readme-banner.sh` | Renders the README header (`docs/images/banner.png`) from `docs/images/banner.html` with headless Chrome |

## Architecture

A Cargo workspace of seven crates. Only `hoprs-app` depends on iced; everything else is
tested without a GUI.

```
crates/
├── hoprs-core/      shared types: Site, Protocol, Workspace, RemotePath, RemoteEntry, Error
├── hoprs-vault/     TOML profiles, keyring, encrypted export and import
├── hoprs-ssh/       SSH session: authentication, PTY, keepalive, known_hosts
├── hoprs-remote/    the RemoteFs trait, with SFTP and FTP/FTPS implementations
├── hoprs-transfer/  transfer engine: queue, workers, resume, conflicts
├── hoprs-import/    FileZilla sitemanager.xml importer
└── hoprs-app/       the iced application: screens, widgets, theme, the `hoprs` binary
```

Dependencies (A → B means A depends on B), and no others:

```
every crate → hoprs-core
hoprs-remote   → hoprs-ssh
hoprs-transfer → hoprs-remote
hoprs-import   → hoprs-vault
hoprs-app      → hoprs-transfer, hoprs-remote, hoprs-ssh, hoprs-vault, hoprs-import
```

One tokio runtime runs all I/O; the UI thread never blocks on it.

| Concern | Library |
|---|---|
| GUI | [iced](https://iced.rs) |
| Async runtime | [tokio](https://tokio.rs) |
| SSH and SFTP | [russh](https://github.com/Eugeny/russh), russh-sftp |
| FTP and FTPS | [suppaftp](https://github.com/veeso/suppaftp) |
| Terminal emulation | [alacritty_terminal](https://github.com/alacritty/alacritty) |
| File watching (remote editing) | [notify](https://github.com/notify-rs/notify) |
| Queue storage | SQLite |
| Keyring | Secret Service |

The full design is in
[docs/superpowers/specs/2026-11-11-hoprs-design.md](docs/superpowers/specs/2026-11-11-hoprs-design.md)
(in Portuguese), and the implementation plans are in
[docs/superpowers/plans/](docs/superpowers/plans/).

## Testing

```sh
cargo test --workspace --all-features
```

Integration tests run against real servers (OpenSSH and vsftpd, including FTPS and servers
that drop idle connections) in Docker:

```sh
docker compose -f docker/compose.yml up -d --build --wait
cargo test --workspace --all-features -- --ignored --test-threads=1
docker compose -f docker/compose.yml -p hoprs-test down -v
```

`crates/hoprs-remote/tests/compare.rs` plants a tree on both servers and checks folder
compare over SFTP and over FTP (`LIST` listings, as vsftpd has no `MLSD`).

The private keys and certificates under `docker/fixtures/` and
`crates/*/tests/fixtures/` exist only for these test servers. They protect nothing.

Every commit must pass:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

CI (`.github/workflows/ci.yml`) runs lint, unit tests, the Docker integration tests, the
benchmarks build and the `.deb` build and check on every pull request and on `main`.

## Performance

The v1 targets and the last numbers from `.github/workflows/perf.yml` (2026-11-13). How each
one is measured: [docs/perf/README.md](docs/perf/README.md).

| Target | Last result |
|---|---|
| Open in under 500 ms | 84 ms |
| Idle memory under 80 MB | 33 MB |
| 100 000 entries scroll at 60 fps (p95 frame under 16 ms) | p95 0.58 ms |
| SFTP 1 GiB at 50 ms RTT, at least 90% of `scp` | 0.99 × `scp` |
| 10 000 small files over FTP, no slower than `lftp` | 1.003 × `lftp` (known gap) |
| 10 000 small files over SFTP, no slower than `lftp` | 2.0 × `lftp` (known gap) |

The perf workflow runs on every push to `main` and weekly. It is never a required check,
because shared runners are noisy.

## Releasing

1. Bump the version in the workspace `Cargo.toml` and add its section to `CHANGELOG.md`.
2. Merge to `main` with CI green.
3. Tag and push:

   ```sh
   git tag -a vX.Y.Z -m "hoprs X.Y.Z"
   git push origin vX.Y.Z
   ```

`.github/workflows/release.yml` checks the tag against the workspace version, builds and
checks the packages on three runners (Linux `.deb`, macOS `.dmg`, Windows `.msi` and `.zip`),
writes `SHA256SUMS`, creates a draft release with the changelog section as notes, downloads
the assets back to verify the checksums, and only then publishes: all or nothing. A tag with
a `-` (`v1.2.0-rc.1`) becomes a pre-release. Running the workflow by hand
(`workflow_dispatch`) is a dry run that builds and checks everything and publishes nothing.

The site at hoprs.online deploys only once `v<version>` is a published, non-prerelease
release, and again after each successful Release run, so its download links never point at
a release that does not exist yet.

## Troubleshooting

**The window is slow or uses a lot of memory in a VM (Linux).** The VM's GPU may be a 2D-only device
(`virtio_gpu`, `vmwgfx`) that wgpu runs through llvmpipe. Start with
`ICED_BACKEND=tiny-skia hoprs`.

**hoprs asks for the password every time.** On Linux no Secret Service answered: install and
unlock a keyring (`gnome-keyring` or KWallet), then save the password again in the site form.
On macOS, click "Always Allow" in the Keychain prompt (it comes back after each update).

**macOS says the app is damaged or from an unidentified developer.** See the steps under
[Install](#macos-the-dmg): "Open Anyway", or the `xattr` command.

**Windows shows "Windows protected your PC".** Choose "More info", then "Run anyway".

**"Choose…" does nothing (Linux).** Install `xdg-desktop-portal` and a backend for your desktop
(`xdg-desktop-portal-gnome` or `xdg-desktop-portal-kde`).

**A connection keeps dropping.** Open the log (status bar, Log) and look at the INFO lines
around the drop; they name the cause (timeout, server close, reconnect attempts). For more,
start hoprs with `RUST_LOG=hoprs_ssh=debug,hoprs_remote=debug`.

**An agent says "hoprs is not reachable".** hoprs is closed, or Settings › Agents (MCP) is off.
Open hoprs and turn the switch on; then the agent's next call works. On Linux, `hoprs mcp`
looks for hoprs's channel under `$XDG_RUNTIME_DIR`: an agent started where that variable is
missing or different (another login session, a service, a container, `sudo`) cannot find
the running hoprs and says the same. Compare `echo $XDG_RUNTIME_DIR` in the agent's terminal
with the one hoprs was started from; start the agent from the same desktop session, or pass
the variable through to it.

**An agent says hoprs and its MCP server are different versions.** The window and the
`hoprs mcp` program are not the same build, and the message says which is older. hoprs was
updated while its window stayed open: quit hoprs and open it again. The `hoprs mcp` program
was started before hoprs was updated (an agent session that stayed open across the update):
restart or reconnect the hoprs MCP server in the MCP client (for example by restarting the
agent session); restarting hoprs does not help then.

**The copied MCP command stops working (macOS).** macOS can run an app opened from the
Downloads folder or from the disk image from a temporary path that changes at each launch,
so the path in the copied command no longer exists. Move hoprs to Applications, open it from
there, and copy the command again.

**The agent cannot start `hoprs` (Windows, macOS).** Register it with the full path to the
executable, as Settings › Agents (MCP) shows it (`C:\Program Files\hoprs\hoprs.exe` on Windows).
`hoprs` alone is not on `PATH` there.

**The host key changed.** hoprs shows the old and new fingerprints and connects only if you
accept the new key. Check with the server's administrator first.

## Contributing

Issues, translations, packaging and pull requests are welcome. Start with
[CONTRIBUTING.md](CONTRIBUTING.md) (setup, checks, translating), the
[architecture guide](docs/ARCHITECTURE.md) and the [roadmap](docs/ROADMAP.md). Questions and
ideas go to [Discussions](https://github.com/engviniciusbs/hoprs/discussions).

## Security

Please report vulnerabilities privately through
[GitHub security advisories](https://github.com/engviniciusbs/hoprs/security/advisories/new)
rather than in a public issue. See [SECURITY.md](SECURITY.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in this work, as defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.
