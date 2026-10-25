//! Raw platform-security facts. `mica-security` interprets them.
//!
//! Linux sources (all read-only):
//!
//! | Fact | Source |
//! | --- | --- |
//! | UEFI boot | `/sys/firmware/efi` |
//! | Secure Boot, Setup Mode, `db`, `MokListRT` | efivarfs, `/sys/firmware/efi/mok-variables` |
//! | TPM | `/sys/class/tpm/tpm0`, `/dev/tpmrm0` |
//! | Measured-boot event log | `/sys/kernel/security/tpm0/binary_bios_measurements` (root only) |
//! | Kernel lockdown | `/sys/kernel/security/lockdown` |
//! | Active LSMs | `/sys/kernel/security/lsm` |
//! | kexec | `/proc/sys/kernel/kexec_load_disabled`, `/sys/kernel/kexec_loaded` |
//! | IOMMU | `/sys/class/iommu`, `/sys/kernel/iommu_groups/*/type` |
//! | Module signing | `/sys/module/module/parameters/sig_enforce`, `/sys/module/*/taint`, `/proc/sys/kernel/tainted` |
//! | Kernel command line | `/proc/cmdline` |

use std::collections::BTreeMap;

use serde::Serialize;

use crate::Context;

/// EFI global-variable vendor GUID (SecureBoot, SetupMode).
pub const EFI_GLOBAL_GUID: &str = "8be4df61-93ca-11d2-aa0d-00e098032b8c";
/// EFI image-security-database GUID (db, dbx).
pub const EFI_IMAGE_SECURITY_GUID: &str = "d719b2cb-3d3a-4596-a3bc-dad00e67656f";
/// shim's vendor GUID (MokListRT).
pub const SHIM_LOCK_GUID: &str = "605dab50-e046-4300-abb6-3dd810dd8b23";

pub const EVENT_LOG: &str = "/sys/kernel/security/tpm0/binary_bios_measurements";

/// A loaded kernel module that taints the kernel, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TaintedModule {
    pub name: String,
    /// Taint letters: `O` out-of-tree, `E` unsigned, `P` proprietary, ...
    pub taint: String,
    /// The module's signer, from `modinfo -F signer`, when available.
    pub signer: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SecurityFacts {
    pub uefi: Option<bool>,
    pub secure_boot: Option<bool>,
    pub setup_mode: Option<bool>,
    pub tpm_present: Option<bool>,
    pub tpm_version: Option<u8>,
    pub event_log_present: bool,
    pub event_log_readable: bool,
    /// The active kernel lockdown mode: `none`, `integrity`, or
    /// `confidentiality`. `None` if the lockdown LSM is not built in.
    pub lockdown: Option<String>,
    pub lsms: Option<Vec<String>>,
    pub kexec_load_disabled: Option<bool>,
    /// The kernel was built with kexec support.
    pub kexec_built_in: Option<bool>,
    pub iommu_units: Vec<String>,
    pub iommu_groups: usize,
    /// Domain types of IOMMU groups (`DMA`, `DMA-FQ`, `identity`, ...) and
    /// how many groups have each.
    pub iommu_group_types: BTreeMap<String, usize>,
    pub cmdline: Option<String>,
    pub module_sig_enforce: Option<bool>,
    pub tainted: Option<u64>,
    pub tainted_modules: Vec<TaintedModule>,
    /// Raw EFI_SIGNATURE_LISTs, attributes stripped.
    #[serde(skip)]
    pub db: Option<Vec<u8>>,
    #[serde(skip)]
    pub mok: Option<Vec<u8>>,
    /// The raw TCG event log, when this process may read it.
    #[serde(skip)]
    pub event_log: Option<Vec<u8>>,
    /// Facts that need administrator or root rights to read.
    pub needs_privilege: Vec<String>,
}

impl SecurityFacts {
    /// Whether the kernel command line has `parameter` (`name` or
    /// `name=value`), matching `name=value` exactly when a value is given.
    pub fn cmdline_has(&self, parameter: &str) -> bool {
        self.cmdline_value_of(parameter.split('=').next().unwrap_or(parameter))
            .is_some_and(|value| match parameter.split_once('=') {
                Some((_, wanted)) => value.as_deref() == Some(wanted),
                None => true,
            })
    }

    /// The value of the last occurrence of a kernel parameter: `Some(None)`
    /// for a bare flag, `None` when absent. The kernel honours the last one.
    pub fn cmdline_value_of(&self, name: &str) -> Option<Option<String>> {
        let cmdline = self.cmdline.as_deref()?;
        cmdline
            .split_whitespace()
            .filter_map(|word| match word.split_once('=') {
                Some((key, value)) if key == name => Some(Some(value.to_string())),
                None if word == name => Some(None),
                _ => None,
            })
            .next_back()
    }
}

