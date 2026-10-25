//! `mica doctor`, `runtimes`, `setup`, `desktop`, `vm`, `config`, `bench`,
//! and `uninstall`.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use serde::Serialize;

use mica_probe::{HostProfile, Init, Os};
use mica_runtime::packages::{self, Feature};
use mica_runtime::{RuntimeKind, Runtimes, desktop};

use crate::session::Session;
use crate::ui::{self, line, more, para};

/// One row of "what can this computer run?".
#[derive(Serialize)]
struct Capability {
    programs: &'static str,
    status: String,
    next_step: Option<String>,
}

fn capabilities(host: &HostProfile, runtimes: &Runtimes) -> Vec<Capability> {
    let mut rows = Vec::new();
    let windows_runner = runtimes
        .first(RuntimeKind::Umu)
        .map(|_| match runtimes.protons.first() {
            Some(proton) => format!("ready: umu-launcher with {}", proton.display_name),
            None => "ready: umu-launcher (it fetches UMU-Proton on first use)".into(),
        })
        .or_else(|| {
            runtimes
                .protons
                .first()
                .map(|proton| format!("ready: {}", proton.display_name))
        })
        .or_else(|| {
            runtimes
                .first(RuntimeKind::Wine)
                .map(|wine| format!("ready: Wine {}", wine.version.clone().unwrap_or_default()))
        });
    let portable = runtimes
        .first(RuntimeKind::PortableHost)
        .map(|_| "ready: portable-host".to_string());
    match host.os {
        Os::Linux => {
            rows.push(Capability {
                programs: "Linux programs",
                status: "native".into(),
                next_step: None,
            });
            rows.push(Capability {
                programs: "Windows programs",
                status: windows_runner
                    .clone()
                    .unwrap_or_else(|| "needs a runtime".into()),
                next_step: windows_runner
                    .is_none()
                    .then(|| "mica setup windows".into()),
            });
            rows.push(Capability {
                programs: "macOS programs",
                status: "not possible: macOS runs only on Apple hardware".into(),
                next_step: None,
            });
        }
        Os::Windows => {
            rows.push(Capability {
                programs: "Windows programs",
                status: "native".into(),
                next_step: None,
            });
            let wsl = runtimes.first(RuntimeKind::Wsl);
            rows.push(Capability {
                programs: "Linux programs",
                status: match wsl {
                    Some(runtime) => format!(
                        "ready: WSL{}",
                        runtime
                            .version
                            .as_ref()
                            .map(|list| format!(" ({list})"))
                            .unwrap_or_else(|| " (no distribution installed yet)".into())
                    ),
                    None => "needs WSL".into(),
                },
                next_step: wsl
                    .is_none()
                    .then(|| "wsl --install (administrator terminal)".into()),
            });
            rows.push(Capability {
                programs: "macOS programs",
                status: "not possible: macOS runs only on Apple hardware".into(),
                next_step: None,
            });
        }
        Os::MacOs => {
            rows.push(Capability {
                programs: "macOS programs",
                status: if host.arch == mica_inspect::Arch::Aarch64 {
                    if runtimes.has(RuntimeKind::Rosetta) {
                        "native (Intel apps through Rosetta 2)".into()
                    } else {
                        "native (Apple Silicon apps; Intel apps need Rosetta 2)".into()
                    }
                } else {
                    "native".into()
                },
                next_step: None,
            });
            let windows = runtimes
                .first(RuntimeKind::Crossover)
                .map(|_| "ready: CrossOver".to_string())
                .or(windows_runner.clone());
            rows.push(Capability {
                programs: "Windows programs",
                status: windows
                    .clone()
                    .unwrap_or_else(|| "needs CrossOver or Wine".into()),
                next_step: windows.is_none().then(|| "mica setup windows".into()),
            });
            rows.push(Capability {
                programs: "Linux programs",
                status: if runtimes.has(RuntimeKind::Lima) {
                    "ready: Lima (in a Linux virtual machine)".into()
                } else {
                    "needs a Linux virtual machine (Lima)".into()
                },
                next_step: (!runtimes.has(RuntimeKind::Lima)).then(|| "mica setup linux".into()),
            });
        }
        Os::Other => {}
    }
    rows.push(Capability {
        programs: "WebAssembly components",
        status: portable
            .unwrap_or_else(|| "needs portable-host (this repository's Portable Host)".into()),
        next_step: None,
    });
    rows.push(Capability {
        programs: "DOS programs",
        status: if runtimes.has(RuntimeKind::DosBox) {
            "ready: DOSBox".into()
        } else {
            "needs DOSBox".into()
        },
        next_step: None,
    });
    rows
}

