#!/usr/bin/env bash
# Builds the hoprs Windows installer (.msi) and portable archive (.zip) and prints their
# absolute paths on stdout, one per line, the .msi first (forward slashes, C:/..., which
# both bash and PowerShell read); everything else goes to stderr.
# Usage: scripts/package-windows.sh   (on Windows, in Git Bash, with the MSVC build tools)
#
# HOPRS_GOOGLE_CLIENT_ID and HOPRS_GOOGLE_CLIENT_SECRET, when set in the environment, are
# inherited by the build (the compiler bakes them in); without them the Google Drive
# destination is hidden.
#
# 1. installs cargo-packager (pinned) if missing, and the x86_64-pc-windows-msvc target
#    for the toolchain rust-toolchain.toml pins;
# 2. `cargo build --release --locked -p hoprs-app --target x86_64-pc-windows-msvc`, with
#    the builder's paths ($HOME, $CARGO_HOME, the repository) remapped out of the binary,
#    as package-deb.sh does, and trimmed by cl.exe from the C code's __FILE__ strings
#    (aws-lc); the C runtime linked statically (no VC++ redistributable needed); and the
#    linker told to record only the .pdb's file name (by default it embeds the .pdb's
#    full path, which is inside the repository);
# 3. cargo-packager's `wix` format makes the per-machine .msi from
#    [package.metadata.packager] in crates/hoprs-app/Cargo.toml, with two overrides:
#    the MSI version is the numeric X.Y.Z of the workspace version (an .msi has no
#    pre-release part: anything from the first `-` is dropped), and the WiX template is
#    packaging/windows/main.wxs with the fixed upgrade code of [package.metadata.msi]
#    filled in. cargo-packager has no command-line switch for either, so the
#    configuration is passed whole, as JSON, built from `cargo metadata`;
#    target/packager/windows/hoprs_<version>_windows_x64.msi;
# 4. the .zip holds hoprs.exe, LICENSE-MIT, LICENSE-APACHE, README.txt and licenses/ (the
#    bundled fonts' licences):
#    target/packager/windows/hoprs_<version>_windows_x64.zip.
#
# WiX 3 (candle, light) is downloaded by cargo-packager itself, checksum-verified. Nothing
# is signed: SmartScreen asks once (README.txt says how to get past it). Check the result
# with scripts/check-windows.ps1 -Msi <msi> -Zip <zip>.
set -euo pipefail

CARGO_PACKAGER_VERSION=0.11.8
TARGET=x86_64-pc-windows-msvc
HOMEPAGE=https://hoprs.online

repo=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo"

case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*) ;;
    *)
        echo "FAIL  $0 runs on Windows (Git Bash) only" >&2
        exit 1
        ;;
