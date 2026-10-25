//! Choosing an execution backend.
//!
//! For a program and a host, the engine lists every backend that could run
//! it, scores the ones that can, and explains each one it rejects. It prefers
//! the least translation that works: a native build, then CPU translation
//! the OS provides (Rosetta 2, Windows on ARM), then an API compatibility
//! layer (Proton/Wine, WSL), then a virtual machine, then remote play. A
//! virtual machine is never chosen automatically for a game behind
//! anti-cheat, and remote play is offered, never chosen.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Serialize;

use mica_compat::{AntiCheat, Authorization, Database, Game, Route};
use mica_inspect::{AntiCheatScan, Arch, Format, GraphicsApi, Installer, Kind, Platform, Report};
use mica_probe::{HostProfile, Os};
use mica_runtime::steam::CompatTool;
use mica_runtime::{Config, PreferredRuntime, RuntimeKind, Runtimes};

use crate::policy::{self, RouteVerdict};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    Native,
    Rosetta,
    PortableHost,
    Dos,
    WindowsCompat,
    Wsl,
    ArchTranslation,
    LinuxVm,
    VirtualMachine,
    Remote,
}

impl Backend {
    pub const ALL: [Backend; 10] = [
        Backend::Native,
        Backend::Rosetta,
        Backend::PortableHost,
        Backend::Dos,
        Backend::WindowsCompat,
        Backend::Wsl,
        Backend::ArchTranslation,
        Backend::LinuxVm,
        Backend::VirtualMachine,
        Backend::Remote,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Backend::Native => "native",
            Backend::Rosetta => "rosetta",
            Backend::PortableHost => "portable-host",
            Backend::Dos => "dos",
            Backend::WindowsCompat => "windows",
            Backend::Wsl => "wsl",
            Backend::ArchTranslation => "arch-translation",
            Backend::LinuxVm => "linux-vm",
            Backend::VirtualMachine => "vm",
            Backend::Remote => "remote",
        }
    }

    pub fn parse(id: &str) -> Option<Backend> {
        Backend::ALL.into_iter().find(|backend| backend.id() == id)
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Backend::Native => "native execution",
            Backend::Rosetta => "native execution under Rosetta 2",
            Backend::PortableHost => "Portable Host (WebAssembly component)",
            Backend::Dos => "DOSBox",
            Backend::WindowsCompat => "Windows compatibility layer (Proton/Wine)",
            Backend::Wsl => "Windows Subsystem for Linux",
            Backend::ArchTranslation => "CPU translation (FEX-Emu/Box64)",
            Backend::LinuxVm => "Linux virtual machine (Lima)",
            Backend::VirtualMachine => "Windows virtual machine",
            Backend::Remote => "remote play (cloud streaming)",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlockerKind {
    /// A runtime that is not installed yet.
    MissingRuntime,
    /// The program's CPU architecture cannot run here this way.
    Architecture,
    /// The publisher or anti-cheat vendor does not permit this route.
    Policy,
    /// It cannot work for technical or legal reasons.
    Technical,
    /// It needs setting up first (a virtual machine).
    NotConfigured,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Blocker {
    pub kind: BlockerKind,
    pub message: String,
    /// `--backend` may run it anyway (MICA still alters nothing).
    pub overridable: bool,
}

/// The concrete runtime a candidate would use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Runner {
    Direct,
    PortableHost {
        path: PathBuf,
    },
    DosBox {
        path: PathBuf,
    },
    Umu {
        path: PathBuf,
        proton: Option<CompatTool>,
    },
    Proton {
        tool: CompatTool,
    },
    Wine {
        path: PathBuf,
    },
    Crossover {
        path: PathBuf,
        bottle: String,
    },
    GamePortingToolkit {
        path: PathBuf,
    },
    Wsl {
        path: PathBuf,
    },
    Translator {
        path: PathBuf,
        name: String,
    },
    Lima {
        path: PathBuf,
    },
    Libvirt {
        domain: String,
        uri: String,
    },
    Cloud {
        providers: Vec<String>,
    },
}

impl fmt::Display for Runner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Runner::Direct => f.write_str("directly"),
            Runner::PortableHost { path } => write!(f, "portable-host ({})", path.display()),
            Runner::DosBox { path } => write!(f, "DOSBox ({})", path.display()),
            Runner::Umu {
                proton: Some(tool), ..
            } => {
                write!(f, "umu-launcher with {}", tool.display_name)
            }
            Runner::Umu { proton: None, .. } => f.write_str("umu-launcher with UMU-Proton"),
            Runner::Proton { tool } => {
                write!(f, "{} (Steam's Proton, run directly)", tool.display_name)
            }
            Runner::Wine { path } => write!(f, "Wine ({})", path.display()),
            Runner::Crossover { bottle, .. } => write!(f, "CrossOver, bottle \"{bottle}\""),
            Runner::GamePortingToolkit { .. } => {
                f.write_str("Apple Game Porting Toolkit (evaluation)")
            }
            Runner::Wsl { .. } => f.write_str("wsl.exe"),
            Runner::Translator { name, .. } => f.write_str(name),
            Runner::Lima { .. } => f.write_str("Lima"),
            Runner::Libvirt { domain, uri } => write!(f, "libvirt domain \"{domain}\" ({uri})"),
            Runner::Cloud { providers } => write!(f, "{}", providers.join(", ")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Candidate {
    pub backend: Backend,
    pub route: Option<Route>,
    pub runner: Option<Runner>,
    pub score: u32,
    pub reasons: Vec<String>,
    pub warnings: Vec<String>,
    pub blockers: Vec<Blocker>,
    /// What to install or configure to make it available.
    pub setup: Vec<String>,
    pub policy: Option<RouteVerdict>,
}

impl Candidate {
    fn new(backend: Backend, route: Option<Route>, score: u32) -> Candidate {
        Candidate {
            backend,
            route,
            runner: None,
            score,
            reasons: Vec::new(),
            warnings: Vec::new(),
            blockers: Vec::new(),
            setup: Vec::new(),
            policy: None,
        }
    }

    pub fn eligible(&self) -> bool {
        self.blockers.is_empty()
    }

    /// Every blocker may be overridden from the command line.
    pub fn overridable(&self) -> bool {
        self.blockers.iter().all(|blocker| blocker.overridable) && self.runner.is_some()
    }

    fn block(&mut self, kind: BlockerKind, message: impl Into<String>, overridable: bool) {
        self.blockers.push(Blocker {
            kind,
            message: message.into(),
            overridable,
        });
    }
}

/// A program to run, with everything known about it.
#[derive(Clone, Debug, Serialize)]
pub struct Target {
    pub path: PathBuf,
    pub report: Report,
    pub scan: Option<AntiCheatScan>,
    pub game: Option<Game>,
    pub steam_app_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Decision {
    pub candidates: Vec<Candidate>,
    /// Index into `candidates`.
    pub chosen: Option<usize>,
    pub forced: bool,
    pub anticheats: Vec<AntiCheat>,
    pub notes: Vec<String>,
}

impl Decision {
    pub fn chosen(&self) -> Option<&Candidate> {
        self.chosen.map(|index| &self.candidates[index])
    }
}

pub struct Inputs<'a> {
    pub host: &'a HostProfile,
    pub runtimes: &'a Runtimes,
    pub database: &'a Database,
    pub config: &'a Config,
}

/// Inspect a file, look for anti-cheat beside it, and match it to the
/// compatibility database.
pub fn analyze(path: &Path, database: &Database) -> Result<Target, mica_inspect::InspectError> {
    analyze_with(path, database, mica_inspect::Options::QUICK)
}

/// [`analyze`] with a chosen inspection depth (`mica inspect` reads more and
/// hashes the file).
pub fn analyze_with(
    path: &Path,
    database: &Database,
    options: mica_inspect::Options,
) -> Result<Target, mica_inspect::InspectError> {
    let report = mica_inspect::inspect_path_with(path, options)?;
    let scan = if report.platform == Platform::Windows && report.kind == Kind::Application {
        scan_game_dirs(path)
    } else {
        None
    };
    let steam_app_id = steam_app_for(path);
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let game = steam_app_id
        .as_deref()
        .and_then(|id| database.find_by_steam_app(id))
        .or_else(|| database.find_by_executable(&file_name))
        .cloned();
    Ok(Target {
        path: path.to_path_buf(),
        report,
        scan,
        game,
        steam_app_id,
    })
}

/// Scan the program's directory and up to three parents for anti-cheat,
/// stopping at a Steam library's `common` directory or a home or drive root.
fn scan_game_dirs(path: &Path) -> Option<AntiCheatScan> {
    let mut merged: Option<AntiCheatScan> = None;
    let mut dir = path.parent();
    for _ in 0..4 {
        let Some(current) = dir else { break };
        let name = current
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let parent_name = current
            .parent()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if name.is_empty()
            || name == "common"
            || name == "downloads"
            || name == "desktop"
            || parent_name == "home"
            || parent_name == "users"
        {
            break;
        }
        if let Ok(scan) = mica_inspect::anticheat::scan_dir(current) {
            let found = !scan.signals.is_empty();
            match &mut merged {
                Some(existing) => {
                    for signal in scan.signals {
                        if !existing.signals.contains(&signal) {
                            existing.signals.push(signal);
                        }
                    }
                    existing.eac_proton_library |= scan.eac_proton_library;
                    existing.entries_scanned += scan.entries_scanned;
                }
                None => merged = Some(scan),
            }
            if found {
                break;
            }
        }
        dir = current.parent();
    }
    merged
}

/// The Steam app ID of a program inside `.../steamapps/common/<dir>/`.
pub fn steam_app_for(path: &Path) -> Option<String> {
    let components: Vec<_> = path.components().collect();
    let index = components.windows(2).position(|pair| {
        pair[0].as_os_str().eq_ignore_ascii_case("steamapps")
            && pair[1].as_os_str().eq_ignore_ascii_case("common")
    })?;
    let install_dir = components
        .get(index + 2)?
        .as_os_str()
        .to_string_lossy()
        .into_owned();
    let steamapps: PathBuf = components[..=index].iter().collect();
    for entry in std::fs::read_dir(&steamapps).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("appmanifest_") {
            continue;
        }
        let text = std::fs::read_to_string(entry.path()).ok()?;
        let document = mica_runtime::vdf::parse(&text).ok()?;
        let state = document.get("AppState")?;
        if state.get("installdir").and_then(|value| value.text()) == Some(install_dir.as_str()) {
            return state
                .get("appid")
                .and_then(|value| value.text())
                .map(str::to_string);
        }
    }
    None
}

fn arch_list(report: &Report) -> String {
    report
        .architectures
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Decide how to run `target`. `forced` is the user's `--backend`.
pub fn decide(inputs: &Inputs, target: &Target, forced: Option<Backend>) -> Decision {
    let host = inputs.host;
    let report = &target.report;
    let game = target.game.as_ref();
    let mut candidates = Vec::new();
    let mut notes = Vec::new();
    let anticheats = policy::anticheats(game, target.scan.as_ref());

    match (&report.format, report.platform) {
        (Format::WasmComponent, _) => candidates.push(portable_host(inputs)),
        (Format::Legacy { .. }, _) => candidates.push(dos(inputs, report)),
        (_, Platform::Windows) => match host.os {
            Os::Windows => candidates.push(native_windows(host, report)),
            Os::Linux => {
                candidates.push(windows_compat_linux(inputs, target));
                candidates.push(virtual_machine(inputs, target));
            }
            Os::MacOs => {
                candidates.push(windows_compat_macos(inputs, target));
                candidates.push(macos_vm(inputs, target));
            }
            Os::Other => {}
        },
        (_, Platform::Linux) => match host.os {
            Os::Linux => {
                if report.installer.is_some() {
                    let mut candidate =
                        Candidate::new(Backend::Native, Some(Route::LinuxNative), 0);
                    candidate.block(
                        BlockerKind::Technical,
                        "this is a Linux package: install it with `mica install`, which uses your \
                         package manager",
                        false,
                    );
                    candidates.push(candidate);
                } else {
                    candidates.push(native_elf(inputs, report));
                    candidates.push(arch_translation(inputs, report));
                }
            }
            Os::Windows => candidates.push(wsl(inputs, report)),
            Os::MacOs => candidates.push(linux_vm(inputs)),
            Os::Other => {}
        },
        (_, Platform::MacOs) => {
            let mut candidate = match host.os {
                Os::MacOs => native_macos(inputs, report),
                _ => {
                    let mut candidate =
                        Candidate::new(Backend::Native, Some(Route::MacosNative), 0);
                    candidate.block(
                        BlockerKind::Technical,
                        "macOS programs need macOS: Apple's license allows macOS only on Apple \
                         hardware, and no maintained compatibility layer runs macOS applications \
                         on Linux or Windows (Darling remains experimental)",
                        false,
                    );
                    candidate
                }
            };
            if report.installer.is_some() && host.os == Os::MacOs {
                candidate
                    .reasons
                    .push("open the disk image or package to install it".into());
            }
            candidates.push(candidate);
        }
        (Format::Script { interpreter }, _) => {
            let mut candidate = Candidate::new(Backend::Native, None, 80);
            if host.os == Os::Windows {
                candidate.block(
                    BlockerKind::Technical,
                    format!("a Unix script (interpreter {interpreter}); run it inside WSL"),
                    false,
                );
            } else {
                candidate.runner = Some(Runner::Direct);
                candidate
                    .reasons
                    .push(format!("a script for {interpreter}"));
            }
            candidates.push(candidate);
        }
        (Format::WasmModule, _) => {
            let mut candidate = Candidate::new(Backend::PortableHost, None, 0);
            candidate.block(
                BlockerKind::Technical,
                "a core WebAssembly module, not a portable:platform component; use a WASI runtime \
                 such as wasmtime",
                false,
            );
            candidates.push(candidate);
        }
        (_, Platform::AppleMobile) => {
            let mut candidate = Candidate::new(Backend::Native, None, 0);
            candidate.block(
                BlockerKind::Technical,
                "an iOS/iPadOS program: only Apple Silicon Macs run some of these, through the App Store",
                false,
            );
            candidates.push(candidate);
        }
        _ => {
            let mut candidate = Candidate::new(Backend::Native, None, 0);
            candidate.block(
                BlockerKind::Technical,
                format!(
                    "MICA does not recognise this file as a program ({:?})",
                    report.format
                ),
                false,
            );
            candidates.push(candidate);
        }
    }

    if let Some(game) = game {
        if let Some(cloud) = game.route(Route::Cloud)
            && cloud.authorization == Authorization::Authorized
            && !cloud.providers.is_empty()
        {
            let mut candidate = Candidate::new(Backend::Remote, Some(Route::Cloud), 5);
            candidate.runner = Some(Runner::Cloud {
                providers: cloud.providers.clone(),
            });
            candidate.reasons.push(format!(
                "{} names these cloud services: {}. The game runs on their computers and \
                     streams to yours; it is not local compatibility.",
                game.publisher,
                cloud.providers.join(", ")
            ));
            candidates.push(candidate);
        }
        if game.eac_linux_v2.is_some() && host.os == Os::Linux {
            notes.push(format!(
                "{} is Epic's pilot for EAC Linux v2. Run `mica compatibility {}` to check this \
                 system against Epic's published requirements.",
                game.name, game.id
            ));
        }
        notes.extend(game.notes.iter().cloned());
    }

    candidates.retain(|candidate| {
        candidate.score > 0 || !candidate.blockers.is_empty() || candidate.runner.is_some()
    });
    // Eligible first, then by score; the remote option never outranks a
    // local one.
    candidates.sort_by(|a, b| {
        (b.eligible(), b.backend != Backend::Remote, b.score).cmp(&(
            a.eligible(),
            a.backend != Backend::Remote,
            a.score,
        ))
    });

    let (chosen, forced_used) = match forced {
        Some(backend) => {
            let index = candidates
                .iter()
                .position(|candidate| candidate.backend == backend);
            match index {
                Some(index) if candidates[index].eligible() || candidates[index].overridable() => {
                    if !candidates[index].eligible() {
                        notes.push(format!(
                            "Running with {backend} because you asked to. MICA changes nothing in \
                             the program or its anti-cheat; expect what the blockers above describe."
                        ));
                    }
                    (Some(index), true)
                }
                Some(_) => {
                    notes.push(format!(
                        "{backend} cannot be used for this program; see its blockers. Some \
                         blockers rest on an official publisher statement and cannot be overridden."
                    ));
                    (None, true)
                }
                None => {
                    notes.push(format!(
                        "{backend} does not apply to this kind of program on {}.",
                        host.os
                    ));
                    (None, true)
                }
            }
        }
        None => (
            candidates
                .iter()
                .position(|candidate| candidate.eligible() && candidate.backend != Backend::Remote),
            false,
        ),
    };
    Decision {
        candidates,
        chosen,
        forced: forced_used,
        anticheats,
        notes,
    }
}

fn portable_host(inputs: &Inputs) -> Candidate {
    let mut candidate = Candidate::new(Backend::PortableHost, None, 100);
    candidate
        .reasons
        .push("a portable:platform WebAssembly component: the same file runs on every host".into());
    match inputs.runtimes.first(RuntimeKind::PortableHost) {
        Some(runtime) => {
            candidate.runner = Some(Runner::PortableHost {
                path: runtime.path.clone(),
            })
        }
        None => {
            candidate.block(
                BlockerKind::MissingRuntime,
                "portable-host is not on PATH",
                true,
            );
            candidate.setup.push(
                "Build it from this repository: cargo build --release -p portable-host (needs \
                 SDL3), or nix build .#host"
                    .into(),
            );
        }
    }
    candidate
}

fn dos(inputs: &Inputs, report: &Report) -> Candidate {
    let mut candidate = Candidate::new(Backend::Dos, None, 90);
    candidate.reasons.extend(report.notes.iter().cloned());
    match inputs.runtimes.first(RuntimeKind::DosBox) {
        Some(runtime) => {
            candidate.runner = Some(Runner::DosBox {
                path: runtime.path.clone(),
            })
        }
        None => {
            candidate.block(BlockerKind::MissingRuntime, "DOSBox is not installed", true);
            candidate
                .setup
                .push("Install DOSBox Staging (https://www.dosbox-staging.org) or dosbox from your package manager".into());
        }
    }
    candidate
}

fn native_windows(host: &HostProfile, report: &Report) -> Candidate {
    let mut candidate = Candidate::new(Backend::Native, Some(Route::WindowsNative), 100);
    candidate.runner = Some(Runner::Direct);
    let arches = &report.architectures;
    match (host.arch, arches.first().copied()) {
        (_, Some(Arch::X86))
        | (Arch::X86_64, Some(Arch::X86_64))
        | (Arch::Aarch64, Some(Arch::Aarch64 | Arch::Arm64ec)) => {
            candidate
                .reasons
                .push("a Windows program for this CPU".into());
        }
        (Arch::Aarch64, Some(Arch::X86_64)) => {
            candidate.score = 90;
            candidate
                .reasons
                .push("an x86-64 program; Windows on ARM emulates it".into());
        }
        _ => candidate.block(
            BlockerKind::Architecture,
            format!(
                "built for {}, which this {} PC cannot run",
                arch_list(report),
                host.arch
            ),
            false,
        ),
    }
    if let Format::Pe(details) = &report.format
        && details.requires_admin
    {
        candidate
            .warnings
            .push("it asks for administrator rights when it starts".into());
    }
    candidate
}

fn graphics_notes(candidate: &mut Candidate, report: &Report, host: &HostProfile, proton: bool) {
    let apis = report.graphics_apis();
    if apis.contains(&GraphicsApi::Direct3D12) {
        candidate
            .reasons
            .push("Direct3D 12: Proton translates it to Vulkan with VKD3D-Proton".into());
        if !proton {
            candidate.warnings.push(
                "plain Wine has only partial Direct3D 12 support; Proton (with VKD3D-Proton) works better"
                    .into(),
            );
        }
    }
    if apis.iter().any(|api| {
        matches!(
            api,
            GraphicsApi::Direct3D8
                | GraphicsApi::Direct3D9
                | GraphicsApi::Direct3D10
                | GraphicsApi::Direct3D11
        )
    }) {
        candidate
            .reasons
            .push("Direct3D 8–11: Proton translates it to Vulkan with DXVK".into());
        if !proton {
            candidate.warnings.push(
                "plain Wine draws Direct3D through OpenGL (wined3d) unless DXVK is installed in the \
                 prefix (`winetricks dxvk`)"
                    .into(),
            );
        }
    }
    if (apis.contains(&GraphicsApi::Vulkan) || proton || !apis.is_empty())
        && host.os == Os::Linux
        && !host.vulkan.loader
    {
        candidate.warnings.push(
            "no Vulkan loader was found; DXVK and VKD3D-Proton need a Vulkan driver (`mica setup vulkan`)".into(),
        );
    }
}

fn apply_policy(candidate: &mut Candidate, verdict: RouteVerdict) {
    if verdict.forbidden() {
        let message = format!(
            "the publisher or anti-cheat vendor does not permit {}: {}",
            verdict.route,
            verdict.reasons.join(" ")
        );
        candidate.block(BlockerKind::Policy, message, !verdict.official_prohibition);
    } else {
        candidate.reasons.extend(verdict.reasons.iter().cloned());
    }
    candidate.warnings.extend(verdict.warnings.iter().cloned());
    candidate.policy = Some(verdict);
}

fn windows_compat_linux(inputs: &Inputs, target: &Target) -> Candidate {
    let report = &target.report;
    let host = inputs.host;
    let runtimes = inputs.runtimes;
    let mut candidate = Candidate::new(Backend::WindowsCompat, Some(Route::LinuxProton), 80);
    candidate.reasons.push(format!(
        "a Windows {} for {}",
        if report.is_installer() {
            "installer"
        } else {
            "program"
        },
        arch_list(report)
    ));
    let x86 = report
        .architectures
        .iter()
        .all(|arch| matches!(arch, Arch::X86 | Arch::X86_64));
    if !x86 {
        candidate.block(
            BlockerKind::Architecture,
            format!(
                "Wine runs x86 and x86-64 Windows programs; this one is {}",
                arch_list(report)
            ),
            false,
        );
    }
    if host.arch == Arch::Aarch64 {
        candidate.score = 30;
        candidate.warnings.push(
            "on ARM64 Linux, x86 Windows programs need Wine together with FEX-Emu or Box64: \
             experimental and slow"
                .into(),
        );
    }

    let preference = inputs.config.windows.runtime;
    let umu = runtimes.first(RuntimeKind::Umu);
    let wine = inputs
        .config
        .windows
        .wine
        .as_ref()
        .map(PathBuf::from)
        .or_else(|| {
            runtimes
                .first(RuntimeKind::Wine)
                .map(|runtime| runtime.path.clone())
        });
    let proton = runtimes.protons.first().cloned();
    let runner = match preference {
        PreferredRuntime::Auto => umu
            .map(|umu| Runner::Umu {
                path: umu.path.clone(),
                proton: proton.clone(),
            })
            .or_else(|| proton.clone().map(|tool| Runner::Proton { tool }))
            .or_else(|| wine.clone().map(|path| Runner::Wine { path })),
        PreferredRuntime::Umu => umu.map(|umu| Runner::Umu {
            path: umu.path.clone(),
            proton: proton.clone(),
        }),
        PreferredRuntime::Proton => proton.clone().map(|tool| Runner::Proton { tool }),
        PreferredRuntime::Wine => wine.clone().map(|path| Runner::Wine { path }),
        PreferredRuntime::Crossover => None,
    };
    let uses_proton = matches!(runner, Some(Runner::Umu { .. } | Runner::Proton { .. }));
    let arm = host.arch == Arch::Aarch64;
    match &runner {
        Some(Runner::Wine { .. }) => candidate.score = if arm { 25 } else { 70 },
        Some(_) => candidate.score = if arm { 30 } else { 85 },
        None => {
            candidate.block(
                BlockerKind::MissingRuntime,
                format!("no Windows runtime ({preference:?} preference) is installed"),
                true,
            );
            candidate.setup.push(
                "`mica setup windows` installs Wine from your distribution; for games, install \
                 umu-launcher (https://github.com/Open-Wine-Components/umu-launcher) or Steam, \
                 whose Proton MICA can use"
                    .into(),
            );
        }
    }
    candidate.runner = runner;
    graphics_notes(&mut candidate, report, host, uses_proton);
    if let Format::Pe(details) = &report.format
        && details.dotnet
    {
        candidate.warnings.push(
                ".NET programs need Wine Mono (Wine offers to install it) or Microsoft .NET in the prefix"
                    .into(),
            );
    }
    if target.game.is_some()
        || target
            .scan
            .as_ref()
            .is_some_and(|scan| !scan.signals.is_empty())
    {
        let verdict = policy::assess(
            inputs.database,
            target.game.as_ref(),
            target.scan.as_ref(),
            Route::LinuxProton,
        );
        apply_policy(&mut candidate, verdict);
    }
    if let Some(game) = &target.game
        && let Some(native) = game.route(Route::LinuxNative)
        && native.authorization == Authorization::Planned
    {
        candidate.warnings.push(format!(
            "{} plans a Linux release; until then: {}",
            game.publisher, native.summary
        ));
    }
    candidate
}

fn protected_multiplayer(target: &Target) -> bool {
    target
        .game
        .as_ref()
        .is_some_and(Game::protected_multiplayer)
        || target
            .scan
            .as_ref()
            .is_some_and(|scan| !scan.signals.is_empty())
}

fn virtual_machine(inputs: &Inputs, target: &Target) -> Candidate {
    let mut candidate = Candidate::new(Backend::VirtualMachine, Some(Route::LinuxVm), 40);
    candidate
        .reasons
        .push("a real Windows in a virtual machine, for programs that need Windows itself".into());
    candidate.warnings.push(
        "a virtual machine needs a Windows license, several GB of memory, and (for good 3D \
         performance) a second GPU passed through; it is a full Windows, not a lightweight layer"
            .into(),
    );
    match &inputs.config.vm.libvirt_domain {
        Some(domain) if inputs.runtimes.has(RuntimeKind::Libvirt) => {
            candidate.runner = Some(Runner::Libvirt {
                domain: domain.clone(),
                uri: inputs
                    .config
                    .vm
                    .libvirt_uri
                    .clone()
                    .unwrap_or_else(|| "qemu:///session".into()),
            });
        }
        _ => {
            candidate.block(
                BlockerKind::NotConfigured,
                "no Windows virtual machine is configured",
                false,
            );
            candidate.setup.push(
                "`mica vm check` shows whether this PC can run one; `mica vm plan` prints the \
                 commands to create one with your own Windows installation media"
                    .into(),
            );
        }
    }
    if protected_multiplayer(target) {
        let verdict = policy::assess(
            inputs.database,
            target.game.as_ref(),
            target.scan.as_ref(),
            Route::LinuxVm,
        );
        let permitted = verdict.authorization == Authorization::Authorized
            && target.game.as_ref().is_some_and(|game| {
                game.route(Route::LinuxVm)
                    .is_some_and(|route| route.authorization == Authorization::Authorized)
            });
        let forbidden = verdict.forbidden();
        apply_policy(&mut candidate, verdict);
        if !permitted && !forbidden {
            candidate.block(
                BlockerKind::Policy,
                "MICA uses a virtual machine for a game behind anti-cheat only when the publisher \
                 explicitly permits it, and never hides the virtual machine",
                false,
            );
        }
    }
    candidate
}

fn windows_compat_macos(inputs: &Inputs, target: &Target) -> Candidate {
    let report = &target.report;
    let runtimes = inputs.runtimes;
    let mut candidate = Candidate::new(Backend::WindowsCompat, Some(Route::MacosWine), 75);
    let preference = inputs.config.windows.runtime;
    let crossover = runtimes.first(RuntimeKind::Crossover);
    let wine = runtimes.first(RuntimeKind::Wine);
    let gptk = runtimes.first(RuntimeKind::GamePortingToolkit);
    let bottle = inputs
        .config
        .windows
        .bottle
        .clone()
        .unwrap_or_else(|| "MICA".into());
    candidate.runner = match preference {
        PreferredRuntime::Wine => wine.map(|runtime| Runner::Wine {
            path: runtime.path.clone(),
        }),
        _ => crossover
            .map(|runtime| Runner::Crossover {
                path: runtime.path.clone(),
                bottle: bottle.clone(),
            })
            .or_else(|| {
                wine.map(|runtime| Runner::Wine {
                    path: runtime.path.clone(),
                })
            })
            .or_else(|| {
                gptk.map(|runtime| Runner::GamePortingToolkit {
                    path: runtime.path.clone(),
                })
            }),
    };
    if candidate.runner.is_none() {
        candidate.block(
            BlockerKind::MissingRuntime,
            "no Windows runtime is installed",
            true,
        );
        candidate
            .setup
            .push("`mica setup windows` explains how to install CrossOver".into());
    }
    if inputs.host.arch == Arch::Aarch64 {
        candidate
            .reasons
            .push("Apple Silicon: x86 Windows code runs through Rosetta 2 inside Wine".into());
        if !runtimes.has(RuntimeKind::Rosetta) {
            candidate.block(
                BlockerKind::MissingRuntime,
                "Rosetta 2 is not installed",
                true,
            );
            candidate
                .setup
                .push("softwareupdate --install-rosetta".into());
        }
        candidate.warnings.push(
            "Apple plans to keep Rosetta 2 fully available through macOS 27, and only a subset for \
             older games after that"
                .into(),
        );
    }
    let apis = report.graphics_apis();
    if apis.contains(&GraphicsApi::Direct3D12) || apis.contains(&GraphicsApi::Direct3D11) {
        candidate.reasons.push(
            "Direct3D 11/12 on macOS goes through Metal (D3DMetal from the Game Porting Toolkit, \
             or DXMT, in CrossOver)"
                .into(),
        );
    }
    if target.game.is_some()
        || target
            .scan
            .as_ref()
            .is_some_and(|scan| !scan.signals.is_empty())
    {
        let verdict = policy::assess(
            inputs.database,
            target.game.as_ref(),
            target.scan.as_ref(),
            Route::MacosWine,
        );
        apply_policy(&mut candidate, verdict);
        if protected_multiplayer(target) {
            candidate
                .warnings
                .push("Windows kernel-level anti-cheat cannot run under Wine on macOS".into());
        }
    }
    candidate
}

fn macos_vm(inputs: &Inputs, target: &Target) -> Candidate {
    let mut candidate = Candidate::new(Backend::VirtualMachine, Some(Route::MacosVm), 30);
    candidate.reasons.push(
        "Windows 11 on Arm in a virtual machine (Parallels, VMware Fusion, UTM) runs most \
         Windows programs on Apple Silicon"
            .into(),
    );
    candidate.block(
        BlockerKind::NotConfigured,
        "MICA does not manage macOS virtual machines yet; start yours from its own application",
        false,
    );
    if protected_multiplayer(target) {
        let verdict = policy::assess(
            inputs.database,
            target.game.as_ref(),
            target.scan.as_ref(),
            Route::MacosVm,
        );
        apply_policy(&mut candidate, verdict);
    }
    candidate
}

fn native_elf(inputs: &Inputs, report: &Report) -> Candidate {
    let host = inputs.host;
    let mut candidate = Candidate::new(Backend::Native, Some(Route::LinuxNative), 100);
    candidate.runner = Some(Runner::Direct);
    let arch = report.architectures.first().copied().unwrap_or(Arch::Other);
    match (host.arch, arch) {
        (a, b) if a == b => candidate
            .reasons
            .push("a Linux program for this CPU".into()),
        (Arch::X86_64, Arch::X86) => {
            candidate.score = 95;
            candidate.reasons.push(
                "a 32-bit x86 program: it runs natively if 32-bit libraries are installed".into(),
            );
        }
        (Arch::Aarch64, Arch::Arm) => {
            candidate.score = 90;
            candidate
                .warnings
                .push("32-bit ARM programs run only on CPUs that kept 32-bit support".into());
        }
        _ => candidate.block(
            BlockerKind::Architecture,
            format!(
                "built for {}, this computer is {}",
                arch_list(report),
                host.arch
            ),
            false,
        ),
    }
    if let Format::Elf(details) = &report.format
        && details.appimage.is_some()
    {
        candidate
            .reasons
            .push("an AppImage (needs FUSE to mount itself)".into());
    }
    candidate
}

fn arch_translation(inputs: &Inputs, report: &Report) -> Candidate {
    let mut candidate = Candidate::new(Backend::ArchTranslation, Some(Route::LinuxNative), 0);
    let x86 = report
        .architectures
        .iter()
        .any(|arch| matches!(arch, Arch::X86 | Arch::X86_64));
    if inputs.host.arch != Arch::Aarch64 || !x86 {
        return candidate;
    }
    candidate.score = 50;
    candidate
        .reasons
        .push("an x86 Linux program on an ARM64 computer".into());
    let fex = inputs.runtimes.first(RuntimeKind::Fex);
    let box64 = inputs.runtimes.first(RuntimeKind::Box64);
    candidate.runner = fex
        .map(|runtime| Runner::Translator {
            path: runtime.path.clone(),
            name: "FEX-Emu".into(),
        })
        .or_else(|| {
            box64.map(|runtime| Runner::Translator {
                path: runtime.path.clone(),
                name: "Box64".into(),
            })
        });
    if candidate.runner.is_none() {
        candidate.block(
            BlockerKind::MissingRuntime,
            "neither FEX-Emu nor Box64 is installed",
            true,
        );
        candidate
            .setup
            .push("Install FEX-Emu (https://fex-emu.com) or Box64 from your distribution".into());
    }
    candidate
}

fn wsl(inputs: &Inputs, report: &Report) -> Candidate {
    let mut candidate = Candidate::new(Backend::Wsl, Some(Route::WindowsWsl), 85);
    candidate
        .reasons
        .push("a Linux program: WSL runs it in a real Linux kernel".into());
    if report.installer.is_some() {
        candidate.block(
            BlockerKind::Technical,
            "a Linux package: install it inside your WSL distribution with its package manager",
            false,
        );
        return candidate;
    }
    let host_arch = inputs.host.arch;
    if !report
        .architectures
        .iter()
        .any(|arch| *arch == host_arch || (*arch == Arch::X86 && host_arch == Arch::X86_64))
    {
        candidate.block(
            BlockerKind::Architecture,
            format!(
                "built for {}, and WSL runs {} Linux programs",
                arch_list(report),
                host_arch
            ),
            false,
        );
    }
    match inputs.runtimes.first(RuntimeKind::Wsl) {
        Some(runtime) => {
            candidate.runner = Some(Runner::Wsl {
                path: runtime.path.clone(),
            });
            if runtime.version.is_none() {
                candidate.warnings.push(
                    "WSL is installed but no Linux distribution was listed; run `wsl --install`"
                        .into(),
                );
            }
            candidate
                .reasons
                .push("graphical programs open in Windows through WSLg on Windows 11".into());
        }
        None => {
            candidate.block(BlockerKind::MissingRuntime, "WSL is not installed", true);
            candidate
                .setup
                .push("From an administrator terminal: wsl --install (then restart)".into());
        }
    }
    candidate
}

fn linux_vm(inputs: &Inputs) -> Candidate {
    let mut candidate = Candidate::new(Backend::LinuxVm, None, 50);
    candidate
        .reasons
        .push("a Linux program: macOS runs it in a Linux virtual machine".into());
    match inputs.runtimes.first(RuntimeKind::Lima) {
        Some(runtime) => {
            candidate.runner = Some(Runner::Lima {
                path: runtime.path.clone(),
            });
            candidate.warnings.push(
                "the program runs inside the VM: copy it there, or keep it under your home \
                 directory, which Lima shares read-only by default"
                    .into(),
            );
        }
        None => {
            candidate.block(BlockerKind::MissingRuntime, "Lima is not installed", true);
            candidate.setup.push("brew install lima".into());
        }
    }
    candidate
}

fn native_macos(inputs: &Inputs, report: &Report) -> Candidate {
    let host = inputs.host;
    let mut candidate = Candidate::new(Backend::Native, Some(Route::MacosNative), 100);
    candidate.runner = Some(Runner::Direct);
    let arches = &report.architectures;
    if arches.contains(&host.arch) {
        candidate
            .reasons
            .push("a macOS program for this CPU".into());
    } else if host.arch == Arch::Aarch64 && arches.contains(&Arch::X86_64) {
        candidate.backend = Backend::Rosetta;
        candidate.score = 85;
        candidate
            .reasons
            .push("an Intel program: macOS runs it through Rosetta 2".into());
        if !inputs.runtimes.has(RuntimeKind::Rosetta) {
            candidate.block(
                BlockerKind::MissingRuntime,
                "Rosetta 2 is not installed",
                true,
            );
            candidate
                .setup
                .push("softwareupdate --install-rosetta".into());
        }
        candidate
            .warnings
            .push("Apple plans general Rosetta 2 support through macOS 27 only".into());
    } else {
        candidate.block(
            BlockerKind::Architecture,
            format!("built for {}, this Mac is {}", arch_list(report), host.arch),
            false,
        );
    }
    candidate
}

/// The installer kind, for messages.
pub fn installer_label(installer: Option<Installer>) -> String {
    installer
        .map(|kind| kind.to_string())
        .unwrap_or_else(|| "program".into())
}
