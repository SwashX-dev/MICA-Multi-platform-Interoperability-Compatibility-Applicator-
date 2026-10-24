# Architecture

Portable Host v0 tests one hypothesis: an application can describe *what* it
wants through a small, typed, versioned contract, and a native host can supply
*how* (windows, input devices, clocks, CPUs) without the application knowing or
caring. The moving square is only the instrument for testing that.

## Three boundaries

```
                 wit/platform.wit: portable:platform@0.1.0
 ┌──────────────────────────────┐            ┌────────────────────────────────────────┐
 │ moving-square.wasm           │  exports   │ portable-host (crates/host)            │
 │ one component, ~30 KB        │  app ─────▶│ runtime.rs   engine, linker, limits,   │
 │                              │            │              diagnostics: the          │
 │ game.rs  state + simulation  │  imports   │              authority boundary        │
 │ lib.rs   contract bindings   │◀───── log  │ headless.rs  script ──▶ trace          │
 └──────────────────────────────┘            │ sdl.rs       keyboard ──▶ window       │
   wasm32, no WASI, no SDL, no OS            │ pacer.rs     real time ──▶ whole ticks │
                                             └────────────────────────────────────────┘
                                               native: Linux x86-64, Linux ARM64
                                               (headless), Windows x86-64
```

**Portable application** (`examples/moving-square`). Owns its state, its
simulation, and the drawing commands that describe that state. It is built for
`wasm32-unknown-unknown` and wrapped into a component with `wasm-tools
component new`. Its core module has exactly one import, `log.write`; it cannot
name a file, a socket, a clock, or a window. The game logic (`game.rs`) uses only
the contract's data types, so it is unit-tested natively as well.

**Application contract** (`wit/platform.wit`). Three interfaces in the package
`portable:platform@0.1.0`:

| Interface | Direction | Contents |
| --- | --- | --- |
| `types` | data only | `buttons` (flags), `input`, `size`, `color`, `rect`, `draw-command`, `app-info` |
| `log` | host → app (import) | `write(level, message)`: output only |
| `app` | app → host (export) | `info() -> app-info`, `update(input)`, `render() -> list<draw-command>` |

**Native host** (`crates/host`). `runtime.rs` is the only module that touches
Wasmtime. Everything else is a backend that feeds an `App` input and consumes
its frames. The headless backend reads no clock and opens no display; the SDL
backend reads the keyboard, paces ticks against real time, and draws.

## Why the contract looks like this

- **`update` and `render` are separate.** The simulation advances in fixed ticks
  independent of the display rate; a 144 Hz monitor renders more often than the
  60 Hz simulation updates. `render` must not change state, and a test calls it
  repeatedly between updates to check that it does not.
- **The application declares its canvas and tick rate.** Both are application
  semantics: the game is written in logical pixels and in "pixels per tick". The
  host maps the logical canvas to the physical window (whole-number scale,
  letterboxed) and schedules ticks. Window size never reaches the simulation,
  which keeps replays identical on every display.
- **Input is the set of buttons held during a tick**, not a stream of key
  events. A snapshot per tick is trivially deterministic, replayable, and
  recordable, and the application can derive presses from changes. The cost: a
  tap shorter than one frame can go unseen, and text, pointers, and analog input
  need more vocabulary.
- **A frame is an immediate-mode list of drawing commands** (`clear`,
  `fill-rect`). Nothing is retained between frames, so the host needs no
  per-application GPU state and a frame is easy to record as text. The cost is
  bandwidth proportional to scene size; textures and meshes will need handles
  (WIT resources) owned by the host.
- **`log` is the only capability.** It is output only, so it cannot make two
  runs diverge, and it carries panic messages out of the sandbox: without it a
  Rust panic is an anonymous `unreachable` trap.
- **The component instance is the application instance.** Lifecycle is
  instantiate → `info` → `update`/`render` → drop. Several instances of one
  compiled component run fully isolated (tested). There is no suspend or
  shutdown hook yet, because nothing can be persisted without a storage
  capability.
- **The version is part of every name** (`portable:platform/app@0.1.0`). While
  the contract is 0.x, hosts match it exactly and any minor bump may break.

## Authority and isolation

The host grants authority only through imports, using Wasmtime's component
linker rather than any custom security machinery:

- A component that imports anything beyond `portable:platform/log` and the
  type-only `portable:platform/types` is refused before any of its code runs,
  with a list of everything it asked for (tested with a component importing
  `wasi:filesystem` and `wasi:sockets`).
- There is no WASI at all: the guest is built without it, and the host does not
  link it.
- Each call into the application gets a fuel budget (10 million units by
  default). Fuel counts executed instructions, so a runaway tick stops at the
  same instruction on every machine (tested with a guest that loops forever).
- Linear memory is capped (64 MiB by default) and growth beyond the cap traps.
- Errors name the application, the tick, and the cause, attach the wasm
  backtrace, and quote the last error the application logged.

## Determinism

- The guest simulates with integers only, in fixed ticks.
- The engine canonicalizes NaNs (x86-64 and ARM64 otherwise produce different
  NaN bit patterns), makes relaxed-SIMD results deterministic, and is built
  without the threads proposal.
- `Pacer` turns elapsed time into whole ticks with exact rational arithmetic,
  so the clock decides only *when* ticks run, never *what* they compute.
- Replays produce a text trace, one line per tick, whose SHA-256 is the
  fingerprint compared across runs, hosts, operating systems, and CPU
  architectures. Golden traces live in `tests/replay`.
- Every graphical session records its input, replays it headlessly in a fresh
  instance on exit, and fails loudly if the two runs end on different frames.

## Backends without a framework

A backend is a module that drives `App::update` and `App::render` from some
input source and does something with the frames. There are two today, headless
and SDL, plus the `Pacer` they could share. A third (a browser canvas, an XR
compositor) would be another module and another CLI subcommand. A backend trait
should appear when a second real-time backend actually shares the SDL loop, not
before.

## Known limits and tradeoffs

- **Exact interface names.** The authority check compares import names as
  strings. wit-component can already emit *canonicalized* names (`@0.1` plus a
  version suffix); a component built that way would be refused until matching is
  semver-aware.
- **Headless ARM64 host.** A cross-compiled SDL3 for ARM64 is not in the binary
  cache (373 derivations to build), so the cross host is built without the
  graphical backend. A native ARM64 machine builds the full host from
  `packages.aarch64-linux.host`.
- **Windows through MinGW.** The Windows host is cross-compiled from Linux for
  `x86_64-pc-windows-gnu` (MinGW-w64), not built with MSVC, and ships SDL3 as a
  DLL beside it. The same host code runs there unchanged; `std::env::consts`
  is the only place it learns the OS, to print it.
- **Level-triggered input** sampled once per rendered frame can miss very short
  taps.
- **Status-bar text** uses SDL's built-in 8x8 debug font: ASCII only.
- **Fuel** costs some speed in exchange for deterministic budgets.

## Where the contract goes next

The square answers the architectural question only for "draw rectangles and
read four buttons". Each next capability should arrive as a new interface the
host may or may not grant, so that an application's imports keep stating its
whole authority: deterministic randomness (a seeded source owned by the host),
pointer and text input, image resources owned by the host, audio, and storage,
which would bring suspend and resume.