#[derive(Serialize)]
struct Doctor<'a> {
    version: &'static str,
    host: &'a HostProfile,
    runtimes: &'a Runtimes,
    capabilities: Vec<Capability>,
}

pub fn doctor(session: &Session, json: bool) -> Result<ExitCode> {
    let host = session.host();
    let runtimes = session.runtimes();
    let capabilities = capabilities(host, runtimes);
    if json {
        ui::json(&Doctor {
            version: env!("CARGO_PKG_VERSION"),
            host,
            runtimes,
            capabilities,
        })?;
        return Ok(ExitCode::SUCCESS);
    }
    println!("MICA {} doctor", env!("CARGO_PKG_VERSION"));
    println!();
    line(
        "system",
        format!("{} ({}, {})", host.os_name, host.os, host.arch),
    );
    if let Some(kernel) = &host.kernel {
        more(format!("kernel {kernel}"));
    }
    if let Some(distro) = &host.distro {
        more(format!("distribution family: {}", distro.family));
    }
    if let Some(immutable) = &host.immutable {
        para("", immutable);
    }
    if host.translated {
        more("MICA itself is running under CPU translation");
    }
    line(
        "cpu",
        format!(
            "{} (hardware virtualization: {})",
            host.cpu_model.as_deref().unwrap_or("unknown"),
            ui::yes_no(host.virtualization.cpu_support)
        ),
    );
    if host.gpus.is_empty() {
        line("gpu", "none found");
    }
    for (index, gpu) in host.gpus.iter().enumerate() {
        line(
            if index == 0 { "gpu" } else { "" },
            format!(
                "{}{}",
                gpu.name,
                gpu.driver
                    .as_ref()
                    .map(|driver| format!(" (driver {driver})"))
                    .unwrap_or_default()
            ),
        );
    }
    if host.os != Os::MacOs {
        line(
            "vulkan",
            format!(
                "loader {}{}",
                if host.vulkan.loader {
                    "installed"
                } else {
                    "not found"
                },
                if host.vulkan.drivers.is_empty() {
                    String::new()
                } else {
                    format!("; drivers: {}", host.vulkan.drivers.join(", "))
                }
            ),
        );
    }
    if host.os == Os::Linux {
        line(
            "kvm",
            match (host.virtualization.kvm, host.virtualization.kvm_accessible) {
                (true, true) => "available",
                (true, false) => "present, but this user cannot open /dev/kvm (join the kvm group)",
                _ => "not available",
            },
        );
        line(
            "init",
            match host.init {
                Init::Systemd => "systemd",
                Init::OpenRc => "OpenRC",
                Init::Runit => "runit",
                Init::S6 => "s6",
                _ => "other",
            },
        );
    }
    if let Some(guest) = &host.virtualization.guest_of {
        line("vm", format!("this system is a virtual machine: {guest}"));
    }
    if host.device.steam_deck {
        line("device", "Steam Deck");
    }
    line(
        "packages",
        if host.package_managers.is_empty() {
            "none recognised".to_string()
        } else {
            host.package_managers
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        },
    );
    let security = &host.security;
    let mut flags = vec![
        format!("UEFI {}", ui::yes_no(security.uefi)),
        format!("Secure Boot {}", ui::yes_no(security.secure_boot)),
        format!(
            "TPM {}",
            match security.tpm_version {
                Some(version) => version.to_string(),
                None => ui::yes_no(security.tpm_present).to_string(),
            }
        ),
    ];
    if host.os == Os::Linux {
        flags.push(format!(
            "lockdown {}",
            security.lockdown.as_deref().unwrap_or("unavailable")
        ));
        flags.push(format!(
            "BPF LSM {}",
            ui::yes_no(
                security
                    .lsms
                    .as_ref()
                    .map(|lsms| lsms.iter().any(|lsm| lsm == "bpf"))
            )
        ));
    }
    line("security", flags.join(" · "));
    more("details: mica security check");

    println!();
    let mut first = true;
    for runtime in &runtimes.found {
        line(
            if first { "runtimes" } else { "" },
            format!(
                "{}: {}{}",
                runtime.kind,
                runtime.name,
                runtime
                    .version
                    .as_ref()
                    .map(|version| format!(" ({version})"))
                    .unwrap_or_default()
            ),
        );
        first = false;
    }
    if first {
        line("runtimes", "none found yet");
    }
    for install in &runtimes.steam {
        line(
            "steam",
            format!(
                "{} ({} libraries)",
                install.root.display(),
                install.libraries.len()
            ),
        );
    }

    println!();
    println!("What this computer can run");
    for capability in &capabilities {
        println!(
            "  {:<24} {}{}",
            capability.programs,
            capability.status,
            capability
                .next_step
                .as_ref()
                .map(|step| format!("  ->  {step}"))
                .unwrap_or_default()
        );
    }
    println!(
        "  {:<24} see `mica compatibility <game>` and `mica security check`",
        "Games with anti-cheat"
    );
    if !host.unknowns.is_empty() {
        println!();
        for unknown in &host.unknowns {
            para("unknown", unknown);
        }
    }
    Ok(ExitCode::SUCCESS)
}

