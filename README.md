# MICA

**Multi-platform Interoperability Compatibility Applicator** helps you inspect,
install, and run applications using the compatibility tools available on your
computer. Start with `mica doctor`; use `mica plan` to see how an application
would run before launching it.

MICA is an early **0.1.0 development version**. It coordinates native execution,
Wine/Proton, WSL, and other runtimes. It does not turn every application into a
cross-platform application, provide a Windows kernel, or bypass anti-cheat.

The original WebAssembly **Portable Host v0** remains a separate subsystem.
Its complete instructions, keys, replay format, Windows MSI, Nix outputs, and
application-writing guide are preserved in [Portable Host](docs/PORTABLE-HOST.md).

## What can I run?

| Your computer | Windows programs | Linux programs | macOS programs |
| --- | --- | --- | --- |
| Linux | Wine/Proton for compatible applications; a small Wine fixture was tested | Native when CPU and dependencies match | Unsupported |
| Windows 11 | Native; publisher requirements still apply | WSL; a Linux ELF launch was tested | Unsupported |
| macOS, Intel or Apple Silicon | CrossOver/Wine integration, experimental and untested on a physical Mac | Lima Linux VM integration, experimental and untested | Native; Intel apps on Apple Silicon may need Rosetta |

CPU compatibility is a separate requirement. ARM64 is not automatically able to
run every x86 program. FEX/Box64 discovery and translation plans are experimental.
VM support starts an existing libvirt guest; you still open the application
inside it. Cloud services are recommendations, not integrated streaming.
See the [support matrix](docs/COMPATIBILITY.md) and [test record](docs/VALIDATION.md).

**Fortnite is not available on Linux yet. Apex is blocked on Linux.** No
protected multiplayer game has been verified playable by this project.

## Install MICA

There is **no published MICA release yet**. The scripts try a release download
with SHA-256 verification, then build from source when no release exists.
Source builds need **Rust 1.99 or newer**, a linker, Git, and internet access;
the scripts use Rust **1.99.1** when rustup is installed. Installing MICA does
not install Wine, Steam, Windows, or games.

These instructions explicitly select `overhaul/mica-universal-compatibility`
because this update is not on `main`. Keep the checkout for updating and
uninstalling. Commands using example application paths need your own files.

**Validation labels:** Windows and WSL launch/install smoke tests are recorded
in [VALIDATION](docs/VALIDATION.md). Fresh-machine prerequisite installation,
macOS use, and Linux desktop gaming are **untested** unless explicitly recorded
there. A CI test of a container is not a test of its desktop or firmware.

### Windows 11 — PowerShell

