# hoprs interface text, English. This file is the reference for every translation.
#
# Format (Fluent, https://projectfluent.org):
#   key = text                 one message per key; keys are kebab-case.
#   # comment                  explains a message to translators; it is never shown.
#   { $name }                  a value the app fills in. Keep the variable names exactly
#                              as they are; the order and the words around them may change.
#   Plurals use a selector, one line per form, the line with * is the default:
#       site-count =
#           { $count ->
#               [one] { $count } site
#              *[other] { $count } sites
#           }
#   Keep technical names (SSH, SFTP, FTP, FTPS) as they are.
# pt-BR.ftl and tr.ftl must define the same keys with the same variables.

app-title = hoprs
already-running = hoprs is already running
cannot-open-profiles = Cannot open the saved sites

# A number of saved sites: "1 site", "3 sites".
site-count =
    { $count ->
        [one] { $count } site
       *[other] { $count } sites
    }

# The settings page: the interface language, and its choices. The two language names are
# written in themselves, so they are the same in every catalog.
settings-language = Language
language-system = System
language-english = English
language-pt-br = Português (Brasil)
language-tr = Türkçe

## Top bar

brand = hoprs
no-sessions = No open sessions
go-to-anything = Go to anything
go-to-anything-key = Ctrl K
sidebar-options = Sidebar options
menu-show-workspaces = Show workspaces
menu-show-sites = Show all sites
menu-open = Open
menu-copy-path = Copy path
menu-refresh = Refresh
menu-show-in-panes = Show in panes
menu-open-files = Open files
menu-close-other-tabs = Close other tabs
key-refresh = F5

## The Ctrl+K palette

palette-placeholder = Go to a tab, site or workspace, or run an action
palette-workspaces-placeholder = Open workspace
palette-no-results = No matches
palette-group-action = Action
palette-group-tab = Tab
palette-group-site = Site
palette-group-workspace = Workspace
palette-kind-terminal = terminal
palette-kind-files = files
palette-toggle-queue = Toggle queue
palette-toggle-log = Toggle log
palette-close-tab = Close tab
palette-close-settings = Close settings
palette-open-workspace = Open workspace…
# $name is the open workspace: its tabs are saved, then closed.
palette-close-all = Close all in { $name }
# $version is the newer release (e.g. 1.4.0).
update-palette = Update hoprs to { $version }

# The update card (bottom left), its footer line and Settings → General.
# $version is a release (e.g. 1.4.0).
update-available = hoprs { $version } is available
# $version is the running version.
update-you-have = You have { $version }.
update-action = Update
update-whats-new = What's new
update-dismiss = Dismiss
# $done and $total are sizes (e.g. 12.4 MB).
update-downloading = Downloading… { $done } of { $total }
update-downloading-size = Downloading… { $done }
update-verifying = Checking…
update-try-again = Try again
update-ready-windows = Ready to install. hoprs will close.
update-transfers-stop =
    { $count ->
        [one] 1 transfer will stop; it continues the next time hoprs starts.
       *[other] { $count } transfers will stop; they continue the next time hoprs starts.
    }
update-close-and-install = Close and install
update-later = Later
update-installing = Opening the installer…
update-linux-opened = Opened with the default app for .deb files. Or run:
update-linux-run = Run this command to install:
update-open-failed = The installer did not open.
update-installer-open = The installer is open. Close hoprs to finish.
update-close-hoprs = Close hoprs
update-copy = Copy
update-macos-steps = Close the hoprs window, then drag hoprs to Applications and choose Replace. If macOS blocks it, choose Open Anyway in System Settings, Privacy & Security.
update-footer-available = { $version } available
update-footer-install = Install { $version }
settings-update-check = Check for updates automatically
settings-update-check-hint = Once a day hoprs asks GitHub for the latest release. Nothing about you is sent.
settings-check-now = Check now
settings-update-checking = Checking…
settings-up-to-date = hoprs is up to date
palette-edit = Edit in editor
palette-compare-folders = Compare folders
palette-upload = Upload selection
palette-download = Download selection
palette-copy = Copy
palette-paste = Paste
palette-find = Find in scrollback
palette-key-copy = Ctrl Shift C
palette-key-paste = Ctrl Shift V
palette-key-find = Ctrl Shift F

## Sidebar

filter-sites = Filter sites
workspaces = Workspaces
all-sites = All sites
sidebar-all-hidden = Both lists are hidden. Show them from the ⋯ menu.
sidebar-collapse = Collapse
sidebar-expand = Expand
no-workspaces = No workspaces yet
no-sites = No sites yet
import-filezilla = Import from FileZilla
settings = Settings
key-settings = Ctrl ,
key-settings-macos = ⌘ ,
new-site = New site
no-matches = No matching sites
menu-open-all = Open all
menu-close-all = Close all
# The tooltip of the mark on the open workspace in the sidebar.
workspace-open-tip = Workspace open
edit = Edit
site-missing = Site missing
cannot-load-sites = Can't load the saved sites
retry = Retry
unreadable-entry = Can't read this entry (written by a newer hoprs?)
open-profile-folder = Open sites.toml folder
# Opening the profile folder failed. $reason is the system's explanation.
cannot-open-folder = Can't open the folder: { $reason }
# A workspace's count when some of its sites are missing: "2 of 3 sites". When all are
# present, site-count is used instead.
workspace-site-count-partial =
    { $total ->
        [one] { $present } of { $total } site
       *[other] { $present } of { $total } sites
    }
