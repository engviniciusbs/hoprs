# Plan 16: Partial export and Google Drive upload (1.6.0) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Export one site, one workspace or everything from the sidebar through one export screen, to a file on this computer or (when the build carries a Google OAuth client) as a manual upload of the encrypted file to a `hoprs` folder in the user's Google Drive.

**Architecture:** `hoprs-vault` gains an export scope, an entry point that returns the sealed bytes, the partial marker (header version 2 + `"partial": true`, Merge only) and a keyring entry that belongs to the app. `hoprs-app` gains the export flow (`app/export.rs`, state in `App::export`, shown as the centre form `Form::Export`, view in `ui/export.rs`) which replaces the inline export panel of Settings → Backup, and a `drive` module beside `update` (build-time client, PKCE and the loopback listener on `std::net`, a blocking `ureq` client with its own rules) behind new `BackendApi` methods. The site gains a privacy page.

**Tech Stack:** Rust 1.99.1 (pinned), iced 0.14, tokio, `ureq` 3.5 on rustls 0.23 + aws-lc-rs, `sha2` 0.10, `base64` 0.22, `percent-encoding` 2, `serde_json` 1, `chrono` 0.4, `rfd` 0.18, `keyring-core` 1, `rcgen` 0.14 (dev); site: Vite + React 19 + vitest.

**Spec:** `docs/superpowers/specs/2026-11-22-export-drive-design.md` (binding, including "Builder-check rulings" 1–5, "Release" and "Needs the author"). Read it with this plan.

## Global Constraints

- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean on every commit; `cargo test --workspace --all-features` green on Linux, macOS and Windows (`tempfile`, never a literal `/tmp`; local paths built with `Path::join`, never by joining strings with `/`). Build in the repository's `target/`, never under `/tmp`.
- Crate rules: only `hoprs-app` depends on iced; the dependency graph of the main spec §4.1 stays. `Cargo.lock` gains **no** `[[package]]`: `base64` (workspace) and `percent-encoding = "2"` become direct dependencies of `hoprs-app` and are already locked (check with `git diff Cargo.lock | grep '^+name'`: no output). No new TLS stack.
- **Order: Tasks 1–5 (partial export) are complete and shippable on their own.** No task before Task 7 may reference Google Drive in code, texts or docs. Tasks 7–13 may ship later.
- Anti-goals (spec): only the sealed file is ever written or uploaded, a plaintext secret never touches the disk or the network; the only Google scope is `https://www.googleapis.com/auth/drive.file`; no request to Google and no keyring read unless the user picked the Google Drive destination, clicked Connect, Export or Disconnect; an export is not lost because the upload failed; a partial file can never be restored with Replace.
- Drive HTTP: HTTPS only, **no redirects followed**, `User-Agent: hoprs/<version>`, system roots, proxy variables; the `Authorization` header and every body that carries a token, a code or the client secret go only to the hosts of the configured endpoints (`www.googleapis.com` and `oauth2.googleapis.com` in release builds). Blocking calls run on the blocking pool.
- Nothing secret is logged or shown: no token, authorisation code, PKCE verifier, client secret or password in `tracing`, in an `Error` (message, detail, args) or in a `Debug` output. Log lines are of the kind `Google Drive: connected`, `Google Drive: uploaded <name>`.
- Texts: every user-visible text in `crates/hoprs-app/i18n/en.ftl`, `pt-BR.ftl` and `tr.ftl` (`same_keys_and_variables_in_both_languages`, `every_en_key_is_used`, `error_codes_exist_in_both_catalogs`, `pt_br_values_differ_from_english`, `tr_values_differ_from_english`); the English text of an `error-<code>` key equals the message literal of its `Error::coded`. Always write "Google Drive" in full; no Google or Drive logo in the app. Mockup v2: no shadows, no modal, errors inline; the export screen is a centre form like `ui/workspace_form.rs` (`max_width(520)`, `theme::SPACE_LG` padding).
- No version bump: `Cargo.toml`, `site/src/facts.ts` and the release asset names stay at 1.5.0 (the release pull request owns them).
- Not tasks of this plan (spec, "Needs the author"): the Google Cloud project and its two repository secrets, the Search Console and brand verification, the NEEDS HUMAN rows. Until the secrets exist every build hides the Drive destination, and Tasks 7–12 are tested without a Google account.
- Commits: `git -c user.name="$(git log -1 --format=%an)" -c user.email="$(git log -1 --format=%ae)" commit …`, the message ends with a blank line and the `Co-Authored-By` line your session's attribution instructions give. Never change git config. Work on the branch `export-drive`.

### Rulings this plan makes (left to the builder, or where the spec is silent)

