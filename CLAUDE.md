# hoprs

Desktop SSH + SFTP + FTP/FTPS client in Rust (iced GUI) for Linux, Windows and macOS, open source (MIT OR Apache-2.0). Replaces FileZilla for the author: same saved-sites idea, clean UI, high performance. Repo is public (since 2026-11-14).

## Where things are

- Design spec (source of truth): `docs/superpowers/specs/2026-11-11-hoprs-design.md`
- Approved UI mockup (open in a browser): `docs/superpowers/specs/assets/2026-11-11-hoprs-mockup-v2.html`
- Plan roadmap (6 plans, one per subsystem, then follow-up plans 7 same-destination and 8 release 1.0): `docs/superpowers/plans/2026-11-11-00-roadmap.md`, `docs/superpowers/plans/2026-11-13-07-same-destination.md`, `docs/superpowers/plans/2026-11-13-08-release-1.0.md`
- Plan 1 (foundation + transfer engine): `docs/superpowers/plans/2026-11-11-01-foundation-transfer-engine.md`
- Plan 2 (SSH + SFTP): `docs/superpowers/plans/2026-11-12-02-ssh-sftp.md`
- Plan 3 (FTP + FTPS): `docs/superpowers/plans/2026-11-12-03-ftp-ftps.md`
- Plan 4 (vault + FileZilla import): `docs/superpowers/plans/2026-11-12-04-vault-import.md`
- Plan 5 (iced app shell): `docs/superpowers/plans/2026-11-12-05-app-shell.md`
- Plan 6 (terminal, remote editing, palette, log, perf job): `docs/superpowers/plans/2026-11-13-06-terminal-polish.md`
- Plan 7 (same-destination serialisation): `docs/superpowers/plans/2026-11-13-07-same-destination.md`
- Plan 8 (release 1.0.0: icons, `.deb`, release workflow): `docs/superpowers/plans/2026-11-13-08-release-1.0.md`
- Plan 10 (Windows and macOS, release 1.1.0): spec `docs/superpowers/specs/2026-11-14-windows-macos-design.md`, plan `docs/superpowers/plans/2026-11-14-10-windows-macos.md`
- Plan 11 (app in Brazilian Portuguese, Fluent catalogs): spec `docs/superpowers/specs/2026-11-15-app-i18n-pt-br-design.md`, plan `docs/superpowers/plans/2026-11-15-11-app-i18n-pt-br.md`
- Landing page for https://hoprs.online: code in `site/` (Vite + React + Tailwind, prerendered), spec `docs/superpowers/specs/2026-11-14-landing-page-design.md`, plan `docs/superpowers/plans/2026-11-14-09-landing-page.md`, deploy notes `site/deploy/README.md`, workflow `.github/workflows/site.yml`
- Plan 12 (folder compare, 1.2.0): spec `docs/superpowers/specs/2026-11-15-folder-compare-design.md`, plan `docs/superpowers/plans/2026-11-15-12-folder-compare.md`, QA `docs/qa/folder-compare-checklist.md`
- Plan 13 (UI polish, 1.3.0): spec `docs/superpowers/specs/2026-11-16-ui-polish-design.md`, plan `docs/superpowers/plans/2026-11-16-13-ui-polish.md`, QA `docs/qa/ui-polish-checklist.md`
- Plan 14 (workspace groups, refresh after transfers, 1.3.1): spec `docs/superpowers/specs/2026-11-16-1.3.1-workspace-and-refresh-design.md`, plan `docs/superpowers/plans/2026-11-16-14-workspace-groups-refresh.md`, QA `docs/qa/release-1.3.1-checklist.md`
- Plan 15 (update notice, 1.4.0): spec `docs/superpowers/specs/2026-11-17-update-notice-design.md`, plan `docs/superpowers/plans/2026-11-17-15-update-notice.md`, QA `docs/qa/update-notice-checklist.md`
- QA checklists (manual runs, NEEDS HUMAN rows with steps): `docs/qa/plan-5-checklist.md`, `docs/qa/plan-6-checklist.md`, `docs/qa/release-1.0-checklist.md`, `docs/qa/release-1.1-checklist.md`, `docs/qa/i18n-checklist.md`, `docs/qa/folder-compare-checklist.md`, `docs/qa/ui-polish-checklist.md`, `docs/qa/release-1.3.1-checklist.md`, `docs/qa/update-notice-checklist.md`
- Performance targets, how they are measured, last numbers: `docs/perf/README.md` (`scripts/perf.sh`, `.github/workflows/perf.yml`)
- Deferred follow-ups: `docs/superpowers/plans/2026-11-12-01-followups.md`
- Contributor docs: `CONTRIBUTING.md` (setup, checks, translating), `docs/ARCHITECTURE.md` (crates, app data flow, where to put a change), `docs/ROADMAP.md` (public roadmap: keep it in sync when plans land), `SECURITY.md`, `CODE_OF_CONDUCT.md` (Contributor Covenant 2.1, reports to conduct@hoprs.online via Cloudflare Email Routing), issue and PR templates in `.github/`