# An unreadable entry that has no name: $number is its position (from 1), $file the file.
unreadable-name-fallback = Entry { $number } in { $file }

## Site form

site-form-new = New site
site-form-edit = Edit site
field-name = Name
field-name-hint = Defaults to the host
field-host = Host
field-port = Port
field-user = User
field-protocol = Protocol
field-password = Password
password-optional = Optional
password-replace = Type to replace
password-saved = Saved in the system keyring
forget-password = Forget the saved password
ssh-options = SSH
use-agent = Use ssh-agent
key-file = Key file
key-file-hint = Path to a private key (optional)
open-as = Open as
open-as-terminal = Terminal
open-as-files = Files
open-as-both = Terminal and files
ftp-options = FTP
ftp-mode = Transfer mode
ftp-passive = Passive
ftp-active = Active
nat-workaround = Ignore the passive address the server sends (NAT)
start-folders = Start folders
remote-dir = Remote folder
local-dir = Local folder
folder-hint = Optional
agents-section = Agents (MCP)
agent-access-none = No access
agent-access-read = Allow reading
agent-access-all = Allow everything
agent-access-read-hint = An agent can list folders, read files and download them.
agent-access-all-hint-ssh = An agent can also write, upload and delete files, and run commands on the server.
agent-access-all-hint-ftp = An agent can also write, upload and delete files. No commands: only SSH sites run them.
host-required = Host is required
port-invalid = Port must be a number from 1 to 65535
save = Save
saving = Saving…
cancel = Cancel
delete = Delete
keep = Keep
delete-site-confirm = Delete this site? It leaves every workspace and its saved password is deleted.

## Workspace form

workspace-form-new = New workspace
workspace-form-edit = Edit workspace
workspace-sites = Sites
workspace-no-sites = No sites yet. Create a site first.
name-required = Name is required
delete-workspace-confirm = Delete this workspace? Its sites are kept.

## Center

no-session-title = No session open
no-session-hint = Pick a site in the sidebar to connect.
keyring-unavailable = The system keyring was unavailable, so the saved password was not used.
edit-site = Edit site

## Tabs

close-tab = Close tab
# The tab strip's actions on a connected SSH tab: another tab on its session.
new-terminal-tab = New terminal tab
new-files-tab = New files tab
terminal-overflowed = Output was too fast and the shell was closed
terminal-reopen = Reopen
terminal-restart = Restart shell
terminal-shell-closed = Shell closed
terminal-open-failed = Can't open a shell
terminal-retry = Retry
terminal-waiting-for-connection = Waiting for the connection
terminal-input-dropped = Some input was dropped: the shell was not taking it fast enough
# Written into the scrollback when a new shell starts after a reconnect.
terminal-reconnected = — reconnected —
terminal-search-placeholder = Search scrollback
terminal-search-next = Next
terminal-search-previous = Previous
terminal-search-close = Close
terminal-search-no-match = No match
# "Shell exited (code N)": the shell ended and the server sent an exit code.
shell-exited-code = Shell exited (code { $code })
# The shell ended and the server sent no exit code.
shell-exited = Shell exited

## File panes
local = Local
col-name = Name
col-size = Size
col-modified = Modified
col-permissions = Permissions
sort-ascending = ↑
sort-descending = ↓
# The row that goes up one folder.
parent-dir = ..
# After a folder name, and between breadcrumb segments.
path-separator = /
# The breadcrumb's first segment for the home folder.
home-dir = ~
# The size column of a folder.
no-size = -
empty-folder = This folder is empty
# A drag between panes, at the bottom of the pane it would drop on: "Upload 2 items to
# /srv", "Download 1 item to ~/Downloads" (the folder in mono, cut with "…").
drop-upload =
    { $count ->
        [one] Upload { $count } item to { $folder }
       *[other] Upload { $count } items to { $folder }
    }
drop-download =
    { $count ->
        [one] Download { $count } item to { $folder }
       *[other] Download { $count } items to { $folder }
    }
# Files from the system file manager over the remote pane; "-unknown" while the system
# has not said how many.
drop-upload-files =
    { $count ->
        [one] Upload { $count } file to { $folder }
       *[other] Upload { $count } files to { $folder }
    }
drop-upload-files-unknown = Upload files to { $folder }
# The chip that follows the pointer in a drag of several items: "+1" after the first
# name, then the badge with how many go ("2").
drag-more = +{ $more }
drag-count = { $count }
reconnect = Reconnect
# Units of [`crate::ui::format::size`], base 1024.
size-unit-b = B
size-unit-kb = KB
size-unit-mb = MB
size-unit-gb = GB
size-unit-tb = TB
size-unit-pb = PB
size-unit-eb = EB
# Short month names for file dates.
month-1 = Jan
month-2 = Feb
month-3 = Mar
month-4 = Apr
month-5 = May
month-6 = Jun
month-7 = Jul
month-8 = Aug
month-9 = Sep
month-10 = Oct
month-11 = Nov
month-12 = Dec