/// An EFI variable's payload, without efivarfs' 4-byte attribute prefix.
fn efi_variable(context: &Context, name: &str, guid: &str) -> Option<Vec<u8>> {
    let data = context.read_bytes(&format!("/sys/firmware/efi/efivars/{name}-{guid}"))?;
    (data.len() >= 4).then(|| data[4..].to_vec())
}

fn efi_flag(context: &Context, name: &str) -> Option<bool> {
    efi_variable(context, name, EFI_GLOBAL_GUID)?
        .first()
        .map(|&value| value == 1)
}

pub(crate) fn probe_linux(context: &Context) -> SecurityFacts {
    let mut facts = SecurityFacts {
        uefi: Some(context.is_dir("/sys/firmware/efi")),
        ..SecurityFacts::default()
    };
    if facts.uefi == Some(true) {
        facts.secure_boot = efi_flag(context, "SecureBoot");
        facts.setup_mode = efi_flag(context, "SetupMode");
        facts.db = efi_variable(context, "db", EFI_IMAGE_SECURITY_GUID);
        facts.mok = context
            .read_bytes("/sys/firmware/efi/mok-variables/MokListRT")
            .or_else(|| efi_variable(context, "MokListRT", SHIM_LOCK_GUID));
        if facts.secure_boot.is_none() {
            facts
                .needs_privilege
                .push("the SecureBoot EFI variable could not be read".into());
        }
    }

    let tpm = "/sys/class/tpm/tpm0";
    facts.tpm_present = Some(context.exists(tpm));
    facts.tpm_version = context
        .read(&format!("{tpm}/tpm_version_major"))
        .and_then(|value| value.parse().ok())
        .or_else(|| context.exists("/dev/tpmrm0").then_some(2));
    facts.event_log_present = context.exists(EVENT_LOG);
    if facts.event_log_present {
        facts.event_log = context.read_bytes(EVENT_LOG).filter(|log| !log.is_empty());
        facts.event_log_readable = facts.event_log.is_some();
        if !facts.event_log_readable {
            facts.needs_privilege.push(format!(
                "the measured-boot event log ({EVENT_LOG}) is readable only by root"
            ));
        }
    }

    facts.lockdown = context
        .read("/sys/kernel/security/lockdown")
        .and_then(|text| active_choice(&text));
    facts.lsms = context.read("/sys/kernel/security/lsm").map(|text| {
        text.split(',')
            .map(str::trim)
            .filter(|lsm| !lsm.is_empty())
            .map(str::to_string)
            .collect()
    });
    facts.kexec_load_disabled = context
        .read("/proc/sys/kernel/kexec_load_disabled")
        .map(|value| value == "1");
    facts.kexec_built_in = Some(
        context.exists("/proc/sys/kernel/kexec_load_disabled")
            || context.exists("/sys/kernel/kexec_loaded"),
    );

    facts.iommu_units = context.list("/sys/class/iommu");
    let groups = context.list("/sys/kernel/iommu_groups");
    facts.iommu_groups = groups.len();
    for group in &groups {
        if let Some(kind) = context.read(&format!("/sys/kernel/iommu_groups/{group}/type")) {
            *facts.iommu_group_types.entry(kind).or_default() += 1;
        }
    }

    facts.cmdline = context.read("/proc/cmdline");
    facts.module_sig_enforce = context
        .read("/sys/module/module/parameters/sig_enforce")
        .map(|value| value == "Y");
    facts.tainted = context
        .read("/proc/sys/kernel/tainted")
        .and_then(|value| value.parse().ok());
    for module in context.list("/sys/module") {
        let Some(taint) = context.read(&format!("/sys/module/{module}/taint")) else {
            continue;
        };
        if taint.is_empty() {
            continue;
        }
        let signer = context
            .run("modinfo", &["-F", "signer", &module])
            .filter(|signer| !signer.is_empty());
        facts.tainted_modules.push(TaintedModule {
            name: module,
            taint,
            signer,
        });
    }
    facts
}

/// The bracketed choice in a sysfs selector such as `none [integrity] confidentiality`.
pub fn active_choice(text: &str) -> Option<String> {
    let start = text.find('[')?;
    let end = text[start..].find(']')? + start;
    Some(text[start + 1..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_bracketed_choices_and_cmdline() {
        assert_eq!(
            active_choice("none [integrity] confidentiality").as_deref(),
            Some("integrity")
        );
        assert_eq!(active_choice("none integrity"), None);
        let facts = SecurityFacts {
            cmdline: Some("BOOT_IMAGE=/vmlinuz quiet iommu=pt lsm=landlock,bpf iommu=on".into()),
            ..SecurityFacts::default()
        };
        assert!(facts.cmdline_has("quiet"));
        assert!(facts.cmdline_has("iommu"));
        assert!(facts.cmdline_has("iommu=on"));
        assert!(!facts.cmdline_has("iommu=pt"), "the last value wins");
        assert_eq!(
            facts.cmdline_value_of("lsm"),
            Some(Some("landlock,bpf".into()))
        );
        assert_eq!(facts.cmdline_value_of("lockdown"), None);
    }
}
