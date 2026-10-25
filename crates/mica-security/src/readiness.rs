//! EAC Linux v2 readiness: does this boot meet Epic's published requirements?
//!
//! Each requirement below quotes Epic's announcement of 2026-10-14 (see
//! [`SOURCE`]). MICA checks what it can see from the running system and says
//! plainly when it cannot see something (most often because only root may
//! read the TPM event log). A "ready" verdict means *every requirement MICA
//! can check is met*; Epic's own anti-cheat service makes the final decision
//! when a protected game starts, and Epic says a failed check shows an error
//! message rather than a ban.
//!
//! Nothing here changes the system. Fixes are described; the reversible ones
//! (kernel parameters) can be applied through [`crate::boot_profile`].

use serde::Serialize;

use mica_probe::{HostProfile, Init, Os};

use crate::efi;
use crate::tcg;
use crate::wellknown::{self, KeyMatch};

pub const SOURCE: &str =
    "https://onlineservices.epicgames.com/news/introducing-easy-anti-cheat-linux-v2";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Met,
    NotMet,
    /// MICA cannot tell (needs root, or the kernel does not expose it).
    Unknown,
    NotApplicable,
}

/// What resolving a failed requirement involves, from least to most
/// invasive. MICA automates only [`FixKind::KernelParameters`], and only
/// through a separate, removable boot entry with explicit consent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FixKind {
    /// Add or remove kernel command-line parameters.
    KernelParameters,
    /// Install or switch software (systemd, a distribution kernel).
    Software,
    /// Change UEFI firmware ("BIOS") settings.
    Firmware,
    /// Remove or replace enrolled Secure Boot keys or MOKs.
    Keys,
    /// Not fixable by configuration (hardware, publisher decision).
    NotFixable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Fix {
    pub kind: FixKind,
    pub summary: String,
    pub steps: Vec<String>,
    /// Kernel parameters this fix adds.
    pub add_parameters: Vec<String>,
    /// Kernel parameters this fix removes (by name).
    pub remove_parameters: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub id: &'static str,
    pub title: &'static str,
    /// Epic's requirement, in brief.
    pub requirement: &'static str,
    pub status: Status,
    pub details: Vec<String>,
    pub fix: Option<Fix>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// Every requirement MICA can check is met.
    Ready,
    /// Nothing failed, but some requirements could not be checked.
    Incomplete,
    NotReady,
    /// EAC Linux v2 does not apply to this kind of system.
    NotApplicable,
}

#[derive(Clone, Debug, Serialize)]
pub struct Readiness {
    pub profile: &'static str,
    pub verdict: Verdict,
    pub findings: Vec<Finding>,
    pub notes: Vec<String>,
    pub source: &'static str,
    /// Summary of the measured-boot log, when it could be read.
    pub event_log: Option<tcg::Summary>,
}

impl Readiness {
    pub fn finding(&self, id: &str) -> Option<&Finding> {
        self.findings.iter().find(|finding| finding.id == id)
    }

    /// Kernel parameters every failed requirement asks to add or remove,
    /// merged, in a stable order.
    pub fn kernel_parameters(&self) -> (Vec<String>, Vec<String>) {
        let mut add: Vec<String> = Vec::new();
        let mut remove: Vec<String> = Vec::new();
        for fix in self
            .findings
            .iter()
            .filter(|finding| finding.status != Status::Met)
            .filter_map(|finding| finding.fix.as_ref())
        {
            for parameter in &fix.add_parameters {
                if !add.contains(parameter) {
                    add.push(parameter.clone());
                }
            }
            for parameter in &fix.remove_parameters {
                if !remove.contains(parameter) {
                    remove.push(parameter.clone());
                }
            }
        }
        (add, remove)
    }
}

/// Policy for the game being checked.
#[derive(Clone, Copy, Debug, Default)]
pub struct Policy {
    /// The publisher forbids virtual machines (Fortnite does).
    pub forbid_virtual_machines: bool,
}

struct Builder {
    findings: Vec<Finding>,
}

impl Builder {
    fn push(
        &mut self,
        id: &'static str,
        title: &'static str,
        requirement: &'static str,
        status: Status,
        details: Vec<String>,
        fix: Option<Fix>,
    ) {
        self.findings.push(Finding {
            id,
            title,
            requirement,
            status,
            details,
            fix: if matches!(status, Status::Met | Status::NotApplicable) {
                None
            } else {
                fix
            },
        });
    }
}