## Queue
transfers = Transfers
queue-active = Active
queue-queued = Queued
queue-failed = Failed
queue-done = Done
queue-hide = Hide
queue-show = Show
no-transfers = No transfers
no-transfers-hint = Select files and press Upload or Download, drag them to the other pane, or drop them here.
queue-loading = Loading transfers…
nothing-active = Nothing running
nothing-queued = Nothing waiting
nothing-failed = No failures
nothing-done = Nothing finished yet
# Removes done, skipped, cancelled and failed rows (the engine's `clear_finished`).
clear-finished = Clear finished and failed

## Technical log panel
log = Log
log-error = Error
log-warn = Warn
log-info = Info
log-debug = Debug
filter-log = Filter log
# Copies the lines the filters show (all of them without a filter).
copy-shown = Copy shown
clear = Clear
clear-hint = Empties this panel only; new lines keep coming.
log-level-hint = Shows lines at least this severe. Only info and above are recorded unless hoprs starts with RUST_LOG (for example RUST_LOG=debug).
no-log-lines = Nothing logged yet
no-log-matches = No lines match
# The log panel's count: "1 line", "120 lines".
log-lines =
    { $count ->
        [one] { $count } line
       *[other] { $count } lines
    }
waiting-for-worker = Waiting for a worker
starting = Starting…
# A folder whose files are being copied.
transferring = Transferring…
# A failed row the user dismissed (still failed in the engine; Retry stays).
dismissed = Dismissed
paused = Paused
cancelled = Cancelled
transfer-done = Done
file-exists = File exists —
overwrite = Overwrite
resume = Resume
skip = Skip
pause = Pause
# The engine also makes the answer its policy for every later conflict of this run.
apply-to-all = Apply to all (this session)
saved-at = saved { $time }
site-paused = Paused — connection lost
# After the total speed: " total, 4 workers".
total-workers =
    { $workers ->
        [one] total, { $workers } worker
       *[other] total, { $workers } workers
    }
# `policy-*` (lowercase) read inline in sentences; `settings-policy-*` (capitalised) are the settings picker's options.
policy-ask = ask
policy-overwrite = overwrite
policy-skip = skip
policy-overwrite-if-newer = overwrite if newer
policy-resume = resume
# In the queue head once an answer went "to all": "Conflicts: overwrite (this session)".
session-policy = Conflicts: { $name } (this session)
policy-if-newer = if newer
# The same in a narrow window, as a short mono tag: "overwrite: session".
session-policy-tag = { $name }: session
# The queue database could not be opened: the engine runs on an in-memory queue.
queue-unavailable = Transfer queue unavailable: { $reason }. Transfers this session won't survive a restart.
skip-exists = exists
skip-not-newer = not newer
skip-already-complete = already complete
skip-by-user = skipped by user
skip-symlink = symlinks are not followed
# "Skipped: exists".
skipped = Skipped: { $reason }

## Status bar
not-connected = Not connected

## Status bar and connection states
connecting = Connecting…
connected = Connected
waiting-for-you = Waiting for you
closing = Saving tabs and closing…
reconnecting-now = Reconnecting…
# A workspace's saved folder is gone; the pane shows a folder above it instead.
saved-folder-gone = The saved folder { $saved } is gone; showing the nearest folder instead.
# "Reconnecting (2/5)".
reconnecting = Reconnecting ({ $attempt }/{ $of })
# "Disconnected — Connection lost".
disconnected = Disconnected — { $reason }
# How an SSH session logged in: "ed25519 via ssh-agent", "rsa key", "password"...
auth-none = no authentication
auth-agent = { $key_type } via ssh-agent
auth-key-file = { $key_type } key
auth-password = password
auth-keyboard-interactive = keyboard-interactive
# An FTP site's security, in place of a login method.
ftp-auth-ftp = FTP (unencrypted)
ftp-auth-ftps-explicit = FTPS explicit
ftp-auth-ftps-implicit = FTPS implicit
# "3 of 3 sites in Shop".
workspace-status = { $connected } of { $sites } in { $name }

## File actions
new-folder = New folder
rename = Rename
permissions = Permissions…
key-new-folder = F7
key-rename = F2
key-delete = Del
key-edit = F4
upload = Upload
download = Download
key-transfer = Ctrl ↵
# The name a new folder starts with.
new-folder-name = New folder
name-empty = The name can't be empty
name-invalid = The name can't contain / or be . or ..
name-invalid-windows = The name can't contain \ on this disk
name-spaces = The name can't start or end with a space
mixed-permissions = Mixed permissions: showing the first item's
name-taken = Something with this name is already here
dismiss = Dismiss
creating-folder = Creating folder…
renaming = Renaming…
changing-permissions = Changing permissions…
delete-folders-note = Folders are deleted with everything in them.
permissions-title = Permissions
perm-owner = Owner
perm-group = Group
perm-others = Others
perm-read = Read
perm-write = Write
perm-execute = Execute
perm-octal = Octal
octal-invalid = Three or four digits from 0 to 7
apply = Apply
# "Delete report.pdf?", "Delete 3 items?".
delete-question-one = Delete { $name }?
delete-question-many = Delete { $count } items?
# "and 4 more".
and-more = and { $count } more
# "Deleting… 12 items".
deleting =
    { $done ->
        [0] Deleting…
        [1] Deleting… 1 item
       *[other] Deleting… { $done } items
    }
