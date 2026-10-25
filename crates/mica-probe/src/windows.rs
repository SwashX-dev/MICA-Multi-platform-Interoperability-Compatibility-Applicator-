//! Windows host facts, from one PowerShell `Get-CimInstance` query and the
//! registry values ordinary users may read.

use serde::Deserialize;

use crate::{Context, Depth, Gpu, GpuVendor, HostProfile, Init};

/// Gathers everything in one PowerShell start-up (about a second), as JSON.
const QUERY: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
$os = Get-CimInstance Win32_OperatingSystem
$cs = Get-CimInstance Win32_ComputerSystem
$cpu = (Get-ItemProperty 'HKLM:\HARDWARE\DESCRIPTION\System\CentralProcessor\0').ProcessorNameString
# Win32_Processor takes about a second; only ask it when no hypervisor runs.
$vt = if ($cs.HypervisorPresent) { $true } else { (Get-CimInstance Win32_Processor | Select-Object -First 1).VirtualizationFirmwareEnabled }
$gpus = @(Get-CimInstance Win32_VideoController | ForEach-Object { @{ name = $_.Name; driver = $_.DriverVersion; pnp = $_.PNPDeviceID } })
$sb = (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\SecureBoot\State').UEFISecureBootEnabled
# The TPM's device name ("Trusted Platform Module 2.0") is readable without
# administrator rights, unlike Win32_Tpm.
$tpm = @(Get-CimInstance Win32_PnPEntity -Filter "PNPClass='SecurityDevices'" | ForEach-Object { $_.Name }) -join '; '
@{
  caption = $os.Caption; version = $os.Version; build = $os.BuildNumber
  manufacturer = $cs.Manufacturer; model = $cs.Model; hypervisor = $cs.HypervisorPresent
  cpu = $cpu; vt = $vt
  gpus = $gpus; secureboot = $sb; firmware = $env:firmware_type; tpm = $tpm
  chassis = @((Get-CimInstance Win32_SystemEnclosure).ChassisTypes)
} | ConvertTo-Json -Depth 4 -Compress
"#;

#[derive(Deserialize, Default)]
struct Facts {
    caption: Option<String>,
    version: Option<String>,
    build: Option<String>,
    manufacturer: Option<String>,
    model: Option<String>,
    hypervisor: Option<bool>,
    cpu: Option<String>,
    vt: Option<bool>,
    #[serde(default)]
    gpus: Vec<GpuFact>,
    secureboot: Option<u32>,
    firmware: Option<String>,
    tpm: Option<String>,
    #[serde(default)]
    chassis: Vec<u32>,
}

#[derive(Deserialize)]
struct GpuFact {
    name: Option<String>,
    driver: Option<String>,
    pnp: Option<String>,
}

pub(crate) fn probe(context: &Context, profile: &mut HostProfile, depth: Depth) {
    profile.init = Init::NotApplicable;
    if depth == Depth::Light {
        light(context, profile);
        return;
    }
    let output = context.run(
        "powershell",
        &["-NoProfile", "-NonInteractive", "-Command", QUERY],
    );
    let Some(facts) = output.and_then(|json| serde_json::from_str::<Facts>(&json).ok()) else {
        profile
            .unknowns
            .push("PowerShell's Get-CimInstance query did not answer".into());
        return;
    };
    apply(&facts, context, profile);
}

/// Facts available without PowerShell: environment and files.
fn light(context: &Context, profile: &mut HostProfile) {
    profile.os_name = "Windows".into();
    let system_root = context.env("SystemRoot").unwrap_or(r"C:\Windows");
    profile.vulkan.loader = std::path::Path::new(system_root)
        .join(r"System32\vulkan-1.dll")
        .is_file();
    let identifier = context.env("PROCESSOR_IDENTIFIER").unwrap_or_default();
    profile.translated = cfg!(target_arch = "x86_64") && identifier.contains("ARM");
}

fn apply(facts: &Facts, context: &Context, profile: &mut HostProfile) {
    if let Some(caption) = &facts.caption {
        profile.os_name = caption.trim().to_string();
    }
    profile.os_version = match (&facts.version, &facts.build) {
        (Some(version), _) => Some(version.clone()),
        (None, Some(build)) => Some(build.clone()),
        _ => None,
    };
    profile.kernel = facts.version.clone();
    profile.cpu_model = facts.cpu.as_ref().map(|cpu| cpu.trim().to_string());
    // With Hyper-V (or virtualization-based security) running, Windows
    // reports firmware virtualization as off because the hypervisor owns it.
    profile.virtualization.cpu_support = match (facts.vt, facts.hypervisor) {
        (_, Some(true)) => Some(true),
        (vt, _) => vt,
    };
    let model = facts.model.clone().unwrap_or_default();
    let manufacturer = facts.manufacturer.clone().unwrap_or_default();
    profile.virtualization.guest_of =
        if model.contains("Virtual Machine") && manufacturer.contains("Microsoft") {
            Some("microsoft hyper-v".into())
        } else if model.contains("VMware") || manufacturer.contains("VMware") {
            Some("vmware".into())
        } else if model.contains("VirtualBox") {
            Some("oracle virtualbox".into())
        } else if manufacturer.contains("QEMU") || model.contains("KVM") {
            Some("kvm".into())
        } else {
            None
        };
    profile.device.vendor = facts.manufacturer.clone();
    profile.device.product = facts.model.clone();
    profile.device.chassis = facts.chassis.first().map(|kind| {
        match kind {
            3..=7 | 15 | 16 | 35 => "desktop",
            8..=10 | 14 => "laptop",
            11 => "handheld",
            30 => "tablet",
            31 => "convertible",
            32 => "detachable",
            _ => "other",
        }
        .to_string()
    });
    profile.device.handheld = profile.device.chassis.as_deref() == Some("handheld");
    profile.gpus = facts
        .gpus
        .iter()
        .map(|gpu| {
            let name = gpu.name.clone().unwrap_or_else(|| "unknown GPU".into());
            Gpu {
                vendor: GpuVendor::from_name(&name),
                name,
                driver: gpu.driver.clone(),
                location: gpu.pnp.clone(),
            }
        })
        .collect();
    let system_root = context.env("SystemRoot").unwrap_or(r"C:\Windows");
    profile.vulkan.loader = std::path::Path::new(system_root)
        .join(r"System32\vulkan-1.dll")
        .is_file();

    let security = &mut profile.security;
    security.uefi = facts
        .firmware
        .as_deref()
        .map(|kind| kind.eq_ignore_ascii_case("UEFI"));
    security.secure_boot = facts.secureboot.map(|value| value == 1);
    let tpm = facts.tpm.clone().unwrap_or_default();
    security.tpm_present = Some(tpm.contains("Trusted Platform Module"));
    security.tpm_version = tpm
        .split("Trusted Platform Module")
        .nth(1)
        .and_then(|rest| rest.trim().split('.').next())
        .and_then(|major| major.parse().ok());
    profile
        .unknowns
        .extend(security.needs_privilege.iter().cloned());

    // An x86-64 MICA on an ARM64 PC runs under Windows' emulation.
    let identifier = context.env("PROCESSOR_IDENTIFIER").unwrap_or_default();
    profile.translated = cfg!(target_arch = "x86_64") && identifier.contains("ARM");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Os, probe_with};

    #[test]
    fn reads_the_powershell_answer() {
        let json = r#"{"caption":"Microsoft Windows 11 Home","version":"10.0.26300","build":"26300",
            "manufacturer":"ASUSTeK","model":"ROG Strix","hypervisor":true,"cpu":"AMD Ryzen 5 7520U  ",
            "vt":false,"gpus":[{"name":"NVIDIA GeForce RTX 4060","driver":"32.0.15.8","pnp":"PCI\\VEN_10DE"}],
            "secureboot":1,"firmware":"UEFI","tpm":"Trusted Platform Module 2.0 AMD PSP 11.0 Device","chassis":[10]}"#;
        let dir = std::env::temp_dir();
        let context = crate::Context::fake(
            &dir,
            &[("SystemRoot", r"C:\nowhere")],
            Box::new(move |program, _| (program == "powershell").then(|| json.as_bytes().to_vec())),
        );
        let profile = probe_with(&context, Os::Windows);
        assert_eq!(profile.os_name, "Microsoft Windows 11 Home");
        assert_eq!(profile.cpu_model.as_deref(), Some("AMD Ryzen 5 7520U"));
        assert_eq!(profile.virtualization.cpu_support, Some(true));
        assert_eq!(profile.gpus[0].vendor, GpuVendor::Nvidia);
        assert_eq!(profile.security.secure_boot, Some(true));
        assert_eq!(profile.security.uefi, Some(true));
        assert_eq!(profile.security.tpm_version, Some(2));
        assert_eq!(profile.device.chassis.as_deref(), Some("laptop"));
    }

    #[test]
    fn survives_no_powershell() {
        let dir = std::env::temp_dir();
        let context = crate::Context::fake(&dir, &[], crate::Context::no_commands());
        let profile = probe_with(&context, Os::Windows);
        assert!(profile.unknowns[0].contains("PowerShell"));
        assert_eq!(profile.security.tpm_present, None);
    }
}
