#!/usr/bin/env bash
# Builds the hoprs macOS disk image (universal: Apple silicon and Intel) and prints its
# absolute path on stdout; everything else goes to stderr.
# Usage: scripts/package-macos.sh   (on macOS, with Xcode's command line tools)
#
# HOPRS_GOOGLE_CLIENT_ID and HOPRS_GOOGLE_CLIENT_SECRET, when set in the environment, are
# inherited by the build (the compiler bakes them in); without them the Google Drive
# destination is hidden.
#
# 1. installs cargo-packager (pinned) if missing, and the two Apple targets for the
#    toolchain rust-toolchain.toml pins;
# 2. `cargo build --release --locked -p hoprs-app` for aarch64-apple-darwin and
#    x86_64-apple-darwin, deployment target macOS 11, with the builder's paths ($HOME,
#    $CARGO_HOME, the repository) remapped out of the binary, as package-deb.sh does;
# 3. `lipo -create` into target/universal-apple-darwin/release/hoprs;
# 4. cargo-packager makes hoprs.app from that binary ([package.metadata.packager] in
#    crates/hoprs-app/Cargo.toml: Info.plist, the .icns);
# 5. signs the .app ad hoc (`codesign --force --deep -s -`): no paid identity, no
#    notarization, so Gatekeeper asks once on first open;
# 6. hdiutil makes the compressed .dmg (the app and an Applications link):
#    target/packager/hoprs_<version>_macos_universal.dmg.
#
# cargo-packager's own dmg format is not used: it drives Finder through AppleScript to
# lay the window out, which is slow and flaky on a headless CI runner. Check the result
# with scripts/check-macos.sh <dmg>.
set -euo pipefail

CARGO_PACKAGER_VERSION=0.11.8
# Must match [package.metadata.packager.macos] minimum-system-version.
MACOS_MIN=11.0
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)

repo=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo"

if [ "$(uname -s)" != Darwin ]; then
    echo "FAIL  $0 runs on macOS only" >&2
    exit 1
fi
if [ $# -ne 0 ]; then
    echo "usage: $0" >&2
    exit 2
fi

rust_version=$(sed -n 's/^channel *= *"\([^"]*\)".*/\1/p' rust-toolchain.toml)
if ! [[ "$rust_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "FAIL  rust-toolchain.toml must pin an exact version (got '$rust_version')" >&2
    exit 1
fi
version=$(scripts/workspace-version.sh)

# Before RUSTFLAGS is set: the tool's own build needs no remapping.
if ! cargo packager --version 2>/dev/null | grep -qx "cargo-packager $CARGO_PACKAGER_VERSION"; then
    cargo install cargo-packager --version "$CARGO_PACKAGER_VERSION" --locked >&2
fi
if command -v rustup >/dev/null 2>&1; then
    rustup target add --toolchain "$rust_version" "${TARGETS[@]}" >&2
fi

# rustc applies the last matching remap, so the most specific prefixes come last.
cargo_home=${CARGO_HOME:-$HOME/.cargo}
# Appended to the caller's RUSTFLAGS, which are space-separated: paths with spaces
# cannot be passed that way, so they are refused.
for path in "$HOME" "$repo" "$cargo_home"; do
    if [[ "$path" == *" "* ]]; then
        echo "FAIL  path with a space, cannot be remapped through RUSTFLAGS: $path" >&2
        exit 1
    fi
done
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }--remap-path-prefix=$HOME=/build \
--remap-path-prefix=$repo=/hoprs --remap-path-prefix=$cargo_home=/cargo"
# The minimum macOS the binary loads on (LC_BUILD_VERSION minos), for both slices; the
# C code that build scripts compile reads it too.
export MACOSX_DEPLOYMENT_TARGET=$MACOS_MIN

target_args=()
for t in "${TARGETS[@]}"; do
    target_args+=(--target "$t")
done
cargo build --release --locked -p hoprs-app "${target_args[@]}" >&2

universal=target/universal-apple-darwin/release
mkdir -p "$universal"
slices=()
for t in "${TARGETS[@]}"; do
    slices+=("target/$t/release/hoprs")
done
lipo -create "${slices[@]}" -output "$universal/hoprs"
lipo -info "$universal/hoprs" >&2

# --target makes cargo-packager take the binary from target/<triple>/release. The out
# dir is absolute: cargo-packager works from the crate's directory.
out=$repo/target/packager
app_dir=$out/macos
rm -rf "$app_dir"
cargo packager -p hoprs-app --release --target universal-apple-darwin \
    --formats app --out-dir "$app_dir" >&2
app=$app_dir/hoprs.app
if [ ! -x "$app/Contents/MacOS/hoprs" ]; then
    echo "FAIL  cargo-packager did not produce $app/Contents/MacOS/hoprs" >&2
    exit 1
fi

# Extended attributes (Finder info, quarantine) make codesign refuse the bundle.
xattr -cr "$app"
codesign --force --deep -s - "$app" >&2
codesign --verify --deep --strict "$app" >&2

# The image's root: the app and a link to /Applications to drag it onto. ditto keeps
# the bundle exactly as signed.
root=$out/dmg-root
rm -rf "$root"
mkdir -p "$root"
ditto "$app" "$root/hoprs.app"
ln -s /Applications "$root/Applications"

dmg=$out/hoprs_${version}_macos_universal.dmg
rm -f "$dmg"
hdiutil create -volname hoprs -srcfolder "$root" -fs HFS+ -format UDZO -ov "$dmg" >&2
rm -rf "$root"
echo "$dmg"