pub fn runtimes(session: &Session, json: bool) -> Result<ExitCode> {
    let runtimes = session.runtimes();
    if json {
        ui::json(runtimes)?;
        return Ok(ExitCode::SUCCESS);
    }
    if runtimes.found.is_empty() {
        println!("No compatibility runtimes found. `mica setup` installs them.");
    }
    for runtime in &runtimes.found {
        line(
            &runtime
                .kind
                .to_string()
                .chars()
                .take(10)
                .collect::<String>(),
            format!(
                "{}{} — {}",
                runtime.name,
                runtime
                    .version
                    .as_ref()
                    .map(|version| format!(" ({version})"))
                    .unwrap_or_default(),
                runtime.path.display()
            ),
        );
    }
    Ok(ExitCode::SUCCESS)
}

pub fn setup(
    session: &Session,
    features: &[String],
    dry_run: bool,
    yes: bool,
    json: bool,
) -> Result<ExitCode> {
    let host = session.host_light();
    let wanted: Vec<Feature> = if features.is_empty() {
        vec![if host.os == Os::Windows {
            Feature::Linux
        } else {
            Feature::Windows
        }]
    } else {
        features
            .iter()
            .map(|name| {
                Feature::parse(name).ok_or_else(|| {
                    anyhow::anyhow!(
                        "unknown feature {name}: choose from windows, vulkan, vm, security, linux"
                    )
                })
            })
            .collect::<Result<_>>()?
    };
    if json {
        let plans: Vec<packages::InstallPlan> = wanted
            .iter()
            .map(|feature| packages::plan(host, *feature))
            .collect();
        ui::json(&plans)?;
        return Ok(ExitCode::SUCCESS);
    }
    for feature in wanted {
        let plan = packages::plan(host, feature);
        println!("{feature}:");
        for line_text in &plan.manual {
            println!("  {line_text}");
        }
        for command in &plan.commands {
            println!("  {}", command.join(" "));
        }
        for note in &plan.notes {
            println!("  note: {}", ui::wrap(note, 8, 100));
        }
        if plan.commands.is_empty() || dry_run {
            continue;
        }
        if !ui::confirm("Run these commands now?", yes) {
            println!("  skipped");
            continue;
        }
        for command in &plan.commands {
            let status = std::process::Command::new(&command[0])
                .args(&command[1..])
                .status()?;
            if !status.success() {
                bail!("`{}` failed ({status})", command.join(" "));
            }
        }
        println!("  done");
    }
    Ok(ExitCode::SUCCESS)
}