# A delete that stopped after removing some items.
deleted-then =
    { $done ->
        [1] Deleted 1 item, then: { $error }
       *[other] Deleted { $done } items, then: { $error }
    }

## Security and credential dialogs
host-key-title = Unknown host key
host-key-intro = hoprs has not connected to this server before. Check the fingerprint with its administrator before you trust it.
trust-and-connect = Trust and connect
cert-untrusted-title = Untrusted certificate
cert-untrusted-intro = The server's certificate can't be verified. Check the fingerprint with its administrator before you trust it.
cert-changed-title = Certificate changed
cert-changed-intro = The server's certificate is not the one you trusted before. Someone may be intercepting the connection.
trust-certificate = Trust this certificate
label-host = Host
label-account = Account
label-key-type = Key type
label-fingerprint = Fingerprint (SHA-256)
label-reason = Reason
label-trusted-before = Trusted before
label-presented-now = Presented now
label-key-file = Key
password-title = Password
passphrase-title = Key passphrase
field-passphrase = Passphrase
kbd-title = Authentication
save-in-keyring = Save in the system keyring
save-in-keyring-hint = Saved only once the connection succeeds
connect = Connect
unlock = Unlock
continue = Continue
# Under a dialog, when others wait behind it: "1 more waiting".
dialogs-waiting = { $count } more waiting
# The warning of a host known with other key types.
other-key-types = This host is known with a different key type ({ $types }). Someone may be impersonating it.
# Why a certificate needs the user's decision.
cert-reason-expired = Expired
cert-reason-not-yet-valid = Not valid yet
cert-reason-wrong-name = Issued for a different name
cert-reason-unknown-issuer = Unknown authority (self-signed or untrusted issuer)

## Changed host key, in the tab
host-key-changed-title = The host key has changed
host-key-changed-body = The server presented a different key than the one recorded for it. That happens when the server was reinstalled, or when someone intercepts the connection. hoprs won't connect while the old entry is in known_hosts.
host-key-changed-session-body = The server presented a different key than the one this session verified when it connected. That happens when the server was reinstalled, or when someone intercepts the connection. A new session checks the key against known_hosts again.
remove-old-entry = Remove the old entry…
close-and-reconnect = Close and reconnect
reading-known-hosts = Reading known_hosts…
removing-known-hosts = Removing…
remove = Remove
connect-again = Connect again
no-known-hosts-entry = known_hosts has no entry for this host any more. Connect again to check the new key.
# "known_hosts line 3".
known-hosts-line = known_hosts line { $line }
# The removal question: "Remove 2 lines for [h]:2222 from known_hosts? …".
remove-known-hosts-question =
    { $lines ->
    [1] Remove 1 line for { $host } from known_hosts? The next connect asks you to trust the new key.
   *[other] Remove { $lines_text } lines for { $host } from known_hosts? The next connect asks you to trust the new key.
    }
# The warning for lines that also name other hosts.
also-covers-one-host = Line { $line } also covers other hosts (a pattern or a list of names): removing it forgets this key for them too.
also-covers-other-hosts = Lines { $lines } also cover other hosts (patterns or lists of names): removing them forgets these keys for them too.
# A connected tab whose site's host or port changed (an edit, a restore).

## Import from FileZilla
import-title = Import from FileZilla
import-step-file = 1  File
import-step-folders = 2  Folders
import-step-report = 3  Report
import-file-label = FileZilla's site manager file
import-file-hint = Usually sitemanager.xml in FileZilla's configuration folder. Passwords go to the system keyring.
import-path-required = Choose the sitemanager.xml file
choose-file = Choose…
next = Next
back = Back
import-folders-question = Turn FileZilla folders into workspaces?
import-folders-hint = Yes: one workspace per folder, named by its path, with the folder's sites. No: the folder path goes in front of each site name ("Work/Alpha").
import-folders-yes = Yes, make workspaces
import-folders-no = No, keep them in the names
import-running = Importing…
import-imported = Imported
import-skipped = Skipped
import-workspaces-created = Workspaces created
import-workspaces-not-created = Workspaces not created
import-failures = Failures
import-none = None
import-failed-keep-source = Some entries could not be saved: keep sitemanager.xml and import again once fixed.
import-passwords-keep-source = The keyring refused the passwords of these sites: keep sitemanager.xml, it still has them.
delete-sitemanager = Delete sitemanager.xml
delete-sitemanager-confirm = Delete FileZilla's sitemanager.xml? FileZilla will no longer have these sites.
deleting-sitemanager = Deleting…
sitemanager-deleted = sitemanager.xml deleted
done = Done
pick-sitemanager = Choose FileZilla's sitemanager.xml
pick-filter-xml = XML files

