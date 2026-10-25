//! Installing runtimes through the system's own package manager.
//!
//! MICA never downloads runtime binaries itself. It prints the exact package
//! manager command for the user's distribution and runs it only after the
//! user confirms; the package manager does its own signature checking. On
//! image-based systems (rpm-ostree, NixOS, SteamOS) MICA prints instructions
//! instead of changing the image.

use std::fmt;

use serde::Serialize;

use mica_probe::{Family, HostProfile, Os, PackageManager};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Feature {
    /// Windows programs on Linux or macOS.
    Windows,
    /// Vulkan drivers and diagnostics (DXVK and VKD3D-Proton need Vulkan).
    Vulkan,
    /// Virtual machines (QEMU/KVM with libvirt).
    Virtualization,
    /// Secure Boot and TPM inspection tools for `mica security check`.
    SecurityTools,
    /// Linux programs on Windows.
    Linux,
}

impl Feature {
    pub fn parse(name: &str) -> Option<Feature> {
        match name {
            "windows" | "wine" => Some(Feature::Windows),
            "vulkan" => Some(Feature::Vulkan),
            "vm" | "virtualization" => Some(Feature::Virtualization),
            "security" | "security-tools" => Some(Feature::SecurityTools),
            "linux" | "wsl" => Some(Feature::Linux),
            _ => None,
        }
    }
}

impl fmt::Display for Feature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Feature::Windows => "Windows applications",
            Feature::Vulkan => "Vulkan graphics",
            Feature::Virtualization => "virtual machines",
            Feature::SecurityTools => "security inspection tools",
            Feature::Linux => "Linux applications",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InstallPlan {
    pub feature: Feature,
    pub manager: Option<PackageManager>,
    pub packages: Vec<String>,
    /// Commands to run, in order, each as program and arguments.
    pub commands: Vec<Vec<String>>,
    /// Instructions when MICA should not run anything itself.
    pub manual: Vec<String>,
    pub notes: Vec<String>,
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

/// Packages for a feature on a distribution family. Package names were
/// checked against each distribution's repositories (docs/VALIDATION.md).
fn packages(feature: Feature, family: Family) -> Option<(&'static [&'static str], &'static str)> {
    use Family::*;
    use Feature::*;
    let entry: (&[&str], &str) = match (feature, family) {
        (Windows, Debian | Fedora | Suse | Alpine | Void) => (&["wine"], ""),
        (Windows, Arch) => (
            &["wine"],
            "Arch's wine package needs the [multilib] repository enabled in /etc/pacman.conf.",
        ),
        (Vulkan, Debian) => (
            &["mesa-vulkan-drivers", "vulkan-tools"],
            "NVIDIA's proprietary driver provides its own Vulkan driver.",
        ),
        (Vulkan, Fedora) => (
            &["mesa-vulkan-drivers", "vulkan-tools"],
            "NVIDIA's proprietary driver provides its own Vulkan driver.",
        ),
        (Vulkan, Arch) => (
            &["vulkan-icd-loader", "vulkan-tools"],
            "Also install your GPU's driver: vulkan-radeon (AMD), vulkan-intel (Intel), or nvidia-utils (NVIDIA).",
        ),
        (Vulkan, Suse) => (
            &[
                "libvulkan1",
                "vulkan-tools",
                "libvulkan_radeon",
                "libvulkan_intel",
            ],
            "",
        ),
        (Virtualization, Debian) => (
            &[
                "qemu-system-x86",
                "libvirt-daemon-system",
                "virt-manager",
                "ovmf",
                "swtpm",
            ],
            "Add yourself to the libvirt group afterwards: sudo usermod -aG libvirt $USER",
        ),
        (Virtualization, Fedora) => (&["@virtualization", "swtpm"], ""),
        (Virtualization, Arch) => (
            &[
                "qemu-full",
                "libvirt",
                "virt-manager",
                "edk2-ovmf",
                "swtpm",
                "dnsmasq",
            ],
            "Then enable libvirt: sudo systemctl enable --now libvirtd",
        ),
        (Virtualization, Suse) => (&["patterns-server-kvm_server", "virt-manager", "swtpm"], ""),
        (SecurityTools, Debian | Fedora) => (&["mokutil", "tpm2-tools"], ""),
        (SecurityTools, Suse) => (&["mokutil", "tpm2.0-tools"], ""),
        (SecurityTools, Arch) => (&["tpm2-tools"], "mokutil is in the AUR on Arch."),
        _ => return None,
    };
    Some(entry)
}