fn mime_default(mime: &str) -> Option<String> {
    let output = std::process::Command::new("xdg-mime")
        .args(["query", "default", mime])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

pub fn desktop_install(session: &Session, make_default: bool) -> Result<ExitCode> {
    if session.os() != Os::Linux {
        bail!(
            "desktop integration is for Linux; on Windows, Windows programs already open natively"
        );
    }
    let home = session.context.home().unwrap_or_default();
    let applications =
        desktop::applications_dir(&home, session.context.env("XDG_DATA_HOME").map(Into::into));
    let path = desktop::install(&applications, &session.mica_exe())?;
    line("created", path.display());
    println!("\"MICA\" now appears under Open With for Windows programs and installers.");
    if make_default {
        let previous: Vec<(String, String)> = desktop::MIME_TYPES
            .iter()
            .filter_map(|mime| mime_default(mime).map(|handler| (mime.to_string(), handler)))
            .filter(|(_, handler)| handler != desktop::ENTRY_NAME)
            .collect();
        let record = desktop::record_defaults(&session.dirs.undo(), &previous)?;
        for mime in desktop::MIME_TYPES {
            std::process::Command::new("xdg-mime")
                .args(["default", desktop::ENTRY_NAME, mime])
                .status()?;
        }
        println!(
            "MICA is now the default for them; previous defaults recorded in {}",
            record.display()
        );
    }
    Ok(ExitCode::SUCCESS)
}

pub fn desktop_remove(session: &Session) -> Result<ExitCode> {
    let home = session.context.home().unwrap_or_default();
    let applications =
        desktop::applications_dir(&home, session.context.env("XDG_DATA_HOME").map(Into::into));
    for (mime, handler) in desktop::recorded_defaults(&session.dirs.undo()) {
        let _ = std::process::Command::new("xdg-mime")
            .args(["default", &handler, &mime])
            .status();
        println!("restored   {mime} -> {handler}");
    }
    if desktop::remove(&applications, &session.dirs.undo())? {
        line("removed", applications.join(desktop::ENTRY_NAME).display());
    } else {
        println!("MICA's desktop entry is not installed.");
    }
    Ok(ExitCode::SUCCESS)
}

#[derive(Serialize)]
struct VmCheck {
    cpu_virtualization: Option<bool>,
    kvm: bool,
    kvm_accessible: bool,
    memory_gib: Option<u64>,
    gpus: usize,
    iommu_groups: usize,
    libvirt: bool,
    uefi_firmware: Option<String>,
    swtpm: bool,
    verdict: Vec<String>,
}

pub fn vm_check(session: &Session, json: bool) -> Result<ExitCode> {
    let host = session.host();
    let runtimes = session.runtimes();
    if host.os != Os::Linux {
        let text = match host.os {
            Os::Windows => {
                "On Windows, Hyper-V (Pro, Enterprise, Education) runs virtual machines; \
                            Linux programs are better served by WSL (`mica setup linux`)."
            }
            Os::MacOs => {
                "On macOS, Windows 11 on Arm runs in Parallels Desktop, VMware Fusion, or \
                          UTM; MICA does not manage those yet."
            }
            _ => "Not supported on this system.",
        };
        para("vm", text);
        return Ok(ExitCode::SUCCESS);
    }
    let memory_gib = session
        .context
        .read("/proc/meminfo")
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("MemTotal:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|kib| kib.parse::<u64>().ok())
        })
        .map(|kib| kib / 1024 / 1024);
    let firmware = [
        "/usr/share/OVMF/OVMF_CODE_4M.secboot.fd",
        "/usr/share/OVMF/OVMF_CODE.secboot.fd",
        "/usr/share/edk2/ovmf/OVMF_CODE.secboot.fd",
        "/usr/share/edk2/x64/OVMF_CODE.secboot.4m.fd",
        "/usr/share/qemu/ovmf-x86_64-smm-ms-code.bin",
        "/usr/share/OVMF/OVMF_CODE.fd",
        "/usr/share/edk2/ovmf/OVMF_CODE.fd",
    ]
    .iter()
    .find(|path| session.context.exists(path))
    .map(|path| path.to_string());
    let swtpm = session.context.which("swtpm").is_some();
    let mut verdict = Vec::new();
    if host.virtualization.cpu_support == Some(false) {
        verdict.push(
            "The CPU does not report VT-x/AMD-V: enable virtualization in the firmware settings."
                .into(),
        );
    }
    if !host.virtualization.kvm {
        verdict.push(
            "KVM is not available: without it, a virtual machine is far too slow for Windows."
                .into(),
        );
    } else if !host.virtualization.kvm_accessible {
        verdict.push(
            "Add yourself to the kvm group (sudo usermod -aG kvm $USER) and log in again.".into(),
        );
    }
    if memory_gib.is_some_and(|gib| gib < 12) {
        verdict.push("Windows 11 needs 4 GB of its own; with under 12 GB in total, expect both systems to be slow.".into());
    }
    if host.gpus.len() < 2 {
        verdict.push(
            "Only one GPU: passing it through to Windows would leave Linux without a display. The VM \
             gets emulated graphics, fine for desktop programs but not for 3D games."
                .into(),
        );
    } else if host.security.iommu_groups == 0 {
        verdict.push(
            "Two GPUs, but the IOMMU is off: GPU passthrough needs AMD-Vi/VT-d enabled.".into(),
        );
    } else {
        verdict.push(
            "Two GPUs and an active IOMMU: GPU passthrough may be possible (check that the second \
             GPU has its own IOMMU group)."
                .into(),
        );
    }
    if firmware.is_none() || !swtpm {
        verdict.push("Windows 11 needs UEFI firmware (OVMF) and a TPM 2.0 emulator (swtpm): `mica setup vm`.".into());
    }
    verdict.push(
        "Anti-cheat: Fortnite and Riot Vanguard games forbid virtual machines, and MICA never hides \
         one. A VM is for programs that need Windows itself, not for those games."
            .into(),
    );
    let check = VmCheck {
        cpu_virtualization: host.virtualization.cpu_support,
        kvm: host.virtualization.kvm,
        kvm_accessible: host.virtualization.kvm_accessible,
        memory_gib,
        gpus: host.gpus.len(),
        iommu_groups: host.security.iommu_groups,
        libvirt: runtimes.has(RuntimeKind::Libvirt),
        uefi_firmware: firmware,
        swtpm,
        verdict,
    };
    if json {
        ui::json(&check)?;
        return Ok(ExitCode::SUCCESS);
    }
    line("cpu virt", ui::yes_no(check.cpu_virtualization));
    line(
        "kvm",
        if check.kvm_accessible {
            "available"
        } else if check.kvm {
            "present, not accessible"
        } else {
            "no"
        },
    );
    line(
        "memory",
        check
            .memory_gib
            .map(|gib| format!("{gib} GiB"))
            .unwrap_or_else(|| "unknown".into()),
    );
    line("gpus", check.gpus);
    line("iommu", format!("{} groups", check.iommu_groups));
    line(
        "libvirt",
        if check.libvirt {
            "installed"
        } else {
            "not installed"
        },
    );
    line(
        "firmware",
        check
            .uefi_firmware
            .clone()
            .unwrap_or_else(|| "OVMF not found".into()),
    );
    line(
        "swtpm",
        if check.swtpm {
            "installed"
        } else {
            "not installed"
        },
    );
    println!();
    for item in &check.verdict {
        para("-", item);
    }
    Ok(ExitCode::SUCCESS)
}