## Settings
settings-general = General
settings-general-description = Language, updates and version.
settings-transfers-description = How many files move at once, what happens to existing files, and how connections stay open.
settings-compare-description = When folders are compared, and the names left out.
settings-editing-description = The editor that opens remote files.
settings-terminal-description = How terminal tabs look and how much they remember.
settings-backup-description = Save your sites, workspaces and passwords to a file, or restore them.
settings-agents-description = Let coding agents work on the sites you allow.
agents-allow = Allow agents to connect to hoprs
agents-allow-hint = While this is on, a program running as you on this computer can reach hoprs. Each site still needs its own level, set in the site's form.
agents-claude-code = Register hoprs in Claude Code
agents-other-clients = Other MCP clients
agents-levels = Allow reading: { $read } · Allow everything: { $all }
settings-transfers = Transfers
settings-workers = Workers per site
settings-workers-hint = Parallel transfers per site, 1 to 16. Applies to the next connections.
settings-policy = When a file already exists
settings-policy-hint = Applies to the next transfers. Ask shows the choice in the queue.
settings-keepalive = Keepalive
settings-keepalive-hint = How often an idle connection is checked, so servers and routers keep it open. Applies to the next connections.
# A keepalive interval: "30 s".
seconds = { $secs } s
settings-unreadable = settings.toml can't be read, so the defaults apply. Changes last until hoprs closes and are not saved, to keep the file as it is: fix or delete it to save settings.
settings-save-failed = Can't save settings.toml
settings-terminal = Terminal
settings-font-size = Font size
settings-scrollback = Scrollback lines
settings-scrollback-hint = Applies to terminals opened from now on.
settings-theme = Theme
theme-hoprs-dark = hoprs dark
settings-policy-ask = Ask
settings-policy-overwrite = Overwrite
settings-policy-skip = Skip
settings-policy-overwrite-if-newer = Overwrite if newer
settings-policy-resume = Resume
decrease = −
increase = +

## Backup
backup = Backup
backup-hint = One encrypted file with every site, workspace and saved password. Keep the password: without it the backup can't be opened.
export-backup = Export backup…
restore-backup = Restore backup…
backup-file = File
backup-password = Password
backup-password-confirm = Password again
backup-path-required = Choose a file
backup-password-required = Enter a password
backup-passwords-differ = The passwords don't match
export = Export
export-overwrite-confirm = This file already exists. Overwrite it with the new backup?
restore = Restore
exporting = Encrypting the backup… this takes a few seconds
restoring = Decrypting the backup… this takes a few seconds
restore-mode = Mode
restore-merge = Merge
restore-merge-hint = Adds what is new; sites and workspaces you have stay.
restore-replace = Replace
restore-replace-hint = Everything becomes the backup: sites, workspaces and passwords not in it are removed.
restore-replace-confirm = Replace every site, workspace and saved password with the backup's?
replace = Replace
export-failed = The backup was not written
restore-failed = Nothing was restored
backup-secret-errors = Passwords not restored
backup-missing-secrets = Saved without their password (not in the keyring)
backup-skipped-unreadable = Not exported (unreadable entries)
backup-skipped-sites = Sites already here (kept)
backup-skipped-workspaces = Workspaces already here (kept)
export-ellipsis = Export…
export-everything = Export everything…
export-title-site = Export "{ $name }" (site)
# { $sites }: "4 sites".
export-title-workspace = Export "{ $name }" (workspace, { $sites })
# { $sites }, { $workspaces }: "12 sites", "3 workspaces".
export-title-all = Export everything ({ $sites }, { $workspaces })
export-password-hint = Keep this password: without it the file can't be opened.
export-partial-note = This file will hold only part of your sites. Restore it with Merge; hoprs 1.5.0 and older cannot open it.
pick-export = Save the hoprs backup
pick-restore = Choose a hoprs backup
# "Exported 12 sites, 3 workspaces and 9 passwords."
export-done = Exported { $sites }, { $workspaces } and { $secrets }.
drive-name-required = Enter a name for the file
# The pages the browser shows after connecting to Google Drive.
drive-page-connected = hoprs is connected. You can close this tab.
drive-page-failed = hoprs could not connect. Go back to hoprs and try again.
export-destination = Destination
export-destination-file = A file on this computer
export-destination-drive = Google Drive
drive-connect = Connect to Google Drive…
drive-account = Account
drive-connected = Connected
drive-disconnect = Disconnect
drive-file-name = Name
drive-waiting = Waiting for the browser…
drive-copy-link = Copy the link
drive-browser-hint = If the browser did not open, copy the link and open it yourself.
drive-session-only = The system keyring is unavailable: this connection lasts until hoprs closes.
drive-uploading = Uploading to Google Drive…
drive-uploaded = Uploaded to Google Drive: { $path }
drive-open = Open in Drive
drive-save-local = Save to this computer…
drive-saved-local = Saved to { $path }
drive-settings-line = Google Drive: connected
workspace-count =
    { $count ->
    [one] { $count } workspace
   *[other] { $count } workspaces
    }
password-count =
    { $count ->
    [one] { $count } password
   *[other] { $count } passwords
    }
