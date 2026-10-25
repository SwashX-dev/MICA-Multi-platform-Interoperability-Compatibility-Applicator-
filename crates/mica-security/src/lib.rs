//! Platform-security readiness for publisher-authorized anti-cheat.
//!
//! MICA's approach to anti-cheat is to help a system *genuinely* meet what a
//! publisher requires, and to say precisely what is missing when it cannot.
//! This crate:
//!
//! * evaluates the requirements Epic published for Easy Anti-Cheat Linux v2
//!   ([`readiness`]), reading UEFI signature databases ([`efi`]), X.509 names
//!   ([`x509`]), and the TPM measured-boot log ([`tcg`]) to tell which keys
//!   verified the current boot and whether they are on Epic's well-known list
//!   ([`wellknown`]);
//! * plans, and with consent creates, a separate boot entry with the
//!   required kernel parameters ([`boot_profile`]), which removing one file
//!   undoes.
//!
//! It never disables, patches, or imitates anti-cheat software, never forges
//! or replays measurements or attestations, and never hides a virtual
//! machine. Firmware settings and Secure Boot keys are only ever described,
//! with recovery steps, for the user to change themselves.

pub mod boot_profile;
pub mod efi;
pub mod readiness;
pub mod tcg;
pub mod wellknown;
pub mod x509;

pub use readiness::{Finding, Fix, FixKind, Policy, Readiness, Status, Verdict, eac_linux_v2};

#[cfg(test)]
mod tests {
    use mica_probe::{Distro, Family, HostProfile, Init, Os, SecurityFacts};

    use crate::efi::tests::x509_list;
    use crate::readiness::{Policy, Status, Verdict, eac_linux_v2, lsm_parameter};
    use crate::tcg::tests::fedora_boot;
    use crate::x509::tests::certificate;

    /// A Fedora desktop that meets every requirement, as root sees it.
    fn ready_fedora() -> HostProfile {
        let mut profile = HostProfile::empty(Os::Linux);
        profile.distro = Some(Distro {
            id: "fedora".into(),
            id_like: Vec::new(),
            name: "Fedora Linux".into(),
            pretty_name: "Fedora Linux 43".into(),
            version_id: Some("43".into()),
            variant_id: None,
            family: Family::Fedora,
        });
        profile.init = Init::Systemd;
        profile.cpu_model = Some("AMD Ryzen 7 7800X3D".into());
        profile.kernel = Some("6.17.4-300.fc43.x86_64".into());
        profile.security = SecurityFacts {
            uefi: Some(true),
            secure_boot: Some(true),
            setup_mode: Some(false),
            tpm_present: Some(true),
            tpm_version: Some(2),
            event_log_present: true,
            event_log_readable: true,
            event_log: Some(fedora_boot()),
            lockdown: Some("integrity".into()),
            lsms: Some(
                ["capability", "lockdown", "yama", "integrity", "selinux", "bpf"]
                    .map(String::from)
                    .to_vec(),
            ),
            kexec_load_disabled: Some(true),
            kexec_built_in: Some(true),
            iommu_units: vec!["ivhd0".into()],
            iommu_groups: 20,
            iommu_group_types: [("DMA-FQ".to_string(), 20)].into_iter().collect(),
            cmdline: Some(
                "BOOT_IMAGE=/vmlinuz root=UUID=1 ro lockdown=integrity \
                 sysctl.kernel.kexec_load_disabled=1 lsm=lockdown,yama,integrity,selinux,bpf"
                    .into(),
            ),
            module_sig_enforce: Some(true),
            tainted: Some(0),
            mok: Some(Vec::new()),
            ..SecurityFacts::default()
        };
        profile
    }

    #[test]
    fn a_fully_configured_system_is_ready() {
        let readiness = eac_linux_v2(&ready_fedora(), Policy::default());
        for finding in &readiness.findings {
            assert_eq!(finding.status, Status::Met, "{finding:#?}");
            assert!(finding.fix.is_none());
        }
        assert_eq!(readiness.verdict, Verdict::Ready);
        assert_eq!(readiness.kernel_parameters(), (Vec::new(), Vec::new()));
        assert!(readiness.notes.iter().any(|note| note.contains("Fortnite")));
    }

