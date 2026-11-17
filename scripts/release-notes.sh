#!/usr/bin/env bash
# Prints the release notes of one version: the body of its `## <version>` section in
# CHANGELOG.md, up to the next `## ` heading, without the heading line and without
# leading or trailing blank lines; then, after a blank line, a fixed `## Installing`
# block naming that version's assets (Linux, macOS, Windows). The heading may carry a suffix after a space
# (`## 1.0.0 — 2026-11-13`); `1.0.0` never matches `## 1.0.0-rc1` or `## 1.0.01`.
# CRLF line endings are accepted (and stripped from the output).
#
# Usage: scripts/release-notes.sh <version> [changelog]   (default: CHANGELOG.md at the repo root)
#        scripts/release-notes.sh --self-test
#
# Exits non-zero with a message if the section is missing or empty.
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)

notes() {
    local version=$1 changelog=$2 body
    if [ ! -f "$changelog" ]; then
        echo "release-notes: $changelog not found" >&2
        return 1
    fi
    # Exit status 3: no such heading. Plain string comparisons, no regex: a version's
    # dots are not wildcards.
    body=$(awk -v want="## $version" '
        { sub(/\r$/, "") }
        /^## / {
            if (inside) exit
            if ($0 == want || index($0, want " ") == 1) { inside = 1; found = 1; next }
        }
        inside { print }
        END { if (!found) exit 3 }
    ' "$changelog") || {
        echo "release-notes: no '## $version' section in $changelog" >&2
        return 1
    }
    # Drop leading blank lines; $(...) already dropped the trailing ones.
    body=$(printf '%s\n' "$body" | sed '/[^[:space:]]/,$!d')
    if [ -z "$body" ]; then
        echo "release-notes: the '## $version' section in $changelog is empty" >&2
        return 1
    fi
    printf '%s\n' "$body"
}