# "Restored 2 sites, 1 workspace and 2 passwords."
restore-done = Restored { $sites }, { $workspaces } and { $secrets }.

## Remote editing
# The Edit action (remote pane).
edit-file = Edit
edit-no-folders = Folders can't be edited
edit-no-links = Links can't be edited
edit-too-large = Files over 50 MB can't be edited
waiting-for-connection = Waiting for connection…
remote-may-be-truncated = The server's copy may be incomplete until then.
changed-on-server = Changed on the server since you opened it
keep-remote = Keep remote
save-a-copy = Save a copy
reload-in-editor = Now the server's version: reload it in your editor.
upload-and-close = Upload pending changes and close
close-keep-copies = Close and keep local copies
upload-and-quit = Upload and quit
quit-keep-copies = Quit and keep local copies
checking-edits = Checking the remote edits…
stop-keep-copies = Stop and keep local copies
# While "Upload" runs (an upload under way has no bound but its protocol's timeouts).
uploading-edits = Uploading… (Stop to keep local copies instead)
# The kept notice, before a remote path: "Server copy of".
server-copy-of = Server copy of
# …after it.
may-be-incomplete = may be incomplete
# The kept notice after a close that did not ask (the site moved, "Close and
# reconnect"): the editor's file is no longer linked to the server.
kept-saves-stopped = The session closed: further saves in the editor won't be uploaded. Your changes are in the recovered copy; open the file again to go on editing.
edit-one-file = Select one file to edit
uploaded = Uploaded
editing = Editing
opening = Opening
close-workspace-edits-title = Some remote edits of this workspace are not uploaded
keeping-copies = Saving the local copies…
close-edits-title = Some remote edits are not uploaded
quit-edits-title = Some remote edits are not uploaded
quit-edits-hint = The window stays open until you choose. Nothing is deleted.
edit-unsent = not uploaded
edit-conflict = changed on the server
editor-command = Editor
# Windows' example names `code.cmd`: the command runs without a shell, which does not
# find `.cmd` shims from a bare name.
editor-command-hint-windows = system opens remote files in the system default editor. Or type a command; the file's path is added last. For example: notepad++.exe, code.cmd --wait.
editor-command-hint-macos = system opens remote files in the system default editor. Or type a command; the file's path is added last. For example: open -W -a "Visual Studio Code" (a full path also works).
editor-command-hint = system opens remote files in the system default editor. Or type a command; the file's path is added last. For example: gedit, code --wait.
# Next to the editor field when it holds `system`.
editor-system-default = System default
editor-command-empty = Type a command
settings-editing = Remote editing
settings-compare = Folder compare
settings-compare-auto = Compare folders automatically
settings-compare-ignore = Ignore when comparing
settings-compare-ignore-hint = Names separated by commas. * matches any run of characters, for example *.log. An empty field ignores nothing.
# "Upload failed: Permission denied".
upload-failed = Upload failed: { $reason }
# "Your version was saved to <path>".
your-version-at = Your version was saved to { $path }
# Where "keep" puts the copies: "Unsent changes are copied to <dir>".
copies-go-to = Unsent changes are copied to { $dir }
# "2 files could not be uploaded".
not-uploaded =
    { $count ->
    [1] 1 file could not be uploaded
   *[other] { $count } files could not be uploaded
    }
# The persistent notice after a close kept unsent edits: "Kept 2 unsent edits as
# recovered copies in" (the folder follows, in mono).
kept-notice =
    { $count ->
    [1] Kept 1 unsent edit as a recovered copy in
   *[other] Kept { $count } unsent edits as recovered copies in
    }
# The delete confirmation of a site with files open for editing.
delete-site-edits =
    { $count ->
    [1] 1 file of this site is open for editing: its unsent changes will be kept as a recovered copy, never deleted.
   *[other] { $count } files of this site are open for editing: their unsent changes will be kept as recovered copies, never deleted.
    }

## Errors — SSH and SFTP

error-auth-cancelled = Cancelled by user
error-sftp-subsystem-refused = The server refused the SFTP subsystem
error-sftp-start-failed = Could not start SFTP
error-sftp-connection-lost = SFTP connection lost: { $path }
error-sftp-operation-failed = SFTP operation failed: { $path }
error-sftp-cannot-resume-size = Cannot resume at { $offset }: { $path } has only { $size } bytes
error-sftp-cannot-resume-missing = Cannot resume a missing file: { $path }
error-sftp-path-too-long = Path too long: { $path }
error-sftp-mtime-out-of-range = Modification time out of range for SFTP: { $path }
error-auth-failed = Authentication failed
error-ssh-kbd-wrong-answer-count = Wrong number of keyboard-interactive answers
error-ssh-kbd-too-many-rounds = Too many keyboard-interactive rounds
error-ssh-too-many-channels = Too many channels on this connection (limit { $limit })
error-known-hosts-changed = known_hosts changed; review the entries again
error-known-hosts-invalid-host = Invalid host name for known_hosts
error-ssh-protocol-error = SSH protocol error
error-host-key-changed = Host key for { $host } has changed
error-host-key-revoked = Host key for { $host } has been revoked
error-host-key-certificate-unsupported = Host certificates are not supported
error-ssh-shell-closed = Shell closed
error-ssh-pty-refused = Server refused the PTY/shell request
error-ssh-closed-during-request = Connection closed during the { $what } request
error-ssh-shell-request-timed-out = Shell request timed out