pub fn vm_plan(
    session: &Session,
    name: &str,
    iso: Option<&Path>,
    memory_gb: u32,
    cpus: u32,
    disk_gb: u32,
) -> Result<ExitCode> {
    if session.os() != Os::Linux {
        bail!("`mica vm plan` prints libvirt commands for Linux hosts");
    }
    let iso = iso
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "/path/to/your/Win11.iso".into());
    println!("Create a Windows 11 virtual machine with your own installation media and license.");
    println!(
        "MICA does not download Windows. Microsoft offers the ISO at https://www.microsoft.com/software-download/windows11"
    );
    println!();
    println!("virt-install --connect qemu:///session \\");
    println!(
        "  --name {name} --memory {} --vcpus {cpus} --cpu host-passthrough \\",
        memory_gb * 1024
    );
    println!("  --os-variant win11 --cdrom '{iso}' \\");
    println!("  --disk size={disk_gb} --network user \\");
    println!(
        "  --boot firmware=efi,firmware.feature0.name=secure-boot,firmware.feature0.enabled=yes,firmware.feature1.name=enrolled-keys,firmware.feature1.enabled=yes \\"
    );
    println!("  --tpm backend.type=emulator,backend.version=2.0,model=tpm-crb \\");
    println!("  --graphics spice --video virtio");
    println!();
    println!("Needs virt-install 4.1 or newer, OVMF, and swtpm (`mica setup vm`). Then:");
    println!("  mica config set vm.libvirt_domain {name}");
    println!("  mica config set vm.libvirt_uri qemu:///session");
    println!(
        "and `mica run --backend vm <program>` starts it. Copy programs in, or share a folder"
    );
    println!("with virtiofs. 3D performance needs a second GPU passed through (`mica vm check`).");
    println!();
    para(
        "anti-cheat",
        "Do not use a virtual machine for games whose publisher forbids it (Fortnite, Riot \
         Vanguard games). MICA never disguises a virtual machine.",
    );
    Ok(ExitCode::SUCCESS)
}

