//! What machine is MICA running on?
//!
//! `mica-probe` gathers the facts the decision engine and the security
//! readiness checks need: operating system and distribution, CPU architecture
//! and virtualization support, GPUs and Vulkan, whether this is itself a
//! virtual machine, the init system and package managers, and (on Linux) the
//! platform security state: UEFI Secure Boot, TPM, kernel lockdown, Linux
//! Security Modules, kexec, IOMMU, and kernel-module signing.
//!
//! Probing is strictly read-only. It reads files, lists directories, and runs
//! a few informational commands (`sw_vers`, `systemd-detect-virt`, PowerShell
//! `Get-CimInstance`), each with a timeout. It never changes the system and
//! needs no privileges; facts only root can read are reported as unknown.

mod context;
mod linux;
mod macos;
pub mod os_release;
mod packages;
pub mod security;
mod windows;

use std::fmt;

use serde::Serialize;

pub use context::{COMMAND_TIMEOUT, Context, Runner, decode_output, run};
pub use mica_inspect::Arch;
pub use os_release::{Distro, Family};
pub use packages::PackageManager;
pub use security::SecurityFacts;

/// The operating system family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Os {
    Linux,
    Windows,
    MacOs,
    Other,
}

impl Os {
    pub fn current() -> Os {
        match std::env::consts::OS {
            "linux" => Os::Linux,
            "windows" => Os::Windows,
            "macos" => Os::MacOs,
            _ => Os::Other,
        }
    }
}

impl fmt::Display for Os {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Os::Linux => "Linux",
            Os::Windows => "Windows",
            Os::MacOs => "macOS",
            Os::Other => "other",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Init {
    Systemd,
    OpenRc,
    Runit,
    S6,
    Other,
    NotApplicable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GpuVendor {
    Amd,
    Intel,
    Nvidia,
    Apple,
    Microsoft,
    Virtual,
    Other,
}

impl GpuVendor {
    pub fn from_pci(vendor: u16) -> GpuVendor {
        match vendor {
            0x1002 | 0x1022 => GpuVendor::Amd,
            0x8086 => GpuVendor::Intel,
            0x10DE => GpuVendor::Nvidia,
            0x106B => GpuVendor::Apple,
            0x1414 => GpuVendor::Microsoft,
            0x1AF4 | 0x15AD | 0x1234 | 0x80EE | 0x1B36 => GpuVendor::Virtual,
            _ => GpuVendor::Other,
        }
    }

    pub fn from_name(name: &str) -> GpuVendor {
        let lower = name.to_ascii_lowercase();
        if lower.contains("nvidia") || lower.contains("geforce") || lower.contains("quadro") {
            GpuVendor::Nvidia
        } else if lower.contains("amd") || lower.contains("radeon") || lower.contains("ati ") {
            GpuVendor::Amd
        } else if lower.contains("intel") || lower.contains("arc ") || lower.contains("iris") {
            GpuVendor::Intel
        } else if lower.contains("apple") {
            GpuVendor::Apple
        } else if lower.contains("microsoft") || lower.contains("basic render") {
            GpuVendor::Microsoft
        } else if lower.contains("vmware")
            || lower.contains("virtio")
            || lower.contains("virtualbox")
            || lower.contains("qxl")
        {
            GpuVendor::Virtual
        } else {
            GpuVendor::Other
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Gpu {
    pub vendor: GpuVendor,
    pub name: String,
    /// The kernel driver (Linux) or driver version (Windows), when known.
    pub driver: Option<String>,
    /// PCI address or device path, when known.
    pub location: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Vulkan {
    /// The Vulkan loader library is installed.
    pub loader: bool,
    /// Installed client drivers (ICD manifest names).
    pub drivers: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Virtualization {
    /// The CPU advertises hardware virtualization (VT-x, AMD-V, or the
    /// platform hypervisor on macOS).
    pub cpu_support: Option<bool>,
    /// `/dev/kvm` exists.
    pub kvm: bool,
    /// This user can open `/dev/kvm`.
    pub kvm_accessible: bool,
    /// This system is itself a virtual machine, and which kind.
    pub guest_of: Option<String>,
    /// Running inside Windows Subsystem for Linux.
    pub wsl: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Device {
    pub vendor: Option<String>,
    pub product: Option<String>,
    /// SMBIOS chassis type, e.g. "desktop", "laptop", "handheld".
    pub chassis: Option<String>,
    pub steam_deck: bool,
    pub handheld: bool,
}

/// Everything probing learned about the host.
#[derive(Clone, Debug, Serialize)]
pub struct HostProfile {
    pub os: Os,
    /// Human-readable OS name and version.
    pub os_name: String,
    pub os_version: Option<String>,
    pub distro: Option<Distro>,
    pub kernel: Option<String>,
    pub arch: Arch,
    /// This process runs under CPU translation (Rosetta 2, Windows on ARM
    /// x64 emulation).
    pub translated: bool,
    pub cpu_model: Option<String>,
    pub gpus: Vec<Gpu>,
    pub vulkan: Vulkan,
    pub virtualization: Virtualization,
    pub init: Init,
    pub package_managers: Vec<PackageManager>,
    /// Why the root filesystem is read-only or declarative, if it is.
    pub immutable: Option<String>,
    pub device: Device,
    pub desktop: Option<String>,
    pub session: Option<String>,
    pub security: SecurityFacts,
    /// Facts that could not be read, and why.
    pub unknowns: Vec<String>,
}

impl HostProfile {
    pub fn empty(os: Os) -> HostProfile {
        HostProfile {
            os,
            os_name: os.to_string(),
            os_version: None,
            distro: None,
            kernel: None,
            arch: Arch::host(),
            translated: false,
            cpu_model: None,
            gpus: Vec::new(),
            vulkan: Vulkan::default(),
            virtualization: Virtualization::default(),
            init: Init::NotApplicable,
            package_managers: Vec::new(),
            immutable: None,
            device: Device::default(),
            desktop: None,
            session: None,
            security: SecurityFacts::default(),
            unknowns: Vec::new(),
        }
    }
}

/// How much to probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    /// Everything, including platform security and slow queries (about a
    /// second on Windows, for PowerShell).
    Full,
    /// What choosing a backend needs: OS, distribution, CPU, GPUs, Vulkan,
    /// virtualization. No security facts and no slow helper commands.
    Light,
}

/// Probe the running system.
pub fn probe() -> HostProfile {
    probe_with(&Context::live(), Os::current())
}

/// Probe the system `context` describes, as an `os` host.
pub fn probe_with(context: &Context, os: Os) -> HostProfile {
    probe_depth(context, os, Depth::Full)
}

pub fn probe_depth(context: &Context, os: Os, depth: Depth) -> HostProfile {
    let mut profile = HostProfile::empty(os);
    match os {
        Os::Linux => linux::probe(context, &mut profile, depth),
        Os::Windows => windows::probe(context, &mut profile, depth),
        Os::MacOs => macos::probe(context, &mut profile, depth),
        Os::Other => profile
            .unknowns
            .push(format!("{} is not a platform MICA probes", std::env::consts::OS)),
    }
    profile.package_managers = packages::detect(context, os);
    profile.desktop = context.env("XDG_CURRENT_DESKTOP").map(str::to_string);
    profile.session = context.env("XDG_SESSION_TYPE").map(str::to_string);
    profile
}
