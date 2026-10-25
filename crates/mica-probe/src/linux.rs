//! Linux host facts.

use crate::{Context, Depth, Gpu, GpuVendor, HostProfile, Init, os_release, security};

pub(crate) fn probe(context: &Context, profile: &mut HostProfile, depth: Depth) {
    let distro = context
        .read("/etc/os-release")
        .or_else(|| context.read("/usr/lib/os-release"))
        .and_then(|text| os_release::parse(&text));
    match &distro {
        Some(distro) => {
            profile.os_name = distro.pretty_name.clone();
            profile.os_version = distro.version_id.clone();
        }
        None => profile
            .unknowns
            .push("no /etc/os-release: the distribution is unknown".into()),
    }
    profile.kernel = context.read("/proc/sys/kernel/osrelease");

    let cpuinfo = context.read("/proc/cpuinfo").unwrap_or_default();
    profile.cpu_model = cpuinfo
        .lines()
        .find(|line| line.starts_with("model name") || line.starts_with("Model"))
        .and_then(|line| line.split_once(':'))
        .map(|(_, value)| value.trim().to_string());
    let flags: Vec<&str> = cpuinfo
        .lines()
        .find(|line| line.starts_with("flags") || line.starts_with("Features"))
        .and_then(|line| line.split_once(':'))
        .map(|(_, value)| value.split_whitespace().collect())
        .unwrap_or_default();
    let x86 = cpuinfo.lines().any(|line| line.starts_with("flags"));
    profile.virtualization.cpu_support =
        x86.then(|| flags.contains(&"vmx") || flags.contains(&"svm"));
    let hypervisor_flag = flags.contains(&"hypervisor");

    let kernel = profile
        .kernel
        .clone()
        .unwrap_or_default()
        .to_ascii_lowercase();
    profile.virtualization.wsl =
        kernel.contains("microsoft") || context.env("WSL_DISTRO_NAME").is_some();
    profile.virtualization.kvm = context.exists("/dev/kvm");
    profile.virtualization.kvm_accessible =
        profile.virtualization.kvm && context.read_writable("/dev/kvm");
    profile.virtualization.guest_of = if depth == Depth::Full {
        guest_of(context, hypervisor_flag, profile.virtualization.wsl)
    } else {
        profile
            .virtualization
            .wsl
            .then(|| "Windows Subsystem for Linux (a Hyper-V virtual machine)".to_string())
    };

    profile.device = device(context);
    profile.gpus = gpus(context);
    profile.vulkan = vulkan(context);
    profile.init = init(context);
    profile.immutable = immutable(context, distro.as_ref());
    if depth == Depth::Full {
        profile.security = security::probe_linux(context);
        profile
            .unknowns
            .extend(profile.security.needs_privilege.iter().cloned());
    }
    profile.distro = distro;
}

fn dmi(context: &Context, field: &str) -> Option<String> {
    context
        .read(&format!("/sys/class/dmi/id/{field}"))
        .filter(|value| !value.is_empty())
}

fn guest_of(context: &Context, hypervisor_flag: bool, wsl: bool) -> Option<String> {
    if wsl {
        return Some("Windows Subsystem for Linux (a Hyper-V virtual machine)".into());
    }
    if let Some(kind) = context.run("systemd-detect-virt", &["--vm"]) {
        if kind != "none" && !kind.is_empty() {
            return Some(kind);
        }
        return None;
    }
    let vendor = dmi(context, "sys_vendor").unwrap_or_default();
    let product = dmi(context, "product_name").unwrap_or_default();
    let known = [
        ("QEMU", "qemu"),
        ("innotek GmbH", "oracle virtualbox"),
        ("VMware, Inc.", "vmware"),
        ("Xen", "xen"),
        ("Parallels", "parallels"),
        ("Apple Inc.", ""),
    ];
    for (needle, kind) in known {
        if vendor.contains(needle) && !kind.is_empty() {
            return Some(kind.into());
        }
    }
    if vendor.contains("Microsoft") && product.contains("Virtual Machine") {
        return Some("microsoft hyper-v".into());
    }
    if product.contains("KVM") || product.contains("Standard PC (Q35") {
        return Some("kvm".into());
    }
    hypervisor_flag.then(|| "an unidentified hypervisor (the CPU reports one)".into())
}

