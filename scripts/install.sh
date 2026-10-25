#!/bin/sh
# Install the `mica` command for the current user on Linux or macOS.
#
#     curl -fsSL https://raw.githubusercontent.com/SwashX-dev/MICA-Multi-platform-Interoperability-Compatibility-Applicator-/main/scripts/install.sh | sh
#
# What it does, and nothing more:
#   1. picks the release build for this OS and CPU;
#   2. downloads it with the release's SHA256SUMS and refuses to install it
#      unless the checksum matches;
#   3. copies `mica` into ~/.local/bin (or $MICA_INSTALL_DIR).
# It never uses sudo and changes nothing outside that directory. When no
# release build exists for this system (or before the first release), it
# builds MICA from source with Cargo instead, if Cargo is installed.
#
# Settings (environment variables):
#   MICA_VERSION      a release tag such as v0.1.0 (default: the latest release)
#   MICA_INSTALL_DIR  where to put mica (default: ~/.local/bin)
#   MICA_FROM_SOURCE  1 = build from source even if a release build exists
#   MICA_BINARY       install this already-built mica instead of downloading
#   MICA_RELEASE_BASE download from here instead of GitHub releases (testing)

set -eu

REPO="SwashX-dev/MICA-Multi-platform-Interoperability-Compatibility-Applicator-"
INSTALL_DIR="${MICA_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf 'mica-install: %s\n' "$*"; }
fail() { printf 'mica-install: error: %s\n' "$*" >&2; exit 1; }

fetch() { # URL FILE
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL --retry 2 -o "$2" "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "$2" "$1"
    else
        fail "neither curl nor wget is installed"
    fi
}

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    else
        fail "no sha256sum or shasum to verify the download with"
    fi
}

install_binary() { # FILE
    mkdir -p "$INSTALL_DIR"
    cp "$1" "$INSTALL_DIR/mica.new"
    chmod 755 "$INSTALL_DIR/mica.new"
    mv "$INSTALL_DIR/mica.new" "$INSTALL_DIR/mica"
    say "installed $INSTALL_DIR/mica"
}

finish() {
    "$INSTALL_DIR/mica" --version || fail "the installed mica does not run on this system"
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) ;;
        *)
            say "$INSTALL_DIR is not on your PATH. Add it, then open a new terminal:"
            say "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.profile"
            ;;
    esac
    say "next: run \`mica doctor\` to see what this computer can run"
}

from_source() {
    say "building MICA from source with Cargo"
    if command -v rustup >/dev/null 2>&1; then
        # MICA pins its toolchain (rust-toolchain.toml); install that one.
        rustup toolchain install 1.99.1 --profile minimal >/dev/null
        cargo_cmd="rustup run 1.99.1 cargo"
    elif command -v cargo >/dev/null 2>&1; then
        cargo_cmd="cargo"
    else
        fail "no release build for this system, and Rust is not installed. Install Rust from https://rustup.rs (or your package manager), then run this again"
    fi
    root=$(dirname "$INSTALL_DIR")
    # shellcheck disable=SC2086
    $cargo_cmd install --locked --git "https://github.com/$REPO" --branch "${MICA_BRANCH:-main}" --root "$root" mica-cli \
        || fail "the source build failed (MICA needs Rust 1.99 or newer)"
    [ "$root/bin" = "$INSTALL_DIR" ] || install_binary "$root/bin/mica"
    finish
    exit 0
}

if [ -n "${MICA_BINARY:-}" ]; then
    [ -f "$MICA_BINARY" ] || fail "MICA_BINARY=$MICA_BINARY does not exist"
    install_binary "$MICA_BINARY"
    finish
    exit 0
fi

os=$(uname -s)
arch=$(uname -m)
case "$arch" in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) say "no release builds for $arch"; from_source ;;
esac
case "$os" in
    Linux) target="$arch-unknown-linux-musl" ;;
    Darwin) target="universal-apple-darwin" ;;
    *) fail "unsupported system $os; on Windows use scripts/install.ps1" ;;
esac

[ "${MICA_FROM_SOURCE:-0}" = 1 ] && from_source

version="${MICA_VERSION:-}"
if [ -z "$version" ] && [ -z "${MICA_RELEASE_BASE:-}" ]; then
    tmp_json=$(mktemp)
    if fetch "https://api.github.com/repos/$REPO/releases/latest" "$tmp_json" 2>/dev/null; then
        version=$(sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' "$tmp_json" | head -n 1)
    fi
    rm -f "$tmp_json"
fi
if [ -z "$version" ] && [ -z "${MICA_RELEASE_BASE:-}" ]; then
    say "no MICA release has been published yet"
    from_source
fi

base="${MICA_RELEASE_BASE:-https://github.com/$REPO/releases/download/$version}"
asset="mica-$target.tar.gz"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
say "downloading $asset ${version:+($version)}"
if ! fetch "$base/$asset" "$work/$asset"; then
    say "this release has no build for $target"
    from_source
fi
fetch "$base/SHA256SUMS" "$work/SHA256SUMS" || fail "could not download SHA256SUMS; not installing an unverified file"
expected=$(awk -v name="$asset" '$2 == name || $2 == "*" name { print $1 }' "$work/SHA256SUMS")
[ -n "$expected" ] || fail "SHA256SUMS does not list $asset; not installing an unverified file"
actual=$(sha256 "$work/$asset")
[ "$expected" = "$actual" ] || fail "checksum mismatch for $asset (expected $expected, got $actual); nothing was installed"
say "checksum verified ($actual)"
tar -xzf "$work/$asset" -C "$work"
[ -f "$work/mica" ] || fail "the archive does not contain mica"
install_binary "$work/mica"
finish