Install [Rust](https://rustup.rs), [Git](https://git-scm.com/downloads/win), and
Visual Studio Build Tools with **Desktop development with C++** and a Windows
SDK. Restart PowerShell after installation. The following uses the tested
PowerShell installer; the full fresh-machine prerequisite sequence is untested.

```powershell
git clone --branch overhaul/mica-universal-compatibility https://github.com/SwashX-dev/MICA-Multi-platform-Interoperability-Compatibility-Applicator-.git MICA
cd MICA
$env:MICA_BRANCH = 'overhaul/mica-universal-compatibility'
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\install.ps1
```

Open a new terminal, then:

```powershell
mica --version
mica doctor
```

The installer places `mica.exe` in `%LOCALAPPDATA%\Programs\MICA` and adds it to
your user PATH. The process-local execution-policy option above allows the
downloaded script to run; review `scripts/install.ps1` first.

### Linux — APT, DNF, pacman, or Zypper

Install build prerequisites using **one** row for your distribution. These
fresh-machine package-install commands are **untested** here; repositories and
package availability depend on your distribution version.

| Distribution family | Command |
| --- | --- |
| Debian, Ubuntu, Kali (APT) | `sudo apt-get update && sudo apt-get install -y build-essential git curl ca-certificates` |
| Fedora (DNF) | `sudo dnf install -y gcc git curl ca-certificates` |
| Arch (pacman) | `sudo pacman -Syu --needed base-devel git curl ca-certificates` |
| openSUSE (Zypper) | `sudo zypper install gcc git curl ca-certificates` |

Install Rust using the instructions at [rustup.rs](https://rustup.rs). Then:

```sh
git clone --branch overhaul/mica-universal-compatibility https://github.com/SwashX-dev/MICA-Multi-platform-Interoperability-Compatibility-Applicator-.git MICA
cd MICA
MICA_BRANCH=overhaul/mica-universal-compatibility sh scripts/install.sh
export PATH="$HOME/.local/bin:$PATH"
mica --version
mica doctor
```

The installer uses `~/.local/bin`. Add that directory to your shell's PATH
configuration to keep it available in new terminals. For Bash, add
`export PATH="$HOME/.local/bin:$PATH"` to `~/.bashrc`.

There is no MICA APT/DNF/pacman/Zypper repository to add. Native package recipes
are in `packaging/`, and CI builds `.deb` and `.rpm` files; these are not yet
published releases. On immutable systems, use the distribution's supported
configuration mechanism; `mica setup` prints guidance instead of modifying
the OS image.

### macOS — Intel and Apple Silicon (untested on a physical Mac)

Install Apple's command-line developer tools with `xcode-select --install`,
then install Rust from [rustup.rs](https://rustup.rs). Use the same Git clone
and `sh scripts/install.sh` block above. Add `~/.local/bin` to `~/.zshrc` for
the default macOS shell. A source build targets your Mac's architecture;
the release workflow is intended to produce a universal binary.

### Nix — Linux and macOS

With Nix and flakes enabled, from the checkout:

```sh
nix run .#mica -- doctor
nix profile add .#mica
```

These new Nix outputs require validation; see the current status in
[VALIDATION](docs/VALIDATION.md). `nix run .` still starts the original
Portable Host demo on Linux. Use **`.#mica`** for the new command.

### Update

From a clean checkout on the overhaul branch:

```sh
git pull --ff-only
```

Then rerun your install command with `MICA_FROM_SOURCE=1` (PowerShell:
`$env:MICA_FROM_SOURCE = '1'`) and the same `MICA_BRANCH`. Source updates are
development snapshots, not stable releases. For a Nix profile, use
`nix profile list` to find its name and `nix profile upgrade NAME`.

## Run applications

### Linux: Windows programs

First preview the packages MICA recommends, then approve installation:

```sh
mica setup windows --dry-run
mica setup windows
mica runtimes
mica plan "/path/to/application.exe"
mica run "/path/to/application.exe" -- argument1 "argument with spaces"
```

An installer can create its own application in a Wine prefix (a private
directory holding that application's Windows files):

```sh
mica install "/path/to/setup.exe" --name "My App"
mica apps
mica run "My App"
```

Registration depends on the installer's shortcuts; check `mica apps` before
running by name. `.msi` is also recognized. The run → install → named run
sequence passed with the repository's fixture under Wine in WSL Kali; it is
not a promise for arbitrary installers. Graphics runtimes and drivers still
matter: `mica setup vulkan --dry-run` shows recommendations.

For a native Linux executable use `mica run "/path/to/linux-program"`.
There is no supported command to run a macOS `.app` on Linux.

### Windows 11: Linux programs

```powershell
mica setup linux --dry-run
```

If WSL is missing, follow the displayed instructions (`wsl --install` in an
administrator terminal, then restart and finish the distro setup). With a
Linux ELF executable stored on your Windows drive:

```powershell
mica inspect "C:\path\to\linux-program"
mica plan "C:\path\to\linux-program"
mica run "C:\path\to\linux-program" -- argument1
```

MICA uses your **default WSL distribution** and its installed libraries.
This path has been smoke-tested with an ELF, not a Linux game or graphical
desktop. Use absolute Windows drive paths; nondefault WSL mounts and
cross-distro UNC paths are not validated. macOS `.app` files are unsupported.

### macOS: Windows and Linux programs (experimental, untested)

Install [CrossOver](https://www.codeweavers.com/crossover) separately and create
a bottle called `MICA` in its interface. On Apple Silicon, install Rosetta when
macOS requests it. Then:

```sh
mica config set windows.runtime crossover
mica config set windows.bottle MICA
mica runtimes
mica plan "$HOME/Downloads/application.exe"
mica run "$HOME/Downloads/application.exe"
```

For Linux programs, install and configure [Lima](https://lima-vm.io/docs/),
including a running instance named `default`. `mica setup linux --dry-run`
shows installation guidance. Put the ELF in a directory shared at the same
absolute path inside that VM, with the correct CPU architecture and libraries:

```sh
mica plan "$HOME/Downloads/linux-program"
mica run "$HOME/Downloads/linux-program"
```

This runs inside Linux in a VM. It is not native Linux execution on macOS.
Native Mac apps use `mica run "/Applications/Example.app"`.

## Steam and desktop integration

On Linux, install Steam and the Proton version you want through Steam first.
These integrations have unit coverage; interactive Steam gameplay is untested.

```sh
mica steam status
mica games
mica compatibility fortnite
mica compatibility apex
```

In Steam → game Properties → General → Launch Options, enter:

```text
mica steam launch -- %command%
```

Steam must be able to find `mica` on PATH; otherwise use its absolute installed
path. MICA checks policy before forwarding Steam's command chain. It does not
change your Steam settings automatically.

Alternatively, `mica steam install-tool` adds an opt-in MICA compatibility
tool. Restart Steam and choose it per game under Compatibility. Remove it with
`mica steam remove-tool`, then restart Steam. MICA still needs an installed
Proton runtime. Do not combine both approaches without a reason.

On Linux, `mica desktop install` adds **Open with MICA**; `mica desktop remove`
removes it. Adding `--default` records and changes file associations. Desktop
behavior is untested outside unit fixtures. Disable library scanning with
`mica config set detect_games false`; restore it with `true`.

## Games with anti-cheat

Read the [beginner anti-cheat guide](docs/ANTI-CHEAT.md) before changing security
settings. Start with read-only commands:

```sh
mica compatibility fortnite
mica security check --game fortnite
mica security boot-profile
```

Fortnite's announced Linux path depends on Epic's EAC Linux v2 rollout; a ready
computer does not make the game available. Apex uses EA Javelin and EA blocks
Linux. EAC/BattlEye support under Proton is per game. Windows kernel anti-cheat
cannot run through Wine/CrossOver on macOS. MICA never chooses a VM automatically
for protected multiplayer or overrides an official prohibition. Dual boot into
a supported Windows installation or an officially supported cloud service may
be legitimate alternatives; see the guide for title-specific limits and sources.

## Troubleshooting

```sh
mica doctor
mica runtimes
mica config path
mica config show
mica plan "/path/to/program"
mica run "/path/to/program" --dry-run
```

`mica config path` prints the settings, data, and state locations, including
logs. Launch logs are under the printed state directory's `logs/` folder.
If a runtime is missing, preview `mica setup windows --dry-run`. If a program
fails, check its log and CPU/library requirements. A policy block is not fixed
by changing runtime or turning off security. Logs can contain file paths and
application output; redact private information before sharing them.

`mica bench --iterations 100` measures MICA operations, not game FPS. It does
not establish a performance advantage over Wine, Proton, or CrossOver.

## How it works

Inspection → host/runtime discovery → publisher-policy check → backend choice
→ launch plan → execution and local logs. The Rust crates separate binary
inspection, system probes, security readiness, compatibility evidence, runtime
integration, decisions, and the CLI. [Architecture](docs/ARCHITECTURE.md) explains
the boundaries; [research](docs/RESEARCH.md) explains the reused technologies.
Arbitrary EXEs are never treated as WebAssembly components.

## Uninstall

Run `mica uninstall` to review removal of MICA's integrations and data. Prefixes
can contain installed applications and saves; back up anything you want to keep.
If you created a Gaming Boot Profile, remove it **before removing the binary**
with `sudo mica security boot-profile --rollback` on that Linux system.

For a script installation, from the saved checkout:

```sh
sh scripts/uninstall.sh
```

Windows PowerShell:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\uninstall.ps1
```

Use the same `MICA_INSTALL_DIR` if you customized it. For a Nix installation,
use `nix profile list` and `nix profile remove NAME`; for a Cargo installation,
use `cargo uninstall mica-cli`. Package installations should be removed with
their package manager. MICA does not remove Wine, Steam, or other independently
installed runtimes. See [security and privacy](docs/SECURITY.md).

## Portable Host and developers

On Linux with Nix: `nix develop`, then `just test` and `just play`.
[The complete original guide](docs/PORTABLE-HOST.md) retains every Portable Host
workflow. `cargo build --locked -p mica-cli` builds only the new command;
plain `cargo build` still defaults to Portable Host.

BSD 3-Clause; see [LICENSE](LICENSE). Third-party runtimes retain their own licenses.
