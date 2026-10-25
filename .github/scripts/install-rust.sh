#!/usr/bin/env bash
# Install $RUSTUP_TOOLCHAIN (and rustup itself, on runner images without it)
# for the rest of a GitHub Actions job.
#
#     bash .github/scripts/install-rust.sh [--target T] [--component C] ...
#
# Extra arguments go to `rustup toolchain install`.
set -euo pipefail

if ! command -v rustup >/dev/null 2>&1; then
    if [ "$RUNNER_OS" = Windows ]; then
        case "$RUNNER_ARCH" in
            ARM64) triple=aarch64-pc-windows-msvc ;;
            *) triple=x86_64-pc-windows-msvc ;;
        esac
        curl -sSfLo "$RUNNER_TEMP/rustup-init.exe" "https://static.rust-lang.org/rustup/dist/$triple/rustup-init.exe"
        "$RUNNER_TEMP/rustup-init.exe" -y --default-toolchain none --profile minimal
        bin=$(cygpath -w "$USERPROFILE/.cargo/bin")
    else
        curl -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none --profile minimal
        bin="$HOME/.cargo/bin"
    fi
    echo "$bin" >> "$GITHUB_PATH"
    export PATH="$HOME/.cargo/bin:$PATH"
fi
rustup toolchain install "$RUSTUP_TOOLCHAIN" --profile minimal "$@"
rustc --version --verbose
