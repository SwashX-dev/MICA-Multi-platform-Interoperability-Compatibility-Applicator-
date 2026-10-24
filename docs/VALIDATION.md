# Validation

What was built and executed for Portable Host v0, on what, and with which
results. Every run below happened on 2026-10-24. Nothing here is claimed from
WebAssembly's theoretical portability alone.

## Environment

| | |
| --- | --- |
| Machine | x86-64 laptop: AMD Ryzen 5 7520U, 8 threads, 16 GB, NixOS, Linux 6.18.56 |
| ARM64 | **No ARM64 hardware was available**: no ARM64 machine, no aarch64 remote builder, no binfmt emulation. ARM64 binaries ran under QEMU 11.1.2 user-mode emulation |
| Windows | **No Windows machine was used.** Windows binaries ran under Wine 11.0 in WoW64 mode on x86-64 Linux, in the Nix sandbox. Wine reimplements the Windows API on Linux; it is not Windows |
| Rust | rustc 1.99.1 (rust-overlay), targets x86_64, aarch64-unknown-linux-gnu, x86_64-pc-windows-gnu, wasm32-unknown-unknown |
| Libraries | Wasmtime 50.0.0, wit-bindgen 0.63.0, wit-component 0.262.0, sdl3 crate 0.20.0 on SDL 3.4.18 (for Windows, SDL 3.4.18 built with MinGW-w64 by nixpkgs) |
| Tools | wasm-tools 1.261.0, cross GCC 16.2.0 for ARM64 and for MinGW-w64 14.0.0, msitools 0.106 (`wixl`), Nix 2.34.9, nixpkgs `02c087f3` (nixos-unstable, 2026-10-20) |

## Definition of done