pub fn config_path(session: &Session) -> Result<ExitCode> {
    line("config", session.dirs.config_file().display());
    line("compat.d", session.dirs.compat_dir().display());
    line("prefixes", session.dirs.prefixes().display());
    line("apps", session.dirs.apps_file().display());
    line("logs", session.dirs.logs().display());
    line("undo", session.dirs.undo().display());
    line("cache", session.dirs.cache.display());
    Ok(ExitCode::SUCCESS)
}

pub fn config_show(session: &Session) -> Result<ExitCode> {
    let config = &session.config;
    line("detect_games", config.detect_games);
    line(
        "windows.runtime",
        format!("{:?}", config.windows.runtime).to_lowercase(),
    );
    line(
        "windows.proton",
        config
            .windows
            .proton
            .as_deref()
            .unwrap_or("(best installed)"),
    );
    line(
        "windows.wine",
        config.windows.wine.as_deref().unwrap_or("(wine on PATH)"),
    );
    line(
        "windows.bottle",
        config.windows.bottle.as_deref().unwrap_or("MICA"),
    );
    line("windows.env", config.windows.env.join(" "));
    line(
        "vm.libvirt_domain",
        config.vm.libvirt_domain.as_deref().unwrap_or("(none)"),
    );
    line(
        "vm.libvirt_uri",
        config
            .vm
            .libvirt_uri
            .as_deref()
            .unwrap_or("qemu:///session"),
    );
    println!();
    println!("MICA has no telemetry: it sends nothing anywhere.");
    Ok(ExitCode::SUCCESS)
}

pub fn config_set(session: &Session, key: &str, value: &str) -> Result<ExitCode> {
    let mut config = session.config.clone();
    if key == "windows.env" {
        config.windows.env = value.split_whitespace().map(str::to_string).collect();
    } else {
        config.set(key, value).map_err(anyhow::Error::msg)?;
    }
    config.save(&session.dirs.config_file())?;
    line("saved", session.dirs.config_file().display());
    Ok(ExitCode::SUCCESS)
}

#[derive(Serialize)]
struct Measurement {
    name: String,
    iterations: u32,
    median_us: f64,
    p95_us: f64,
}

fn measure(name: &str, iterations: u32, mut work: impl FnMut()) -> Measurement {
    let mut samples: Vec<Duration> = (0..iterations)
        .map(|_| {
            let started = Instant::now();
            work();
            started.elapsed()
        })
        .collect();
    samples.sort();
    let micros = |duration: Duration| duration.as_secs_f64() * 1e6;
    Measurement {
        name: name.into(),
        iterations,
        median_us: micros(samples[samples.len() / 2]),
        p95_us: micros(samples[(samples.len() * 95 / 100).min(samples.len() - 1)]),
    }
}