fn install_command(manager: PackageManager, packages: &[&str]) -> Option<Vec<String>> {
    let base = match manager {
        PackageManager::Apt => "sudo apt-get install -y",
        PackageManager::Dnf => "sudo dnf install -y",
        PackageManager::Pacman => "sudo pacman -S --needed --noconfirm",
        PackageManager::Zypper => "sudo zypper --non-interactive install",
        PackageManager::Apk => "sudo apk add",
        PackageManager::Xbps => "sudo xbps-install -y",
        _ => return None,
    };
    let mut command = words(base);
    command.extend(packages.iter().map(|package| package.to_string()));
    Some(command)
}

/// How to get `feature` on this host.
pub fn plan(profile: &HostProfile, feature: Feature) -> InstallPlan {
    let mut plan = InstallPlan {
        feature,
        manager: None,
        packages: Vec::new(),
        commands: Vec::new(),
        manual: Vec::new(),
        notes: Vec::new(),
    };
    match profile.os {
        Os::Windows => {
            match feature {
                Feature::Linux => {
                    plan.manual = vec![
                        "Install Windows Subsystem for Linux from an administrator terminal, then restart:".into(),
                        "  wsl --install".into(),
                        "WSL installs Ubuntu by default; `wsl --list --online` shows other distributions.".into(),
                    ];
                }
                Feature::Virtualization => {
                    plan.manual = vec![
                        "Windows includes Hyper-V on Pro, Enterprise, and Education editions; Home \
                         editions can use WSL for Linux programs."
                            .into(),
                    ];
                }
                _ => plan
                    .notes
                    .push(format!("{feature} need nothing extra on Windows.")),
            }
            return plan;
        }
        Os::MacOs => {
            match feature {
                Feature::Windows => {
                    plan.manual = vec![
                        "Install CrossOver (commercial, by CodeWeavers; a trial is available) from \
                         https://www.codeweavers.com/crossover — MICA uses its bottles for Windows \
                         programs."
                            .into(),
                        "Apple Silicon also needs Rosetta 2: softwareupdate --install-rosetta".into(),
                    ];
                    plan.notes.push(
                        "Apple's Game Porting Toolkit is licensed for evaluating ports, not for \
                         redistribution; MICA detects an existing installation but never \
                         installs or bundles it."
                            .into(),
                    );
                }
                Feature::Linux | Feature::Virtualization => {
                    if profile.package_managers.contains(&PackageManager::Homebrew) {
                        plan.manager = Some(PackageManager::Homebrew);
                        plan.packages = vec!["lima".into()];
                        plan.commands = vec![words("brew install lima")];
                        plan.notes
                            .push("Lima runs Linux virtual machines on macOS.".into());
                    } else {
                        plan.manual.push(
                            "Install Homebrew (https://brew.sh), then: brew install lima".into(),
                        );
                    }
                }
                _ => plan
                    .notes
                    .push(format!("{feature} need nothing extra on macOS.")),
            }
            return plan;
        }
        Os::Other => {
            plan.manual
                .push("MICA does not manage packages on this operating system.".into());
            return plan;
        }
        Os::Linux => {}
    }

    if feature == Feature::Linux {
        plan.notes.push("This is Linux already.".into());
        return plan;
    }
    let Some(distro) = &profile.distro else {
        plan.manual
            .push("MICA could not identify the distribution (no /etc/os-release).".into());
        return plan;
    };
    if profile.immutable.is_some() {
        plan.manual = match distro.family {
            Family::Nix => vec![
                "Add the packages to environment.systemPackages in configuration.nix (for \
                 Windows programs: wineWowPackages.stable and umu-launcher), then run sudo \
                 nixos-rebuild switch."
                    .into(),
            ],
            _ => vec![
                format!(
                    "This is an image-based system ({}).",
                    profile.immutable.clone().unwrap_or_default()
                ),
                "Prefer Flatpak applications (for example Bottles or Heroic for Windows \
                 programs), or layer packages with your system's tool (rpm-ostree install)."
                    .into(),
            ],
        };
        return plan;
    }
    let Some((packages, note)) = packages(feature, distro.family) else {
        plan.manual.push(format!(
            "MICA has no verified package list for {feature} on {} yet; install them with your \
             package manager.",
            distro.pretty_name
        ));
        return plan;
    };
    let manager = profile.package_managers.iter().copied().find(|manager| {
        matches!(
            (manager, distro.family),
            (PackageManager::Apt, Family::Debian)
                | (PackageManager::Dnf, Family::Fedora)
                | (PackageManager::Pacman, Family::Arch)
                | (PackageManager::Zypper, Family::Suse)
                | (PackageManager::Apk, Family::Alpine)
                | (PackageManager::Xbps, Family::Void)
        )
    });
    let Some(manager) = manager else {
        plan.manual.push(format!(
            "Install these packages with your package manager: {}",
            packages.join(" ")
        ));
        return plan;
    };
    plan.manager = Some(manager);
    plan.packages = packages.iter().map(|package| package.to_string()).collect();
    if manager == PackageManager::Apt {
        plan.commands.push(words("sudo apt-get update"));
    }
    if let Some(command) = install_command(manager, packages) {
        plan.commands.push(command);
    }
    if !note.is_empty() {
        plan.notes.push(note.into());
    }
    if feature == Feature::Windows {
        plan.notes.push(
            "For games, MICA prefers Proton through umu-launcher. Install umu-launcher from your \
             distribution if it packages it, or from https://github.com/Open-Wine-Components/umu-launcher/releases \
             (it ships .deb and .rpm packages); `mica runtimes` shows what MICA found."
                .into(),
        );
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use mica_probe::Distro;

    fn linux(id: &str, family: Family, managers: &[PackageManager]) -> HostProfile {
        let mut profile = HostProfile::empty(Os::Linux);
        profile.distro = Some(Distro {
            id: id.into(),
            id_like: Vec::new(),
            name: id.into(),
            pretty_name: id.into(),
            version_id: None,
            variant_id: None,
            family,
        });
        profile.package_managers = managers.to_vec();
        profile
    }

    #[test]
    fn plans_per_distribution() {
        let ubuntu = plan(
            &linux("ubuntu", Family::Debian, &[PackageManager::Apt]),
            Feature::Windows,
        );
        assert_eq!(ubuntu.commands[0], words("sudo apt-get update"));
        assert_eq!(ubuntu.commands[1], words("sudo apt-get install -y wine"));

        let fedora = plan(
            &linux(
                "fedora",
                Family::Fedora,
                &[PackageManager::Dnf, PackageManager::Flatpak],
            ),
            Feature::Virtualization,
        );
        assert_eq!(
            fedora.commands,
            vec![words("sudo dnf install -y @virtualization swtpm")]
        );

        let arch = plan(
            &linux("arch", Family::Arch, &[PackageManager::Pacman]),
            Feature::Windows,
        );
        assert!(arch.notes[0].contains("multilib"));

        let suse = plan(
            &linux(
                "opensuse-tumbleweed",
                Family::Suse,
                &[PackageManager::Zypper],
            ),
            Feature::Vulkan,
        );
        assert_eq!(suse.manager, Some(PackageManager::Zypper));

        let mut nixos = linux("nixos", Family::Nix, &[PackageManager::Nix]);
        nixos.immutable = Some("NixOS".into());
        let nix = plan(&nixos, Feature::Windows);
        assert!(nix.commands.is_empty());
        assert!(nix.manual[0].contains("configuration.nix"));

        let gentoo = plan(
            &linux("gentoo", Family::Gentoo, &[PackageManager::Emerge]),
            Feature::Windows,
        );
        assert!(gentoo.commands.is_empty());
        assert!(!gentoo.manual.is_empty());
    }

    #[test]
    fn windows_and_macos_get_instructions() {
        let windows = plan(&HostProfile::empty(Os::Windows), Feature::Linux);
        assert!(
            windows
                .manual
                .iter()
                .any(|line| line.contains("wsl --install"))
        );
        let mac = plan(&HostProfile::empty(Os::MacOs), Feature::Windows);
        assert!(mac.manual[0].contains("CrossOver"));
        assert!(mac.commands.is_empty());
    }
}