fn fix(kind: FixKind, summary: &str, steps: &[&str]) -> Fix {
    Fix {
        kind,
        summary: summary.into(),
        steps: steps.iter().map(|step| step.to_string()).collect(),
        add_parameters: Vec::new(),
        remove_parameters: Vec::new(),
    }
}

fn parameters(summary: &str, add: &[String], remove: &[&str]) -> Fix {
    Fix {
        kind: FixKind::KernelParameters,
        summary: summary.into(),
        steps: vec![
            "Run `mica security boot-profile` to see a separate boot entry with these parameters, \
             then `sudo mica security boot-profile --apply` to create it."
                .into(),
        ],
        add_parameters: add.to_vec(),
        remove_parameters: remove.iter().map(|name| name.to_string()).collect(),
    }
}

/// The `lsm=` value that keeps every active LSM, in order, and adds
/// `lockdown` and `bpf` if missing. `capability` is always active and is
/// left out, as the kernel documentation does.
pub fn lsm_parameter(active: &[String]) -> String {
    let mut list: Vec<&str> = active
        .iter()
        .map(String::as_str)
        .filter(|lsm| *lsm != "capability")
        .collect();
    if !list.contains(&"lockdown") {
        list.insert(0, "lockdown");
    }
    if !list.contains(&"bpf") {
        list.push("bpf");
    }
    format!("lsm={}", list.join(","))
}