    #[test]
    fn a_typical_ubuntu_desktop_needs_parameters_and_its_dkms_key_removed() {
        let mut profile = ready_fedora();
        let security = &mut profile.security;
        security.lockdown = Some("none".into());
        security.lsms = Some(
            ["lockdown", "capability", "landlock", "yama", "apparmor", "ima", "evm"]
                .map(String::from)
                .to_vec(),
        );
        security.kexec_load_disabled = Some(false);
        security.module_sig_enforce = Some(false);
        security.iommu_group_types = [("identity".to_string(), 20)].into_iter().collect();
        security.cmdline = Some("BOOT_IMAGE=/vmlinuz root=UUID=1 ro quiet splash iommu=pt".into());
        security.mok = Some(x509_list(&certificate(
            ("ubuntu Secure Boot Module Signature key", "ubuntu"),
            ("ubuntu Secure Boot Module Signature key", "ubuntu"),
        )));
        security.tainted = Some(4096 | 1);
        security.tainted_modules = vec![mica_probe::security::TaintedModule {
            name: "nvidia".into(),
            taint: "POE".into(),
            signer: None,
        }];
        let readiness = eac_linux_v2(&profile, Policy::default());
        assert_eq!(readiness.verdict, Verdict::NotReady);
        let status = |id: &str| readiness.finding(id).unwrap().status;
        assert_eq!(status("lockdown"), Status::NotMet);
        assert_eq!(status("kexec"), Status::NotMet);
        assert_eq!(status("bpf-lsm"), Status::NotMet);
        assert_eq!(status("iommu"), Status::NotMet);
        assert_eq!(status("machine-owner-keys"), Status::NotMet);
        assert_eq!(status("signed-modules"), Status::NotMet);
        assert_eq!(status("secure-boot"), Status::Met);

        let (add, remove) = readiness.kernel_parameters();
        assert_eq!(
            add,
            vec![
                "iommu.passthrough=0",
                "lockdown=integrity",
                "sysctl.kernel.kexec_load_disabled=1",
                "lsm=lockdown,landlock,yama,apparmor,ima,evm,bpf",
            ]
        );
        assert_eq!(remove, vec!["iommu", "lsm"]);
    }

    #[test]
    fn unreadable_facts_are_unknown_not_failures() {
        let mut profile = ready_fedora();
        profile.security.event_log = None;
        profile.security.event_log_readable = false;
        profile.security.mok = None;
        let readiness = eac_linux_v2(&profile, Policy::default());
        assert_eq!(readiness.finding("measured-boot").unwrap().status, Status::Unknown);
        assert_eq!(readiness.finding("boot-keys").unwrap().status, Status::Unknown);
        assert_eq!(
            readiness.finding("machine-owner-keys").unwrap().status,
            Status::Unknown
        );
        assert_eq!(readiness.verdict, Verdict::Incomplete);
    }

    #[test]
    fn steam_deck_vms_non_systemd_and_other_oses() {
        let mut deck = ready_fedora();
        deck.device.steam_deck = true;
        deck.device.handheld = true;
        let readiness = eac_linux_v2(&deck, Policy::default());
        assert_eq!(readiness.finding("supported-device").unwrap().status, Status::NotMet);
        assert_eq!(readiness.finding("iommu").unwrap().status, Status::NotApplicable);

        let mut vm = ready_fedora();
        vm.virtualization.guest_of = Some("kvm".into());
        let generic = eac_linux_v2(&vm, Policy::default());
        assert_eq!(generic.finding("bare-metal").unwrap().status, Status::Unknown);
        let fortnite = eac_linux_v2(
            &vm,
            Policy {
                forbid_virtual_machines: true,
            },
        );
        assert_eq!(fortnite.finding("bare-metal").unwrap().status, Status::NotMet);

        let mut openrc = ready_fedora();
        openrc.init = Init::OpenRc;
        assert_eq!(
            eac_linux_v2(&openrc, Policy::default())
                .finding("systemd")
                .unwrap()
                .status,
            Status::NotMet
        );

        let windows = HostProfile::empty(Os::Windows);
        assert_eq!(
            eac_linux_v2(&windows, Policy::default()).verdict,
            Verdict::NotApplicable
        );
    }

    #[test]
    fn a_self_signed_boot_chain_fails_the_key_check() {
        let mut profile = ready_fedora();
        let custom = certificate(("My Platform Key", "Me"), ("My Platform Key", "Me"));
        profile.security.event_log = Some(
            crate::tcg::tests::LogBuilder::new()
                .variable(7, crate::tcg::EV_EFI_VARIABLE_DRIVER_CONFIG, "SecureBoot", &[1])
                .variable(7, crate::tcg::EV_EFI_VARIABLE_AUTHORITY, "db", &[vec![0; 16], custom].concat())
                .event(4, crate::tcg::EV_EFI_BOOT_SERVICES_APPLICATION, &[0; 8])
                .build(),
        );
        let readiness = eac_linux_v2(&profile, Policy::default());
        let keys = readiness.finding("boot-keys").unwrap();
        assert_eq!(keys.status, Status::NotMet);
        assert!(keys.details[0].contains("My Platform Key"));
        assert!(keys.details[0].contains("NOT on Epic's well-known list"));
    }

    #[test]
    fn lsm_lists_keep_order_and_exclusive_modules() {
        let active = ["capability", "landlock", "yama", "apparmor"].map(String::from);
        assert_eq!(lsm_parameter(&active), "lsm=lockdown,landlock,yama,apparmor,bpf");
        let ready = ["lockdown", "selinux", "bpf"].map(String::from);
        assert_eq!(lsm_parameter(&ready), "lsm=lockdown,selinux,bpf");
    }
}