fn device(context: &Context) -> crate::Device {
    let vendor = dmi(context, "sys_vendor");
    let product = dmi(context, "product_name");
    let board_vendor = dmi(context, "board_vendor").unwrap_or_default();
    let chassis = dmi(context, "chassis_type")
        .and_then(|value| value.parse::<u32>().ok())
        .map(|kind| {
            match kind {
                3 | 4 | 5 | 6 | 7 | 15 | 16 | 35 => "desktop",
                8 | 9 | 10 | 14 => "laptop",
                11 => "handheld",
                30 => "tablet",
                31 => "convertible",
                32 => "detachable",
                13 => "all-in-one",
                17 | 23 | 25 | 28 | 29 => "server",
                _ => "other",
            }
            .to_string()
        });
    let product_name = product.clone().unwrap_or_default();
    let steam_deck = board_vendor == "Valve"
        && (product_name.contains("Jupiter") || product_name.contains("Galileo"));
    crate::Device {
        handheld: steam_deck || chassis.as_deref() == Some("handheld"),
        vendor,
        product,
        chassis,
        steam_deck,
    }
}

fn gpus(context: &Context) -> Vec<Gpu> {
    let mut gpus = Vec::new();
    for card in context.list("/sys/class/drm") {
        let is_card = card
            .strip_prefix("card")
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()));
        if !is_card {
            continue;
        }
        let base = format!("/sys/class/drm/{card}/device");
        let vendor_id = context
            .read(&format!("{base}/vendor"))
            .and_then(|value| u16::from_str_radix(value.trim_start_matches("0x"), 16).ok());
        let device_id = context.read(&format!("{base}/device")).unwrap_or_default();
        let vendor = vendor_id
            .map(GpuVendor::from_pci)
            .unwrap_or(GpuVendor::Other);
        let driver = context.link_name(&format!("{base}/driver"));
        let location = context.link_name(&base);
        let name = format!(
            "{vendor:?} GPU {}:{}",
            vendor_id
                .map(|id| format!("{id:04x}"))
                .unwrap_or_else(|| "????".into()),
            device_id.trim_start_matches("0x")
        );
        gpus.push(Gpu {
            vendor,
            name,
            driver,
            location,
        });
    }
    if gpus.is_empty() && context.exists("/dev/dxg") {
        gpus.push(Gpu {
            vendor: GpuVendor::Microsoft,
            name: "WSL GPU paravirtualization (/dev/dxg): the Windows host's GPU".into(),
            driver: Some("dxgkrnl".into()),
            location: Some("/dev/dxg".into()),
        });
    }
    gpus
}

const VULKAN_LIBRARY_DIRS: &[&str] = &[
    "/usr/lib",
    "/usr/lib64",
    "/usr/lib/x86_64-linux-gnu",
    "/usr/lib/aarch64-linux-gnu",
    "/lib/x86_64-linux-gnu",
    "/lib64",
    "/run/opengl-driver/lib",
];

const VULKAN_ICD_DIRS: &[&str] = &[
    "/usr/share/vulkan/icd.d",
    "/etc/vulkan/icd.d",
    "/usr/local/share/vulkan/icd.d",
    "/run/opengl-driver/share/vulkan/icd.d",
];

fn vulkan(context: &Context) -> crate::Vulkan {
    let loader = VULKAN_LIBRARY_DIRS
        .iter()
        .any(|dir| context.exists(&format!("{dir}/libvulkan.so.1")));
    let mut drivers: Vec<String> = VULKAN_ICD_DIRS
        .iter()
        .flat_map(|dir| context.list(dir))
        .filter(|name| name.ends_with(".json"))
        .collect();
    drivers.sort();
    drivers.dedup();
    crate::Vulkan { loader, drivers }
}

fn init(context: &Context) -> Init {
    if context.is_dir("/run/systemd/system") {
        Init::Systemd
    } else if context.exists("/run/openrc") {
        Init::OpenRc
    } else if context.exists("/run/runit") || context.exists("/etc/runit/runsvdir") {
        Init::Runit
    } else if context.exists("/run/s6") || context.exists("/run/s6-rc") {
        Init::S6
    } else {
        Init::Other
    }
}