pub fn bench(session: &Session, iterations: u32, json: bool) -> Result<ExitCode> {
    let iterations = iterations.max(5);
    let exe = session.mica_exe();
    let database = session.database();
    let mut results = Vec::new();
    results.push(measure("inspect the mica executable", iterations, || {
        let _ = mica_inspect::inspect_path(&exe);
    }));
    let target = mica_engine::analyze(&exe, database)?;
    let inputs = session.inputs();
    results.push(measure("decide a backend (no I/O)", iterations, || {
        let _ = mica_engine::decide(&inputs, &target, None);
    }));
    results.push(measure(
        "load the compatibility database",
        iterations,
        || {
            let _ = mica_compat::Database::builtin();
        },
    ));
    let probe_runs = iterations.min(5);
    results.push(measure(
        "probe the host (runs helper commands)",
        probe_runs,
        || {
            let _ = mica_probe::probe_with(&session.context, Os::current());
        },
    ));
    let (program, args): (&str, &[&str]) = if cfg!(windows) {
        ("cmd", &["/C", "exit 0"])
    } else {
        ("true", &[])
    };
    let launches = iterations.min(50);
    results.push(measure(
        "start a trivial program directly",
        launches,
        || {
            let _ = std::process::Command::new(program).args(args).status();
        },
    ));
    let plan = mica_runtime::LaunchPlan::new("native", program).args(args.iter().copied());
    results.push(measure(
        "start it through MICA's launcher (no log)",
        launches,
        || {
            let _ = plan.run(None, "bench");
        },
    ));
    if json {
        ui::json(&results)?;
        return Ok(ExitCode::SUCCESS);
    }
    println!("MICA's own overhead on this computer (not game performance):");
    for result in &results {
        println!(
            "  {:<44} median {:>10.1} µs   p95 {:>10.1} µs   ({} runs)",
            result.name, result.median_us, result.p95_us, result.iterations
        );
    }
    Ok(ExitCode::SUCCESS)
}

pub fn uninstall(session: &Session, yes: bool) -> Result<ExitCode> {
    println!("This removes what MICA created. It asks before each step.");
    for install in &session.runtimes().steam {
        if mica_runtime::steam::remove_tool(&install.root)? {
            println!(
                "removed    MICA's Steam compatibility tool from {}",
                install.root.display()
            );
        }
    }
    if session.os() == Os::Linux {
        let _ = desktop_remove(session);
        let entries: Vec<String> = [
            "/boot/loader/entries",
            "/efi/loader/entries",
            "/boot/efi/loader/entries",
        ]
        .iter()
        .flat_map(|dir| {
            session
                .context
                .list(dir)
                .into_iter()
                .filter(|name| name.starts_with(mica_security::boot_profile::ENTRY_PREFIX))
                .map(move |name| format!("{dir}/{name}"))
        })
        .collect();
        if !entries.is_empty() {
            println!(
                "MICA boot entries exist ({}); remove them with:",
                entries.join(", ")
            );
            println!("  sudo mica security boot-profile --rollback");
        }
    }
    let prefixes = session.dirs.prefixes();
    if prefixes.is_dir()
        && ui::confirm(
            &format!(
                "Delete {} — every Windows program installed through MICA, with its saves?",
                prefixes.display()
            ),
            false,
        )
    {
        std::fs::remove_dir_all(&prefixes)?;
        println!("deleted    {}", prefixes.display());
    }
    let mut dirs = vec![
        session.dirs.config.clone(),
        session.dirs.data.clone(),
        session.dirs.state.clone(),
        session.dirs.cache.clone(),
    ];
    dirs.sort();
    dirs.dedup();
    let existing: Vec<_> = dirs.into_iter().filter(|dir| dir.exists()).collect();
    if !existing.is_empty()
        && ui::confirm(
            &format!(
                "Delete MICA's settings, logs, and caches ({})?",
                existing
                    .iter()
                    .map(|dir| dir.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            yes,
        )
    {
        for dir in existing {
            std::fs::remove_dir_all(&dir)?;
            println!("deleted    {}", dir.display());
        }
    }
    println!();
    println!(
        "Finally, delete the mica program itself: {}",
        session.mica_exe().display()
    );
    println!(
        "{}",
        match session.os() {
            Os::Windows => r"(scripts\uninstall.ps1 does this and removes it from your PATH).",
            Os::MacOs =>
                "(scripts/uninstall.sh does this; or `nix profile remove mica` if Nix installed it).",
            _ =>
                "(scripts/uninstall.sh does this; or remove the package: sudo apt remove mica, sudo dnf remove mica, sudo pacman -R mica, sudo zypper remove mica).",
        }
    );
    Ok(ExitCode::SUCCESS)
}
