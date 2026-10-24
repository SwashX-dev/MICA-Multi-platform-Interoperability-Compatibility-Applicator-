# Development commands for Portable Host. Run them inside `nix develop`.
# `just` with no arguments lists them.

set shell := ["nu", "-c"]

guest := "target/component/moving-square.wasm"

# List the available commands
default:
    @just --list

# Build the moving-square application component, independently of any host
guest:
    cargo build -p moving-square --target wasm32-unknown-unknown --profile guest
    mkdir target/component
    wasm-tools component new target/wasm32-unknown-unknown/guest/moving_square.wasm -o {{guest}}
    wasm-tools validate {{guest}}

# Run every test: guest logic, host units, golden replays, diagnostics
test: guest
    cargo test --workspace

# Play in a window: arrows/WASD move, P pause, Q quit (extra args go to `play`)
play *args: guest
    cargo run --release -p portable-host -- play {{guest}} {{args}}

# Replay a script headlessly and compare it with its golden trace
replay script="tests/replay/tour.script": guest
    cargo run --release -p portable-host -- replay {{guest}} {{script}} --expect ('{{script}}' | str replace '.script' '.trace')

# Print the component's hash, imports, exports, and declared info
inspect: guest
    cargo run --release -p portable-host -- inspect {{guest}}

# Drive the window with real key events on a private virtual X server
test-keyboard: guest
    cargo build --release -p portable-host
    xvfb-run -a nu tests/keyboard.nu target/release/portable-host {{guest}} target/keyboard-test

# Rewrite the golden traces after an intentional behaviour change
bless: guest
    BLESS=1 cargo test -p portable-host --test replay

# Cross-compile the headless host for ARM64 and run its tests under QEMU
test-arm64: guest
    cargo test -p portable-host --no-default-features --target aarch64-unknown-linux-gnu