esac
if [ $# -ne 0 ]; then
    echo "usage: $0" >&2
    exit 2
fi
for tool in cygpath jq pwsh; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "FAIL  $tool not found (needed by $0)" >&2
        exit 1
    fi
done

rust_version=$(sed -n 's/^channel *= *"\([^"]*\)".*/\1/p' rust-toolchain.toml)
if ! [[ "$rust_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "FAIL  rust-toolchain.toml must pin an exact version (got '$rust_version')" >&2
    exit 1
fi
version=$(scripts/workspace-version.sh)
msi_version=${version%%-*}
if ! [[ "$msi_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "FAIL  no numeric X.Y.Z in the workspace version '$version'" >&2
    exit 1
fi

# The package's metadata, as cargo-packager itself would read it. jq on Windows may end
# its lines with CRLF: the CRs are dropped.
metadata=$(cargo metadata --format-version 1 --no-deps --locked |
    jq -c '.packages[] | select(.name == "hoprs-app") | .metadata' | tr -d '\r')
upgrade_code=$(jq -r '.msi["upgrade-code"] // empty' <<<"$metadata" | tr -d '\r')
if ! [[ "$upgrade_code" =~ ^[0-9A-F]{8}-([0-9A-F]{4}-){3}[0-9A-F]{12}$ ]]; then
    echo "FAIL  [package.metadata.msi] upgrade-code must be an upper-case GUID (got '$upgrade_code')" >&2
    exit 1
fi

# Before RUSTFLAGS is set: the tool's own build needs no remapping.
if ! cargo packager --version 2>/dev/null | grep -qx "cargo-packager $CARGO_PACKAGER_VERSION"; then
    cargo install cargo-packager --version "$CARGO_PACKAGER_VERSION" --locked >&2
fi
if command -v rustup >/dev/null 2>&1; then
    rustup target add --toolchain "$rust_version" "$TARGET" >&2
fi

# rustc sees Windows paths (C:\Users\...), so the prefixes are given in that form. It
# applies the last matching remap, so the most specific prefixes come last.
home_w=$(cygpath -w "$HOME")
repo_w=$(cygpath -w "$repo")
cargo_home_w=$(cygpath -w "${CARGO_HOME:-$HOME/.cargo}")
# Appended to the caller's RUSTFLAGS, which are space-separated: paths with spaces
# cannot be passed that way, so they are refused.
for path in "$home_w" "$repo_w" "$cargo_home_w"; do
    if [[ "$path" == *" "* ]]; then
        echo "FAIL  path with a space, cannot be remapped through RUSTFLAGS: $path" >&2
        exit 1
    fi
done
# +crt-static: the C runtime is linked in (cc then builds the C code, aws-lc, with /MT),
# so hoprs.exe imports no VCRUNTIME140.dll and runs without the VC++ redistributable.
# /PDBALTPATH:%_PDB%: the executable names its .pdb by file name only.
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }--remap-path-prefix=$home_w=/build \
--remap-path-prefix=$repo_w=/hoprs --remap-path-prefix=$cargo_home_w=/cargo \
-Ctarget-feature=+crt-static -Clink-arg=/PDBALTPATH:%_PDB%"
# The C code built by cc (aws-lc-sys, under CARGO_HOME) is out of rustc's reach: its
# __FILE__ strings (aws-lc's error records) would keep the full source paths. cl.exe
# trims them with -d1trimfile:<prefix>\ (most specific prefix first); the `-` form,
# because MSYS would rewrite a value starting with `/` as a path.
export CFLAGS_x86_64_pc_windows_msvc="${CFLAGS_x86_64_pc_windows_msvc:+$CFLAGS_x86_64_pc_windows_msvc }\
-d1trimfile:$cargo_home_w\\ -d1trimfile:$repo_w\\ -d1trimfile:$home_w\\"

cargo build --release --locked -p hoprs-app --target "$TARGET" >&2
bin_dir=$repo/target/$TARGET/release
if [ ! -f "$bin_dir/hoprs.exe" ]; then
    echo "FAIL  cargo build did not produce $bin_dir/hoprs.exe" >&2
    exit 1
fi

out=$repo/target/packager/windows
rm -rf "$out"
mkdir -p "$out/wix"
template=$out/wix/main.wxs
sed "s/@UPGRADE_CODE@/$upgrade_code/g" packaging/windows/main.wxs >"$template"
if ! grep -q "UpgradeCode=\"$upgrade_code\"" "$template"; then
    echo "FAIL  the upgrade code did not reach $template" >&2
    exit 1
fi

# cargo-packager's configuration: [package.metadata.packager] as is, plus what it would
# otherwise take from cargo metadata (name, binary) or from the package version, and the
# filled-in template. Relative paths in it are from crates/hoprs-app, where it runs.
# Paths are given as C:/... (cygpath -m), which Windows programs read and MSYS leaves
# alone.
config=$(jq -c \
    --arg version "$msi_version" \
    --arg target "$TARGET" \
    --arg binary "$(cygpath -m "$bin_dir/hoprs")" \
    --arg out "$(cygpath -m "$out")" \
    --arg template "$(cygpath -m "$template")" \
    '.packager
     + { name: "hoprs-app", version: $version, targetTriple: $target,
         binaries: [{ path: $binary, main: true }], outDir: $out, formats: ["wix"] }
     | .wix.template = $template' <<<"$metadata" | tr -d '\r')
(cd crates/hoprs-app && MSYS2_ARG_CONV_EXCL='*' cargo packager --config "$config" >&2)

built=$out/hoprs_${msi_version}_x64_en-US.msi
if [ ! -f "$built" ]; then
    echo "FAIL  cargo-packager did not produce $built" >&2
    exit 1
fi
msi=$out/hoprs_${version}_windows_x64.msi
mv "$built" "$msi"

# The portable archive. README.txt in CRLF, for any Windows editor.
root=$out/zip-root
mkdir -p "$root"
cp "$bin_dir/hoprs.exe" LICENSE-MIT LICENSE-APACHE "$root/"
mkdir -p "$root/licenses"
cp crates/hoprs-app/assets/fonts/OFL-Inter.txt crates/hoprs-app/assets/fonts/OFL-JetBrainsMono.txt \
    crates/hoprs-app/assets/fonts/LICENSE-Phosphor.txt "$root/licenses/"
sed 's/$/\r/' >"$root/README.txt" <<EOF
hoprs $version for Windows (64-bit), portable

hoprs is a desktop client for SSH, SFTP, FTP and FTPS: saved sites and
workspaces, a terminal, local and remote files side by side, and a transfer
queue that resumes.

Running it
  Extract the archive anywhere and double-click hoprs.exe. Nothing is
  installed; to remove it, delete the folder. For a Start menu entry and
  upgrades, use hoprs_${version}_windows_x64.msi from the same release instead.

The first start ("Windows protected your PC")
  hoprs is not code-signed, so Microsoft Defender SmartScreen may stop the
  first start. Click "More info", then "Run anyway". It asks only once.

Your data
  Settings, sites and workspaces are kept in %APPDATA%\\hoprs, and passwords
  in Windows Credential Manager; the .msi install uses the same ones. Deleting
  this folder or uninstalling never touches them.

Homepage: $HOMEPAGE
License: MIT OR Apache-2.0 (LICENSE-MIT, LICENSE-APACHE). Font licences: licenses\\.
EOF

zip=$out/hoprs_${version}_windows_x64.zip
pwsh -NoProfile -NonInteractive -Command \
    "\$ErrorActionPreference = 'Stop'; Compress-Archive -Path '$(cygpath -w "$root")\\*' -DestinationPath '$(cygpath -w "$zip")'" >&2
rm -rf "$root"

cygpath -m "$msi"
cygpath -m "$zip"
