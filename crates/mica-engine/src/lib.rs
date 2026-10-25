//! MICA's decision engine.
//!
//! [`decide::analyze`] identifies a program and the anti-cheat beside it;
//! [`decide::decide`] lists the backends that could run it on this host,
//! applies what publishers permit ([`policy`]), and picks the least
//! translation that works; [`prepare::prepare`] turns the choice into a
//! command. [`compatibility`] answers "can I play this game here?".

pub mod compatibility;
pub mod decide;
pub mod policy;
pub mod prepare;

pub use decide::{Backend, Blocker, BlockerKind, Candidate, Decision, Inputs, Runner, Target, analyze, decide};
pub use prepare::prepare;

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use mica_compat::{Database, Route};
    use mica_inspect::{AntiCheat, AntiCheatScan, AntiCheatSignal, Arch, Confidence, inspect_bytes};
    use mica_probe::{HostProfile, Os};
    use mica_runtime::{Config, Runtime, RuntimeKind, Runtimes};

    use super::*;

    fn runtimes(kinds: &[RuntimeKind]) -> Runtimes {
        Runtimes {
            found: kinds
                .iter()
                .map(|kind| Runtime {
                    kind: *kind,
                    name: kind.to_string(),
                    path: PathBuf::from(format!("/usr/bin/{kind:?}").to_lowercase()),
                    version: None,
                })
                .collect(),
            ..Runtimes::default()
        }
    }

    fn host(os: Os, arch: Arch) -> HostProfile {
        let mut host = HostProfile::empty(os);
        host.arch = arch;
        host.vulkan.loader = true;
        host
    }

    /// A minimal PE image: MZ, PE header, a 64-bit optional header, no
    /// sections, so imports are empty but the format is unambiguous.
    fn pe(machine: u16) -> Vec<u8> {
        let mut data = vec![0u8; 0x200];
        data[0..2].copy_from_slice(b"MZ");
        data[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        data[0x80..0x84].copy_from_slice(b"PE\0\0");
        data[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        data[0x94..0x96].copy_from_slice(&240u16.to_le_bytes());
        data[0x98..0x9A].copy_from_slice(&0x20Bu16.to_le_bytes());
        data
    }

    fn target(name: &str, data: &[u8], game: Option<&str>, scan: Option<AntiCheatScan>) -> Target {
        let database = Database::builtin();
        Target {
            path: PathBuf::from(format!("/games/{name}")),
            report: inspect_bytes(name, data),
            scan,
            game: game.and_then(|id| database.find(id)).cloned(),
            steam_app_id: None,
        }
    }

    fn decide_on(host: &HostProfile, runtimes: &Runtimes, config: &Config, target: &Target, forced: Option<Backend>) -> Decision {
        let database = Database::builtin();
        decide(
            &Inputs {
                host,
                runtimes,
                database: &database,
                config,
            },
            target,
            forced,
        )
    }

    #[test]
    fn windows_programs_on_linux_prefer_umu_then_wine() {
        let linux = host(Os::Linux, Arch::X86_64);
        let config = Config::default();
        let app = target("tool.exe", &pe(0x8664), None, Some(AntiCheatScan::default()));

        let both = runtimes(&[RuntimeKind::Umu, RuntimeKind::Wine]);
        let decision = decide_on(&linux, &both, &config, &app, None);
        let chosen = decision.chosen().unwrap();
        assert_eq!(chosen.backend, Backend::WindowsCompat);
        assert!(matches!(chosen.runner, Some(Runner::Umu { .. })));

        let wine_only = runtimes(&[RuntimeKind::Wine]);
        let decision = decide_on(&linux, &wine_only, &config, &app, None);
        assert!(matches!(decision.chosen().unwrap().runner, Some(Runner::Wine { .. })));

        let none = runtimes(&[]);
        let decision = decide_on(&linux, &none, &config, &app, None);
        assert!(decision.chosen().is_none());
        let windows = decision.candidates.iter().find(|c| c.backend == Backend::WindowsCompat).unwrap();
        assert_eq!(windows.blockers[0].kind, BlockerKind::MissingRuntime);
        assert!(!windows.setup.is_empty());

        // The VM is offered but not configured.
        let vm = decision.candidates.iter().find(|c| c.backend == Backend::VirtualMachine).unwrap();
        assert_eq!(vm.blockers[0].kind, BlockerKind::NotConfigured);
    }

    #[test]
    fn apex_is_blocked_on_linux_and_the_block_cannot_be_overridden() {
        let linux = host(Os::Linux, Arch::X86_64);
        let mut config = Config::default();
        config.vm.libvirt_domain = Some("win11".into());
        let all = runtimes(&[RuntimeKind::Umu, RuntimeKind::Wine, RuntimeKind::Libvirt]);
        let apex = target("r5apex.exe", &pe(0x8664), Some("apex-legends"), None);
        let decision = decide_on(&linux, &all, &config, &apex, None);
        assert!(decision.chosen().is_none(), "{:#?}", decision.candidates);
        assert_eq!(decision.anticheats, vec![AntiCheat::EaJavelin]);
        let proton = decision.candidates.iter().find(|c| c.backend == Backend::WindowsCompat).unwrap();
        assert!(proton.blockers.iter().any(|b| b.kind == BlockerKind::Policy && !b.overridable));
        let vm = decision.candidates.iter().find(|c| c.backend == Backend::VirtualMachine).unwrap();
        assert!(vm.blockers.iter().any(|b| b.kind == BlockerKind::Policy));

        let forced = decide_on(&linux, &all, &config, &apex, Some(Backend::WindowsCompat));
        assert!(forced.chosen().is_none(), "official prohibitions are not overridable");
    }

    #[test]
    fn fortnite_offers_cloud_and_points_at_readiness() {
        let linux = host(Os::Linux, Arch::X86_64);
        let config = Config::default();
        let fortnite = target(
            "FortniteLauncher.exe",
            &pe(0x8664),
            Some("fortnite"),
            None,
        );
        let decision = decide_on(&linux, &runtimes(&[RuntimeKind::Umu]), &config, &fortnite, None);
        assert!(decision.chosen().is_none());
        let remote = decision.candidates.iter().find(|c| c.backend == Backend::Remote).unwrap();
        assert!(remote.eligible());
        assert!(decision.notes.iter().any(|note| note.contains("mica compatibility fortnite")));
    }

    #[test]
    fn eac_games_are_judged_by_the_proton_library() {
        let linux = host(Os::Linux, Arch::X86_64);
        let config = Config::default();
        let scan = |library| AntiCheatScan {
            signals: vec![AntiCheatSignal {
                anticheat: AntiCheat::EasyAntiCheat,
                marker: "EasyAntiCheat".into(),
                confidence: Confidence::High,
            }],
            eac_proton_library: library,
            entries_scanned: 5,
            truncated: false,
        };
        let opted_in = target("Game.exe", &pe(0x8664), None, Some(scan(true)));
        let decision = decide_on(&linux, &runtimes(&[RuntimeKind::Umu]), &config, &opted_in, None);
        let chosen = decision.chosen().unwrap();
        assert_eq!(chosen.backend, Backend::WindowsCompat);
        assert!(chosen.warnings.iter().all(|warning| !warning.contains("probably not enabled")));
        // A VM is never chosen for a protected game without permission.
        let vm = decision.candidates.iter().find(|c| c.backend == Backend::VirtualMachine).unwrap();
        assert!(vm.blockers.iter().any(|b| b.kind == BlockerKind::Policy));

        let not = target("Game.exe", &pe(0x8664), None, Some(scan(false)));
        let decision = decide_on(&linux, &runtimes(&[RuntimeKind::Umu]), &config, &not, None);
        assert!(decision.chosen().unwrap().warnings.iter().any(|warning| warning.contains("probably not enabled")));
    }

    #[test]
    fn hosts_and_formats() {
        let config = Config::default();
        let windows = host(Os::Windows, Arch::X86_64);
        let exe = target("app.exe", &pe(0x8664), None, None);
        assert_eq!(decide_on(&windows, &runtimes(&[]), &config, &exe, None).chosen().unwrap().backend, Backend::Native);

        let elf = crate::tests::elf();
        let linux_program = target("tool", &elf, None, None);
        let decision = decide_on(&windows, &runtimes(&[RuntimeKind::Wsl]), &config, &linux_program, None);
        assert_eq!(decision.chosen().unwrap().backend, Backend::Wsl);
        let decision = decide_on(&windows, &runtimes(&[]), &config, &linux_program, None);
        assert!(decision.chosen().is_none());

        let linux = host(Os::Linux, Arch::X86_64);
        assert_eq!(decide_on(&linux, &runtimes(&[]), &config, &linux_program, None).chosen().unwrap().backend, Backend::Native);

        let arm_linux = host(Os::Linux, Arch::Aarch64);
        let decision = decide_on(&arm_linux, &runtimes(&[RuntimeKind::Fex]), &config, &linux_program, None);
        assert_eq!(decision.chosen().unwrap().backend, Backend::ArchTranslation);

        let component = target("game.wasm", b"\0asm\x0d\0\x01\0", None, None);
        let decision = decide_on(&linux, &runtimes(&[RuntimeKind::PortableHost]), &config, &component, None);
        assert_eq!(decision.chosen().unwrap().backend, Backend::PortableHost);

        let mac = host(Os::MacOs, Arch::Aarch64);
        let decision = decide_on(&mac, &runtimes(&[RuntimeKind::Crossover, RuntimeKind::Rosetta]), &config, &exe, None);
        assert!(matches!(decision.chosen().unwrap().runner, Some(Runner::Crossover { .. })));
        let decision = decide_on(&linux, &runtimes(&[]), &config, &target("App", &mac_binary(), None, None), None);
        assert!(decision.chosen().is_none());
        assert!(decision.candidates[0].blockers[0].message.contains("macOS programs need macOS"));
    }

    #[test]
    fn compatibility_reports() {
        let database = Database::builtin();
        let mut linux = host(Os::Linux, Arch::X86_64);
        linux.init = mica_probe::Init::Systemd;
        let fortnite = database.find("fortnite").unwrap();
        let report = compatibility::report(&database, fortnite, &linux);
        assert!(report.readiness.is_some());
        assert_eq!(report.routes[0].0, Route::LinuxNative);
        assert!(report.recommendation[0].starts_with("Not playable on Linux today"));
        assert!(report.recommendation.iter().any(|line| line.contains("GeForce NOW")));

        let apex = database.find("apex").unwrap();
        let report = compatibility::report(&database, apex, &linux);
        assert!(report.readiness.is_none(), "Javelin is not EAC Linux v2");
        assert!(report.recommendation[0].contains("does not permit"));

        let windows = host(Os::Windows, Arch::X86_64);
        let report = compatibility::report(&database, apex, &windows);
        assert!(report.recommendation[0].contains("play natively"));
    }

    pub(crate) fn elf() -> Vec<u8> {
        let mut data = vec![0u8; 64];
        data[0..4].copy_from_slice(b"\x7fELF");
        data[4] = 2;
        data[5] = 1;
        data[16..18].copy_from_slice(&2u16.to_le_bytes());
        data[18..20].copy_from_slice(&62u16.to_le_bytes());
        data
    }

    fn mac_binary() -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&0xFEED_FACFu32.to_le_bytes());
        data.extend_from_slice(&0x0100_000Cu32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&2u32.to_le_bytes());
        data.extend_from_slice(&[0u8; 16]);
        data
    }
}