| Requirement | Result | Evidence |
| --- | --- | --- |
| A component built independently of the host | Met | `just guest` and `nix build .#guest` build it with no host code; see [Component](#component) |
| It runs in Wasmtime and responds to scripted input | Met | Three scripts reproduce their golden traces (860 ticks); see [x86-64](#x86-64-native) |
| The graphical host renders and takes keyboard input | Met | Real display (X11 and Wayland); real X key events on Xvfb; see [Graphical backend](#graphical-backend) |
| Headless output is deterministic and tested | Met | Fresh instances, a fresh engine, and repeated `render` calls give identical traces |
| x86-64 and ARM64 hosts run the exact same component | Met for x86-64 natively and ARM64 **under QEMU only** | Same file, same hash, byte-identical traces; see [Cross-platform](#cross-platform) |
| A Windows x86-64 host runs the same component | Met **under Wine only** | Test suite, golden replays, a window pixel-identical to Linux, real X key events; see [Windows](#windows-x86-64-under-wine) |
| A Windows installer (.msi) | Met **under Wine only**; unsigned | Installed, used, opened from the Start menu, and uninstalled; see [Installer](#installer) |
| Rebuild and test from documented commands | Met | `just test`, `just test-arm64`, and `nix flake check` (every check passed); see [Nix](#nix) and [Reproduce](#reproduce) |

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
no WASI. Rebuilding gives the same hash. The Nix-built component has a
different hash, `e5eded92…`, because two embedded standard-library paths
differ; it too rebuilds bit for bit. See [Nix](#nix). No embedded path contains
a home directory.

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

## Windows x86-64 (under Wine)

`nix build .#host-windows` cross-compiles the host on Linux for
`x86_64-pc-windows-gnu` with MinGW-w64 (GCC 16.2.0) and keeps the graphical
backend, linking the SDL 3.4.18 that nixpkgs builds for MinGW:

```text
portable-host.exe  PE32+ x86-64 console executable, 28082576 bytes
  imports          KERNEL32, msvcrt, ntdll, USERENV, WS2_32, bcryptprimitives,
                   api-ms-win-core-synch-l1-2-0, SDL3
SDL3.dll           5289815 bytes
  imports          ADVAPI32, GDI32, IMM32, KERNEL32, msvcrt, ole32, OLEAUT32,
                   SETUPAPI, SHELL32, USER32, VERSION, WINMM
```

Rust links the MinGW runtime (libgcc's unwinder, winpthreads) statically, so
the two files need only DLLs that ship with Windows. `nix build --rebuild`
rebuilt `portable-host.exe` bit for bit.

Everything in this section ran under **Wine 11.0 in WoW64 mode** on x86-64
Linux, in the Nix sandbox on the laptop or its x86-64 remote builder, each
check in a fresh Wine prefix. There the host reports `x86_64-windows`.

| Check | Under Wine | Result |
| --- | --- | --- |
| `hostWindowsTests` | `cargo test --target x86_64-pc-windows-gnu`, every test binary run through `wine` | **33 passed, 0 failed**: 18 host unit tests (the window-layout tests included, since this build has the graphical backend), the 8 diagnostics tests (a trap with the app's own report, fuel and memory limits), and the 7 replay tests, including all three golden traces |
| `crossArch` | `portable-host.exe replay`, every script | traces byte-identical to the Linux hosts' and to the golden files; see [Cross-platform](#cross-platform) |
| `windowsPlay` | `portable-host.exe play --script tour.script --exit-at-end --screenshot` on Xvfb, beside the Linux host doing the same | both played 300 ticks, ended on the golden final frame `e202d75e…`, and passed their own headless cross-check; the two 960x568 screenshots are **pixel-identical** (ImageMagick, 0 differing pixels) |
| `windowsKeyboard` | `tests/keyboard.nu`: real X key events from xdotool | 61 ticks of right, 31 of down, exactly 3 of left while paused, 18 idle after resuming; the replay of the recording ended on the same frame, `f4fc4532…`, where the Linux host's keyboard test also ended |
| `msiInstall` | the installer; see [Installer](#installer) | installed, used, and uninstalled |

SDL drew with its software renderer under Wine, as the Linux host did in the
sandbox, which has no GPU: SDL's Direct3D, OpenGL, and Vulkan renderers drew
nothing in these runs. Under Wine the diagnostics and replay suites took
0.05 s and 0.09 s, as long as the Linux host's: Wine runs the x86-64 code
directly, where QEMU translates ARM64 code.

## Installer

`nix build .#msi` builds `portable-host-0.1.0-x64.msi` on Linux with `wixl`
from msitools 0.106, from `packaging/windows/portable-host.wxs`: an x64,
per-machine (`ALLUSERS=1`) package that installs into
`C:\Program Files\Portable Host`, adds the Start-menu shortcut
Portable Host › Moving Square, registers for uninstall, and replaces an older
version (a major upgrade; the product code is derived from the version,
`{4C4BB675-219F-5031-8CBE-6D9FE4CAF4E8}` for 0.1.0). The package is
**unsigned**. It is not bit-for-bit reproducible: wixl stamps each build with a
random package code and the current time. The files it carries are the
reproducible ones above.

`tests/wine-msi.nu` (the `msiInstall` check) installed it under Wine with
`msiexec /i /qn`, then:

| Step | Result |
| --- | --- |
| Files | exactly the packaged files in `C:\Program Files\Portable Host`, each byte-identical to its source: `portable-host.exe` (`4a19868d…`), `SDL3.dll` (`5ff48c04…`), `moving-square.wasm` (`e5eded92…`), and `LICENSE.txt` |
| Uninstall entry | "Portable Host" 0.1.0 by MICA, under `HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\{4C4BB675-…}` |
| Shortcut | `Moving Square.lnk` targets `C:\Program Files\Portable Host\portable-host.exe` with the arguments `play "C:\Program Files\Portable Host\moving-square.wasm"`, in that directory (read back with `winedump`) |
| Replays | the installed host, with the installed component, reproduced all three golden traces |
| Shortcut opened | `start /unix`, the way Wine's own menu entries open shortcuts: a console window with the host's output and the game window appeared; the game window closed on Q |
| Uninstall | `msiexec /x /qn` removed every installed file and the shortcut |

Wine put the shortcut in the per-user Start menu
(`C:\users\nixbld\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Portable Host`).
For a per-machine package, Windows Installer documents the all-users Start
menu (`C:\ProgramData\Microsoft\Windows\Start Menu\Programs`) instead; that has
not been seen on Windows.

## Cross-platform

`tests/cross-arch.nu` replays every script on several hosts with one component
file and requires byte-identical traces. It ran twice: in the dev shell, with
both Linux release hosts and the dev-shell component (sha256
`b14292c8…6610`), and in the Nix sandbox (`cross-arch-report`), with all three
hosts and the Nix-built component (`e5eded92…`).

| Host | Binary reports | Executed | Runs |
| --- | --- | --- | --- |
| x86_64 | `x86_64-linux (portable-host 0.1.0, contract portable:platform@0.1.0)` | natively | both |
| aarch64 | `aarch64-linux (portable-host 0.1.0, contract portable:platform@0.1.0)` | QEMU user-mode emulation | both |
| windows | `x86_64-windows (portable-host 0.1.0, contract portable:platform@0.1.0)` | Wine 11.0 (WoW64), not Windows | Nix sandbox |

Every host reported the hash of the file it was given, and both runs produced
these traces:

| Script | Ticks | Trace sha256 | Result |
| --- | --- | --- | --- |
| corners | 530 | `3e9649f2fe580d7c4e4da52a6fa0ef158fa08cc27b57f26d72cde5b4d5069ee3` | identical on every host and to the golden trace |
| idle | 30 | `1242f182dffa275c31ef88aaa3f224cf21b7af597b79e9e7a05909db5dec4b35` | identical on every host and to the golden trace |
| tour | 300 | `406a75ed90b73ee1791e34ec961a9c29d8c7b2add3c4109fe6e624e294f57232` | identical on every host and to the golden trace |

## Nix

`nix flake check` passes every check at this change. Adding the Windows target
to the toolchain rebuilt every Rust derivation; those rebuilds and the Windows
checks ran over several invocations during this work, on the laptop and on
its x86-64 remote builder, and a last run at the final commit found every
check already built. Before the Windows work, a cold run of the earlier checks
took 19 min 35 s.

| Check | Result |
| --- | --- |
| `guest` | the component; `nix build --rebuild` rebuilt it bit for bit |
| `guestLogic` | 7 passed |
| `host` | builds with SDL3 and runs its test suite: 18 unit, 8 diagnostics, 7 replay passed |
| `clippy` | no warnings with `--deny warnings` |
| `keyboard` | Xvfb in the sandbox, SDL's software renderer (no GPU there): right 61, down 31, left 3 while paused, 18 idle after resuming; the replay of the recording ended on the same frame |
| `hostAarch64` | the headless ARM64 host, cross-compiled with the ARM64 GCC toolchain |
| `hostAarch64Tests` | its test suite under QEMU: 13 unit, 8 diagnostics, 7 replay passed |
| `hostWindows` | the Windows host with SDL3, cross-compiled with MinGW-w64; `nix build --rebuild` rebuilt it bit for bit |
| `hostWindowsTests` | its test suite under Wine: 18 unit, 8 diagnostics, 7 replay passed |
| `windowsPlay` | the Linux host and the Windows host under Wine play `tour.script` on Xvfb: both end on the golden frame, screenshots pixel-identical |
| `windowsKeyboard` | the keyboard test with the Windows host under Wine: right 61, down 31, left 3 while paused, 18 idle after resuming |
| `msi` | the installer, built by wixl |
| `msiInstall` | the installer under Wine: installed, files and shortcut checked, golden replays with the installed files, opened from the Start menu, uninstalled |
| `crossArch` | the x86-64 host natively, the ARM64 host under QEMU, and the Windows host under Wine, on the Nix-built component: every trace identical across hosts and to the golden files |

The Nix-built component:

```text
moving-square.wasm  sha256 e5eded92c168762c43b99e50db4d14c5cf43789b20ef444be8fa8a190b55c43d  29896 bytes
```

It differs from the dev-shell build in exactly two embedded source paths: the
dev shell's toolchain includes `rust-src` and records its local copy of two
standard-library files, where the Nix toolchain records the usual
`/rustc/<hash>/` paths. Behavior is the same: in the sandbox every host produced
the trace hashes listed under [Cross-platform](#cross-platform). Adding the
Windows target to the toolchain left the Nix-built component's hash unchanged.

## Not validated

- **ARM64 on real hardware.** Every ARM64 result above is under QEMU user-mode
  emulation on x86-64. QEMU translates instructions, so it shows the host and
  component behave identically under an AArch64 instruction stream, but not on
  real ARM64 silicon, memory ordering, or timing.
- **A graphical ARM64 host.** The cross build leaves out SDL3: a cross-built
  SDL3 is not in the binary cache. `packages.aarch64-linux.host` (with SDL3)
  evaluates but has never been built.
- **Windows itself.** Every Windows result above is under Wine 11.0 on Linux.
  Wine reimplements the Windows API (the loader, the kernel, the console,
  windowing, GDI, and Windows Installer), so these runs show that the binaries
  and the package are well-formed and that the host behaves exactly as on
  Linux under that reimplementation, not that Windows 10 or 11 runs them. Not
  seen in
  particular: Windows Installer itself (Wine's `msi.dll` stood in for it), the
  SmartScreen and administrator prompts for the unsigned package, where
  Windows puts the Start-menu shortcut (see [Installer](#installer)), drawing
  with Direct3D, OpenGL, or Vulkan on a real GPU, high-DPI displays, and how
  Windows Defender treats an unsigned program that compiles code at run time.
- **Signing.** Neither the MSI nor the executables are signed, so SmartScreen
  warns before installing (see the README).
- **Other Windows builds.** No MSVC build, no Windows on ARM64, no 32-bit
  build.
- **macOS.** Not attempted.
- **A person playing.** See [Graphical backend](#graphical-backend). On
  Windows, under Wine, the keyboard was driven by xdotool's synthetic X events.

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

Everything, in the Nix sandbox (on x86-64 this includes the ARM64 build and its
tests under QEMU, the Windows build, its tests, graphical runs, and installer
under Wine, and the cross-platform report):

```bash
nix flake check
```

The Windows host, the installer, and the reports behind the Windows results:

```bash
nix build .#host-windows .#msi
```

```bash
nix build .#cross-arch-report .#windows-play-report .#msi-install-report
```

Each report is a `report.md` with its evidence: traces, screenshots, the
installer's logs, and `winedump`'s reading of the shortcut.

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

## Validating on Windows

Copy `result/portable-host-0.1.0-x64.msi` (from `nix build .#msi`) to a
Windows 10 or 11 x64 machine, install it, and open Portable Host › Moving
Square from the Start menu. Then, from a command prompt in a folder holding
`tour.script` and `tour.trace` from `tests/replay`, the installed host checks
itself against the golden trace:

```text
"C:\Program Files\Portable Host\portable-host.exe" replay "C:\Program Files\Portable Host\moving-square.wasm" tour.script --expect tour.trace
```

The installed component's sha256 should read `e5eded92…`, and the trace's
`406a75ed…`, as under Wine.
