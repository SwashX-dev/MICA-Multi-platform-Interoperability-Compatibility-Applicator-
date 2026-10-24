# MICA: Multi-platform Interoperability Compatibility Applicator

MICA explores running one application on any operating system and CPU
architecture. The application says *what* it wants through a typed, versioned
contract; a native host decides *how*.

This repository holds the first vertical slice, **Portable Host v0**. A small
game, a square you move around the screen, is compiled once into a 30 KB
WebAssembly component. That exact file runs:

- in a native **SDL3 window** with keyboard controls (`portable-host play`), and
- **headless**, from a script, producing a deterministic trace of every frame
  (`portable-host replay`),

on **x86-64** and on **ARM64** hosts. The ARM64 host has so far run only under
QEMU user-mode emulation; [docs/VALIDATION.md](docs/VALIDATION.md) records
exactly what was executed where.

## Quick start

Everything is pinned by the Nix flake. On Linux with Nix (flakes enabled):

```bash
nix develop
```

Then, inside the shell:

```bash
just test
```

```bash
just play
```

Without entering the shell, `nix run .` plays the game and `nix flake check`
builds and tests everything, including the ARM64 host under QEMU. Run `just`
to list every command.

### Keys

The window's status bar always shows the current mode and every key active in
it.

| Mode | Keys |
| --- | --- |
| NORMAL | Arrows or WASD move · `P` pause · `Q` quit |
| PAUSED | `N` steps one tick (with any movement keys held) · `P` or `Esc` resume · `Q` quit |
| ENDED (scripted sessions) | `Q` quit |

## Commands

| Command | What it does |
| --- | --- |
| `just guest` | Build the application component into `target/component/moving-square.wasm`, independently of any host |
| `just test` | Guest game logic, host unit tests, golden replays, and diagnostics tests |
| `just play` | Play in a window; extra arguments go to `portable-host play` (see below) |
| `just replay [SCRIPT]` | Replay a script headlessly and compare it with its golden trace |
| `just inspect` | Show the component's hash, imports (granted or refused), exports, and declared info |
| `just test-keyboard` | Drive the window with real X key events on a private virtual X server |
| `just test-arm64` | Cross-compile the headless host for ARM64 and run its whole test suite under QEMU |
| `just bless` | Rewrite the golden traces after an intentional behaviour change |

`portable-host` itself:

```text
portable-host play COMPONENT [--record FILE] [--script FILE [--exit-at-end]] [--screenshot PNG] [--scale N]
portable-host replay COMPONENT SCRIPT [--trace FILE] [--expect GOLDEN]
portable-host inspect COMPONENT
```

`play` and `replay` also take `--fuel UNITS` (per call) and `--max-memory MIB`.

Nix outputs:

| Output | Contents |
| --- | --- |
| `packages.<system>.guest` | The component, built reproducibly: `result/moving-square.wasm` |
| `packages.<system>.host` | The native host with the SDL3 backend; its build runs the full test suite |
| `packages.x86_64-linux.host-aarch64` | The headless host cross-compiled for ARM64 |
| `packages.x86_64-linux.cross-arch-report` | Both hosts replay every script; traces must be byte-identical (`result/report.md`) |
| `checks.<system>.*` | The above plus guest logic tests, clippy, the keyboard test, and the ARM64 test suite under QEMU |

## Layout

```text
wit/platform.wit            the contract: portable:platform@0.1.0
examples/moving-square/     the application (Rust → wasm32 → component)
  src/game.rs               state and simulation; unit-tested natively
  src/lib.rs                contract bindings and the panic reporter
crates/host/                portable-host
  src/runtime.rs            Wasmtime engine, granted imports, limits, diagnostics
  src/headless.rs           script in, trace out
  src/sdl.rs                window, keyboard, renderer, status bar
  src/pacer.rs              real time → fixed ticks
  src/script.rs             input scripts and recording
  src/trace.rs              text traces and fingerprints
  tests/                    golden replays, determinism, diagnostics
tests/replay/               input scripts and their golden traces
tests/keyboard.nu           end-to-end keyboard test (Xvfb + xdotool)
tests/cross-arch.nu         compares hosts on every script
docs/ARCHITECTURE.md        boundaries, contract design, tradeoffs
docs/VALIDATION.md          what was executed, on what, with which results
```

## Scripts and traces

A script lists which buttons are held from which tick on:

```text
ticks 300          # length of the run
0 right            # from tick 0, hold right
90 down            # from tick 90, hold down
250 -              # from tick 250, hold nothing
```

A trace has one line per tick, giving the input and the frame rendered after
it:

```text
0 right | clear #14161c; fill 154,82 16x16 #f2c14e
```

`play --record FILE` saves what you played as a script, and every `play`
session replays its own input headlessly on exit and checks that both runs end
on the same frame.

## Writing another application

Target the `application` world in `wit/platform.wit`: export `info`, `update`,
and `render`; import nothing but `log`. With Rust, copy
`examples/moving-square`, build it for `wasm32-unknown-unknown`, and wrap the
result with `wasm-tools component new`, as `just guest` does. Any language
whose toolchain produces components works the same way. The host refuses a
component that imports anything else, such as WASI, and `portable-host inspect`
shows why.

## Status

See [docs/VALIDATION.md](docs/VALIDATION.md) for exactly what was built and
run, and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the design and its
known limits. In short: the contract, the component, both backends, the
headless replays, and the ARM64 host all work; ARM64 has been exercised only
under QEMU, and the ARM64 host has no graphical backend yet.

### Working with JJ

Nix flakes built from a Git checkout see only files Git tracks. In a JJ working
copy, new files reach Git once their change is committed (`jj new`, or
`jj squash` into a described change), after which `nix build .#…` sees them.
Avoid `path:.` here: it copies the whole directory, `target/` included.

## License

BSD 3-Clause; see [LICENSE](LICENSE).