/// Evaluate EAC Linux v2's requirements against a probed host.
pub fn eac_linux_v2(profile: &HostProfile, policy: Policy) -> Readiness {
    let mut notes = vec![
        format!(
            "Requirements as Epic published them on 2026-10-14 ({SOURCE}); Epic expects to \
             iterate on them."
        ),
        "No game has shipped with EAC Linux v2 yet: Epic is piloting it in Fortnite, which is \
         not available on Linux as of 2026-10-25. A ready system can run nothing new until \
         Epic enables it."
            .into(),
        "Epic's anti-cheat service makes the final decision when a protected game starts. MICA \
         only reads your system's state; it does not attest, sign, or alter anything Epic \
         checks."
            .into(),
    ];
    if profile.os != Os::Linux {
        return Readiness {
            profile: "eac-linux-v2",
            verdict: Verdict::NotApplicable,
            findings: Vec::new(),
            notes: vec![format!(
                "EAC Linux v2 is for Linux. This is {}; anti-cheat on this system uses the \
                 publisher's native {} client.",
                profile.os, profile.os
            )],
            source: SOURCE,
            event_log: None,
        };
    }
    let security = &profile.security;
    let mut builder = Builder {
        findings: Vec::new(),
    };

    // The anti-cheat service.
    let systemd = profile.init == Init::Systemd;
    builder.push(
        "systemd",
        "systemd init",
        "Players are prompted to install an anti-cheat systemd service that runs only while a \
         protected game runs.",
        if systemd { Status::Met } else { Status::NotMet },
        vec![format!("init system: {:?}", profile.init)],
        Some(fix(
            FixKind::Software,
            "EAC Linux v2 ships as a systemd service; Epic has announced no alternative for other \
             init systems.",
            &["Use a systemd-based distribution, or wait for Epic to support your init system."],
        )),
    );

    // Hardware Epic has said lacks the required features.
    builder.push(
        "supported-device",
        "Supported device",
        "The Steam Deck does not support the required features yet.",
        if profile.device.steam_deck {
            Status::NotMet
        } else {
            Status::Met
        },
        profile
            .device
            .product
            .iter()
            .map(|product| format!("device: {product}"))
            .collect(),
        Some(fix(
            FixKind::NotFixable,
            "Epic is talking with Valve about Steam Deck support; nothing on the Deck can \
             change this today.",
            &[],
        )),
    );

    let vm = profile.virtualization.guest_of.clone();
    builder.push(
        "bare-metal",
        "Not a virtual machine",
        "Fortnite does not run in virtual machines (Epic Games support). A VM's TPM and Secure \
         Boot are emulated by the host.",
        match (&vm, policy.forbid_virtual_machines) {
            (None, _) => Status::Met,
            (Some(_), true) => Status::NotMet,
            (Some(_), false) => Status::Unknown,
        },
        vm.iter().map(|kind| format!("running inside: {kind}")).collect(),
        Some(fix(
            FixKind::NotFixable,
            "Install Linux directly on the computer. MICA never hides a virtual machine from \
             anti-cheat software.",
            &[],
        )),
    );

    // 3. Secure Boot (and UEFI, which it needs).
    let secure_boot = match (security.uefi, security.secure_boot, security.setup_mode) {
        (Some(false), _, _) => Status::NotMet,
        (_, Some(true), Some(true)) => Status::NotMet,
        (_, Some(true), _) => Status::Met,
        (_, Some(false), _) => Status::NotMet,
        _ => Status::Unknown,
    };
    builder.push(
        "secure-boot",
        "UEFI Secure Boot",
        "Secure Boot is enabled in the firmware and only well-known keys are used to boot.",
        secure_boot,
        vec![format!(
            "UEFI: {}, Secure Boot: {}, Setup Mode: {}",
            yes_no(security.uefi),
            yes_no(security.secure_boot),
            yes_no(security.setup_mode)
        )],
        Some(fix(
            FixKind::Firmware,
            "Enable Secure Boot in your UEFI firmware settings, with the factory (Microsoft) keys.",
            &[
                "Distribution kernels boot through shim, which Microsoft's UEFI CA signs, so the \
                 factory keys are what you want.",
                "If the computer boots in legacy BIOS/CSM mode, switch it to UEFI first. That \
                 usually requires a GPT disk; converting is disruptive, so back up first.",
                "Recovery: if the system no longer boots, disable Secure Boot in the same menu.",
            ],
        )),
    );

    // 2. TPM 2.0.
    let tpm = match (security.tpm_present, security.tpm_version) {
        (_, Some(2)) => Status::Met,
        (_, Some(_)) => Status::NotMet,
        (Some(false), _) => Status::NotMet,
        (Some(true), None) => Status::Unknown,
        (None, None) => Status::Unknown,
    };
    builder.push(
        "tpm2",
        "TPM 2.0",
        "A TPM 2.0 is present and enabled in the firmware.",
        tpm,
        vec![format!(
            "TPM: {}, version: {}",
            yes_no(security.tpm_present),
            security
                .tpm_version
                .map(|version| version.to_string())
                .unwrap_or_else(|| "unknown".into())
        )],
        Some(fix(
            FixKind::Firmware,
            "Enable the TPM in your firmware settings (often called fTPM, PTT, or Security \
             Device Support).",
            &["A TPM 1.2 cannot be upgraded by software; it does not qualify."],
        )),
    );

    // 4. Measured boot, and which keys verified this boot.
    let log = security.event_log.as_deref().and_then(tcg::parse);
    let summary = log.as_ref().map(tcg::EventLog::summary);
    let measured = match (&summary, security.event_log_present) {
        (Some(summary), _) if summary.boot_applications > 0 => Status::Met,
        (Some(_), _) => Status::NotMet,
        (None, true) => Status::Unknown,
        (None, false) => Status::NotMet,
    };
    let mut measured_details = Vec::new();
    match &summary {
        Some(summary) => measured_details.push(format!(
            "{} events; {} EFI applications measured into PCR 4; PCRs with events: {:?}",
            summary.events,
            summary.boot_applications,
            summary.events_per_pcr.keys().collect::<Vec<_>>()
        )),
        None if security.event_log_present => measured_details.push(
            "the event log exists but only root can read it: run `sudo mica security check`".into(),
        ),
        None => measured_details.push("no TPM event log is exposed by this kernel".into()),
    }
    builder.push(
        "measured-boot",
        "Measured boot",
        "Boot measurements are present in the TPM (systemd-boot and GRUB record them by \
         default).",
        measured,
        measured_details,
        Some(fix(
            FixKind::Software,
            "Use a boot loader that measures what it loads (systemd-boot, or GRUB with its TPM \
             module) on a system with an enabled TPM 2.0.",
            &[],
        )),
    );

    let authorities: Vec<(String, String, KeyMatch)> = summary
        .iter()
        .flat_map(|summary| summary.authorities.iter())
        .map(|authority| match &authority.certificate {
            Some(certificate) => (
                authority.source.clone(),
                certificate.subject.display(),
                wellknown::classify(certificate),
            ),
            None => (
                authority.source.clone(),
                "(unreadable certificate)".into(),
                KeyMatch::NotListed,
            ),
        })
        .collect();
    let boot_keys = if summary.is_none() {
        Status::Unknown
    } else if authorities.is_empty() {
        if secure_boot == Status::Met {
            Status::Unknown
        } else {
            Status::NotMet
        }
    } else if authorities.iter().all(|(_, _, key)| key.is_well_known()) {
        Status::Met
    } else {
        Status::NotMet
    };
    builder.push(
        "boot-keys",
        "Boot chain signed with well-known keys",
        "The kernel used for protected play is signed with a well-known key, and only \
         well-known keys verified the boot chain.",
        boot_keys,
        authorities
            .iter()
            .map(|(source, name, key)| {
                format!(
                    "{source}: {name} — {}",
                    match key {
                        KeyMatch::Listed { key } => format!("well-known ({})", key.name),
                        KeyMatch::IssuedByListed { key } => format!("issued by well-known {}", key.name),
                        KeyMatch::NotListed => "NOT on Epic's well-known list".into(),
                    }
                )
            })
            .chain(summary.is_none().then(|| {
                "the authorities are in the TPM event log, which only root can read".to_string()
            }))
            .collect(),
        Some(fix(
            FixKind::Keys,
            "Boot your distribution's own signed kernel through shim, with factory Secure Boot \
             keys. A self-built or self-signed kernel does not qualify, even with Secure Boot on.",
            &[
                "Keep custom kernels on a separate boot entry and choose the distribution kernel \
                 when you play (Epic's FAQ allows this).",
                "Epic matches the actual keys; MICA matches their names, so treat a pass here \
                 as a strong hint.",
            ],
        )),
    );

    // Machine-owner keys: Epic requires every enrolled MOK to be well-known.
    let moks = security.mok.as_deref().map(efi::certificates);
    let unknown_moks: Vec<String> = moks
        .iter()
        .flatten()
        .filter(|certificate| !wellknown::classify(certificate).is_well_known())
        .map(|certificate| certificate.subject.display())
        .collect();
    let mok_status = match (&moks, security.uefi) {
        (_, Some(false)) => Status::NotApplicable,
        (None, _) => Status::Unknown,
        (Some(_), _) if unknown_moks.is_empty() => Status::Met,
        (Some(_), _) => Status::NotMet,
    };
    builder.push(
        "machine-owner-keys",
        "Machine Owner Keys (MOKs)",
        "All enrolled MOKs are well-known, because the kernel trusts every MOK for module \
         signatures. Put personal keys in the firmware db instead.",
        mok_status,
        match &moks {
            Some(list) if list.is_empty() => vec!["no MOKs enrolled".into()],
            Some(list) => list
                .iter()
                .map(|certificate| {
                    format!(
                        "MOK: {} — {}",
                        certificate.subject.display(),
                        if wellknown::classify(certificate).is_well_known() {
                            "well-known"
                        } else {
                            "NOT well-known"
                        }
                    )
                })
                .collect(),
            None => vec!["MokListRT could not be read (it may need root)".into()],
        },
        Some(Fix {
            kind: FixKind::Keys,
            summary: "Remove personal MOKs, typically created to sign DKMS or akmods modules \
                      such as NVIDIA's driver."
                .into(),
            steps: vec![
                "Before removing a MOK, switch to driver packages your distribution signs (for \
                 example Ubuntu's signed linux-modules-nvidia packages), or modules that are \
                 signed with it will stop loading."
                    .into(),
                "List the keys with `mokutil --list-enrolled`, export the one to remove, and run \
                 `sudo mokutil --delete key.der`; confirm in MokManager on the next boot."
                    .into(),
                "Recovery: `sudo mokutil --import key.der` re-enrolls it; confirm on the next boot."
                    .into(),
            ],
            add_parameters: Vec::new(),
            remove_parameters: Vec::new(),
        }),
    );

    // Kernel modules.
    const TAINT_UNSIGNED_MODULE: u64 = 1 << 13;
    let unsigned: Vec<String> = security
        .tainted_modules
        .iter()
        .filter(|module| module.taint.contains('E'))
        .map(|module| module.name.clone())
        .collect();
    let enforced = security.module_sig_enforce == Some(true)
        || matches!(security.lockdown.as_deref(), Some("integrity" | "confidentiality"));
    let unsigned_taint = security
        .tainted
        .is_some_and(|tainted| tainted & TAINT_UNSIGNED_MODULE != 0);
    let modules = if !unsigned.is_empty() || unsigned_taint {
        Status::NotMet
    } else if enforced {
        Status::Met
    } else if security.tainted.is_none() {
        Status::Unknown
    } else {
        Status::NotMet
    };
    let mut module_details: Vec<String> = security
        .tainted_modules
        .iter()
        .map(|module| {
            format!(
                "{} (taint {}{})",
                module.name,
                module.taint,
                module
                    .signer
                    .as_ref()
                    .map(|signer| format!(", signed by {signer}"))
                    .unwrap_or_default()
            )
        })
        .collect();
    module_details.push(format!(
        "signature enforcement: {}",
        if enforced { "on" } else { "off" }
    ));
    builder.push(
        "signed-modules",
        "Signed kernel modules",
        "Every loaded kernel module is built and signed by an organization with a well-known \
         key (out-of-tree modules such as NVIDIA's included).",
        modules,
        module_details,
        Some(fix(
            FixKind::Software,
            "Load only modules your distribution builds and signs, and turn on signature \
             enforcement (kernel lockdown does this).",
            &[
                "For NVIDIA, prefer your distribution's prebuilt, signed driver packages over \
                 locally built DKMS/akmods modules signed with a personal MOK.",
            ],
        )),
    );

    // 5. IOMMU.
    let passthrough_flags: Vec<&str> = ["iommu=pt", "iommu.passthrough=1", "amd_iommu=off", "intel_iommu=off"]
        .into_iter()
        .filter(|flag| security.cmdline_has(flag))
        .collect();
    let identity_groups = security.iommu_group_types.get("identity").copied().unwrap_or(0);
    let translated_groups: usize = security
        .iommu_group_types
        .iter()
        .filter(|(kind, _)| kind.starts_with("DMA"))
        .map(|(_, count)| count)
        .sum();
    let intel = profile
        .cpu_model
        .as_deref()
        .is_some_and(|model| model.contains("Intel"));
    let iommu = if profile.device.handheld {
        Status::NotApplicable
    } else if security.iommu_units.is_empty() {
        Status::NotMet
    } else if !passthrough_flags.is_empty() || (identity_groups > 0 && translated_groups == 0) {
        Status::NotMet
    } else {
        Status::Met
    };
    let mut iommu_add = Vec::new();
    if security.iommu_units.is_empty() && intel {
        iommu_add.push("intel_iommu=on".to_string());
    }
    if !passthrough_flags.is_empty() || identity_groups > 0 {
        iommu_add.push("iommu.passthrough=0".to_string());
    }
    let mut iommu_fix = parameters(
        "Enable the IOMMU with DMA translation (not passthrough).",
        &iommu_add,
        &["iommu", "amd_iommu", "intel_iommu", "iommu.passthrough"],
    );
    iommu_fix.steps.insert(
        0,
        "Enable the IOMMU in the firmware (AMD-Vi / AMD IOMMU, or Intel VT-d) if it is off.".into(),
    );
    iommu_fix.remove_parameters.retain(|name| {
        passthrough_flags
            .iter()
            .any(|flag| flag.split('=').next() == Some(name.as_str()))
    });
    builder.push(
        "iommu",
        "IOMMU enabled, passthrough off",
        "For non-handheld devices, the IOMMU is enabled in the firmware and the kernel, with \
         passthrough disabled.",
        iommu,
        vec![
            format!(
                "IOMMU units: {}",
                if security.iommu_units.is_empty() {
                    "none".to_string()
                } else {
                    security.iommu_units.join(", ")
                }
            ),
            format!(
                "{} IOMMU groups; domain types: {:?}",
                security.iommu_groups, security.iommu_group_types
            ),
            format!("passthrough parameters on the command line: {passthrough_flags:?}"),
        ],
        Some(iommu_fix),
    );

    // 6. Lockdown integrity mode, explicitly at boot.
    let explicit_lockdown = security.cmdline_has("lockdown=integrity");
    let lockdown = match security.lockdown.as_deref() {
        Some("integrity") if explicit_lockdown => Status::Met,
        Some("integrity") => Status::NotMet,
        Some("confidentiality") => Status::Unknown,
        _ => Status::NotMet,
    };
    let lsm_value = security.lsms.as_deref().map(lsm_parameter);
    let mut lockdown_add = vec!["lockdown=integrity".to_string()];
    if let Some(lsm) = &lsm_value
        && !security
            .lsms
            .as_ref()
            .is_some_and(|lsms| lsms.iter().any(|lsm| lsm == "lockdown"))
    {
        lockdown_add.push(lsm.clone());
    }
    builder.push(
        "lockdown",
        "Kernel lockdown: integrity, set at boot",
        "Lockdown integrity mode is explicitly enabled at boot time.",
        lockdown,
        vec![
            format!(
                "active mode: {}",
                security.lockdown.as_deref().unwrap_or("lockdown LSM not available")
            ),
            format!("lockdown=integrity on the command line: {}", explicit_lockdown),
            match security.lockdown.as_deref() {
                Some("integrity") if !explicit_lockdown => "integrity mode is active (probably \
                    enabled automatically by Secure Boot), but Epic asks for it to be set \
                    explicitly at boot; adding lockdown=integrity changes nothing else"
                    .into(),
                Some("confidentiality") => "confidentiality is stricter than integrity and also \
                    restricts what eBPF programs may read; Epic names integrity, so whether \
                    confidentiality is accepted is unknown"
                    .into(),
                _ => String::new(),
            },
        ]
        .into_iter()
        .filter(|line| !line.is_empty())
        .collect(),
        Some(parameters(
            "Boot with lockdown=integrity.",
            &lockdown_add,
            &[],
        )),
    );

    // 7. kexec disabled, explicitly at boot.
    let explicit_kexec = security.cmdline_has("sysctl.kernel.kexec_load_disabled=1");
    let kexec = match (security.kexec_built_in, security.kexec_load_disabled) {
        (Some(false), _) => Status::Met,
        (_, Some(true)) if explicit_kexec => Status::Met,
        (_, Some(true)) => Status::NotMet,
        (_, Some(false)) => Status::NotMet,
        _ => Status::Unknown,
    };
    builder.push(
        "kexec",
        "kexec disabled at boot",
        "kexec is explicitly disabled at boot time, so a different kernel cannot be loaded \
         without a measured reboot.",
        kexec,
        vec![
            format!(
                "kernel.kexec_load_disabled = {}",
                match security.kexec_load_disabled {
                    Some(true) => "1",
                    Some(false) => "0",
                    None => "unknown",
                }
            ),
            format!("set on the command line: {explicit_kexec}"),
        ],
        Some(parameters(
            "Boot with sysctl.kernel.kexec_load_disabled=1 (Linux 5.8 and later read sysctl.* \
             parameters from the command line).",
            &["sysctl.kernel.kexec_load_disabled=1".to_string()],
            &[],
        )),
    );

    // 8. BPF LSM.
    let bpf = match &security.lsms {
        Some(lsms) if lsms.iter().any(|lsm| lsm == "bpf") => Status::Met,
        Some(_) => Status::NotMet,
        None => Status::Unknown,
    };
    builder.push(
        "bpf-lsm",
        "BPF LSM active",
        "The BPF Linux Security Module is loaded and active.",
        bpf,
        vec![format!(
            "active LSMs: {}",
            security
                .lsms
                .as_ref()
                .map(|lsms| lsms.join(","))
                .unwrap_or_else(|| "unknown (is securityfs mounted?)".into())
        )],
        Some(match &lsm_value {
            Some(lsm) => parameters(
                "Add bpf to the active LSMs, keeping every LSM already active in the same order.",
                std::slice::from_ref(lsm),
                &["lsm"],
            ),
            None => fix(
                FixKind::KernelParameters,
                "Add bpf to the kernel's lsm= list (the kernel must be built with \
                 CONFIG_BPF_LSM).",
                &[],
            ),
        }),
    );

    let findings = builder.findings;
    let verdict = if findings.iter().any(|finding| finding.status == Status::NotMet) {
        Verdict::NotReady
    } else if findings.iter().any(|finding| finding.status == Status::Unknown) {
        Verdict::Incomplete
    } else {
        Verdict::Ready
    };
    if profile.immutable.is_some() {
        notes.push(format!(
            "This is an image-based system ({}). Kernel parameters are changed through its own \
             tools (rpm-ostree kargs, NixOS configuration), which MICA prints rather than runs.",
            profile.immutable.as_deref().unwrap_or_default()
        ));
    }
    if profile
        .distro
        .as_ref()
        .is_some_and(|distro| distro.id == "nixos")
    {
        notes.push(
            "NixOS kernels are not signed by any key on Epic's well-known list as of 2026-10-14, \
             so NixOS cannot meet the signed-kernel requirement today, whatever its \
             configuration."
                .into(),
        );
    }
    Readiness {
        profile: "eac-linux-v2",
        verdict,
        findings,
        notes,
        source: SOURCE,
        event_log: summary,
    }
}

fn yes_no(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    }
}
