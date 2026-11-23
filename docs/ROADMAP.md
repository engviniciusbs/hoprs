# Roadmap

What is planned, roughly in order, and where help is most welcome. Plans change; the
[issues](https://github.com/engviniciusbs/hoprs/issues) and
[discussions](https://github.com/engviniciusbs/hoprs/discussions) are where they get decided.
Released work is in the [CHANGELOG](../CHANGELOG.md): 1.6.0 (2026-11-23) added the export of one
site or one workspace, as a partial file that restores with Merge, and an MCP server so coding
agents can work on the sites you allow (`hoprs mcp`, a level per site); 1.5.0 (2026-11-18) added the app in
Turkish (a contributed translation); 1.4.0 (2026-11-17) added the update
notice (a card and a footer line when a new version is out; Update downloads, verifies and
opens the system installer); 1.3.1 (2026-11-16) made workspaces
plain groups in the sidebar (Open all / Close all in their menu) and shows a finished
transfer in the pane without Refresh; 1.3.0 (2026-11-16) polished the interface (icons, Settings as a tab, right-click menus, a visible drag, a resizable compare
panel and queue); 1.2.0 (2026-11-16) added the app in Brazilian Portuguese (the language
follows the OS or is set in Settings) and folder compare (a panel lists what is new, changed
or missing on each side and transfers only the difference).

## Next

- **Sync on top of compare:** delete one-sided files, mirror a folder, compare by content
  or checksum (see the Plan 12 follow-ups).

## Later

- **More for agents:** an interactive terminal and folder compare for agents; finer limits than
  the site (folders, commands); a headless mode that works without the window.
- **Export to one of your own sites** (upload the encrypted file over SFTP or FTP).
- **Restore from Google Drive** (pick the file in hoprs, instead of downloading it first), and
  other clouds as export destinations. Uploads above 5 MB (a resumable upload).
- **Import `~/.ssh/config`:** hosts, users, ports, identity files.
- **Jump hosts (`ProxyJump`)** for servers behind a bastion.
- **Tunnels and port forwarding** saved with the site (for example a local port that reaches
  the server's database).
- **Signed builds** for macOS (Developer ID + notarisation) and Windows, so the first launch
  needs no "Open anyway" steps. Also sign `SHA256SUMS` (for example with minisign), so the
  in-app update can verify who published a release, not only that the download is intact.

## Help wanted

These don't need deep knowledge of the code. Open an issue or a discussion to say you are on
one, so nobody duplicates the work.

- **Translations:** a new language is one Fluent file; see
  [Translating hoprs](../CONTRIBUTING.md#translating-hoprs).
- **Packaging:** a Homebrew cask, winget and Scoop manifests, a Flatpak on Flathub, an AUR
  package, packages for other distributions. Release assets have stable names
  (`hoprs_<version>-1_amd64.deb`, `hoprs_<version>_macos_universal.dmg`,
  `hoprs_<version>_windows_x64.msi`, `hoprs_<version>_windows_x64.zip`) and a `SHA256SUMS`.
- **Windows on ARM64 and Linux on ARM64** builds.
- **Pageant** (PuTTY's agent) support on Windows; today only the OpenSSH agent works there.
- **Performance of many small files:** SFTP is about 2× slower than `lftp` on 10 000 small
  files (see [docs/perf/README.md](perf/README.md)).

## Not planned

- **Telemetry** of any kind in the app. hoprs collects no data. The only requests it makes on
  its own behalf are the update check to GitHub, which can be turned off (see
  [Updates](../README.md#updates)), and Google, only when you connect Google Drive, export
  there or disconnect it (see [Google Drive](../README.md#google-drive)).
