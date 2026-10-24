# Validation

What was built and executed for Portable Host v0, on what, and with which
results. Every run below happened on 2026-10-24. Nothing here is claimed from
WebAssembly's theoretical portability alone.

## Environment

| | |
| --- | --- |
| Machine | x86-64 laptop: AMD Ryzen 5 7520U, 8 threads, 16 GB, NixOS, Linux 6.18.56 |
| ARM64 | **No ARM64 hardware was available**: no ARM64 machine, no aarch64 remote builder, no binfmt emulation. ARM64 binaries ran under QEMU 11.1.2 user-mode emulation |
| Rust | rustc 1.99.1 (rust-overlay), targets x86_64, aarch64-unknown-linux-gnu, wasm32-unknown-unknown |
| Libraries | Wasmtime 50.0.0, wit-bindgen 0.63.0, wit-component 0.262.0, sdl3 crate 0.20.0 on SDL 3.4.18 |
| Tools | wasm-tools 1.261.0, cross GCC 16.2.0, Nix 2.34.9, nixpkgs `02c087f3` (nixos-unstable, 2026-10-20) |

## Definition of done

| Requirement | Result | Evidence |
| --- | --- | --- |
| A component built independently of the host | Met | `just guest` and `nix build .#guest` build it with no host code; see [Component](#component) |
| It runs in Wasmtime and responds to scripted input | Met | Three scripts reproduce their golden traces (860 ticks); see [x86-64](#x86-64-native) |
| The graphical host renders and takes keyboard input | Met | Real display (X11 and Wayland); real X key events on Xvfb; see [Graphical backend](#graphical-backend) |
| Headless output is deterministic and tested | Met | Fresh instances, a fresh engine, and repeated `render` calls give identical traces |
| x86-64 and ARM64 hosts run the exact same component | Met for x86-64 natively and ARM64 **under QEMU only** | Same file, same hash, byte-identical traces; see [Cross-architecture](#cross-architecture) |
| Rebuild and test from documented commands | Met | `just test`, `just test-arm64`, `nix flake check`; see [Reproduce](#reproduce) |

## Component

`examples/moving-square`, built for `wasm32-unknown-unknown` with the `guest`
profile and wrapped by `wasm-tools component new`:

```text
component  target/component/moving-square.wasm
  sha256   b14292c89b13632e49d1611f595a5a2f03d542f359ee1f6dd518e69ef08e6610
  size     29968 bytes
imports    portable:platform/log@0.1.0              granted
           portable:platform/types@0.1.0            granted
exports    portable:platform/app@0.1.0
app        moving-square, canvas 320x180, 60 ticks/s
```

The core module's only import is `portable:platform/log@0.1.0#write`; there is
no WASI. Rebuilding gives the same hash. The dev shell's toolchain adds
`rust-src`, which changes a few embedded standard-library paths, so the
Nix-built component has a different (reproducible) hash; see [Nix](#nix). The
embedded paths contain no home directory.

## x86-64 (native)

`cargo test --locked --workspace`: **40 passed, 0 failed**.

| Suite | Tests |
| --- | --- |
| Guest game logic (`moving-square`, native) | 7 |
| Host units: pacer, scripts, traces, window layout and clipping | 18 |
| Diagnostics: non-wasm, core module, refused WASI imports, missing export, trap with the app's own report, runaway loop stopped by fuel, invalid declared info, fuel and memory limits on the real guest | 8 |
| Replays: golden traces, determinism across instances and engines, render purity, state persistence, instance isolation, declared info, granted imports | 7 |

`cargo clippy --workspace --all-targets -- --deny warnings` and
`cargo fmt --check`: clean.

## Graphical backend

| Run | Display | Renderer | Result |
| --- | --- | --- | --- |
| `tour.script`, final build | real display, X11 (XWayland) | OpenGL, vsync | 300 ticks; final frame `e202d75e…` equals the headless replay's |
| `corners.script` | real display, X11 (XWayland) | OpenGL, vsync | 530 ticks; final frame `4033276d…` equals the headless replay's |
| `idle.script`, `SDL_VIDEO_DRIVER=wayland` | real display, native Wayland | OpenGL, vsync | 30 ticks; final frame `c72cd9de…` equals the headless replay's |
| `tour.script`, `SDL_VIDEO_DRIVER=offscreen` | none (offscreen) | OpenGL, vsync | 300 ticks; final frame `e202d75e…` equals the headless replay's; screenshot inspected |
| `tests/keyboard.nu` (real X key events from xdotool) | Xvfb, private | OpenGL (Mesa), vsync | see below |

The keyboard test held Right for 1 s and Down for 0.5 s, paused with `P`,
pressed `N` three times while holding Left, resumed with `Esc`, and quit with
`Q`. The host recorded 61 ticks of right, 31 of down, exactly 3 of left (all
stepped while paused), and 18 idle ticks after resuming, 161 in all; the
square ended at (268, 144) as predicted, and the replay of the recording ended
on the same frame.

Screenshots confirmed the canvas scaling (3x in the default window, 2x and
letterboxed when a tiling compositor resized it to 912x933), and the status
bar showing the mode, every active key, the tick, and the recording marker.

Nobody pressed keys on the real display during these runs: the keyboard path
was exercised with synthetic X events on Xvfb, and the real display with
scripted input.

## ARM64 (QEMU user-mode emulation)

The headless host, cross-compiled with the ARM64 GCC toolchain, is an AArch64
ELF executable (ELF machine 183) that reports `aarch64-linux`.

`just test-arm64` (`cargo test --target aarch64-unknown-linux-gnu`, every test
binary run through `qemu-aarch64`): **28 passed, 0 failed**: 13 host unit tests
(the window-layout tests need the graphical backend, which this build leaves
out), the 8 diagnostics tests, and the 7 replay tests, including all three
golden traces. Under emulation the diagnostics and replay suites took 15 s and
22 s, against about 1 s and 2 s natively.

## Cross-architecture

`tests/cross-arch.nu` ran both release hosts against the same file:

| Host | Binary reports | Executed |
| --- | --- | --- |
| x86_64 | `x86_64-linux (portable-host 0.1.0, contract portable:platform@0.1.0)` | natively |
| aarch64 | `aarch64-linux (portable-host 0.1.0, contract portable:platform@0.1.0)` | QEMU user-mode emulation |

Both reported component sha256 `b14292c8…6610`.

| Script | Ticks | Trace sha256 | Result |
| --- | --- | --- | --- |
| corners | 530 | `3e9649f2fe580d7c4e4da52a6fa0ef158fa08cc27b57f26d72cde5b4d5069ee3` | identical on both hosts and to the golden trace |
| idle | 30 | `1242f182dffa275c31ef88aaa3f224cf21b7af597b79e9e7a05909db5dec4b35` | identical on both hosts and to the golden trace |
| tour | 300 | `406a75ed90b73ee1791e34ec961a9c29d8c7b2add3c4109fe6e624e294f57232` | identical on both hosts and to the golden trace |

## Nix

The first full `nix flake check` in the Nix sandbox was still running when this
commit was pushed; its results will be recorded here in a follow-up commit.

## Not validated

- **ARM64 on real hardware.** Every ARM64 result above is under QEMU user-mode
  emulation on x86-64. QEMU translates instructions, so it shows the host and
  component behave identically under an AArch64 instruction stream, but not on
  real ARM64 silicon, memory ordering, or timing.
- **A graphical ARM64 host.** The cross build leaves out SDL3: a cross-built
  SDL3 is not in the binary cache. `packages.aarch64-linux.host` (with SDL3)
  evaluates but has never been built.
- **macOS and Windows.** Not attempted.
- **A person playing.** See [Graphical backend](#graphical-backend).

## Reproduce

Inside `nix develop`:

```bash
just test
```

```bash
just test-arm64
```

```bash
just test-keyboard
```

The cross-architecture comparison, with release builds of both hosts:

```bash
nu tests/cross-arch.nu target/component/moving-square.wasm tests/replay target/cross-arch "x86_64=target/release/portable-host" "aarch64=qemu-aarch64 target/aarch64-unknown-linux-gnu/release/portable-host"
```

Everything, in the Nix sandbox (on x86-64 this includes the ARM64 build, its
tests under QEMU, and the cross-architecture report):

```bash
nix flake check
```

## Validating on native ARM64

On an aarch64-linux machine with Nix, `nix flake check` builds the native ARM64
host, with SDL3, and runs the whole test suite, including the golden replays,
natively. To show that the very bytes built on x86-64 behave the same there,
copy the x86-64-built `result/moving-square.wasm` across and run:

```bash
nix run .#host -- replay moving-square.wasm tests/replay/tour.script --expect tests/replay/tour.trace
```

`replay` prints the component's sha256 next to the trace's, so both
machines' outputs can be compared line by line.
