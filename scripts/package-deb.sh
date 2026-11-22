#!/usr/bin/env bash
# Builds the hoprs Debian package and prints its path.
# Usage: scripts/package-deb.sh [--host]
#
# By default it builds inside an Ubuntu 24.04 image (docker/deb-build; needs docker and
# the network), so the binary links against glibc 2.39 and installs on Ubuntu 24.04 and
# later whatever the host runs. Output: target/deb-ubuntu-24.04/debian/.
#
# --host builds on this machine instead (for a CI runner that already is Ubuntu 24.04):
# installs cargo-deb (pinned) if missing, `cargo build --release --locked -p hoprs-app`,
# then `cargo deb -p hoprs-app --no-build`. It fails if the package would need a glibc
# newer than Ubuntu 24.04's. Output: target/debian/.
#
# Both modes remap the builder's paths ($HOME, $CARGO_HOME, the repository) out of the
# binary (panic locations, debug info). The toolchain is the one rust-toolchain.toml
# pins. HOPRS_GOOGLE_CLIENT_ID and HOPRS_GOOGLE_CLIENT_SECRET, when set, are passed into
# the build (the compiler bakes them in); without them the Drive destination is hidden.
# Package metadata: [package.metadata.deb] in crates/hoprs-app/Cargo.toml. Check
# the result with scripts/check-deb.sh <deb>.
set -euo pipefail

CARGO_DEB_VERSION=3.8.0
# glibc of Ubuntu 24.04: the package must not need a newer one.
MAX_GLIBC=2.39

repo=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo"

# The one place the toolchain version lives.
rust_version=$(sed -n 's/^channel *= *"\([^"]*\)".*/\1/p' rust-toolchain.toml)
if ! [[ "$rust_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "FAIL  rust-toolchain.toml must pin an exact version (got '$rust_version')" >&2
    exit 1
fi

case "${1:-}" in
    "")
        image=hoprs-deb-build:ubuntu-24.04-rust-$rust_version
        docker build -t "$image" \
            --build-arg RUST_VERSION="$rust_version" \
            --build-arg CARGO_DEB_VERSION="$CARGO_DEB_VERSION" \
            docker/deb-build >&2
        target=target/deb-ubuntu-24.04
        mkdir -p "$target/cargo-home"
        # The build runs every dependency's build script and proc-macro, so the container
        # gets no .git: an empty tmpfs hides it (a CI checkout may keep a token in
        # .git/config, and nothing in the build reads git). In a git worktree .git is a
        # file pointing outside the mount, so there is nothing to hide.
        hide_git=()
        if [ -d "$repo/.git" ]; then
            hide_git=(--tmpfs /src/.git)
        fi
        deb=$(docker run --rm --user "$(id -u):$(id -g)" \
            -v "$repo:/src" "${hide_git[@]}" -w /src \
            -e HOME=/tmp \
            -e CARGO_HOME="/src/$target/cargo-home" \
            -e CARGO_TARGET_DIR="/src/$target" \
            -e HOPRS_GOOGLE_CLIENT_ID -e HOPRS_GOOGLE_CLIENT_SECRET \
            "$image" scripts/package-deb.sh --host)
        echo "$repo/${deb#/src/}"
        exit 0
        ;;
    --host) ;;
    *)
        echo "usage: $0 [--host]" >&2
        exit 2
        ;;
esac

# rustc applies the last matching remap, so the most specific prefixes come last
# ($CARGO_HOME may live inside the repository, as in the container build).
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

if [ "$(cargo deb --version 2>/dev/null)" != "cargo-deb $CARGO_DEB_VERSION" ]; then
    cargo install cargo-deb --version "$CARGO_DEB_VERSION" --locked >&2
fi
cargo build --release --locked -p hoprs-app >&2
deb=$(cargo deb -p hoprs-app --no-build | tail -n 1)

# `$auto` derives the libc6 version from the glibc symbols the binary uses.
libc=$(dpkg-deb -f "$deb" Depends | sed -n 's/.*libc6 (>= \([^)]*\)).*/\1/p')
if [ -n "$libc" ] && dpkg --compare-versions "$libc" gt "$MAX_GLIBC"; then
    echo "FAIL  $deb needs glibc $libc; Ubuntu 24.04 has $MAX_GLIBC." >&2
    echo "      Build on Ubuntu 24.04, or without --host (in the Ubuntu 24.04 image)." >&2
    exit 1
fi
echo "$deb"