## Errors — FTP and TLS

error-ftp-login-failed = Login failed: { $context }
error-ftp-server-closed = Server closed the connection: { $context }
error-ftp-cannot-open-data-connection = Cannot open data connection: { $context }
error-ftp-transfer-aborted = Transfer aborted: { $context }
error-ftp-reply-no-such-file = No such file or directory: { $context }
error-ftp-reply-permission-denied = Permission denied: { $context }
error-ftp-file-unavailable = File unavailable: { $context }
error-ftp-server-failed-file = Server failed to process the file: { $context }
error-ftp-storage-full = Server storage full or quota exceeded: { $context }
error-ftp-name-not-allowed = File name not allowed: { $context }
error-ftp-unexpected-reply = Unexpected FTP reply: { $context }
error-ftp-connection-lost = Connection lost: { $context }
error-tls-failed = TLS failed: { $context }
error-ftp-unexpected-reply-path = Unexpected FTP reply: { $path }
error-ftp-name-line-break = FTP cannot use a name with a line break: { $name }
error-ftp-listing-format = Cannot read the server's listing format
error-ftp-connection-lost-path = Connection lost: { $path }
error-ftp-offset-out-of-range = Offset { $offset } out of range: { $path }
error-ftp-cannot-continue = Cannot continue { $path } at byte { $offset }
error-ftp-cannot-continue-missing = Cannot continue a missing file: { $path }
error-ftp-mtime-out-of-range = Modification time out of range for FTP: { $context }
error-tls-certificate-not-trusted = Certificate not trusted
error-ftp-ftps-refused = Server refused FTPS: { $target }
error-ftp-tls-config-missing = FTPS needs a TLS configuration
error-ftp-control-command-timed-out = Control command timed out
error-ftp-command-timed-out = Command timed out
error-ftp-no-data-timeout = Connection lost: no data for { $secs } s
error-ftp-upload-refused = Upload refused by the server: { $context }
error-ftp-write-after-end = Write after the end of the upload: { $context }
error-tls-trusted-cert-unusable = Cannot use trusted certificate { $path }
error-tls-verifier-build-failed = Cannot build the certificate verifier
error-tls-configure-failed = Cannot configure TLS

## Errors — network, files and local disk

error-fs-permission-denied = Permission denied: { $path }
error-fs-no-such-file = No such file or directory: { $path }
error-fs-not-a-regular-file = Not a regular file: { $path }
error-fs-is-a-directory = Is a directory: { $path }
error-fs-not-a-directory = Not a directory: { $path }
error-net-connection-timed-out = Connection timed out
error-net-connection-failed = Connection failed
error-net-not-connected = Not connected
error-net-disconnected-by-user = Disconnected by user
error-net-connection-lost = Connection lost
error-local-invalid-path = Invalid local path
error-local-not-a-file = not a file: { $path }
error-local-offset-beyond-end = offset { $offset } beyond end of { $path }
error-local-cannot-resume-missing = cannot resume missing file: { $path }
error-local-is-a-directory = is a directory: { $path }
error-local-not-a-directory = not a directory: { $path }
error-local-set-modified-failed = set_modified task failed: { $reason }


## Errors — vault, profiles and backups
error-backup-unsupported-parameters = Unsupported backup parameters
error-backup-not-a-backup = Not a hoprs backup
error-backup-truncated = Backup file is truncated
error-backup-unsupported-version = Unsupported backup version
error-backup-invalid-key-length = Invalid backup key length
error-backup-random-failed = System random generator failed
error-backup-cannot-encrypt = Cannot encrypt the backup
error-backup-wrong-password = Wrong password or damaged file
error-backup-too-large = Backup file is too large
error-backup-cannot-build = Cannot build the backup
error-backup-invalid-content = Invalid backup content
error-backup-password-empty = Backup password is empty
error-backup-cannot-read-secret = Cannot read a secret for the backup
error-backup-partial-no-replace = This file holds only part of a backup. Restore it with Merge.
error-vault-keyring-error = System keyring error
error-vault-keyring-unavailable = System keyring unavailable
error-vault-unknown-site = Unknown site
error-vault-unknown-workspace = Unknown workspace
error-vault-workspace-unknown-site = Unknown site in workspace
error-vault-site-secrets-not-deleted = Site removed, but its secrets could not be deleted from the keyring
error-profile-invalid-path = Invalid profile path
error-profile-symlink-loop = Too many levels of symbolic links
error-profile-newer-version = Profile was written by a newer hoprs
error-profile-cannot-read = Cannot read { $file }
error-profile-duplicate-id = Duplicate id in { $file }
error-profile-cannot-save = Cannot save { $file }
error-profile-site-id-unreadable = Site id is taken by an unreadable entry
error-profile-workspace-id-unreadable = Workspace id is taken by an unreadable entry