1. **Where the state lives.** `Form::Export` is a unit variant, like `Form::Import`; the flow is `App::export: Option<ExportFlow>`, because an export's result must land after the view is gone. Leaving the form (anything that makes `App::form` something else) empties the passwords and questions, and drops the flow unless an export runs; a result that arrives with the form gone drops the flow.
2. **File destination = one vault call**, as today (`Vault::export` with the scope). The Drive destination seals in memory (`Vault::export_bytes`), then uploads; the same bytes serve Try again and "Save to this computer…" (`hoprs_vault::write_atomic`).
3. **Partial files.** A file is partial when its header says version 2 **or** its JSON says `"partial": true`; either refuses Replace. The check runs after the file is authenticated (the header is the AEAD's associated data, so a flipped version byte fails as "wrong password or damaged file"). A full export's JSON has no `partial` key, so its bytes are laid out exactly as 1.5.0 wrote them.
4. **Workspace scope:** sites in vault order; the workspace keeps its own entry order and saved tabs; a workspace with no readable site still exports (0 sites, 1 workspace).
5. **Default file name:** `hoprs-<slug>-<YYYY-MM-DD>.hoprs`, local date. Slug: lowercase; every run of characters that are not Unicode alphanumerics becomes one `-`; leading and trailing `-` removed; at most 40 characters; empty → `export`. Everything: `hoprs-backup-<date>.hoprs`. Names are not translated: the catalog key `backup-file-name` goes away. The File field starts as `<Documents, else home>/<name>`; empty when neither exists.
6. **Export… in the site and workspace forms** exports the entry **as saved**, is shown only when editing, and replaces the form (unsaved edits are dropped, as any other navigation away from a form does today).
7. **Menu icon:** `Icon::ShieldCheck` (the Backup section's) for the three Export items; no new glyph, so the icon font subset is untouched.
8. **Upload:** multipart (`uploadType=multipart`), limit 5 MB (Google's limit for it): a larger file fails with `drive-upload-failed` and stays available to save locally. The `hoprs` folder is looked up on every upload (it may have been trashed in between).
9. **Loopback listener:** only a request carrying the right `state` ends the wait (with `code`: success; with `error`: denied). Anything else (wrong or missing `state`, another path, junk, a connection that sends nothing for 2 s) gets the error page with status 400 and the wait goes on, so a stray local request cannot cancel a connection. Gives up after 5 minutes or on Cancel.
10. **Keyring access:** a new trait `AppSecretStore` in `hoprs-vault` (the existing `SecretStore` is keyed by site). One entry, service `hoprs/google-drive`, account `refresh-token`, whose value is the JSON `{"refresh_token":"…","email":"…"}` (`email` omitted when unknown). The backend also keeps the connection in memory, which is what makes it last "for the session only" without a keyring.
11. **Connection state in the app** (`App::drive_link`) lasts for the session: the keyring is read the first time the Drive destination is picked, not on every opening of the form.
12. **Names:** build-time `HOPRS_GOOGLE_CLIENT_ID` and `HOPRS_GOOGLE_CLIENT_SECRET` (also read at run time in debug builds, where the run-time value wins); debug builds only: `HOPRS_DRIVE_URL`, one `https://` base under which the five endpoints live (`/auth`, `/token`, `/revoke`, `/drive/v3`, `/upload/drive/v3`).
13. **Randomness** (PKCE verifier, `state`, multipart boundary) comes from `rustls::crypto::aws_lc_rs::default_provider().secure_random`: no new dependency.
14. **Account email:** `GET <api>/about?fields=user(emailAddress)` after the token exchange; any failure means "Connected" without an email.
15. **Privacy page URLs:** `/privacy/` and `/pt-br/privacy/`.
16. **CHANGELOG heading:** `## 1.6.0 — unreleased` at the top (CI only requires a section for the workspace version, 1.5.0; the release pull request puts the date).

### Texts (English · pt-BR · Turkish)

Add each key in the task named, with a function of the same name (snake case) in `ui/strings.rs`. Turkish is the builder's; @P1LOTRX reviews it after the merge. `$sites` and `$workspaces` are the existing `site-count` / `workspace-count` texts.

| Key (task) | en | pt-BR | tr |
|---|---|---|---|
| `error-backup-partial-no-replace` (1) | This file holds only part of a backup. Restore it with Merge. | Este arquivo tem só uma parte de um backup. Restaure com Mesclar. | Bu dosya bir yedeğin yalnızca bir kısmını içeriyor. Birleştir ile geri yükleyin. |
| `export-ellipsis` (3) | Export… | Exportar… | Dışa aktar… |
| `export-everything` (3) | Export everything… | Exportar tudo… | Tümünü dışa aktar… |
| `export-title-site` (3) | Export "{ $name }" (site) | Exportar "{ $name }" (site) | "{ $name }" dışa aktar (site) |
| `export-title-workspace` (3) | Export "{ $name }" (workspace, { $sites }) | Exportar "{ $name }" (workspace, { $sites }) | "{ $name }" dışa aktar (çalışma alanı, { $sites }) |
| `export-title-all` (3) | Export everything ({ $sites }, { $workspaces }) | Exportar tudo ({ $sites }, { $workspaces }) | Tümünü dışa aktar ({ $sites }, { $workspaces }) |
| `export-password-hint` (3) | Keep this password: without it the file can't be opened. | Guarde esta senha: sem ela o arquivo não pode ser aberto. | Bu parolayı saklayın: o olmadan dosya açılamaz. |
| `export-partial-note` (3) | This file will hold only part of your sites. Restore it with Merge; hoprs 1.5.0 and older cannot open it. | Este arquivo terá só uma parte dos seus sites. Restaure com Mesclar; o hoprs 1.5.0 e anteriores não conseguem abri-lo. | Bu dosya sitelerinizin yalnızca bir kısmını içerecek. Birleştir ile geri yükleyin; hoprs 1.5.0 ve öncesi bu dosyayı açamaz. |
| `error-drive-connect-cancelled` (8) | Connection to Google Drive cancelled | Conexão com o Google Drive cancelada | Google Drive bağlantısı iptal edildi |
| `error-drive-connect-timed-out` (8) | The browser did not answer in 5 minutes | O navegador não respondeu em 5 minutos | Tarayıcı 5 dakika içinde yanıt vermedi |
| `error-drive-connect-denied` (8) | Access to Google Drive was not granted | O acesso ao Google Drive não foi concedido | Google Drive erişimi verilmedi |
| `error-drive-connect-failed` (8) | Could not connect to Google Drive | Não foi possível conectar ao Google Drive | Google Drive'a bağlanılamadı |
| `error-drive-not-connected` (9) | Not connected to Google Drive | Não conectado ao Google Drive | Google Drive'a bağlı değil |
| `error-drive-upload-failed` (9) | Could not upload to Google Drive | Não foi possível enviar para o Google Drive | Google Drive'a yüklenemedi |
| `error-drive-not-configured` (10) | Google Drive is not available in this build | O Google Drive não está disponível neste build | Google Drive bu derlemede kullanılamıyor |
| `drive-page-connected` (11) | hoprs is connected. You can close this tab. | O hoprs está conectado. Você pode fechar esta aba. | hoprs bağlandı. Bu sekmeyi kapatabilirsiniz. |
| `drive-page-failed` (11) | hoprs could not connect. Go back to hoprs and try again. | O hoprs não conseguiu conectar. Volte ao hoprs e tente de novo. | hoprs bağlanamadı. hoprs'a dönüp yeniden deneyin. |
| `export-destination` (12) | Destination | Destino | Hedef |
| `export-destination-file` (12) | A file on this computer | Um arquivo neste computador | Bu bilgisayarda bir dosya |
| `export-destination-drive` (12) | Google Drive | Google Drive | Google Drive |
| `drive-connect` (12) | Connect to Google Drive… | Conectar ao Google Drive… | Google Drive'a bağlan… |
| `drive-account` (12) | Account | Conta | Hesap |
| `drive-connected` (12) | Connected | Conectado | Bağlı |
| `drive-disconnect` (12) | Disconnect | Desconectar | Bağlantıyı kes |
| `drive-file-name` (12) | Name | Nome | Ad |
| `drive-name-required` (12) | Enter a name for the file | Informe um nome para o arquivo | Dosya için bir ad girin |
| `drive-waiting` (12) | Waiting for the browser… | Aguardando o navegador… | Tarayıcı bekleniyor… |
| `drive-copy-link` (12) | Copy the link | Copiar o link | Bağlantıyı kopyala |
| `drive-browser-hint` (12) | If the browser did not open, copy the link and open it yourself. | Se o navegador não abriu, copie o link e abra você mesmo. | Tarayıcı açılmadıysa bağlantıyı kopyalayıp kendiniz açın. |
| `drive-session-only` (12) | The system keyring is unavailable: this connection lasts until hoprs closes. | O chaveiro do sistema não está disponível: esta conexão vale até o hoprs fechar. | Sistem anahtarlığı kullanılamıyor: bu bağlantı hoprs kapanana kadar geçerli. |
| `drive-uploading` (12) | Uploading to Google Drive… | Enviando para o Google Drive… | Google Drive'a yükleniyor… |
| `drive-uploaded` (12) | Uploaded to Google Drive: { $path } | Enviado para o Google Drive: { $path } | Google Drive'a yüklendi: { $path } |
| `drive-open` (12) | Open in Drive | Abrir no Drive | Drive'da aç |
| `drive-save-local` (12) | Save to this computer… | Salvar neste computador… | Bu bilgisayara kaydet… |
| `drive-saved-local` (12) | Saved to { $path } | Salvo em { $path } | { $path } konumuna kaydedildi |
| `drive-settings-line` (12) | Google Drive: connected | Google Drive: conectado | Google Drive: bağlı |

In `crates/hoprs-app/src/i18n.rs`, `SAME_IN_BOTH` loses `backup-file-name` (Task 3) and gains `export-destination-drive` (Task 12).

## Review Focus

1. **The target changed between opening the screen and Export** (the site was deleted, the workspace emptied, a restore replaced the vault). Expected: an inline error, nothing written, no panic; a menu about a vanished entry is empty. Tests: Task 1 `scope_of_unknown_site_or_workspace_is_not_found`; Task 3 `export_of_a_vanished_target_fails_inline`, `open_for_a_missing_target_does_nothing`.
2. **Names that make bad file names** (`/`, `\`, `:`, only symbols, 200 characters, accents, emoji). Expected: the default name is always one valid file name on the three systems. Test: Task 2 `slug_rules`.
3. **Google answers something unexpected** (an HTML error page, 200 without `access_token`, a `webViewLink` that is missing or not an `https://…google.com` URL, 401 during the upload, a refused refresh, a redirect). Expected: an inline error with the bytes kept, the link left out, the connection dropped only on a refused refresh. Tests: Task 9 `unexpected_answers_are_errors_not_panics`, `redirects_are_refused`, `unsafe_view_link_is_left_out`; Task 10 `refused_refresh_drops_the_connection`.
4. **The browser side goes wrong** (the browser never opens, the tab is closed, Connect clicked twice, a favicon request or a port scanner hits the listener, Cancel as the callback lands). Expected: one listener, the wait survives stray requests, Cancel always frees the port, a success that races Cancel is kept. Tests: Task 8 `stray_requests_do_not_end_the_wait`, `cancel_and_timeout_free_the_port`; Task 11 `second_connect_click_is_ignored`, `late_success_after_cancel_is_kept`.
5. **The keyring is locked, absent, or its entry was deleted behind the app** (the marker in `settings.toml` says connected). Expected: no keyring read until the Drive destination is picked; then Connect is shown and the marker cleared; without a keyring the connection works for the session and the form says so. Tests: Task 10 `status_reads_the_keyring_only_when_asked`, `keyring_down_keeps_the_connection_for_the_session`; Task 11 `stale_marker_is_cleared_when_the_entry_is_gone`.

---

## Part A — partial export (Tasks 1–5)

### Task 1: Vault: export scope, sealed bytes, partial files

**Files:**
- Modify: `crates/hoprs-vault/src/export.rs` (module docs too), `crates/hoprs-vault/src/profiles.rs`, `crates/hoprs-vault/src/lib.rs`, `crates/hoprs-app/src/backend/mod.rs` (the one call of `Vault::export`, passing `&ExportScope::All`), the three `.ftl`

**Interfaces — Produces:**
- `#[derive(Clone, Debug, PartialEq, Eq)] pub enum ExportScope { All, Workspace(WorkspaceId), Site(SiteId) }` (in `export.rs`, re-exported from `lib.rs`).
- `Vault::export(&self, path: &Path, scope: &ExportScope, password: &SecretString, params: ExportParams) -> Result<ExportReport>` and `Vault::export_bytes(&self, scope: &ExportScope, password: &SecretString, params: ExportParams) -> Result<(ExportReport, Vec<u8>)>` (`export` is `export_bytes` + `write_atomic`).
- `Profiles::unreadable_site(&self, id: &str) -> Option<&UnreadableEntry>` (the n-th unreadable `[[site]]` raw entry of `sites_rest` pairs with the n-th `UnreadableEntry` whose `file` is `sites.toml`: `load` pushes both together).
- Header: `Header` gains `version: u16`; constants `FORMAT_FULL: u16 = 1`, `FORMAT_PARTIAL: u16 = 2` replace `FORMAT_VERSION`; `seal(plaintext, password, params, version)`; `Header::decode` accepts 1 and 2, anything else stays `backup-unsupported-version`; `read_backup` also returns the version. `Backup` gains `#[serde(default, skip_serializing_if = …)] partial: bool`.
- `Vault::import_backup`: unchanged signature. After `validate`, when the file is partial (ruling 3) and `mode` is `Replace`: `Err(Error::coded(ErrorKind::Protocol, "backup-partial-no-replace", "This file holds only part of a backup. Restore it with Merge."))`, before anything changes.
- Scope rules (spec table): `All` as today, version 1; `Workspace(id)`: the workspace's members that are readable sites, with their secrets, plus that workspace with `sites` trimmed to them, version 2, `skipped_unreadable` = its members that are unreadable sites; `Site(id)`: that site and its secrets, no workspace, version 2, `skipped_unreadable` empty. An unknown id is the existing `vault-unknown-site` / `vault-unknown-workspace` error (`ErrorKind::NotFound`; give the workspace one a helper beside `unknown_site` in `lib.rs`), and nothing is written.

- [ ] **Step 1: Failing tests** in `export.rs` (`source()` gives Alpha with a password, Bravo with a key passphrase, workspace Prod):
  - `scope_all_writes_version_1_without_the_marker`: bytes `[8..10] == [0, 1]`; `unseal` → JSON without `"partial"`; report `(2, 1, 2)`.
  - `scope_site_exports_that_site_only`: `Site(a)` → report `(1, 0, 1)`; bytes `[8..10] == [0, 2]`; the plaintext has `"partial":true`, `alpha-secret-1`, and neither `Bravo` nor `bravo-phrase-2`; Merge into an empty vault gives 1 site, 0 workspaces and Alpha's password.
  - `scope_workspace_trims_missing_and_unreadable_members`: the vault of `unreadable_entries_are_not_exported` (unreadable `odd`, readable Alpha, workspace Mixed over both, Alpha's entry with one `SavedTab`) plus a readable Charlie outside the workspace → report sites 1, workspaces 1, `skipped_unreadable` == the one entry of `odd`; restored: sites `[Alpha]`, the workspace's `sites` == `[a.id]` with its saved tab, no Charlie.
  - `scope_of_unknown_site_or_workspace_is_not_found` (RF1): codes `vault-unknown-site` and `vault-unknown-workspace`; `export` leaves no file.
  - `partial_file_merges`: a target that already holds site X with a password; Merge of a `Site(a)` file → X and Alpha, both passwords stored.
  - `partial_file_refuses_replace_and_changes_nothing`: same target, `Replace` → code `backup-partial-no-replace`, `ErrorKind::Protocol`; `snapshot(dir)` unchanged; X's password still in the store.
  - `either_marker_alone_refuses_replace`: a version-1 file sealed over JSON with `"partial":true`, and a version-2 file sealed over JSON without it → both refuse `Replace` and accept `Merge`.
  - `flipped_version_byte_fails_authentication`: a partial file with byte 9 set to 1, and a full file with byte 9 set to 2 → `backup-wrong-password` in both modes; nothing changes.
  - `bytes_entry_point_equals_the_file_one`: for `All` and `Site(a)`, `export_bytes` and `export` give equal reports and `unseal` to the same JSON bytes.
  - In `unknown_magic_or_version_is_protocol` the unknown version byte becomes 3 (2 is valid now). `merge_keeps_existing_ids` and `replace_overwrites` stay as they are: version 1 restores in both modes.
  - In `profiles.rs`: `unreadable_site_finds_the_entry_by_id` (two unreadable sites and one unreadable workspace: each site id gives its own entry; a readable id and the workspace's id give `None`).
- [ ] **Step 2: Run** `cargo test -p hoprs-vault export:: profiles::` — FAIL (missing items).
- [ ] **Step 3: Implement** the interfaces; update the callers of `export` and `seal` in the tests and the backend's call.
- [ ] **Step 4: Add** `error-backup-partial-no-replace` to the three catalogs (Texts).
- [ ] **Step 5: Run** `cargo test -p hoprs-vault`, `cargo test -p hoprs-app i18n::`, fmt, clippy — PASS.
- [ ] **Step 6: Commit** `feat(vault): export one site or one workspace; partial files restore with Merge only`.

### Task 2: Backend scope and the naming helpers

**Files:**
- Create: `crates/hoprs-app/src/app/export.rs`
- Modify: `crates/hoprs-app/src/app.rs` (`pub mod export;`, `Effect::ExportBackup` gains `scope: ExportScope`), `app/settings.rs` (`start_export` sends `ExportScope::All`), `effects.rs` (`export_backup(backend, scope, path, password)`), `backend/mod.rs`, `backend/fake.rs`, `app/tests/import_settings.rs`

**Interfaces:**
- Consumes: Task 1 `ExportScope`, `Vault::export`.
- Produces:
  - `BackendApi::export_backup(&self, scope: ExportScope, path: PathBuf, password: SecretString) -> Result<ExportReport>`. `FakeBackend`: `BackupCall::Export { scope, path, password }`; the report counts what the scope holds in the fake's profiles; a scope naming a site or workspace the fake does not hold answers `ErrorKind::NotFound`; `fail_next_export(&self, error: Error)`.
  - `app/export.rs`: `pub fn slug(name: &str) -> String` (ruling 5); `pub fn default_file_name(name: Option<&str>, today: chrono::NaiveDate) -> String`; `#[derive(Clone, Debug, PartialEq, Eq)] pub enum ExportTitle { Site { name: String }, Workspace { name: String, sites: usize }, All { sites: usize, workspaces: usize } }`; `impl App { pub fn export_title(&self, scope: &ExportScope) -> Option<ExportTitle> }` (`None` when the site or workspace is not in `App::sites` / `App::workspaces`; a workspace's `sites` counts its entries that `App::site` finds; `All` counts `App::sites` and `App::workspaces`) and `impl ExportTitle { pub fn name(&self) -> Option<&str> }`.

- [ ] **Step 1: Failing tests.**
  - `app/export.rs`: `slug_rules` (RF2): `"Cliente Acme"` → `cliente-acme`; `"São Paulo / prod"` → `são-paulo-prod`; `"a/b\\c:d"` → `a-b-c-d`; `"  --  "`, `"///"`, `"🙂"` and `""` → `export`; 200 × `a` → 40 × `a`; a 39-character word followed by `" x"` has no trailing `-`. `default_file_names`: `(Some("Cliente Acme"), 2026-11-22)` → `hoprs-cliente-acme-2026-11-22.hoprs`; `(None, 2026-11-22)` → `hoprs-backup-2026-11-22.hoprs`.
  - `app/tests/import_settings.rs`: `export_title_per_scope` (a fake with sites a, b, c and a workspace over a, b and a missing id): `Site(a)` → `Site { name }`; the workspace → `Workspace { name, sites: 2 }`; `All` → `All { sites: 3, workspaces: 1 }`; unknown ids → `None`. `effects_reach_the_backend` passes `ExportScope::Site(id)` and expects it in `BackupCall::Export`.
  - `backend/mod.rs` (beside the existing backup test, real `Backend` over a tempdir): `export_of_one_site_restores_as_one_site`: two sites, `export_backup(ExportScope::Site(a), …)` → report `(1, 0, _)`; `restore_backup(…, Merge)` into a second backend adds one site; `restore_backup(…, Replace)` → code `backup-partial-no-replace`.
- [ ] **Step 2: Run** `cargo test -p hoprs-app app::export backend::` — FAIL.
- [ ] **Step 3: Implement.** The Settings export panel keeps working, with scope `All`.
- [ ] **Step 4: Run** `cargo test -p hoprs-app`, fmt, clippy — PASS.
- [ ] **Step 5: Commit** `feat(app): export scope through the backend; file-name slug and titles`.

### Task 3: The export screen (file destination)

**Files:**
- Create: `crates/hoprs-app/src/ui/export.rs`, `crates/hoprs-app/src/app/tests/export.rs`
- Modify: `app/export.rs`, `app.rs` (`Form::Export`, `App::export`, `App::export_dir`, `Message::Export(ExportMsg)`, effects, `apply` arm, the hook at the end of `App::update`, `form_done`'s `finished` match), `app/settings.rs`, `app/dialogs.rs` (`behind_overlay`), `app/lifecycle.rs` (`while_closing`), `effects.rs`, `ui/mod.rs`, `ui/layout.rs` (`center`), `ui/settings.rs`, `ui/strings.rs`, `i18n.rs` (`SAME_IN_BOTH`), the three `.ftl`, `app/tests.rs` (`mod export;`), `app/tests/import_settings.rs`

**Interfaces:**
- Consumes: Task 2 `ExportTitle`, `App::export_title`, `default_file_name`, `BackendApi::export_backup`.
- Produces:
  - `pub struct ExportFlow { pub scope: ExportScope, pub title: ExportTitle, pub file_name: String, pub path: String, pub path_picked: bool, pub checking: bool, pub confirm_overwrite: bool, pub password: Password, pub confirm: Password, pub field_error: Option<&'static str>, pub state: BackupState<ExportReport> }` with `pub fn open(scope: ExportScope, title: ExportTitle, dir: Option<&Path>, today: chrono::NaiveDate) -> ExportFlow` and `pub fn running(&self) -> bool`.
  - `#[derive(Debug, Clone)] pub enum ExportMsg { Open(ExportScope), Path(String), Choose, Picked(Option<PathBuf>), Password(Password), Confirm(Password), Submit, TargetChecked { path: PathBuf, exists: bool }, ConfirmOverwrite, CancelOverwrite, Done(Result<ExportReport, Error>), Close }` with `pub fn is_user(&self) -> bool` (all but `Picked`, `TargetChecked`, `Done`).
  - `App::export: Option<ExportFlow>`; `App::export_dir: Option<PathBuf>` (`dirs::document_dir().or_else(dirs::home_dir)` in `App::new`); `impl App { pub(super) fn export_msg(&mut self, msg: ExportMsg) -> Vec<Effect>; pub fn export_ready(&self) -> bool }` (the Export button: a flow, nothing running in it, no check or question pending, no restore running).
  - Effects: `Effect::ExportBackup { scope, path, password }` → `ExportMsg::Done`; `Effect::CheckExportTarget(PathBuf)` → `ExportMsg::TargetChecked`; new `Effect::PickExportFile { name: String }` (the save dialog, `strings::pick_export()`, the file name set to `name`) → `ExportMsg::Picked`.
  - Removed: `Message::BackupExported`, `Message::ExportTargetChecked`, `PickTarget::ExportBackup`; from `app/settings.rs` the export half of `Backup` (`export_path`, `export_password`, `export_confirm`, `export_path_picked`, `export_checking`, `confirm_overwrite`, `export`), `BackupPanel::Export`, `BackupMsg::{OpenExport, ExportPath, ExportPassword, ExportConfirm, Export, ConfirmOverwrite, CancelOverwrite}`, `export_target_checked`, `start_export`, `backup_exported`; `strings::backup_file_name` and its key.
  - `ui/export.rs`: `pub fn view(app: &App) -> Element<'_, Message>`; `pub fn title(title: &ExportTitle) -> String`; `pub fn partial_note(scope: &ExportScope) -> Option<&'static str>` (`None` for `All`); `export_report` moves here from `ui/settings.rs`.
- Rules:
  - `Open(scope)`: nothing when `export_title` is `None`; while a flow runs, only shows the form (the running flow stays); else a new flow from `ExportFlow::open(scope, title, self.export_dir.as_deref(), chrono::Local::now().date_naive())` and `form = Some(Form::Export)`.
  - Fields and `Submit` behave as the Settings panel did: `Path` clears `path_picked`, `checking` and `confirm_overwrite`; `Submit` is ignored while `!export_ready()`; it validates in this order: path (`strings::backup_path_required()`), password (`backup_password_required`), equal passwords (`backup_passwords_differ`); a picked path starts at once, a typed one sends `CheckExportTarget` and, if it exists, waits for `ConfirmOverwrite`; a `TargetChecked` for another path is dropped. Starting moves the password into the effect's `SecretString`, empties both fields and sets `Running`.
  - `Choose` → `[PickExportFile { name: flow.file_name }]`; `Picked(Some(path))` sets the path and `path_picked`; `Picked(None)` changes nothing.
  - `Done` lands only while `Running`: `BackupState::Done(report)` or `Failed(error)`; if the form is no longer `Form::Export`, the flow is dropped instead.
  - `Close` sets `form = None`.
  - The hook, at the end of `App::update` beside the Settings one: when `form` is not `Form::Export`, a flow that does not run is dropped; one that runs has `password`, `confirm`, `checking` and `confirm_overwrite` emptied.
  - One at a time: `start_restore` does nothing while an export runs; `App::busy()` is true while an export runs.
  - `behind_overlay`: `Message::Export(msg) => msg.is_user()`; `while_closing`: a user `ExportMsg` → `Some(Vec::new())`.
- View (top to bottom, `spacing(theme::SPACE_SM)`): the title (`TEXT_MD`, `INTER_SEMIBOLD`); `partial_note` muted; `field(strings::backup_file(), text input in `theme::MONO` + outlined `strings::choose_file()`)`; a row of `field(strings::backup_password(), …)` and `field(strings::backup_password_confirm(), …)` (secure inputs; the second submits); `strings::export_password_hint()` muted; the field error in `palette().error`, `TEXT_XS`; the outcome (`widgets::busy(strings::exporting(), app.busy_phase)`, the report, or the error under `strings::export_failed()`); the actions: the overwrite question (`confirm_with(strings::export_overwrite_confirm(), strings::overwrite(), …)`), else `strings::export()` (`theme::button_primary`, pressed only when `export_ready()`) and `strings::cancel()` (`theme::button_text`, `Close`); after a successful export the primary button is `strings::done()` (`Close`).
- Settings → Backup: "Export backup…" sends `Message::Export(ExportMsg::Open(ExportScope::All))` (off while a restore or an export runs); the export panel, its outcome and its busy line leave `backup_view`; the restore panel is untouched.

- [ ] **Step 1: Failing tests** in `app/tests/export.rs` (`loaded(&fake).await`, then `app.export_dir = Some(dir)` from a `tempfile::tempdir()`); move the export cases of `export_refuses_empty_password_in_ui`, `export_over_an_existing_file_needs_confirmation`, `backup_passwords_redacted_and_forgotten` and `typed_passwords_forgotten_when_settings_not_shown` here and keep their restore halves in `import_settings.rs`:
  - `flow_open_prefills_the_file` (unit, `ExportFlow::open` with 2026-11-22): `file_name == "hoprs-cliente-acme-2026-11-22.hoprs"`, `path == dir.join(file_name)`; `All` → `hoprs-backup-2026-11-22.hoprs`; `dir: None` → empty `path`.
  - `open_shows_the_form_for_each_scope`: `Open(Site(a))` → `Form::Export`, `flow.title == ExportTitle::Site { .. }`, no effect; the same for a workspace and `All`.
  - `open_for_a_missing_target_does_nothing` (RF1): unknown site and workspace ids → `form` and `export` unchanged.
  - `validation_in_order`: empty path, then empty password, then different passwords → the three field errors in turn, no effect.
  - `typed_path_is_checked_then_asks_before_overwriting`: `Submit` → `[CheckExportTarget(path)]`; `TargetChecked { exists: true }` → `confirm_overwrite`, no effect; `CancelOverwrite`; `Submit`, `TargetChecked`, `ConfirmOverwrite` → `[ExportBackup { scope, path, .. }]`; a `TargetChecked` for an edited path is dropped; `exists: false` starts at once.
  - `picked_path_skips_the_question`: `Choose` → `[PickExportFile { name }]` with the flow's `file_name`; `Picked(Some(p))`; `Submit` → `[ExportBackup]`; `Picked(None)` changes nothing.
  - `password_leaves_the_form_with_the_effect`: after the start both fields are empty, `state` is `Running`, and neither `format!("{:?}", app.export)` nor the effect's `Debug` contains the password.
  - `one_export_at_a_time_and_not_during_a_restore`: `Submit` while `Running` → no effect; with `app.backup.restore = BackupState::Running` → no effect and `!export_ready()`; `BackupMsg::Restore` while an export runs → no `RestoreBackup`.
  - `result_shows_report_or_error`, and `export_of_a_vanished_target_fails_inline` (RF1): `Done(Err(e))` with `ErrorKind::NotFound` → `BackupState::Failed(e)`, the form stays; a `Done` while not running is ignored.
  - `leaving_the_form_drops_an_idle_flow`: passwords typed, then `Message::ConnectSite`, `Message::OpenSettings` and `ExportMsg::Close` in turn → `app.export.is_none()`.
  - `leaving_while_running_keeps_the_flow_without_passwords`: leave while `Running` → the flow stays, its fields empty; `Open(other scope)` shows it again with its scope unchanged; leave again, then `Done(Ok(_))` → `app.export.is_none()`.
  - `export_input_ignored_while_closing_or_behind_a_dialog`.
  - In `ui/export.rs`: `title_per_scope` (`Export "Cliente Acme" (site)`, `Export "Cliente Acme" (workspace, 4 sites)`, `Export everything (12 sites, 3 workspaces)`); `partial_note_only_for_partial_scopes`.
- [ ] **Step 2: Run** `cargo test -p hoprs-app app::tests::export ui::export` — FAIL.
- [ ] **Step 3: Implement** the flow, the effects, the view, the Settings button and the removals; add the Task 3 texts (Texts) and remove `backup-file-name` from the three catalogs and from `SAME_IN_BOTH`.
- [ ] **Step 4: Run** `cargo test -p hoprs-app`, fmt, clippy — PASS. Run the app (`cargo run -p hoprs-app`): Settings → Backup → "Export backup…" opens the screen titled "Export everything (…)"; export to a file, then restore that file with Merge.
- [ ] **Step 5: Commit** `feat(app): one export screen; Settings → Backup opens it`.

### Task 4: Where it opens from

**Files:**
- Modify: `crates/hoprs-app/src/app/menu.rs`, `ui/menu.rs`, `app/palette.rs`, `ui/site_form.rs`, `ui/workspace_form.rs`, `app.rs` (`site_form`, `workspace_form`), `ui/sidebar.rs` (the two comments on the right-press menus), `app/tests/menu.rs`, `app/tests/palette.rs`, `app/tests/import_settings.rs` (`sidebar_menu_hides_and_shows`), `app/tests.rs`

**Interfaces:**
- Consumes: Task 3 `ExportMsg::Open`, `strings::export_ellipsis`, `strings::export_everything`.
- Produces: `MenuAction::Export` (last item of `MenuTarget::Site` and of `MenuTarget::Workspace`) and `MenuAction::ExportAll` (last item of `MenuTarget::SidebarOptions`, `rule_before: true`, `checked: None`), each mapped in `menu_message` to `Message::Export(ExportMsg::Open(scope))`; in `ui/menu.rs` the labels `strings::export_ellipsis()` / `strings::export_everything()`, `Icon::ShieldCheck`, no shortcut; `PaletteItem::ExportAll` (label `strings::export_everything()`, among the global actions, in the last group of `empty_query_order`); `SiteFormMsg::Export` → `SiteFormOutcome::Export(SiteId)` and `WorkspaceFormMsg::Export` → `WorkspaceFormOutcome::Export(WorkspaceId)`, only when `editing && !busy` (else `Nothing`), which `App::site_form` / `App::workspace_form` turn into `self.export_msg(ExportMsg::Open(..))`; in both forms an **Export…** button (`theme::button_text`) left of Delete, shown only when editing.

- [ ] **Step 1: Failing tests.**
  - `app/tests/menu.rs`: in `site_and_workspace_menus` the site's actions are `[OpenSite, EditSite, Export]`, the workspace's `[OpenAll, EditWorkspace, Export]` and `[OpenAll, CloseAll, EditWorkspace, Export]` for the open one, all enabled; choosing `Export` gives `Message::Export(ExportMsg::Open(ExportScope::Site(a.id)))` and the workspace's scope. `sidebar_options_offer_export_everything`: actions `[ShowWorkspaces, ShowSites, ExportAll]`, the last with `rule_before` and `checked == None`; choosing it opens `Form::Export` with `ExportTitle::All`. `export_labels_and_icon`: `label(MenuAction::Export) == "Export…"`, `label(MenuAction::ExportAll) == "Export everything…"`, `icon == Icon::ShieldCheck`, `shortcut == None`.
  - `app/tests/palette.rs`: `palette_lists_export_everything`: a row labelled `Export everything…` whose message is `Message::Export(ExportMsg::Open(ExportScope::All))`.
  - `app/tests.rs`: `site_form_export_opens_the_export_screen`: `EditSite(a)`, a new name typed and not saved, `SiteFormMsg::Export` → `Form::Export` with the **saved** name in its title; from `NewSite` and from a busy form → the site form stays. `workspace_form_export_opens_the_export_screen`: the same for a workspace.
- [ ] **Step 2: Run** `cargo test -p hoprs-app app::tests` — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p hoprs-app`, fmt, clippy — PASS.
- [ ] **Step 5: Commit** `feat(app): Export… in the sidebar menus, the ⋯ menu, Ctrl+K and the forms`.

### Task 5: Docs and QA for the partial export

**Files:** `README.md`, `CHANGELOG.md`, `docs/ROADMAP.md`, `docs/ARCHITECTURE.md`, `CLAUDE.md`, `docs/superpowers/plans/2026-11-12-01-followups.md`; Create `docs/qa/export-drive-checklist.md`. Nothing here mentions Google Drive.

- [ ] **Step 1: Write.**
  - README: the feature bullet "Encrypted backup" names the three scopes; "First steps" item 4 is corrected (restore offers Merge, the default, and Replace, which asks first; today's text says restoring always replaces); a new "Backup and export" subsection under "Using hoprs": the scopes and what each file holds, right-click a site or a workspace → Export…, the ⋯ menu and Ctrl+K → Export everything…, the Export… button in the site and workspace forms (keyboard), Settings → Backup, the default file name, and that a file of one site or one workspace restores with Merge only and needs hoprs 1.6.0 or newer (1.5.0 and older answer "Unsupported backup version"); "Right-click menus" lists Export… for sites and workspaces; "Settings" no longer describes an export panel.
  - CHANGELOG: `## 1.6.0 — unreleased` above 1.5.0, in user terms: export of one site or one workspace, the export screen, where it opens from, partial files and Merge.
  - ROADMAP: the partial export among the landed work (coming in 1.6.0); under "Later": "**Export to one of your own sites** (upload the encrypted file over SFTP or FTP)".
  - ARCHITECTURE: in the `hoprs-vault` row "export of everything, one workspace or one site"; a row for `src/app/export.rs` + `src/ui/export.rs` (the export flow as a centre form, state in `App::export`).
  - CLAUDE.md: Plan 16 in "Where things are" (spec, plan, QA) and a status line (partial export on the branch, release 1.6.0 with the MCP server).
  - followups, section Plan 16: a result that lands after the form was left is only in the log; several sites at once; restore of a partial file as its own "Add from file" wording.
  - `docs/qa/export-drive-checklist.md`, in the format of `docs/qa/update-notice-checklist.md`: an "Automated" table (each row a behaviour and the tests of Tasks 1–4 that pin it) and **NEEDS HUMAN** rows with numbered steps: the screen's look in English, pt-BR and Turkish against mockup v2 (Linux, macOS, Windows); right-click → Export… on a site and on a workspace, with the title naming it; the Export… buttons reached with the keyboard only; the save dialog proposing `hoprs-<slug>-<date>.hoprs`; the overwrite question for a typed path; **restoring a partial file on a second machine** (Merge adds, Replace is refused inline, nothing changes); **a partial file refused by hoprs 1.5.0** ("Unsupported backup version", vault untouched); a full backup written by this build restored by 1.5.0 in both modes.
- [ ] **Step 2: Run** `cargo test --workspace --all-features`, fmt, clippy, `bash scripts/release-notes.sh --self-test` and `scripts/release-notes.sh "$(scripts/workspace-version.sh)" >/dev/null` — PASS.
- [ ] **Step 3: Commit** `docs: partial export (README, changelog, roadmap, architecture, QA checklist)`.

**Part A is shippable here.**

---

## Part B — the site (Task 6)

### Task 6: Privacy page on hoprs.online

Independent of the app; it can be merged and deployed before Part C (Google's brand verification needs it live).

**Files:**
- Create: `site/src/Privacy.tsx`, `site/src/Privacy.test.tsx`
- Modify: `site/src/i18n/en.ts`, `site/src/i18n/pt-br.ts`, `site/src/i18n/index.tsx`, `site/src/facts.ts`, `site/src/App.tsx`, `site/src/entry-server.tsx`, `site/src/main.tsx`, `site/src/sections/Nav.tsx`, `site/src/sections/Footer.tsx`, `site/src/seo.ts`, `site/scripts/prerender.tsx`, `site/src/prerender.test.ts`, `site/src/sitemap.test.ts`, `site/src/i18n.test.tsx` (`sameInBoth` gains `Google Drive` if a leaf is exactly that), `site/deploy/README.md` (the `dist/` table), `.github/workflows/site.yml` ("Check the live page" also fetches `/privacy/` and greps `privacy@hoprs.online`)

**Interfaces — Produces:**
- `export type Page = "home" | "privacy"`; `export const pagePaths: Record<Page, Record<Lang, string>>` = home `/`, `/pt-br/`; privacy `/privacy/`, `/pt-br/privacy/` (`pagePath` stays as the home paths).
- `App({ lang, page = "home" })` renders `<Privacy />` between `Nav` and `Footer` for `"privacy"`; `render(lang: Lang, page: Page = "home")`; `main.tsx` reads the page from `document.documentElement.dataset.page`; the prerender writes the two privacy files with `<html lang="…" data-page="privacy">`.
- `headHtml(lang, page = "home")`: the page's own title, description, canonical and hreflang links; JSON-LD of the privacy page is `WebSite` + `WebPage` only. `sitemapXml` lists four URLs (home pair, then privacy pair, each with its own alternates). `llmsTxt` links the privacy page.
- `facts.responsible = "Vinícius Sarmento"`, `facts.privacyEmail = "privacy@hoprs.online"` (in `facts.ts`, not in the dictionaries: the parity test wants every dictionary value translated).
- Dictionaries: `footer.privacy` ("Privacy" / "Privacidade") and `privacy: { title, metaTitle, metaDescription, updated, back, sections: { heading, paragraphs }[] }`. `Nav` on the privacy page links the other language's privacy page and `<home>#install`; `Footer` links `pagePaths.privacy[lang]` on every page.
- Content, six sections in this order, each fact stated plainly (Portuguese written naturally, not word for word):
  1. **Who is responsible:** hoprs is an open source desktop app published by `facts.responsible`; questions to `facts.privacyEmail` (a `mailto:` link).
  2. **The app collects nothing:** no account, no telemetry, no hoprs server; sites, workspaces and settings stay in files on the computer, passwords in the system keyring; it connects to the servers the user adds.
  3. **Update check:** one request to GitHub's API for the latest release, a few seconds after start and once a day, carrying only `User-Agent: hoprs/<version>`; Settings → General turns it off; Update downloads the installer from GitHub.
  4. **Google Drive (in versions that offer it):** used only when the user clicks Connect or exports to Google Drive; what goes to Google: the OAuth authorisation exchange and the encrypted export file, uploaded to a `hoprs` folder; the only permission asked is `drive.file` (files the app itself created: hoprs cannot see, list or change anything else in the Drive); the file is encrypted on the computer with a password that never leaves it; the authorisation is kept in the system keyring, and the account email is read only to show which account is connected; Disconnect in hoprs revokes it and removes it, and `https://myaccount.google.com/permissions` removes it from Google's side; the sentence "hoprs's use of information received from Google APIs adheres to the Google API Services User Data Policy, including the Limited Use requirements."
  5. **This website:** visits are counted with Cloudflare Web Analytics, which sets no cookies and stores no personal data; the site sits behind Cloudflare.
  6. **Changes:** the date of the last change (`updated`, written by hand: 2026-11-22) and that changes are published on this page.

- [ ] **Step 1: Failing tests.** `Privacy.test.tsx`: for both languages the page has one `h1`, six `h2`, a link `mailto:privacy@hoprs.online`, the texts `Vinícius Sarmento`, `drive.file` and `Cloudflare Web Analytics`, and the pt-BR `h1` differs from the English one; the footer of the home page links `/privacy/` (en) and `/pt-br/privacy/` (pt-BR); the privacy page's language link points at the other privacy page. `sitemap.test.ts`: the four `loc`s in order, each with the alternates of its own page. `prerender.test.ts`: a second `describe.each` over `privacy/index.html` and `pt-br/privacy/index.html`: `<html lang="…" data-page="privacy">`, the title, the canonical URL, both hreflang links, one `<title>`, the `h1` text, JSON-LD types `["WebPage", "WebSite"]`.
- [ ] **Step 2: Run** `npm test --prefix site` — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `npm run lint --prefix site`, `npm run build --prefix site`, `npm test --prefix site` (the `dist/` tests now run) — PASS; open `site/dist/privacy/index.html` and `site/dist/pt-br/privacy/index.html` through `npm run preview --prefix site` and read both pages once.
- [ ] **Step 5: Commit** `feat(site): privacy page in English and Portuguese`.

---

## Part C — Google Drive (Tasks 7–13)

### Task 7: Foundations: the app's keyring entry, the marker, the build-time client

**Files:**
- Create: `crates/hoprs-app/src/drive/mod.rs`, `crates/hoprs-app/src/drive/config.rs`
- Modify: `crates/hoprs-vault/src/secrets.rs`, `crates/hoprs-vault/src/lib.rs` (re-exports), `crates/hoprs-app/src/backend/vault.rs` (`LazyKeyringStore`), `crates/hoprs-app/src/lib.rs` (`pub mod drive;`), `crates/hoprs-app/src/settings.rs`, `.github/workflows/release.yml`, `scripts/package-deb.sh`, the header comments of `scripts/package-macos.sh` and `scripts/package-windows.sh`

**Interfaces — Produces:**
- `hoprs-vault`: `#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub enum AppSecret { GoogleDrive }` with `pub fn service(self) -> &'static str` (`"hoprs/google-drive"`) and `pub fn account(self) -> &'static str` (`"refresh-token"`); `pub trait AppSecretStore: Send + Sync { fn get_app(&self, which: AppSecret) -> Result<Option<SecretString>>; fn set_app(&self, which: AppSecret, value: &SecretString) -> Result<()>; fn delete_app(&self, which: AppSecret) -> Result<()>; }` (a missing entry is `Ok(None)` / `Ok(())`; errors through the existing `map_error`), implemented by `KeyringStore`, `MemoryStore` and, in the app, `LazyKeyringStore`.
- `Settings.google_drive: bool` (file key `google_drive`, written only when `true`, so in `OPTIONAL_KEYS`; a non-boolean value reads as `false`).
- `drive/config.rs`: `pub const CLIENT_ID_ENV: &str = "HOPRS_GOOGLE_CLIENT_ID"; pub const CLIENT_SECRET_ENV: &str = "HOPRS_GOOGLE_CLIENT_SECRET"; pub const BASE_URL_ENV: &str = "HOPRS_DRIVE_URL"; pub const SCOPE: &str = "https://www.googleapis.com/auth/drive.file"; pub const FOLDER: &str = "hoprs";` `pub struct OAuthClient { pub id: String, pub secret: String }` (its `Debug` shows neither); `pub fn client() -> Option<OAuthClient>` over the pure `fn choose(built: (Option<&str>, Option<&str>), run_time: (Option<String>, Option<String>), debug: bool) -> Option<OAuthClient>` (built = the two `option_env!`; both values non-empty or `None`; the run-time pair is looked at only when `debug`, and wins); `#[derive(Clone, Debug, PartialEq, Eq)] pub struct Endpoints { pub auth: String, pub token: String, pub revoke: String, pub api: String, pub upload: String }` with `pub fn google() -> Endpoints` (`https://accounts.google.com/o/oauth2/v2/auth`, `https://oauth2.googleapis.com/token`, `https://oauth2.googleapis.com/revoke`, `https://www.googleapis.com/drive/v3`, `https://www.googleapis.com/upload/drive/v3`), `pub fn under(base: &str) -> Option<Endpoints>` (`None` unless `base` starts with `https://`) and `pub fn current() -> Endpoints` (`google()`, or in a debug build `under(HOPRS_DRIVE_URL)` when set and valid).
- Release: in `release.yml`, the build steps of the `linux`, `macos` and `windows` jobs (not the check steps) get `env:` `HOPRS_GOOGLE_CLIENT_ID: ${{ secrets.HOPRS_GOOGLE_CLIENT_ID }}` and `HOPRS_GOOGLE_CLIENT_SECRET: ${{ secrets.HOPRS_GOOGLE_CLIENT_SECRET }}`, and a step in `prepare` that prints `::notice::Google Drive destination: enabled` or `…: hidden (no client id in the repository secrets)` from whether the id is non-empty (never the value). `scripts/package-deb.sh` passes the two variables into the container (`-e HOPRS_GOOGLE_CLIENT_ID -e HOPRS_GOOGLE_CLIENT_SECRET`, the pass-through form); the macOS and Windows scripts inherit the environment and only say so in their headers. Unset secrets arrive as empty strings, which `choose` treats as absent.

- [ ] **Step 1: Failing tests.**
  - `secrets.rs`: `app_secret_names` (`hoprs/google-drive`, `refresh-token`); `memory_store_app_roundtrip` (get `None`, delete of a missing entry `Ok`, set, get, delete; a site secret with any id is untouched); `keyring_store_app_entry_uses_service_and_account` (the mock store's entry `("hoprs/google-drive", "refresh-token")` holds the value; a backend failure is `is_keyring_unavailable`; `Debug` shows no value).
  - `settings.rs`: `google_drive_marker_roundtrip_and_invalid` (default `false` and no key rendered; `true` round-trips; `google_drive = "yes"` → `false`; set back to `false` removes the key; unknown keys kept).
  - `drive/config.rs`: `client_needs_both_values` (`(Some("id"), Some("s"))` → `Some`; a missing or empty one → `None`); `run_time_values_only_in_debug` (run-time pair with `debug: false` is ignored, with `debug: true` wins over the built pair); `endpoints_under_a_base` (`under("https://localhost:8443")` gives the five URLs of ruling 12; `under("http://localhost")` → `None`); `only_the_drive_file_scope` (`SCOPE` ends with `/auth/drive.file`, and no file under `src/drive/` — read with `std::fs` from `CARGO_MANIFEST_DIR` — contains `auth/drive"`, `drive.readonly`, `drive.metadata` or `drive.appdata` outside its tests).
- [ ] **Step 2: Run** `cargo test -p hoprs-vault secrets::` and `cargo test -p hoprs-app settings:: drive::` — FAIL.
- [ ] **Step 3: Implement**, then the workflow and script changes.
- [ ] **Step 4: Run** `cargo test --workspace --all-features`, fmt, clippy — PASS; `grep -n HOPRS_GOOGLE .github/workflows/release.yml scripts/package-deb.sh` shows the three build steps, the notice step and the two `-e` flags.
- [ ] **Step 5: Commit** `feat: Google Drive foundations (app keyring entry, settings marker, build-time client)`.

### Task 8: OAuth pieces: PKCE, state, the authorisation URL, the loopback listener

**Files:**
- Create: `crates/hoprs-app/src/drive/oauth.rs`, `crates/hoprs-app/src/drive/loopback.rs`
- Modify: `crates/hoprs-app/src/drive/mod.rs`, `crates/hoprs-app/Cargo.toml` (`base64 = { workspace = true }`, `percent-encoding = "2"`, each with a one-line comment like its neighbours: already in the tree), the three `.ftl`

**Interfaces:**
- Consumes: Task 7 `Endpoints`, `SCOPE`.
- Produces:
  - `oauth.rs`: `pub fn random<const N: usize>() -> Result<[u8; N], Error>` (ruling 13; failure is `drive-connect-failed`); `pub fn verifier(random: [u8; 32]) -> String` (base64url without padding, 43 characters); `pub fn challenge(verifier: &str) -> String` (base64url without padding of its SHA-256); `pub fn state_token(random: [u8; 16]) -> String`; `pub fn auth_url(endpoints: &Endpoints, client_id: &str, redirect_uri: &str, challenge: &str, state: &str) -> String` (query, each value percent-encoded: `client_id`, `redirect_uri`, `response_type=code`, `scope=<SCOPE>`, `code_challenge`, `code_challenge_method=S256`, `state`); `#[derive(Debug, PartialEq, Eq)] pub enum Callback { Code(String), Denied(String), Other }` and `pub fn parse_callback(head: &[u8], state: &str) -> Callback` (`head` is the request up to the blank line; only `GET /?…` with a `state` equal to `state`, compared in constant time over equal lengths, is `Code` (its `code`, percent-decoded, non-empty) or `Denied` (its `error`); everything else is `Other`).
  - `loopback.rs`: `pub const CONNECT_TIMEOUT: Duration` = 5 minutes; `pub struct Pages { pub connected: String, pub failed: String }` (plain texts; the module wraps each in a small page: `<!doctype html>`, UTF-8, the text HTML-escaped, no script, no external resource); `pub struct Listener` with `pub fn bind() -> Result<Listener, Error>` (`127.0.0.1:0`, non-blocking), `pub fn redirect_uri(&self) -> String` (`http://127.0.0.1:<port>`) and `pub fn wait(self, state: &str, cancel: &AtomicBool, timeout: Duration, pages: &Pages) -> Result<String, Error>` (ruling 9: polls `accept` every 50 ms; per connection reads at most 8 KiB with a 2 s read time-out; answers `Content-Type: text/html; charset=utf-8`, `Cache-Control: no-store`, `Connection: close`; `Code` → 200 with the connected page and `Ok(code)`; `Denied` → 200 with the failed page and `drive-connect-denied`; `Other` → 400 with the failed page and the wait goes on; cancel → `drive-connect-cancelled`; time-out → `drive-connect-timed-out`; the listener is closed when `wait` returns).
  - Errors: `Error::coded(ErrorKind::Network, "drive-connect-failed", "Could not connect to Google Drive")` (detail = the cause), `(ErrorKind::Io, "drive-connect-cancelled", "Connection to Google Drive cancelled")`, `(ErrorKind::Network, "drive-connect-timed-out", "The browser did not answer in 5 minutes")`, `(ErrorKind::Auth, "drive-connect-denied", "Access to Google Drive was not granted")` (detail = Google's `error` value, at most 100 characters).

- [ ] **Step 1: Failing tests.**
  - `oauth.rs`: `pkce_matches_rfc_7636` (`challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk") == "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"`; `verifier([0; 32])` is 43 characters of `[A-Za-z0-9_-]`); `state_and_verifier_differ_each_time` (two `random()` values differ); `auth_url_has_exactly_the_parameters` (parsed back: the seven parameters, `scope` equal to `SCOPE`, `redirect_uri` equal to `http://127.0.0.1:49152`, nothing else; a client id with `&` or a space survives the round trip); `callback_parser` (good code; `code=a%2Fb` → `a/b`; wrong state, missing state, a state that is a prefix of the right one → `Other`; `error=access_denied` with the right state → `Denied("access_denied")`; `GET /favicon.ico`, `POST /?code=…`, an empty code, non-UTF-8 bytes, an empty slice, a request line of 10 000 bytes → `Other`).
  - `loopback.rs` (real sockets on `127.0.0.1`, short time-outs passed in): `good_callback_returns_the_code_and_the_page` (a client thread sends `GET /?code=abc&state=S`; `wait` gives `abc`; the client read `200` and the connected text); `denied_callback_fails_with_the_failed_page`; `stray_requests_do_not_end_the_wait` (RF4): a favicon request, a wrong-state request, 5 bytes of junk and a connection that sends nothing each get 400 or are dropped, then the good callback still succeeds; `cancel_and_timeout_free_the_port` (RF4): setting `cancel` ends `wait` within 500 ms with `drive-connect-cancelled`, a 200 ms time-out ends with `drive-connect-timed-out`, and after each a new `TcpListener` binds the same port; `pages_escape_their_text` (`<b>` in a text comes out as `&lt;b&gt;`).
- [ ] **Step 2: Run** `cargo test -p hoprs-app drive::oauth drive::loopback` — FAIL.
- [ ] **Step 3: Implement**; add the four error keys to the three catalogs (Texts).
- [ ] **Step 4: Run** `cargo test -p hoprs-app`, fmt, clippy — PASS; `git diff Cargo.lock | grep '^+name'` prints nothing.
- [ ] **Step 5: Commit** `feat(app): OAuth for a desktop app: PKCE, state, loopback listener`.

### Task 9: The Drive HTTPS client

**Files:**
- Create: `crates/hoprs-app/src/drive/client.rs`, `crates/hoprs-app/src/test_https.rs` (`#[cfg(test)] pub(crate) mod test_https;` in `lib.rs`)
- Modify: `crates/hoprs-app/src/update/http.rs` (its tests use `test_https`; `pub fn system_roots() -> Vec<CertificateDer<'static>>` extracted from `Client::system`), `crates/hoprs-app/src/drive/mod.rs`, the three `.ftl`

**Interfaces:**
- Consumes: Task 7 `Endpoints`, `OAuthClient`, `FOLDER`.
- Produces:
  - `test_https.rs`: the local TLS server of `update/http.rs`'s tests (`Answer`, `answer`, `redirect`, `serve`, the test CA and leaf for `localhost`), moved; `serve` now also reads a request body of `Content-Length` bytes, records `(head, body)` and routes on `(method, path, body)`.
  - `drive/mod.rs`: `#[derive(Clone, Debug, PartialEq, Eq)] pub struct DriveFile { pub name: String, pub link: Option<String> }`.
  - `client.rs`: `pub struct Tokens { pub access: SecretString, pub expires_in: u64, pub refresh: Option<SecretString> }`; `pub struct Client` with `pub fn new(endpoints: Endpoints, oauth: OAuthClient, roots: Vec<CertificateDer<'static>>) -> Client`, `pub fn system(endpoints: Endpoints, oauth: OAuthClient) -> Client`, `pub fn exchange(&self, code: &str, verifier: &str, redirect_uri: &str) -> Result<Tokens, Error>` (form `grant_type=authorization_code`, `code`, `code_verifier`, `client_id`, `client_secret`, `redirect_uri`), `pub fn refresh(&self, refresh: &SecretString) -> Result<Tokens, Error>` (form `grant_type=refresh_token`; a 400 or 401 whose JSON `error` is `invalid_grant` → `drive-not-connected`), `pub fn revoke(&self, refresh: &SecretString) -> Result<(), Error>` (form `token`), `pub fn email(&self, access: &SecretString) -> Option<String>` (ruling 14), `pub fn folder(&self, access: &SecretString) -> Result<String, Error>` (`GET <api>/files` with `q = name = 'hoprs' and mimeType = 'application/vnd.google-apps.folder' and trashed = false`, `fields=files(id)`, `pageSize=1`; none → `POST <api>/files?fields=id` with `{"name":"hoprs","mimeType":"application/vnd.google-apps.folder"}`), `pub fn upload(&self, access: &SecretString, folder: &str, name: &str, bytes: &[u8]) -> Result<DriveFile, Error>` (`POST <upload>/files?uploadType=multipart&fields=id,name,webViewLink`, `multipart/related` with a random boundary: a JSON part `{"name": name, "parents": [folder]}`, then the bytes as `application/octet-stream`).
  - Agent: as `update::http::Client` (rustls with aws-lc-rs, the given roots, `Proxy::try_from_env()`, `https_only(true)`, `user_agent("hoprs/<version>")`, connect and response time-outs 10 s) with `max_redirects(0)`; every answer with a 3xx status is an error; token and API calls end within 10 s, an upload within 60 s; JSON answers are read up to 64 KiB.
  - One private send path checks the URL before every call: scheme `https` and host equal to the host of the endpoint it belongs to (`token`/`revoke` for forms, `api`/`upload` for `Authorization: Bearer`); nothing else may attach a token.
  - Errors: token exchange → `drive-connect-failed`; refused refresh → `Error::coded(ErrorKind::Auth, "drive-not-connected", "Not connected to Google Drive")`; folder and upload → `Error::coded(ErrorKind::Network, "drive-upload-failed", "Could not upload to Google Drive")`. Details are `HTTP <status>`, Google's `error` code, the transport error, `redirect refused`, `larger than 5 MB` or `unexpected answer`; never a response body, a token or a URL with a query. `upload` refuses more than 5 MB before any request. `link` is kept only when it starts with `https://` and its host is `google.com` or ends with `.google.com`.

- [ ] **Step 1: Failing tests** in `client.rs` over `test_https` (`Client::new(Endpoints::under("https://localhost:<port>"), …, vec![ca])`):
  - `exchange_sends_the_pkce_form`: the body recorded at `/token` has exactly the six fields; the answer `{"access_token":"A","expires_in":3599,"refresh_token":"R"}` gives those `Tokens`; no `Authorization` header on that request.
  - `refresh_and_revoke`: `/token` gets `grant_type=refresh_token&refresh_token=R` plus the client pair; `{"error":"invalid_grant"}` with 400 → code `drive-not-connected`; a 500 → another code; `/revoke` gets `token=R`.
  - `folder_is_found_or_created`: a list with one file → its id and no POST; an empty list → one POST with the folder JSON and the new id; the list request's `q` decodes to the query above.
  - `upload_sends_metadata_and_bytes`: the recorded body has the JSON part with `name` and `parents`, then the exact bytes; the request carries `authorization: bearer a`; the answer's `name` and `webViewLink` come back.
  - `unsafe_view_link_is_left_out` (RF3): `http://drive.google.com/x`, `https://evil.example/x`, `https://google.com.evil.example/x` and a missing field → `link == None`; `https://drive.google.com/file/d/1/view` is kept.
  - `unexpected_answers_are_errors_not_panics` (RF3): for `exchange`, `folder` and `upload`: an HTML body with 200, `{}` with 200, 200 with an empty body, a 401, a body of 1 MiB → each an `Err` of that call's code whose `Debug` holds no token.
  - `redirects_are_refused` (RF3): `/token`, `/drive/v3/files` and `/upload/drive/v3/files` answering 302 to `/elsewhere` → `Err` with detail `redirect refused`, and `/elsewhere` saw no request.
  - `tokens_go_only_to_their_endpoint`: a `Client` whose `upload` endpoint is on another host than the server refuses before connecting; an `http://` endpoint is refused; `email` failing (a 403) → `None`.
  - `upload_over_five_megabytes_is_refused_without_a_request`.
- [ ] **Step 2: Run** `cargo test -p hoprs-app drive::client update::http` — FAIL.
- [ ] **Step 3: Implement**; add `error-drive-not-connected` and `error-drive-upload-failed` to the three catalogs.
- [ ] **Step 4: Run** `cargo test -p hoprs-app`, fmt, clippy — PASS.
- [ ] **Step 5: Commit** `feat(app): Google Drive client: tokens, the hoprs folder, multipart upload`.

### Task 10: The connection holder and the backend methods

**Files:**
- Modify: `crates/hoprs-app/src/drive/mod.rs`, `crates/hoprs-app/src/backend/mod.rs`, `crates/hoprs-app/src/backend/fake.rs`, `crates/hoprs-app/src/lib.rs` (the one `LazyKeyringStore` goes to `Backend::open` and to `with_app_secrets`), the three `.ftl`

**Interfaces:**
- Consumes: Tasks 7–9 (`AppSecretStore`, `AppSecret`, `config`, `oauth`, `loopback`, `Client`, `DriveFile`); Task 1 `Vault::export_bytes`; `hoprs_vault::write_atomic`.
- Produces:
  - `drive/mod.rs`: `#[derive(Clone, Debug, PartialEq, Eq)] pub struct DriveAccount { pub email: Option<String>, pub stored: bool }` (`stored: false` = for this session only); `#[derive(Clone, Debug)] pub enum ConnectEvent { Waiting { url: String }, Done(Result<DriveAccount, Error>) }`; `#[derive(Clone)] pub struct Sealed(Arc<Vec<u8>>)` with `pub fn new(bytes: Vec<u8>) -> Sealed`, `pub fn bytes(&self) -> &[u8]` and a `Debug` that prints only the length; `pub struct Drive` with `pub fn new(store: Arc<dyn AppSecretStore>) -> Drive` (the client is built on first use from `config::client()` and `Endpoints::current()`), `pub fn with_client(client: Option<Client>, store: Arc<dyn AppSecretStore>) -> Drive`, `pub fn status(&self) -> Result<Option<DriveAccount>, Error>`, `pub fn connect(&self, pages: &Pages, send: &mut dyn FnMut(ConnectEvent))` (exactly one `Done`, last), `pub fn cancel(&self)`, `pub fn disconnect(&self) -> Result<(), Error>`, `pub fn upload(&self, name: &str, bytes: &[u8]) -> Result<DriveFile, Error>`. All blocking.
  - Rules: without a client every method answers `Error::coded(ErrorKind::NotFound, "drive-not-configured", "Google Drive is not available in this build")`. `status`: the in-memory connection if there is one (no keyring call), else one `get_app`; a value that is not the JSON of ruling 10 is deleted and reads as `None`. `connect`: bind, send `Waiting { url }`, wait, exchange, read the email, `set_app` (a failure keeps the connection in memory with `stored: false`), log `Google Drive: connected`; a second `connect` while one waits cancels the first. `upload`: an access token from memory while it has more than 60 s left, else `refresh`; `folder`, then `upload`; a 401 from Drive refreshes once and retries; a refused refresh deletes the entry and the memory and answers `drive-not-connected`; success logs `Google Drive: uploaded <name>`. `disconnect`: reads the entry if memory is empty, revokes (a failure is only logged), deletes the entry (its error is returned) and the memory.
  - `Backend::with_app_secrets(self, store: Arc<dyn AppSecretStore>) -> Backend` (default: a `MemoryStore`).
  - `BackendApi`: `async fn drive_status(&self) -> Result<Option<DriveAccount>>;` `fn drive_connect(&self, pages: Pages) -> BoxStream<'static, ConnectEvent>;` (the pattern of `start_download`: a worker on the blocking pool, events through a channel, a panicking worker ends with `Done(Err(app-background-task-failed))`) `fn drive_cancel(&self);` `async fn drive_disconnect(&self) -> Result<()>;` `async fn seal_export(&self, scope: ExportScope, password: SecretString) -> Result<(ExportReport, Sealed)>;` `async fn drive_upload(&self, name: String, sealed: Sealed) -> Result<DriveFile>;` `async fn save_sealed(&self, path: PathBuf, sealed: Sealed) -> Result<()>;` (`write_atomic` on the blocking pool).
  - `FakeBackend`: `with_drive(self, account: Option<DriveAccount>) -> Self` (what `drive_status` answers; default `None`), `fail_drive_status(&self, error: Error)`, `set_drive_connect(&self, events: Vec<ConnectEvent>)`, `fail_next_upload(&self, error: Error)`, `drive_status_reads(&self) -> u32`, `drive_uploads(&self) -> Vec<(String, usize)>`, `drive_cancels(&self) -> u32`, `drive_disconnects(&self) -> u32`, `sealed_saves(&self) -> Vec<PathBuf>`; `seal_export` records `BackupCall::Seal { scope, password }` and answers the scope's report with `Sealed::new(b"sealed".to_vec())`; `drive_upload` answers `DriveFile { name, link: Some("https://drive.google.com/file/d/fake/view") }`.

- [ ] **Step 1: Failing tests** in `drive/mod.rs` (`Drive::with_client` over `test_https` serving `/token`, `/drive/v3/about`, `/drive/v3/files`, `/upload/drive/v3/files`, `/revoke`; a `MemoryStore`, a store that counts calls, and one that fails as "keyring unavailable"; the test plays the browser: it takes `redirect_uri` and `state` out of the `Waiting` URL and sends `GET /?code=abc&state=…` over a `TcpStream`):
  - `connect_stores_token_and_email_in_one_entry`: events are `Waiting`, then `Done(Ok(DriveAccount { email: Some(_), stored: true }))`; the store's entry is JSON with `refresh_token` and `email`; the `Waiting` URL has `scope` = `drive.file` only.
  - `keyring_down_keeps_the_connection_for_the_session` (RF5): the failing store → `stored: false`; `status()` still gives the account; an upload works; a new `Drive` over the same store is not connected.
  - `status_reads_the_keyring_only_when_asked` (RF5): building a `Drive` makes 0 store calls; the first `status()` 1; the second 0 more.
  - `missing_or_garbage_entry_is_not_connected`: `None`; `"not json"` → `None` and the entry deleted.
  - `upload_refreshes_then_finds_the_folder_and_uploads`: with only the stored entry (a new `Drive`), `upload` calls `/token` once, lists, uploads; a second upload within the token's life does not call `/token`.
  - `refused_refresh_drops_the_connection` (RF3): `/token` answers `invalid_grant` → `drive-not-connected`; the entry is gone; `status()` is `None`.
  - `disconnect_revokes_and_deletes`: `/revoke` saw the token; the entry is gone; with `/revoke` answering 500 the entry is still deleted and the result is `Ok`.
  - `cancel_ends_the_wait`: `cancel()` from another thread → `Done(Err(_))` with code `drive-connect-cancelled` within 1 s; a second `connect` started while the first waits ends the first the same way.
  - `not_configured_answers_every_call`: `with_client(None, …)` → code `drive-not-configured` from `status`, `connect`, `upload`, `disconnect`.
  - `nothing_secret_in_debug_or_errors`: the `Debug` of the `Drive`, of every event and of every error above contains neither the refresh token, the access token, the code nor the client secret.
  - In `backend/mod.rs`: `seal_then_save_restores` (real `Backend`: `seal_export(Site(a), …)`, `save_sealed` to a tempdir, then `restore_backup(…, Merge)` into a second backend adds the site; the bytes start with `HOPRSVLT` and byte 9 is 2). In `backend/fake.rs`: `fake_drive_follows_its_script`.
- [ ] **Step 2: Run** `cargo test -p hoprs-app drive:: backend::` — FAIL.
- [ ] **Step 3: Implement**; add `error-drive-not-configured` to the three catalogs.
- [ ] **Step 4: Run** `cargo test -p hoprs-app`, fmt, clippy — PASS.
- [ ] **Step 5: Commit** `feat(app): Google Drive connection behind the backend (connect, upload, disconnect)`.

### Task 11: The Drive destination in the export flow

**Files:**
- Modify: `crates/hoprs-app/src/app/export.rs`, `app.rs` (`App::drive_available`, `App::drive_link`, `App::drive_error`, effects), `effects.rs`, `ui/strings.rs`, the three `.ftl` (the two page texts), `app/tests/export.rs`

**Interfaces:**
- Consumes: Task 10 `BackendApi` methods, `DriveAccount`, `ConnectEvent`, `Sealed`, `DriveFile`, `Pages`; Task 7 `config::client`, `Settings.google_drive`; Task 3 `ExportFlow`, `ExportMsg`.
- Produces:
  - `App::drive_available: bool` (`drive::config::client().is_some()` in `App::new`; tests set it); `App::drive_link: DriveLink`; `App::drive_error: Option<Error>`.
  - `#[derive(Clone, Debug, PartialEq, Eq)] pub enum DriveLink { Unknown, Reading, Disconnected, Waiting { url: Option<String> }, Connected(DriveAccount) }`; `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Destination { File, Drive }`; `#[derive(Clone, Debug)] pub enum DriveExport { Idle, Sealing, Uploading, Uploaded { report: ExportReport, file: DriveFile }, Failed(Error), Saving, Saved { report: ExportReport, path: PathBuf } }`.
  - `ExportFlow` gains `pub destination: Destination` (starts `File`), `pub drive_name: String` (starts as `file_name`), `pub sealed: Option<(ExportReport, Sealed)>`, `pub drive: DriveExport`; `running()` also covers `Sealing`, `Uploading`, `Saving`.
  - `ExportMsg` gains `Destination(Destination)`, `DriveStatus(Result<Option<DriveAccount>, Error>)`, `Connect`, `Connecting(ConnectEvent)`, `CancelConnect`, `CopyLink`, `Disconnect`, `Disconnected(Result<(), Error>)`, `DriveName(String)`, `SealDone(Result<(ExportReport, Sealed), Error>)`, `UploadDone(Result<DriveFile, Error>)`, `TryAgain`, `SaveLocal`, `SavePicked(Option<PathBuf>)`, `SaveDone { path: PathBuf, result: Result<(), Error> }`, `OpenInDrive`, `PageOpened(Result<(), Error>)`; `is_user` adds `Destination`, `Connect`, `CancelConnect`, `CopyLink`, `Disconnect`, `DriveName`, `TryAgain`, `SaveLocal`, `OpenInDrive`.
  - Effects: `DriveStatus` → `ExportMsg::DriveStatus`; `DriveConnect` → `Task::run(backend.drive_connect(Pages { connected: strings::drive_page_connected(), failed: strings::drive_page_failed() }), …)` → `ExportMsg::Connecting`; `DriveCancel` (no message); `DriveDisconnect` → `ExportMsg::Disconnected`; `SealExport { scope, password }` → `ExportMsg::SealDone`; `DriveUpload { name, sealed }` → `ExportMsg::UploadDone`; `PickSealedTarget { name }` (the same save dialog as `PickExportFile`) → `ExportMsg::SavePicked`; `SaveSealed { path, sealed }` → `ExportMsg::SaveDone`; `OpenDrivePage(String)` (`opener::open`) → `ExportMsg::PageOpened`.
- Rules:
  - `Destination(Drive)` is ignored when `!drive_available` or while the flow runs. The first time (`drive_link == Unknown`) it sets `Reading` and sends `[DriveStatus]`: this is the only place a form reads the keyring. `DriveStatus(Ok(Some(a)))` → `Connected(a)`, and the marker set (and saved) if it was off; `Ok(None)` → `Disconnected`, and the marker cleared (and saved) if it was on; `Err(e)` → `Disconnected` with `drive_error`.
  - `Connect` only from `Disconnected`: `Waiting { url: None }`, `[DriveConnect]`. `Connecting(Waiting { url })` stores the URL and sends `[OpenDrivePage(url)]`. `Connecting(Done(Ok(a)))` → `Connected(a)`, marker set and saved, whatever the link was (a success that raced a cancel is kept). `Connecting(Done(Err(e)))` → `Disconnected` when the link is `Waiting`, with `drive_error = Some(e)` unless its code is `drive-connect-cancelled`; ignored otherwise. `CancelConnect` → `Disconnected` and `[DriveCancel]`. `CopyLink` → `[WriteClipboard(url)]`. `PageOpened(Err(e))` → `drive_error`.
  - `Disconnect` (from the form or from Settings; ignored while a flow runs or the link is `Waiting`) → `[DriveDisconnect]`; `Disconnected(result)` → the link is `Disconnected`, the marker cleared and saved, `drive_error = result.err()`.
  - `Submit` with `Destination::Drive`: ignored unless the link is `Connected` and `export_ready()`; validates the name (`strings::drive_name_required()` when blank), then the passwords as for a file; then `Sealing` and `[SealExport { scope, password }]` (the password leaves the form as for a file). `SealDone(Ok((report, sealed)))` → kept in `flow.sealed`, `Uploading`, `[DriveUpload { name, sealed }]`; `SealDone(Err(e))` → `Failed(e)` with no bytes. `UploadDone(Ok(file))` → `DriveExport::Uploaded { report, file }` and the bytes dropped; `UploadDone(Err(e))` → `Failed(e)` with the bytes kept, and when `e.code == Some("drive-not-connected")` the link becomes `Disconnected` and the marker is cleared and saved.
  - `TryAgain` (only `Failed` with bytes and a `Connected` link) → `Uploading`, `[DriveUpload]` with the same bytes. `SaveLocal` (only `Failed` with bytes) → `[PickSealedTarget { name: drive_name }]`; `SavePicked(Some(path))` → `Saving`, `[SaveSealed { path, sealed }]`; `SaveDone { path, result: Ok }` → `Saved { report, path }` and the bytes dropped; `Err(e)` → back to `Failed(e)` with the bytes kept.
  - `OpenInDrive` → `[OpenDrivePage(link)]` when the file has one.
  - The hook of Task 3 also: a flow dropped drops its bytes; leaving while the link is `Waiting` sets it `Disconnected` and sends `DriveCancel`. `drive_link` otherwise survives the form (ruling 11). A result of a Drive step that arrives with the form gone drops the flow.

- [ ] **Step 1: Failing tests** (append to `app/tests/export.rs`; `app.drive_available = true` unless said):
  - `drive_is_not_offered_without_a_client`: `drive_available = false` → `Destination(Drive)` changes nothing and sends nothing.
  - `opening_the_form_never_reads_the_keyring` (RF5): `Open` and every file-destination message → no `Effect::DriveStatus`; `Destination(Drive)` → `[DriveStatus]` once; switching to File and back → no second one.
  - `stale_marker_is_cleared_when_the_entry_is_gone` (RF5): `settings.google_drive = true`, `DriveStatus(Ok(None))` → `Disconnected`, marker `false`, an `Effect::SaveSettings`; `DriveStatus(Err(e))` → `Disconnected`, `drive_error`, marker untouched.
  - `connect_waits_then_connects`: `Connect` → `[DriveConnect]`, `Waiting { url: None }`; `Connecting(Waiting { url })` → `[OpenDrivePage(url)]`; `Connecting(Done(Ok(a)))` → `Connected(a)`, marker `true`, `SaveSettings`.
  - `second_connect_click_is_ignored` (RF4): `Connect` while `Waiting` → no effect.
  - `cancel_and_timeout_states`: `CancelConnect` → `Disconnected`, `[DriveCancel]`, no error shown when the late `Done(Err(drive-connect-cancelled))` lands; `Done(Err(drive-connect-timed-out))` while `Waiting` → `Disconnected` with that error.
  - `late_success_after_cancel_is_kept` (RF4): `CancelConnect`, then `Connecting(Done(Ok(a)))` → `Connected(a)`, marker `true`.
  - `leaving_while_waiting_cancels`: `Message::ConnectSite` while `Waiting` → an `Effect::DriveCancel`, link `Disconnected`.
  - `export_to_drive_seals_then_uploads`: `Submit` → `[SealExport { scope, .. }]`, `Sealing`, both password fields empty; `SealDone(Ok(_))` → `[DriveUpload { name, .. }]` with the typed name; `UploadDone(Ok(file))` → `DriveExport::Uploaded { .. }`, `flow.sealed.is_none()`. A blank name → `strings::drive_name_required()`; not connected → no effect.
  - `upload_failure_offers_try_again_and_save`: `UploadDone(Err(e))` → `Failed`, bytes kept; `TryAgain` → `[DriveUpload]` with the same bytes; again failed, `SaveLocal` → `[PickSealedTarget { name }]`; `SavePicked(None)` changes nothing; `SavePicked(Some(p))` → `[SaveSealed { path: p, .. }]`; `SaveDone { Ok }` → `Saved { path: p, .. }`, bytes dropped; `SaveDone { Err }` → `Failed`, bytes still there.
  - `refused_authorisation_asks_to_connect_again`: `UploadDone(Err(drive-not-connected))` → link `Disconnected`, marker `false`, bytes kept; after `Connect` … `Done(Ok(_))`, `TryAgain` uploads the same bytes.
  - `seal_failure_has_no_bytes`: `SealDone(Err(e))` → `Failed(e)`, `sealed.is_none()`, `TryAgain` and `SaveLocal` do nothing.
  - `one_export_at_a_time_for_drive_too`: `Submit`, `Destination(File)` and `Disconnect` while `Sealing` or `Uploading` → no effect; `BackupMsg::Restore` then → no `RestoreBackup`.
  - `bytes_are_dropped_with_the_form`: `Failed` with bytes, `ExportMsg::Close` → `app.export.is_none()`.
  - `disconnect_clears_the_marker`: `Disconnect` → `[DriveDisconnect]`; `Disconnected(Ok(()))` → `Disconnected`, marker `false`, `SaveSettings`; `Disconnected(Err(e))` → the same, and `drive_error`.
  - An effects test beside `effects_reach_the_backend`: the Drive effects reach a `FakeBackend` scripted with `set_drive_connect` and come back as the messages above; `drive_uploads()` holds the name and the byte count.
- [ ] **Step 2: Run** `cargo test -p hoprs-app app::tests::export` — FAIL.
- [ ] **Step 3: Implement**; add `drive-page-connected` and `drive-page-failed`.
- [ ] **Step 4: Run** `cargo test -p hoprs-app`, fmt, clippy — PASS.
- [ ] **Step 5: Commit** `feat(app): Google Drive destination: connect, seal, upload, fall back to a file`.

### Task 12: The Drive rows of the export screen and the Settings line

**Files:**
- Modify: `crates/hoprs-app/src/ui/export.rs`, `ui/settings.rs`, `ui/strings.rs`, `i18n.rs` (`SAME_IN_BOTH`), the three `.ftl`

**Interfaces:**
- Consumes: Task 11 `Destination`, `DriveLink`, `DriveExport`, `ExportMsg`, `App::drive_available`, `App::drive_error`.
- Produces:
  - `#[derive(Debug, PartialEq)] pub enum DriveRow { Reading, Connect, Waiting { can_copy: bool }, Account { label: String, session_only: bool } }` and `pub fn drive_row(app: &App) -> Option<DriveRow>` (`None` unless the flow's destination is `Drive`; `Account.label` is the email, or `strings::drive_connected()` without one).
  - `#[derive(Debug, PartialEq)] pub enum DriveOutcome { None, Busy(&'static str), Uploaded { path: String, link: bool }, Failed { try_again: bool, save: bool }, Saved(String) }` and `pub fn drive_outcome(app: &App) -> DriveOutcome` (`Busy` is `strings::exporting()` while sealing, `strings::drive_uploading()` while uploading or saving; `Uploaded.path` is `hoprs/<name>`).
  - `pub fn drive_line(app: &App) -> Option<&'static str>` in `ui/settings.rs` (`strings::drive_settings_line()` only when `drive_available && settings.google_drive`).
- View: with `drive_available`, a **Destination** field above File: two `theme::segment` buttons (`strings::export_destination_file()`, `strings::export_destination_drive()`), as the restore panel's Merge / Replace; without it the row is absent and the screen is exactly Task 3's. For the Drive destination the File field gives way to: `Reading` → nothing under the Destination row until the status lands (Export off); `Connect` → an outlined **Connect to Google Drive…** button (no logo) and the Export button off; `Waiting` → `strings::drive_waiting()`, **Cancel** (`CancelConnect`), **Copy the link** (off until the URL is known) and the hint `strings::drive_browser_hint()`; `Account` → `strings::drive_account()`: the label, a text button **Disconnect**, `strings::drive_session_only()` muted when `session_only`, and the **Name** field (`drive_name`, `theme::MONO`). `drive_error` shows in `palette().error` under the row. Outcome: `Uploaded` → the report, `strings::drive_uploaded(path)` and a text button **Open in Drive** when there is a link; `Failed` → the error under `strings::export_failed()`, then **Try again** (`strings::update_try_again()`) and **Save to this computer…**, each only when its flag is set; `Saved` → the report and `strings::drive_saved_local(path)`. After `Uploaded` or `Saved` the primary button is `strings::done()`.
- Settings → Backup: under the two buttons, when `drive_line` is `Some`, the line and a text button **Disconnect** (`Message::Export(ExportMsg::Disconnect)`); `drive_error` under it. No email, no keyring read.

- [ ] **Step 1: Failing tests** in `ui/export.rs`: `no_drive_row_for_the_file_destination`; `drive_row_per_link` (each `DriveLink` → its `DriveRow`; `Waiting { url: None }` → `can_copy: false`; an account without email → label `Connected`; `stored: false` → `session_only`); `drive_outcome_per_state` (each `DriveExport`; `Failed` with bytes and a connected link → both flags; with bytes and no connection → only `save`; without bytes → neither; `Uploaded` with `link: None` → `link: false`; path `hoprs/hoprs-cliente-acme-2026-11-22.hoprs`). In `ui/settings.rs`: `drive_line_follows_the_marker_and_the_build` (the four combinations).
- [ ] **Step 2: Run** `cargo test -p hoprs-app ui::export ui::settings` — FAIL.
- [ ] **Step 3: Implement** the view and the Task 12 texts; `SAME_IN_BOTH` gains `export-destination-drive`.
- [ ] **Step 4: Run** `cargo test -p hoprs-app`, fmt, clippy — PASS. If the author's Google project exists: `HOPRS_GOOGLE_CLIENT_ID=… HOPRS_GOOGLE_CLIENT_SECRET=… cargo run -p hoprs-app`, connect, export one site to Drive, Open in Drive, Disconnect; without the two variables the Destination row is absent.
- [ ] **Step 5: Commit** `feat(app): Google Drive on the export screen and in Settings → Backup`.

### Task 13: Drive docs, the FAQ line and the QA rows

**Gate:** do this task only when the author confirms the Google project is ready (the two repository secrets exist, so the release build will carry the client id). Otherwise stop after Task 12, add a line to the Plan 16 section of the followups ("Drive docs wait for the Google project") and report it: README, changelog and FAQ must not mention Drive before that (spec, ruling 2).

**Files:** `README.md`, `CHANGELOG.md`, `docs/ROADMAP.md`, `docs/ARCHITECTURE.md`, `CLAUDE.md`, `docs/qa/export-drive-checklist.md`, `docs/superpowers/plans/2026-11-12-01-followups.md`, `site/src/i18n/en.ts`, `site/src/i18n/pt-br.ts`

- [ ] **Step 1: Write.**
  - README: "Backup and export" gains the Google Drive destination (Connect once in the browser, the `hoprs` folder, each export a new file, Disconnect; only the encrypted file leaves the computer; scope `drive.file`; to restore, download the file from Drive and use "Restore backup…"); "Where your data lives" gains the keyring entry `hoprs/google-drive` and the `google_drive` marker in `settings.toml`; "Uninstall" names that entry; "Building from source": the destination is hidden unless the build gets `HOPRS_GOOGLE_CLIENT_ID` and `HOPRS_GOOGLE_CLIENT_SECRET` of your own Google project; a link to the privacy page.
  - CHANGELOG, in `## 1.6.0 — unreleased`: the Google Drive destination in user terms.
  - ROADMAP: "Telemetry: not planned" keeps true and names the two requests hoprs can make on its own behalf (GitHub, and Google only when asked); under "Later": restore from Drive, other clouds.
  - ARCHITECTURE: a row for `src/drive/` (config, OAuth pieces, loopback listener, the blocking client with its rules, the connection holder) and the Drive methods of `BackendApi`; the keyring row of "Files hoprs keeps" gains the app entry.
  - Site FAQ, "Does it collect data?" in both dictionaries: "… plus a request to GitHub to check for a new version (you can turn it off in Settings), and Google Drive only when you connect it and export there …", ending with a pointer to the privacy page.
  - CLAUDE.md: the status line names the Drive destination and the debug variables (`HOPRS_GOOGLE_CLIENT_ID`, `HOPRS_GOOGLE_CLIENT_SECRET`, `HOPRS_DRIVE_URL`).
  - QA checklist: automated rows for Tasks 7–12 and **NEEDS HUMAN** rows with steps, each on Linux, macOS and Windows with a real Google account: Connect (the browser opens; the consent screen shows the app name and only "See, edit, create, and delete only the specific Google Drive files you use with this app"; note whether an "unverified app" screen appears — the spec's open risk); the tab says "hoprs is connected" in the app's language; the form shows the account email, or "Connected" (the spec's second open risk: record which); export one site: the file is in `hoprs/` on Drive, Open in Drive opens it, the downloaded file restores with Merge; a second export the same day is a second file; Cancel while waiting, and a browser that did not open (Copy the link); offline upload → the error, Try again after reconnecting, Save to this computer…; Disconnect in the form and in Settings → Backup (the app disappears from `https://myaccount.google.com/permissions`); revoke from the Google account page, then export → Connect is shown again and the file can still be saved; a locked or absent keyring (the session-only note; after a restart Connect is shown); a release build without the secrets shows no Destination row; `settings.toml` holds `google_drive = true` and nothing else about Google; the log panel shows no token.
  - followups, Plan 16: resumable upload above 5 MB; a local fake for manual QA needs a certificate the system trusts (`HOPRS_DRIVE_URL` is HTTPS-only); "Export to site…"; restore from Drive.
- [ ] **Step 2: Run** `cargo test --workspace --all-features`, fmt, clippy, `npm test --prefix site`, `npm run build --prefix site` — PASS.
- [ ] **Step 3: Commit** `docs: Google Drive destination (README, changelog, FAQ, QA checklist)`; push `export-drive` and open the pull request "Partial export and Google Drive upload" (no version bump; the release pull request for 1.6.0 follows the MCP server's).
