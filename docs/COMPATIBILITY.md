# Compatibility and support matrix

MICA 0.1.0 is a development build. **Implemented** means code exists, not that
every application works. **Smoke-tested** refers to a specific fixture or
command. **Experimental** means the execution path lacks end-to-end validation.
The [validation record](VALIDATION.md) distinguishes native Windows, WSL, Wine,
containers, and emulation. No protected multiplayer title is verified playable.

| Host → target | Implementation | Evidence and limits |
| --- | --- | --- |
| Windows → Windows | Direct launch | Native Windows CLI and executable inspection tested; games unverified |
| Linux → Linux, matching CPU | Direct launch | Linux mica binary exercised in WSL Kali; normal Linux libraries still required |
| macOS → macOS | Direct Mach-O launch / `open` for bundles | Parser fixtures; physical Mac launch untested |
| Linux → Windows | umu, Proton, Wine discovery and command construction | Wine fixture run/install/named run passed in WSL Kali; Proton gameplay unverified |
| Windows → Linux | `wsl.exe --exec` | ELF smoke test; default distro and standard drive mounts assumed; GUI/games untested |
| macOS → Windows | CrossOver bottle / Wine / discovered GPTK evaluation runtime | Experimental; no physical Mac, kernel drivers unsupported |
| macOS → Linux | `limactl shell default --` | Experimental; existing default guest, shared path, matching architecture and guest libraries required |
| Linux ARM64 → x86 Linux | FEX/Box64 | Experimental discovery/launch plans, not a validated ARM gaming stack |
| Apple Silicon → Intel macOS | Rosetta | Experimental MICA integration; depends on Apple's support window |
| Linux → Windows VM | libvirt readiness/creation plan and start of configured domain | Experimental; application launch inside guest remains manual; no GPU passthrough automation |
| Windows/Linux → macOS | No backend | Unsupported; recognizing Mach-O does not implement macOS frameworks |
| Any host → supported cloud title | Provider information | No streaming client, authentication or automatic remote launch |
| Supported host → Portable Host component | Separate `portable-host` executable | Existing replay/diagnostic suite; only components implementing MICA's WIT contract |

MICA identifies PE, ELF, Mach-O, universal Mach-O, `.app`, WebAssembly, MSI and
Windows shortcuts. Import names and nearby files provide clues about graphics
APIs and anti-cheat, not complete runtime dependency resolution. macOS binary
plist bundles, unusual installer layouts and proprietary launchers can require
manual handling. An arbitrary Wasm module is not a compatible WIT component.

## Operating systems and distribution plans

| Platform | Install/setup path | Validation scope |
| --- | --- | --- |
| Windows 11 x86-64 | PowerShell source/release installer, WSL instructions | Real Windows local tests; clean-machine setup not validated |
| Windows ARM64 | CI target and native ARM runner | See CI results; not physical ARM hardware validation |
| Debian/Ubuntu/Kali | Shell installer; APT runtime plans | Kali under WSL tested; container checks verify package names only |
| Fedora | Shell installer; DNF plans | CI container checks, not boot/security or desktop validation |
| Arch | Shell installer; pacman plans and PKGBUILD | CI checks; repository availability varies |
| openSUSE | Shell installer; Zypper plans | CI checks for Leap and Tumbleweed; no physical desktop test |
| NixOS / Nix | `.#mica`, `.#checks.SYSTEM.micaTests` | New flake outputs require successful evaluation/build; recorded separately |
| Immutable Linux | Manual supported image configuration | No silent host-image mutation |
| Non-systemd Linux | General inspection and compatible runtimes | EAC Linux v2's systemd-service prerequisite is unsatisfied |
| macOS Intel / Apple Silicon | Source installer; planned universal release | CI is separate from actual Mac application/game testing |

## Policy and evidence

Database statuses: `verified-playable`, `partially-functional`, `unverified`,
`blocked-by-anti-cheat`, `blocked-by-publisher`, and `unsupported`.
Authorization (`authorized`, `per-game`, `planned`, `forbidden`, `unknown`) is
separate from test status. A vendor-authorized Windows route can still be
unverified by MICA. Tests must include environment, date, reproducible evidence,
and a completed protected session for multiplayer claims.

Fortnite's Linux path is planned, Apex's Linux route is blocked, and Vanguard
does not work under Wine/Proton or in VMs. EAC and BattlEye require per-title
opt-in. See [ANTI-CHEAT](ANTI-CHEAT.md) for sources and supported alternatives.
Official prohibitions cannot be overridden by a local compatibility file.
MICA never automatically selects a VM for protected multiplayer; a configured
explicit route still requires publisher authorization and cannot override a ban.

## What remains future work

Physical Linux/macOS/ARM application tests; representative game and graphics
benchmarks; authorized multiplayer validation; vendor service integration;
signed release provenance; broader installer registration; complete VM guest
integration and remote streaming. GPU passthrough needs hardware-specific
planning, a Windows license and recovery procedures. It is not an automatic
or negligible-overhead fallback in this release.