# The fixed "Installing" block, for the assets of one version (the names of
# .github/workflows/release.yml).
installing() {
    local v=$1 deb_v
    # cargo-deb's version: the first `-` (a prerelease) becomes `~`, so 1.0.0-rc1 is
    # hoprs_1.0.0~rc1-1_amd64.deb (docker/deb-check/inside.sh maps it back).
    deb_v=${v/-/\~}
    cat <<EOF
## Installing

**Linux** (Ubuntu, amd64): download \`hoprs_${deb_v}-1_amd64.deb\`, then

    sudo apt install ./hoprs_${deb_v}-1_amd64.deb

**macOS** (11 or later, Apple silicon and Intel): open \`hoprs_${v}_macos_universal.dmg\` and drag hoprs to Applications. The app is not notarized, so macOS refuses the first open: go to System Settings → Privacy & Security and click Open Anyway, or run

    xattr -dr com.apple.quarantine /Applications/hoprs.app

After each update, Keychain asks again before hoprs can read its saved passwords; choose Always Allow.

**Windows** (x64): run \`hoprs_${v}_windows_x64.msi\`. It is not signed, so SmartScreen says "Windows protected your PC": click More info, then Run anyway. Or use the portable \`hoprs_${v}_windows_x64.zip\`: unzip it and run \`hoprs.exe\`.

If something is off on your system, please open an issue.
EOF
}

# The release notes: the CHANGELOG section, then the Installing block. Nothing is
# printed if the section is missing or empty.
release_notes() {
    local body
    body=$(notes "$1" "$2") || return 1
    printf '%s\n\n' "$body"
    installing "$1"
}

self_test() {
    local dir got fails=0
    dir=$(mktemp -d)
    # shellcheck disable=SC2064 # expand $dir now
    trap "rm -rf '$dir'" EXIT

    cat >"$dir/CHANGELOG.md" <<'EOF'
# Changelog

## 1.0.01 — 2026-12-02

Not 1.0.0 either.

## 1.0.0-rc1 — 2026-12-01

Release candidate, must not match 1.0.0.

## 1.0.0 — 2026-11-13

First release.

- Item one.
- Item two.

## 0.9.0

## 0.1.0 — 2026-01-01

Last section.
EOF
    printf '# Changelog\r\n\r\n## 2.0.0 — 2027-01-01\r\n\r\nWindows line.\r\n\r\n## 1.0.0\r\n\r\nOld.\r\n' \
        >"$dir/CRLF.md"

    expect_ok() { # name version file expected
        local got
        if got=$(notes "$2" "$3" 2>&1) && [ "$got" = "$4" ]; then
            echo "OK    $1"
        else
            echo "FAIL  $1: got [$got]"
            fails=$((fails + 1))
        fi
    }
    expect_fail() { # name version file
        local got
        if got=$(notes "$2" "$3" 2>&1); then
            echo "FAIL  $1: expected an error, got [$got]"
            fails=$((fails + 1))
        else
            echo "OK    $1 ($got)"
        fi
    }

    expect_ok "normal section, no heading, no other sections" 1.0.0 "$dir/CHANGELOG.md" \
        "$(printf 'First release.\n\n- Item one.\n- Item two.')"
    expect_ok "prefix collision: 1.0.0-rc1 is its own section" 1.0.0-rc1 "$dir/CHANGELOG.md" \
        "Release candidate, must not match 1.0.0."
    expect_ok "prefix collision: 1.0.01 is its own section" 1.0.01 "$dir/CHANGELOG.md" \
        "Not 1.0.0 either."
    expect_ok "last section, up to end of file" 0.1.0 "$dir/CHANGELOG.md" "Last section."
    expect_ok "CRLF line endings" 2.0.0 "$dir/CRLF.md" "Windows line."
    expect_ok "CRLF, last section" 1.0.0 "$dir/CRLF.md" "Old."
    expect_fail "missing version" 3.0.0 "$dir/CHANGELOG.md"
    expect_fail "prefix of a version is not a version" 1.0 "$dir/CHANGELOG.md"
    expect_fail "empty section" 0.9.0 "$dir/CHANGELOG.md"
    expect_fail "missing changelog file" 1.0.0 "$dir/nope.md"

    # The full notes: the CHANGELOG section first, then the Installing block for that
    # version.
    expect_contains() { # name version file needle...
        local name=$1 version=$2 file=$3 got needle
        shift 3
        if ! got=$(release_notes "$version" "$file" 2>&1); then
            echo "FAIL  $name: exited non-zero: [$got]"
            fails=$((fails + 1))
            return
        fi
        for needle in "$@"; do
            if ! grep -qF -- "$needle" <<<"$got"; then
                echo "FAIL  $name: no [$needle] in [$got]"
                fails=$((fails + 1))
                return
            fi
        done
        echo "OK    $name"
    }
    local full
    full=$(release_notes 1.0.0 "$dir/CHANGELOG.md" 2>&1) || true
    if [[ "$full" == "$(printf 'First release.\n\n- Item one.\n- Item two.\n\n## Installing\n')"* ]]; then
        echo "OK    full notes: the CHANGELOG section, then the Installing block"
    else
        echo "FAIL  full notes: section then Installing expected, got [$full]"
        fails=$((fails + 1))
    fi
    expect_contains "Installing block: Linux" 1.0.0 "$dir/CHANGELOG.md" \
        "sudo apt install ./hoprs_1.0.0-1_amd64.deb"
    expect_contains "Installing block: macOS" 1.0.0 "$dir/CHANGELOG.md" \
        "hoprs_1.0.0_macos_universal.dmg" "Applications" \
        "System Settings → Privacy & Security" "Open Anyway" \
        "xattr -dr com.apple.quarantine /Applications/hoprs.app" "Always Allow"
    expect_contains "Installing block: Windows" 1.0.0 "$dir/CHANGELOG.md" \
        "hoprs_1.0.0_windows_x64.msi" '"Windows protected your PC"' "More info" \
        "Run anyway" "hoprs_1.0.0_windows_x64.zip"
    # cargo-deb turns the first `-` of a prerelease into `~` (docker/deb-check/inside.sh).
    expect_contains "Installing block: a prerelease's asset names" 1.0.0-rc1 \
        "$dir/CHANGELOG.md" "sudo apt install ./hoprs_1.0.0~rc1-1_amd64.deb" \
        "hoprs_1.0.0-rc1_macos_universal.dmg" "hoprs_1.0.0-rc1_windows_x64.msi"
    expect_contains "Installing block: the issue line" 1.0.0 "$dir/CHANGELOG.md" \
        "If something is off on your system, please open an issue."
    if got=$(release_notes 3.0.0 "$dir/CHANGELOG.md" 2>/dev/null) || [ -n "$got" ]; then
        echo "FAIL  full notes: a missing section must fail and print nothing, got [$got]"
        fails=$((fails + 1))
    else
        echo "OK    full notes: a missing section fails and prints nothing"
    fi

    if [ "$fails" -ne 0 ]; then
        echo "release-notes self-test: $fails failed" >&2
        return 1
    fi
    echo "release-notes self-test: all passed"
}

case "${1:-}" in
    --self-test) self_test ;;
    "" | -*)
        echo "usage: $0 <version> [changelog] | --self-test" >&2
        exit 2
        ;;
    *) release_notes "$1" "${2:-$repo/CHANGELOG.md}" ;;
esac
