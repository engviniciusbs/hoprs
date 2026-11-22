# Contributing to hoprs

Thanks for wanting to help. hoprs is a small project with one maintainer, so this page tries
to make your first contribution land on the first try.

## What helps the most

These are easy to review and make a real difference:

- **Bug reports** with your OS, the hoprs version and the steps. The technical log (the
  "Log" button in the status bar) often shows the cause; paste the relevant lines (it never
  contains passwords or file contents).
- **Translations.** hoprs ships in English and Brazilian Portuguese; a new language is one
  text file. See [Translating hoprs](#translating-hoprs).
- **Packaging.** Homebrew, winget, Scoop, Flathub, the AUR, other distributions. The release
  assets have stable names; see the [roadmap](docs/ROADMAP.md).
- **Small fixes** from issues labelled
  [`good first issue`](https://github.com/engviniciusbs/hoprs/labels/good%20first%20issue) or
  [`help wanted`](https://github.com/engviniciusbs/hoprs/labels/help%20wanted).
- **Docs:** anything in the README that was unclear to you is worth a pull request.

For a **new feature or a large change**, open a
[discussion](https://github.com/engviniciusbs/hoprs/discussions) or an issue first and wait
for a reply before writing much code. hoprs has a deliberate scope and a fixed visual style
([decisions below](#decisions-already-made)); agreeing on the shape first saves you rework.

Security problems go through a private report, never a public issue: see
[SECURITY.md](SECURITY.md).

## Set up

You need:

- **Rust**, through [rustup](https://rustup.rs). `rust-toolchain.toml` pins the exact
  version (with `rustfmt` and `clippy`); the first `cargo` command fetches it.
- **A C toolchain and CMake** for the crypto dependencies:
  - Ubuntu / Debian: `sudo apt install build-essential cmake pkg-config`
  - macOS: `xcode-select --install` and `brew install cmake`
  - Windows: Visual Studio Build Tools (the "Desktop development with C++" workload),
    [CMake](https://cmake.org) and [NASM](https://www.nasm.us) on `PATH`
- **Docker** (optional) for the integration tests against real SSH and FTP servers.

```sh
git clone https://github.com/engviniciusbs/hoprs.git
cd hoprs
cargo run            # debug build; `cargo run --release` for the real speed
```

Your saved sites are real: a development build reads and writes the same profile and
transfer queue as an installed hoprs. To keep them apart on Linux, point it at scratch
folders:

```sh
XDG_CONFIG_HOME=/tmp/hoprs-dev/config XDG_DATA_HOME=/tmp/hoprs-dev/data cargo run
```

## Before you open a pull request

Every commit must pass these (CI checks them on Linux, macOS and Windows):

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

If you touched SSH, SFTP, FTP or the transfer engine, run the integration tests too:

```sh
docker compose -f docker/compose.yml up -d --build --wait
cargo test --workspace --all-features -- --ignored --test-threads=1
docker compose -f docker/compose.yml -p hoprs-test down -v
```

And please:

- **Add a test** for the bug you fix or the behaviour you add. Most of the app is tested
  without a window: see [the architecture guide](docs/ARCHITECTURE.md#testing).
- **Keep tests portable:** use `tempfile`, never a literal `/tmp`; never assume `/` as the
  path separator or LF line endings in files you read.
- **Never log secrets:** no password, passphrase, key, file contents or terminal bytes in
  logs, errors or panics.
- **Write in English:** code, identifiers, comments, commit messages and UI text (UI text
  goes through the catalogs, see below).
- **One topic per pull request.** Small pull requests get reviewed quickly.
- **Add a line to `CHANGELOG.md`** under the unreleased version when users would notice the
  change.

Commit messages are short and in the imperative (`fix(ftp): retry the data connection`,
`docs: explain the queue file`); the scope is the crate or area.

By contributing you agree that your work is licensed under MIT OR Apache-2.0, like the rest
of hoprs (see the [License](README.md#license) section of the README).

## Translating hoprs

All the text the app shows lives in [Fluent](https://projectfluent.org) files:

- `crates/hoprs-app/i18n/en.ftl`: English, the reference. Its `#` comments say where each
  text appears and what each variable is.
- `crates/hoprs-app/i18n/pt-BR.ftl`: Brazilian Portuguese.
- `crates/hoprs-app/i18n/tr.ftl`: Turkish.

**To improve a translation**, edit the `.ftl` file (the GitHub web editor is fine) and open a
pull request. Keep every `{ $variable }` name as it is, and keep the plural forms
(`[one]`, `*[other]`, …) that your language needs.

**To add a language**, copy `en.ftl` to `<tag>.ftl` (for example `es.ftl`), translate the
values, and open a pull request; say if you'd like someone else to do the Rust part. The
Rust part is small:

1. a `Language` variant and its tag in `crates/hoprs-app/src/i18n.rs`, with the file in the
   catalog list and the OS-locale mapping (`language_from_locale`);
2. an entry in the Settings language picker (`crates/hoprs-app/src/settings.rs`,
   `crates/hoprs-app/src/ui/settings.rs`).

Tests then check that your file has the same keys and variables as English, that it parses,
and that no value was left in English by accident (product names and symbols are on an
allowlist, `SAME_IN_BOTH`). Run `cargo test -p hoprs-app i18n` to see them.

Style: write the way your language's developers talk, not word for word. Keep product terms
that developers keep in English (SSH, SFTP, FTP, terminal, host, upload, download …), use
sentence case, and keep labels about as short as the English ones (buttons are narrow).

## Adding an icon

Add a line to `crates/hoprs-app/assets/fonts/phosphor-glyphs.txt` (`name U+XXXX`, from the
Phosphor Regular `style.css`), run `scripts/icons-subset.sh` (needs `pip install fonttools`),
and add the variant to `Icon` in `crates/hoprs-app/src/ui/icons.rs`. The tests check that
the font, the list and the enum agree.

## Decisions already made

These shape what gets merged. Proposals to change them are welcome in a discussion, but a
pull request that changes one silently will be asked to revert it.

- **Platforms:** Linux, macOS and Windows; tests pass on all three.
- **Stack:** iced for the GUI, tokio for async, russh for SSH/SFTP, suppaftp for FTP/FTPS,
  alacritty_terminal for the terminal. Only `hoprs-app` depends on iced, and the crate
  dependency graph is fixed (see [ARCHITECTURE.md](docs/ARCHITECTURE.md)).
- **Credentials** live in the OS keyring, never in the profile file. ssh-agent is preferred.
- **Visual style:** near-black, white accent, Inter for UI and JetBrains Mono for data, 4 px
  radius, 1 px rules, no shadows, no modal cascades (errors show inline).
- **Plain FTP is allowed,** labelled `ftp (unencrypted)`, with no nagging.

## Getting help

Ask in [Discussions](https://github.com/engviniciusbs/hoprs/discussions): questions, ideas
and "how do I…" are all welcome there. Everyone taking part follows the
[Code of Conduct](CODE_OF_CONDUCT.md); report problems to
[conduct@hoprs.online](mailto:conduct@hoprs.online).
