# Partial export and Google Drive upload — design

Status: approved by the author (2026-11-22) in conversation ("sim"). Decided there: export
of one site or one workspace from the right-click menu ("quero a escolha de exportar só
algum, talvez com o botão direito, sem ter que ir para configurações"), cloud as a manual
one-shot upload ("só para enviar o arquivo para o drive mesmo, não precisa se manter
atualizado sozinho"), approach A (Google Drive inside the app), and the export screen
playback ("tudo certo"). Ships in the same release as the MCP server (separate design).

## Intent

Today the only export is Settings → Backup, and it always writes everything: every site,
every workspace, every saved password. The author wants to **export just one site or one
workspace, from where they are (the sidebar)**, and to **send the exported file to Google
Drive** without leaving the app.

## Goals

- **Export…** in the right-click menu of a site and of a workspace in the sidebar.
- One export screen for every scope (one site, one workspace, everything); Settings →
  Backup's "Export backup…" opens the same screen with scope "everything".
- A partial export is the same encrypted file as a full backup, with fewer entries, so the
  existing "Restore backup" (Merge) opens it.
- A destination choice on that screen: **a file on this computer** (as today) or **Google
  Drive** (upload of the encrypted file to a `hoprs` folder in the user's Drive).
- Connect once (browser authorisation), then upload with one click; **Disconnect** forgets
  and revokes the authorisation.

## Non-goals

- Automatic or scheduled backups, sync, keeping a cloud copy up to date.
- Restoring from Drive inside the app. To restore, the user downloads the file from Drive
  (web or desktop client) and uses "Restore backup" as today.
- Listing, deleting or overwriting files on Drive.
- Other clouds (Dropbox, OneDrive, S3). A folder synced by any cloud client already works
  with "a file on this computer".
- Uploading the file to one of the user's own SFTP/FTP sites ("Export to site…"): a good
  later addition, not in this work.
- Exporting an arbitrary selection of several sites (the sidebar has no multi-selection).
- Export without passwords, or without a file password.

## Anti-goals

- **A plaintext secret leaving the machine or touching the disk.** Only the sealed file
  (Argon2id + XChaCha20-Poly1305, as today) is ever written or uploaded.
- **hoprs seeing the user's Drive.** The only scope requested is `drive.file` (files the app
  itself created). Never `drive`, `drive.readonly` or `drive.metadata`.
- **Any request to Google the user did not ask for.** hoprs talks to Google only after the
  user picks Google Drive and clicks Connect or Export. Nothing in the background, nothing
  at startup. "hoprs collects no data" stays true: there is no hoprs server in this flow.
- **Losing an export because the upload failed.** The sealed file stays available to save
  locally until the screen closes.
- **A partial file wiping a vault.** Restoring a partial file in Replace mode would delete
  every site not in it; that must be impossible, on old versions too.
- Modal cascades: the export screen is a centre form like the site form; errors are inline.

## Approaches considered

**What "cloud" means** (author: a manual upload, nothing automatic).

**How the file reaches the cloud** (author chose A):
- **A. Google Drive inside the app (chosen):** OAuth for a desktop app, scope `drive.file`,
  upload only. Cost: a Google Cloud project owned by the author and Google's brand
  verification (home page and privacy policy on hoprs.online, domain ownership); each
  further cloud is another integration.
- B. Save into a folder a cloud client already syncs. No work, already possible; depends on
  each user's setup and shows no "Drive" in the app. Stays available as the file
  destination.
- C. Upload to one of the user's own sites over SFTP/FTP. Little work, no third party, but
  not the Drive the author asked for. Deferred (non-goals).

## Design

### What an export contains

A new scope decides the entries; everything else in the file is as today.

| Scope | Sites | Workspaces | Secrets |
|---|---|---|---|
| Everything | all readable sites | all | of those sites |
| One workspace | the workspace's sites that exist and are readable | that workspace (its `sites` list trimmed to the exported ones, its saved tabs as stored) | of those sites |
| One site | that site | none | of that site |

`hoprs-vault`: `Vault::export` takes the scope (for example
`ExportScope::{All, Workspace(WorkspaceId), Site(SiteId)}`) and a second entry point returns
the sealed bytes instead of writing a path (the Drive destination uploads from memory; no
temporary file). `ExportReport` is unchanged; `skipped_unreadable` lists only entries in
scope (for one workspace: its unreadable members; for one site: nothing, an unreadable site
has no Export item).

### Partial files and Replace (the controller's call)

A file written with a scope other than Everything is a **partial file**:

- Its header carries **format version `2`**; the layout is identical to version 1. Its JSON
  has `"partial": true`.
- Full backups keep writing version `1`, so they restore on every released version.
- The reader accepts 1 and 2. A version-2 file is restored with **Merge only**: choosing
  Replace with a partial file fails before anything changes, with an inline error that says
  the file holds only part of a backup and to use Merge (new coded error, for example
  `backup-partial-no-replace`).
- hoprs ≤ 1.5.0 refuses version 2 with its existing "Unsupported backup version". That is
  the point: an old version cannot be led to wipe a vault with a partial file. The price is
  that a partial file needs the release that ships this work or newer.

### The export screen

A new centre form (`Form::Export`), in the style of the site form:

```
Export "Cliente Acme" (workspace, 4 sites)

Destination      (•) A file on this computer   ( ) Google Drive
File             [ ~/Documents/hoprs-cliente-acme-2026-11-22.hoprs ] [Choose…]
Password         [ ••••••••      ]
Password (again) [ ••••••••      ]
                                              [Cancel]  [Export]
```

- **Title** names the scope: `Export "<name>" (site)`, `Export "<name>" (workspace,
  N sites)`, `Export everything (N sites, M workspaces)`.
- **File destination:** as the Settings panel works today (typed path or the save dialog,
  the overwrite question for a typed path that exists, the report afterwards). The default
  file name is `hoprs-<slug of the name>-<YYYY-MM-DD>.hoprs` (`hoprs-backup-<date>.hoprs`
  for everything).
- **Google Drive destination:** the File row becomes the account row:
  - not connected: a **Connect to Google Drive…** button; Export is disabled until connected;
  - connected: `Account: <email> · Disconnect` and a **Name** field with the default file
    name. If the email cannot be read with `drive.file`, the row says "Connected".
  - Picking this destination is what reads the keyring (see "Knowing it is connected").
- **Password** rules are today's: required, typed twice, the hint about keeping it.
- **Export** to Drive seals in memory, uploads, then shows the usual report plus
  `Uploaded to Google Drive: hoprs/<name>` and an **Open in Drive** link (the file's
  `webViewLink`, opened in the browser).
- **Upload failure:** an inline error with **Try again** and **Save to this computer…**
  (the save dialog; writes the same sealed bytes). The bytes are dropped when the form
  closes. Passwords are cleared as the Settings panel clears them today.
- One export at a time, and not while a restore runs (both lock the vault), as today.

**Where it opens from:**
- Sidebar right-click on a site: Open, Edit, **Export…**
- Sidebar right-click on a workspace: Open all, (Close all), Edit, **Export…**
- Sidebar ⋯ menu: **Export everything…** (the controller's call: the author was offered it
  and did not object).
- Settings → Backup: "Export backup…" opens the form with scope Everything. The inline
  export panel in Settings goes away; the restore panel stays where it is. Settings →
  Backup also shows the Drive connection when there is one (`Google Drive: connected ·
  Disconnect`, without the email).
- Ctrl+K: "Export everything…".
- **Keyboard:** sidebar menus are mouse-only and the project's rule is that every menu
  item has another way in. The site form and the workspace form (Edit) get an **Export…**
  button, which opens the same screen for that site or workspace.

### Connecting to Google Drive

OAuth 2.0 for a desktop app, as Google documents it:

1. hoprs binds a listener on `127.0.0.1` on a free port, builds the authorisation URL
   (client id, `redirect_uri=http://127.0.0.1:<port>`, `scope=…/auth/drive.file`,
   `response_type=code`, PKCE `S256` challenge, a random `state`) and opens it in the
   system browser with the existing opener. The form shows "Waiting for the browser…" with
   **Cancel**, and the URL to copy if the browser did not open.
2. The browser comes back to the listener with `code` and `state`. hoprs checks `state`,
   answers with a small page ("hoprs is connected. You can close this tab." in the app's
   language) and closes the listener. One request is served; anything else gets an error
   page. The listener gives up after 5 minutes or on Cancel.
3. hoprs exchanges the code (with the PKCE verifier) for tokens at Google's token endpoint.
4. The **refresh token** goes to the system keyring (service `hoprs/google-drive`, account
   `refresh-token`), through `hoprs-vault`'s keyring access, never to a file. Access tokens
   live in memory only and are refreshed when needed. Without a usable keyring the
   connection lasts for the session only and the form says so, like passwords today.
5. **Disconnect** revokes the token at Google (best effort) and deletes the keyring entry.
   A refresh that Google refuses (revoked from the Google account page) drops the
   connection and shows Connect again.

**Knowing it is connected** (the controller's call). hoprs must not call Google or touch
the keyring just to draw a screen (the keyring is never reached at startup today, and an
unlock prompt for someone who wants a local file would be wrong):
- `settings.toml` keeps only a marker (for example `google_drive = true`), written on
  Connect and removed on Disconnect. Settings → Backup draws its line from the marker.
- The keyring entry holds the refresh token **and the account email** together; the email
  never goes to a plain file.
- The keyring is read when the user picks the Google Drive destination in the form (they
  asked for Drive, so an unlock prompt is expected) and on Disconnect. If the entry is
  gone, the marker is cleared and the form shows Connect.

The OAuth **client id and client secret are injected at build time**, not committed
(the controller's call; see the rulings). Google treats a desktop client as public: its
"secret" is not confidential, PKCE is the protection; but a committed value would be
flagged by secret scanning and would put every fork under the author's Google project.
The release workflow passes them from repository secrets to the four package builds
(`option_env!` or equivalent). A build without them (a contributor's, a fork's, a distro's,
or the official one before the author creates the Google project) **does not show the
Google Drive destination at all**; everything else works. Debug builds also accept the two
values from the environment at run time, next to the endpoint overrides.

### Uploading

- Find the app's `hoprs` folder (`files.list` for a non-trashed folder of that name; with
  `drive.file` only folders hoprs created are visible) or create it, then a multipart
  upload of the sealed bytes into it. Each upload creates a new file; nothing is updated
  or deleted. Drive allows two files with one name, so a second export on the same day is
  simply a second file.
- HTTP: the stack of the update check (`ureq` on the tree's rustls, system roots, proxy
  variables, `User-Agent: hoprs/<version>`), as its own client with its own rules: HTTPS
  only, **no redirects followed**, and the `Authorization` header only ever sent to
  `www.googleapis.com` and `oauth2.googleapis.com`. Blocking calls run on the blocking
  pool.
- The code lives in `hoprs-app` (a `drive` module beside `update`), behind `BackendApi`, so
  the fake backend covers it in app tests. `hoprs-vault` gains only the scope, the bytes
  entry point, the partial marker and the keyring entry for the token.
- Nothing secret is logged: no token, no authorisation code, no password. The log panel
  gets INFO lines such as `Google Drive: connected`, `Google Drive: uploaded <name>`.
- Debug builds only: environment overrides for the Google endpoints (as `HOPRS_UPDATE_URL`
  does for updates), so tests and QA can run against a local fake.

### Text, site and docs

- New keys in `en.ftl`, `pt-BR.ftl` and `tr.ftl` (the tests keep the three in step). The
  controller writes the Turkish and asks the translator (@P1LOTRX) to review it after the
  merge.
- **Privacy policy page** on hoprs.online (`site/`, English and Portuguese), linked from
  the home page footer. It covers everything, not only Drive, so it cannot contradict the
  FAQ: the app collects nothing and has no server; the update check (a request to GitHub,
  can be turned off); Google Drive (only on Connect and Export to Drive: the OAuth exchange
  and the encrypted file; scope `drive.file`; how to disconnect and revoke); the site
  itself (Cloudflare Web Analytics, as the site already states). It names the responsible
  party (Vinícius Sarmento) and a contact address (`privacy@hoprs.online`). Google's brand
  verification needs this page.
- README: a "Backup" section update (scopes, right-click, the Export… buttons, partial
  files need this version), `docs/ROADMAP.md`, `CHANGELOG.md` (an entry for the coming
  release, without a version bump). The site FAQ line "talks only to the servers you add,
  plus a request to GitHub" gains Google Drive.
- **If the Google project is not ready at release time**, the release ships the partial
  export only: the Drive destination is hidden, and README, changelog and FAQ do not
  mention Drive until a release build carries the client id. The privacy page can go live
  earlier (verification needs it first).

## Tests

- `hoprs-vault`: each scope exports exactly its rows and secrets; a workspace export trims
  missing and unreadable members; a partial file has version 2 and `partial`; Merge of a
  partial file adds what is new; Replace of a partial file is refused and changes nothing;
  version 1 files still restore in both modes; the bytes entry point equals the file one.
- `hoprs-app` (fake backend): the menu items exist for readable sites and workspaces; the
  form's validation, overwrite question and password clearing; destination switch; Drive
  not configured hides the destination; connect, cancel and time-out states; upload
  failure offers Try again and Save; one export at a time.
- Drive client: PKCE and `state` generation and checking; the loopback request parser
  (good code, wrong state, error parameter, junk); token refresh; folder find-or-create and
  upload against a local fake server; `Authorization` never sent to another host;
  redirects refused.
- i18n: the existing parity tests over the three catalogs.
- QA checklist `docs/qa/export-drive-checklist.md` with NEEDS HUMAN rows: a real Google
  account on Linux, macOS and Windows (connect, upload, open in Drive, disconnect, revoke
  from the Google account page), restoring a partial file on a second machine, a partial
  file refused by 1.5.0.

## Constraints

- Crate rules: only `hoprs-app` depends on iced; the dependency graph of the main spec
  §4.1 stays. No new TLS stack; prefer no new dependency for OAuth (PKCE needs SHA-256 and
  base64url; the listener is `std::net`).
- Tests pass on Linux, macOS and Windows; `tempfile`, never a literal `/tmp`.
- `cargo fmt --all --check` and clippy with `-D warnings` clean on every commit.
- Visual rules of mockup v2 (no shadows, no modal cascades, errors inline).
- Google's branding rules apply to the Connect button and the consent screen name.

## Release

This work and the MCP server (its own design and plan, written after this one) ship in one
release, **1.6.0**. Each lands through its own pull request with its docs and changelog
entry and no version bump; a separate release pull request bumps the version, as in the
earlier releases. This plan goes first; the MCP plan rebases on it if both touch
`BackendApi` or the site form.

## Needs the author (cannot be done by the builder)

1. A Google Cloud project: enable the Drive API; OAuth consent screen "External", in
   production, with the single scope `drive.file`; an OAuth client of type **Desktop app**.
   Its client id and secret go into two repository secrets for the release workflow.
2. Verify ownership of `hoprs.online` in Google Search Console and submit the brand
   verification once the privacy page is live.
3. The NEEDS HUMAN rows of the QA checklist.

Until step 1 is done the Drive destination stays hidden, so the partial export can ship
without waiting for Google.

## Builder-check rulings (2026-11-22, the controller's calls unless marked)

1. **Client id and secret:** injected at build time from repository secrets, never
   committed. Absent values hide the Drive destination.
2. **Release:** 1.6.0, together with the MCP server; a separate release pull request owns
   the version bump. Drive is documented only once a release build carries the client id.
3. **Connected state:** a marker in `settings.toml`; the token and the email together in
   the keyring; the keyring is read only when the Drive destination is picked or on
   Disconnect.
4. **Privacy page:** covers the app, the update check, Drive and the site's analytics.
   Responsible party and contact (the author's answer): **Vinícius Sarmento**,
   `privacy@hoprs.online` (a Cloudflare Email Routing rule created on 2026-11-22, forwarding
   to the same destination as `conduct@hoprs.online`).
5. **Keyboard access:** an Export… button in the site form and the workspace form.

## Risks

- **"Unverified app" screen.** Google's documents say non-sensitive scopes such as
  `drive.file` need only brand verification and define the unverified-app warning for
  sensitive and restricted scopes; no document was found that states it for a desktop
  client specifically. To be confirmed with the real project before the release notes
  promise anything.
- Whether `about.get` (the account's email) is allowed with `drive.file` alone is not
  confirmed; the design has the fallback ("Connected").

## Left to the builder

- Module and type names, the exact wording of the new texts, the slug rule for file names.
- The shape of the `BackendApi` additions and of the keyring access for the token.
- Multipart or resumable upload (the file is a few kilobytes to a few hundred).
- The loopback page's HTML and the time-out handling details.
- Whether the folder id is looked up on every upload or remembered for the session.