## Current status (2026-11-17)

- Landing page is live at https://hoprs.online, deployed by `.github/workflows/site.yml` on push to `main` (VPS via KoreGo, Cloudflare in front; see `site/deploy/README.md`).
- Plans 1–7 merged to `main` (v1 feature-complete): core + transfer engine, SSH/SFTP, FTP/FTPS, vault + FileZilla import, the iced app, Plan 6 (terminal tabs over the site's SSH connection, remote editing with re-upload on save, Ctrl+K palette, technical log panel, idle-drop fixes, perf job) Plan 7 (one writer per destination) and Plan 8 (release 1.0.0: `.deb` packaging, icon, `.desktop`, release workflow).
- **v1.0.0 released (2026-11-14):** tag `v1.0.0` on `342ff80`, GitHub Release with `hoprs_1.0.0-1_amd64.deb` + `SHA256SUMS` (checksum verified after download).
- `perf.yml` first ran green on `main` (2026-11-13): startup 84 ms, idle 33 MB, 100k rows p95 0.58 ms, SFTP 1 GiB 0.99x `scp`; small files are known gaps (FTP 1.003x, SFTP 2.0x `lftp`). Numbers and method: `docs/perf/README.md`.
- QA checklists `docs/qa/plan-5-checklist.md` and `docs/qa/plan-6-checklist.md` still have NEEDS HUMAN rows for the author (real input devices, a GUI editor, a 10-minute idle on the author's FTP server).
- Deferred follow-ups: `docs/superpowers/plans/2026-11-12-01-followups.md` (per plan, sections Plan 6, 7 and 8).
- `docs/qa/release-1.0-checklist.md` has the NEEDS HUMAN rows (install the `.deb`, icon sizes, dock grouping, uninstall keeps data).
- **Next step (1.0 follow-ups):** the author runs the NEEDS HUMAN rows; then work the follow-ups (Plan 8 section first: tag-on-`main` check in `release.yml`, `actions/checkout@v5` for the Node 20 deprecation).
- Docker integration tests: `docker compose -f docker/compose.yml up -d --build --wait`, then `cargo test --workspace --all-features -- --ignored --test-threads=1`.
- **v1.1.0 released (2026-11-14), Plan 10 merged (PR #11):** tag `v1.1.0` on `1f06850`, GitHub Release with `.deb`, universal `.dmg`, `.msi`, portable `.zip` and `SHA256SUMS` (checksums verified after download); the site switched to 1.1.0 through the post-release deploy. Follow-ups: `docs/superpowers/plans/2026-11-12-01-followups.md` section Plan 10.
- **Next step (Plan 10):** the author runs the NEEDS HUMAN rows of `docs/qa/release-1.1-checklist.md` on real Windows and macOS machines (the macOS Cmd+Q known issue first).
- **v1.2.0 released (2026-11-16):** tag `v1.2.0` on `41655fe` (PR #25), GitHub Release with `.deb`, `.dmg`, `.msi`, `.zip` and `SHA256SUMS` (checksums verified after download); the site switched to 1.2.0. It ships Plan 11 (pt-BR UI, PR #15) and Plan 12 (folder compare, PR #23). Contributor docs merged (PR #16), starter issues #17–#22.
- **Next step (1.2.0 follow-ups):** the open NEEDS HUMAN rows of `docs/qa/i18n-checklist.md` and `docs/qa/folder-compare-checklist.md` (macOS, Windows, large tree, keyboard-only; the author's Linux run on 2026-11-16 passed); then the follow-ups, sections Plan 11 and Plan 12 in `docs/superpowers/plans/2026-11-12-01-followups.md`.
- **v1.3.0 released (2026-11-16):** tag `v1.3.0` on `50baccf` (PR #32), GitHub Release with `.deb`, `.dmg`, `.msi`, `.zip` and `SHA256SUMS` (checksums verified after download); the site switched to 1.3.0. It ships Plan 13 (UI polish, PRs #29, #30, #31): Phosphor icons, Settings as a tab (Ctrl+, / ⌘,), right-click menus (Shift+F10), a visible drag with a drop label, resizable compare panel and queue. The README has an "Updating" section; capture scripts take `LOCAL_DEB=…`.
- **Next step (1.3.0 follow-ups):** the NEEDS HUMAN rows of `docs/qa/ui-polish-checklist.md` (right-click and drag on Linux, macOS, Windows; icon crispness; Shift+F10); then section Plan 13 of `docs/superpowers/plans/2026-11-12-01-followups.md`. Open question from the author: FTPS download of a 6.7 GB file at 12 MB/s (61 ms RTT) — compare with `curl --ssl-reqd` before judging hoprs.
- **v1.3.1 released (2026-11-16):** tag `v1.3.1` on `cc3bd07` (PR #34), assets and checksums verified, site on 1.3.1. Plan 14: workspaces are plain groups in the sidebar (no Open button; Open all / Close all in the right-click menu and the Ctrl+K palette; the open workspace has a green mark), and a finished transfer re-lists its destination folder quietly (selection, scroll and hover kept; one listing per pane per 500 ms, longer after a slow listing; never starts or blocks a comparison). Screenshots not re-taken (only the workspace row changed). QA: `docs/qa/release-1.3.1-checklist.md`; follow-ups: section Plan 14 in `docs/superpowers/plans/2026-11-12-01-followups.md`.
- **v1.4.0 released (2026-11-17):** tag `v1.4.0` on `5ff3580` (PR #36), GitHub Release with `.deb`, `.dmg`, `.msi`, `.zip` and `SHA256SUMS` (checksums verified after download); the site switched to 1.4.0. It ships Plan 15 (update notice): the app checks GitHub's latest release (5 s after start, then every 24 h; Settings → General turns it off), shows a card once per version plus a footer line, and Update downloads the package, verifies it against `SHA256SUMS` and opens the system installer (integrity only, not authenticity). HTTP client: `ureq` on the existing rustls stack. Debug builds only: `HOPRS_UPDATE_URL`, `HOPRS_UPDATE_AS_VERSION`, `HOPRS_UPDATE_AS_PACKAGE`.
- **Next step (1.4.0 follow-ups):** the NEEDS HUMAN rows of `docs/qa/update-notice-checklist.md` (the author reports, 2026-11-17, that other people tested the app on macOS and Windows and it works; the in-app update itself is first exercised by 1.4.0 → 1.4.x); then section Plan 15 of `docs/superpowers/plans/2026-11-12-01-followups.md` (signing `SHA256SUMS` first). Build artifacts: `target/` grows fast (90 GB in a few days of agent builds) — clean it now and then, and never build under `/tmp` (RAM-backed on the author's machine).

## Decisions already made (do not re-litigate)

- Target platforms: **Linux (Ubuntu), Windows and macOS** (Plan 10 changed this; v1.0.0 was Linux-only). Tests must pass on all three (use `tempfile`, never literal `/tmp`). macOS and Windows builds are unsigned (ad-hoc signature only on macOS).
- GUI: **iced**. Runtime: **tokio**. SSH/SFTP: `russh` + `russh-sftp`. FTP/FTPS: `suppaftp`. Terminal: `alacritty_terminal`. File watching (remote editing): `notify`.
- Sites have a protocol: `ssh` (opens terminal, files via SFTP, or both in tabs, over one connection) or `ftp` / `ftps-explicit` / `ftps-implicit` (files). FTP is first-class, not an afterthought.
- **Workspace = a bundle of sites**; "Open workspace" connects all of them and restores tabs. A site can be in several workspaces.
- Credentials: OS keyring (Secret Service) + encrypted export/import (Argon2id + XChaCha20-Poly1305). Profile TOML never holds secrets. ssh-agent preferred.
- Import from FileZilla (`sitemanager.xml`) is in v1.
- UI language: **English and Brazilian Portuguese** (Fluent catalogs in `crates/hoprs-app/i18n/`; `en.ftl` is the reference, tests keep both in step). Visual: mockup v2 — neutral near-black, **white accent**, Inter for UI, JetBrains Mono for data (file names, paths, permissions), 4px radius, 1px rules, no shadows, no modal cascades (errors inline).
- Unencrypted FTP is allowed, labelled `ftp (unencrypted)`, no nagging.
- Terminal: Ctrl+K goes to the shell; the palette opens with Ctrl+Shift+K there. Copy/paste are Ctrl+Shift+C/V. OSC 52 (server writing the clipboard) is off.
- Remote-edit uploads go straight through `RemoteFs`, not the transfer queue. The perf job is separate and never a required PR check.

## Conventions

- Cargo workspace under `crates/`; only `hoprs-app` may depend on iced. Dependency graph is fixed in the spec §4.1.
- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` must be clean on every commit.
- `rust-toolchain.toml` pins the exact toolchain (`1.99.1`) for reproducible release builds; `scripts/package-deb.sh` builds the `.deb` with it. Bumping it is a deliberate change, in its own commit.
- Conversation with the author is in Portuguese (pt-BR); code, identifiers, UI strings and commit messages in English.