## Errors — transfers and the queue
error-transfer-already-finished = Transfer already finished
error-transfer-not-paused = Transfer is not paused
error-transfer-duplicate-name-no-retry = A duplicate name in a directory listing cannot be retried
error-transfer-not-failed-or-cancelled = Transfer is not failed or cancelled
error-transfer-not-awaiting-decision = Transfer is not awaiting a decision
error-transfer-size-mismatch = Size mismatch after transfer
error-transfer-items-failed = { $failed } items failed
error-transfer-duplicate-name = Duplicate name in directory listing
error-transfer-worker-panicked = Worker panicked
error-transfer-destination-not-file = Destination is not a regular file
error-transfer-invalid-listing-name = Invalid name in directory listing
error-transfer-unknown = Unknown transfer
error-transfer-engine-stopped = Transfer engine stopped
error-queue-database-error = Queue database error
error-queue-newer-version = The transfer queue was saved by a newer version of hoprs

## Errors — import, settings, editing and sessions
error-import-cannot-read = Cannot read sitemanager.xml
error-import-file-too-large = sitemanager.xml is too large
error-import-delete-symlink = not deleting: the path is a symlink
error-import-delete-not-regular = not deleting: not a regular file
error-import-delete-not-filezilla = not deleting: not a FileZilla site manager file
error-settings-cannot-read = Can't read settings.toml
error-update-bad-response = The update server gave an unexpected answer
error-update-checksum-missing = { $name } is not in the release's SHA256SUMS
error-update-checksum-mismatch = The download does not match its checksum
error-update-check-failed = Could not check for updates
error-update-download-failed = The update could not be downloaded
error-update-dev-build = Update checks are off in development builds (set HOPRS_UPDATE_URL)
error-update-download-busy = This update is already downloading
error-app-opener-task-failed = Opener task failed
error-app-opener-failed = { $opener } failed
error-app-vault-task-failed = Vault task failed
error-app-background-task-failed = Background task failed
error-app-no-config-dir = No configuration directory
error-app-drop-needs-site = Open a connected site to drop files into it
error-app-drop-pane-busy = The remote folder is busy or not listed yet; drop again in a moment
error-app-drop-not-connected = The site is not connected; reconnect it to drop files into it
error-app-drop-unsupported = Can't upload this path
error-app-site-endpoint-changed = The site's host or port changed: this session was closed. Reconnect to use the new settings.
error-session-closed = Session closed
error-session-shell-closed = Shell closed
error-session-terminal-needs-ssh = Terminals need an SSH site
error-fs-already-exists = Already exists
error-edit-unknown = No such edit
error-edit-only-files = Only files can be edited
error-edit-remote-changed = The remote file changed
error-edit-local-gone = The local file is gone
error-edit-copy-exists = The copy already exists
error-edit-cannot-watch = Cannot watch the edited files
error-edit-no-parent = No parent directory
error-edit-unmatched-quote = The editor command has an unmatched quote
error-edit-no-editor = No editor command is set
error-edit-cannot-start-editor = Cannot start the editor
error-edit-path-refused = This remote path cannot be edited locally
error-drive-connect-cancelled = Connection to Google Drive cancelled
error-drive-connect-timed-out = The browser did not answer in 5 minutes
error-drive-connect-denied = Access to Google Drive was not granted
error-drive-connect-failed = Could not connect to Google Drive
error-drive-not-connected = Not connected to Google Drive
error-drive-upload-failed = Could not upload to Google Drive
error-drive-not-configured = Google Drive is not available in this build

# The folder compare panel (between the panes and the queue).
compare-button = Compare
compare-title = Differences
compare-auto = Compare automatically
compare-close = Close the differences panel
compare-collapse = Collapse
compare-expand = Expand
compare-group-only-local = Only on this computer ({ $count })
compare-group-only-remote = Only on the server ({ $count })
compare-group-different = Different ({ $count })
compare-group-unreadable = Could not read ({ $count })
compare-summary-only-here = { $count } only here
compare-summary-only-server = { $count } only on the server
compare-summary-different =
    { $count ->
    [1] 1 different
   *[other] { $count } different
    }
compare-local-newer = Local is newer
compare-remote-newer = Server is newer
compare-size-differs = Size differs
compare-kind-differs = File vs folder
compare-unreadable = Could not read: { $error }
compare-queued = Queued
compare-tip-local = This computer: { $when }
compare-tip-remote = Server: { $when }
compare-upload-selected = Upload selected
compare-download-selected = Download selected
compare-identical =
    { $count ->
    [1] 1 identical
   *[other] { $count } identical
    }
compare-all-identical =
    { $count ->
    [1] The two folders are identical (1 item)
   *[other] The two folders are identical ({ $count } items)
    }
compare-running =
    { $count ->
    [1] Comparing… 1 item
   *[other] Comparing… { $count } items
    }
compare-paused = { $count } items seen so far; the list is partial.
compare-continue = Compare everything
compare-stop = Stop
compare-stopped = Comparison stopped: { $error }
compare-partial = Stopped; the list is partial.
compare-again = Compare again
compare-roots = { $local } ↔ { $remote }
compare-ellipsis = …