fn immutable(context: &Context, distro: Option<&os_release::Distro>) -> Option<String> {
    if context.exists("/run/ostree-booted") {
        return Some(
            "an rpm-ostree image (Fedora Atomic, Universal Blue): install system packages with \
             rpm-ostree or prefer Flatpak"
                .into(),
        );
    }
    let distro = distro?;
    let variant = distro.variant_id.clone().unwrap_or_default();
    match distro.id.as_str() {
        "nixos" => Some(
            "NixOS: system packages are declared in configuration.nix; MICA prints the \
             configuration instead of installing"
                .into(),
        ),
        "steamos" => Some("SteamOS: the root filesystem is read-only; use Flatpak or ~/".into()),
        id if id.contains("microos")
            || id.contains("aeon")
            || id.contains("kalpa")
            || variant.contains("microos") =>
        {
            Some("openSUSE MicroOS family: system packages need transactional-update".into())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{Context, Family, Init, Os, probe_with};

    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Tree {
            let root =
                std::env::temp_dir().join(format!("mica-probe-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Tree(root)
        }

        fn file(&self, path: &str, contents: impl AsRef<[u8]>) -> &Tree {
            let full = self.0.join(path.trim_start_matches('/'));
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, contents).unwrap();
            self
        }

        fn dir(&self, path: &str) -> &Tree {
            std::fs::create_dir_all(self.0.join(path.trim_start_matches('/'))).unwrap();
            self
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn efivar(value: u8) -> Vec<u8> {
        vec![0x06, 0, 0, 0, value]
    }

    /// A Fedora gaming desktop that meets every EAC Linux v2 requirement
    /// MICA can see without root.
    fn hardened_fedora(name: &str) -> Tree {
        let tree = Tree::new(name);
        tree.file(
            "/etc/os-release",
            "NAME=\"Fedora Linux\"\nID=fedora\nVERSION_ID=43\nPRETTY_NAME=\"Fedora Linux 43\"\n",
        )
        .file("/proc/sys/kernel/osrelease", "6.17.4-300.fc43.x86_64\n")
        .file(
            "/proc/cpuinfo",
            "processor : 0\nmodel name : AMD Ryzen 7 7800X3D\nflags : fpu vme svm sse2\n",
        )
        .file(
            "/proc/cmdline",
            "BOOT_IMAGE=(hd0,gpt2)/vmlinuz root=UUID=1 ro rhgb quiet lockdown=integrity \
             sysctl.kernel.kexec_load_disabled=1 lsm=lockdown,yama,integrity,selinux,bpf\n",
        )
        .dir("/sys/firmware/efi/efivars")
        .file(
            &format!(
                "/sys/firmware/efi/efivars/SecureBoot-{}",
                crate::security::EFI_GLOBAL_GUID
            ),
            efivar(1),
        )
        .file(
            &format!(
                "/sys/firmware/efi/efivars/SetupMode-{}",
                crate::security::EFI_GLOBAL_GUID
            ),
            efivar(0),
        )
        .file("/sys/class/tpm/tpm0/tpm_version_major", "2\n")
        .file(crate::security::EVENT_LOG, b"")
        .file(
            "/sys/kernel/security/lockdown",
            "none [integrity] confidentiality\n",
        )
        .file(
            "/sys/kernel/security/lsm",
            "capability,lockdown,yama,integrity,selinux,bpf",
        )
        .file("/proc/sys/kernel/kexec_load_disabled", "1\n")
        .dir("/sys/class/iommu/ivhd0")
        .file("/sys/kernel/iommu_groups/0/type", "DMA-FQ\n")
        .file("/sys/kernel/iommu_groups/1/type", "DMA-FQ\n")
        .file("/sys/module/module/parameters/sig_enforce", "Y\n")
        .file("/proc/sys/kernel/tainted", "0\n")
        .file("/sys/class/drm/card1/device/vendor", "0x1002\n")
        .file("/sys/class/drm/card1/device/device", "0x744c\n")
        .dir("/sys/class/drm/card1-DP-1")
        .file("/usr/lib64/libvulkan.so.1", "")
        .file("/usr/share/vulkan/icd.d/radeon_icd.x86_64.json", "{}")
        .dir("/run/systemd/system")
        .file(
            "/sys/class/dmi/id/sys_vendor",
            "Micro-Star International Co., Ltd.\n",
        )
        .file("/sys/class/dmi/id/chassis_type", "3\n")
        .file("/usr/bin/dnf", "")
        .file("/usr/bin/flatpak", "");
        tree
    }

    fn context(tree: &Tree) -> Context {
        Context::fake(
            &tree.0,
            &[("PATH", "/usr/bin"), ("HOME", "/home/player")],
            Context::no_commands(),
        )
    }

    #[test]
    fn probes_a_hardened_fedora_desktop() {
        let tree = hardened_fedora("fedora");
        let profile = probe_with(&context(&tree), Os::Linux);
        let distro = profile.distro.as_ref().unwrap();
        assert_eq!(distro.family, Family::Fedora);
        assert_eq!(profile.os_name, "Fedora Linux 43");
        assert_eq!(profile.cpu_model.as_deref(), Some("AMD Ryzen 7 7800X3D"));
        assert_eq!(profile.virtualization.cpu_support, Some(true));
        assert_eq!(profile.virtualization.guest_of, None);
        assert_eq!(profile.init, Init::Systemd);
        assert_eq!(profile.gpus.len(), 1, "connectors are not GPUs");
        assert_eq!(profile.gpus[0].vendor, crate::GpuVendor::Amd);
        assert!(profile.vulkan.loader);
        assert_eq!(profile.vulkan.drivers, vec!["radeon_icd.x86_64.json"]);
        assert_eq!(profile.device.chassis.as_deref(), Some("desktop"));
        assert!(!profile.device.steam_deck);
        assert_eq!(
            profile
                .package_managers
                .iter()
                .map(|manager| manager.program())
                .collect::<Vec<_>>(),
            vec!["dnf", "flatpak"]
        );

        let security = &profile.security;
        assert_eq!(security.uefi, Some(true));
        assert_eq!(security.secure_boot, Some(true));
        assert_eq!(security.setup_mode, Some(false));
        assert_eq!(security.tpm_version, Some(2));
        assert!(security.event_log_present);
        assert!(
            !security.event_log_readable,
            "an empty read counts as unreadable"
        );
        assert_eq!(security.lockdown.as_deref(), Some("integrity"));
        assert!(security.lsms.as_ref().unwrap().contains(&"bpf".to_string()));
        assert_eq!(security.kexec_load_disabled, Some(true));
        assert_eq!(security.iommu_units, vec!["ivhd0"]);
        assert_eq!(security.iommu_groups, 2);
        assert_eq!(security.module_sig_enforce, Some(true));
        assert!(security.cmdline_has("lockdown=integrity"));
    }

    #[test]
    fn probes_a_steam_deck_and_wsl() {
        let deck = Tree::new("deck");
        deck.file(
            "/etc/os-release",
            "ID=steamos\nID_LIKE=arch\nNAME=SteamOS\n",
        )
        .file("/sys/class/dmi/id/board_vendor", "Valve\n")
        .file("/sys/class/dmi/id/product_name", "Jupiter\n")
        .file("/usr/bin/pacman", "");
        let profile = probe_with(&context(&deck), Os::Linux);
        assert!(profile.device.steam_deck);
        assert!(profile.device.handheld);
        assert!(profile.immutable.as_deref().unwrap().contains("SteamOS"));
        assert_eq!(profile.distro.as_ref().unwrap().family, Family::Arch);
        assert_eq!(profile.security.uefi, Some(false));
        assert_eq!(profile.init, Init::Other);

        let wsl = Tree::new("wsl");
        wsl.file("/etc/os-release", "ID=kali\nID_LIKE=debian\n")
            .file(
                "/proc/sys/kernel/osrelease",
                "6.6.87.2-microsoft-standard-WSL2\n",
            )
            .file("/dev/dxg", "");
        let profile = probe_with(&context(&wsl), Os::Linux);
        assert!(profile.virtualization.wsl);
        assert!(profile.virtualization.guest_of.unwrap().contains("Hyper-V"));
        assert_eq!(profile.gpus[0].vendor, crate::GpuVendor::Microsoft);
    }

    #[test]
    fn uses_systemd_detect_virt_when_present() {
        let tree = Tree::new("vm");
        tree.file("/etc/os-release", "ID=debian\n");
        let context = Context::fake(
            &tree.0,
            &[],
            Box::new(|program, args| {
                (program == "systemd-detect-virt" && args == ["--vm"]).then(|| b"kvm\n".to_vec())
            }),
        );
        let profile = probe_with(&context, Os::Linux);
        assert_eq!(profile.virtualization.guest_of.as_deref(), Some("kvm"));
    }
}
