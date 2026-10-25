//! The Anti-Cheat Gaming Boot Profile: a *separate* boot entry with the
//! kernel parameters EAC Linux v2 asks for.
//!
//! Epic's FAQ allows exactly this: "You can maintain a separate boot entry
//! with the required kernel and command line, and select that entry when you
//! want to play a protected game." The profile boots the same,
//! distribution-signed kernel and initramfs as the current entry; only the
//! command line differs. MICA never edits the existing entry, the default
//! entry, the boot loader, firmware variables, or keys.
//!
//! Automation covers Boot Loader Specification entries (systemd-boot, and
//! GRUB with `blscfg` as on Fedora, RHEL, and their derivatives): MICA writes
//! one new file, marked as its own, and removing that file undoes
//! everything. Everywhere else MICA prints exact manual steps.

use std::path::{Path, PathBuf};

use serde::Serialize;

use mica_probe::{Context, HostProfile};

/// First line of every entry MICA writes; rollback deletes only such files.
pub const MARKER: &str = "# Created by MICA (mica security boot-profile). Delete this file to remove the entry.";
pub const ENTRY_PREFIX: &str = "mica-anticheat-";

const ENTRY_DIRS: &[&str] = &["/boot/loader/entries", "/efi/loader/entries", "/boot/efi/loader/entries"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LoaderKind {
    /// Boot Loader Specification entries, read by systemd-boot or GRUB's
    /// blscfg module.
    BootLoaderSpec,
    /// GRUB with a generated grub.cfg (Debian, Ubuntu, Arch, openSUSE).
    GrubGenerated,
    /// An image-based system that manages kernel arguments itself.
    RpmOstree,
    NixOs,
    Unknown,
}

/// A Boot Loader Specification Type #1 entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlsEntry {
    pub file: String,
    pub lines: Vec<(String, String)>,
}

impl BlsEntry {
    pub fn parse(file: &str, text: &str) -> BlsEntry {
        let lines = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| match line.split_once(char::is_whitespace) {
                Some((key, value)) => (key.to_string(), value.trim().to_string()),
                None => (line.to_string(), String::new()),
            })
            .collect();
        BlsEntry {
            file: file.to_string(),
            lines,
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.lines
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    /// All `options` lines, joined, as the specification says.
    pub fn options(&self) -> String {
        self.lines
            .iter()
            .filter(|(name, _)| name == "options")
            .map(|(_, value)| value.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Apply additions and removals to a kernel command line. An added
/// `name=value` replaces every existing `name` or `name=...`; removals are by
/// name. Order is otherwise preserved, and additions go at the end.
pub fn edit_parameters(options: &str, add: &[String], remove: &[String]) -> String {
    let name_of = |word: &str| word.split('=').next().unwrap_or(word).to_string();
    let replaced: Vec<String> = add.iter().map(|parameter| name_of(parameter)).collect();
    let mut words: Vec<String> = options
        .split_whitespace()
        .filter(|word| {
            let name = name_of(word);
            !replaced.contains(&name) && !remove.contains(&name)
        })
        .map(str::to_string)
        .collect();
    words.extend(add.iter().cloned());
    words.join(" ")
}

#[derive(Clone, Debug, Serialize)]
pub struct BootPlan {
    pub loader: LoaderKind,
    pub add: Vec<String>,
    pub remove: Vec<String>,
    /// The entry the profile copies.
    pub base_entry: Option<String>,
    /// The file MICA would create, and its contents.
    pub new_entry: Option<(String, String)>,
    /// MICA can create the entry itself (with root, after confirmation).
    pub automatic: bool,
    pub steps: Vec<String>,
    /// How to boot the profile once it exists.
    pub use_it: Vec<String>,
    pub recovery: Vec<String>,
    pub warnings: Vec<String>,
}

impl BootPlan {
    pub fn nothing_to_do(&self) -> bool {
        self.add.is_empty() && self.remove.is_empty()
    }
}

fn entries_dir(context: &Context) -> Option<&'static str> {
    ENTRY_DIRS
        .iter()
        .copied()
        .find(|dir| context.list(dir).iter().any(|name| name.ends_with(".conf")))
}

/// Plan the boot profile for parameters `add` and `remove` (by name).
pub fn plan(context: &Context, profile: &HostProfile, add: &[String], remove: &[String]) -> BootPlan {
    let mut plan = BootPlan {
        loader: LoaderKind::Unknown,
        add: add.to_vec(),
        remove: remove.to_vec(),
        base_entry: None,
        new_entry: None,
        automatic: false,
        steps: Vec::new(),
        use_it: Vec::new(),
        recovery: Vec::new(),
        warnings: Vec::new(),
    };
    let joined = add.join(" ");
    let distro = profile.distro.as_ref().map(|distro| distro.id.as_str());

    if distro == Some("nixos") {
        plan.loader = LoaderKind::NixOs;
        plan.steps = vec![
            "Add a specialisation to configuration.nix; NixOS then adds a separate boot entry \
             named \"anticheat\":"
                .into(),
            format!(
                "  specialisation.anticheat.configuration.boot.kernelParams = [ {} ];",
                add.iter().map(|parameter| format!("\"{parameter}\"")).collect::<Vec<_>>().join(" ")
            ),
            "Then run `sudo nixos-rebuild boot` and pick the entry at boot.".into(),
        ];
        plan.recovery = vec![
            "Remove the specialisation line and run `sudo nixos-rebuild boot`.".into(),
        ];
        plan.warnings.push(
            "NixOS kernels are not signed with a key on Epic's well-known list, so this entry \
             alone does not make NixOS eligible."
                .into(),
        );
        return plan;
    }
    if context.exists("/run/ostree-booted") {
        plan.loader = LoaderKind::RpmOstree;
        plan.steps = vec![
            "rpm-ostree changes the arguments of the next deployment rather than adding an \
             entry. Run:"
                .into(),
            format!(
                "  sudo rpm-ostree kargs {}{}",
                add.iter().map(|parameter| format!("--append-if-missing={parameter} ")).collect::<String>(),
                remove.iter().map(|name| format!("--delete-if-present={name} ")).collect::<String>()
            )
            .trim_end()
            .to_string(),
            "and reboot.".into(),
        ];
        plan.recovery = vec![
            "Choose the previous deployment in the boot menu, or run `sudo rpm-ostree rollback`.".into(),
        ];
        return plan;
    }

    if let Some(dir) = entries_dir(context) {
        plan.loader = LoaderKind::BootLoaderSpec;
        let kernel = profile.kernel.clone().unwrap_or_default();
        let entries: Vec<BlsEntry> = context
            .list(dir)
            .into_iter()
            .filter(|name| name.ends_with(".conf") && !name.starts_with(ENTRY_PREFIX))
            .filter_map(|name| {
                let text = context.read(&format!("{dir}/{name}"))?;
                Some(BlsEntry::parse(&name, &text))
            })
            .collect();
        let base = entries.iter().find(|entry| {
            !kernel.is_empty()
                && (entry.get("version") == Some(kernel.as_str())
                    || entry.get("linux").is_some_and(|linux| linux.contains(&kernel)))
        });
        let Some(base) = base else {
            plan.steps.push(format!(
                "No boot entry in {dir} matches the running kernel ({kernel}); MICA will not \
                 guess which one to copy. Copy your usual entry to {dir}/{ENTRY_PREFIX}custom.conf \
                 and append to its options line: {joined}"
            ));
            return plan;
        };
        let options = base.options();
        if options.contains("$kernelopts") {
            plan.steps.push(
                "This entry takes its options from GRUB's $kernelopts variable (Fedora 32 and \
                 earlier). Run `sudo grub2-switch-to-blscfg` or edit the entry by hand."
                    .into(),
            );
            return plan;
        }
        let new_options = edit_parameters(&options, add, remove);
        let version = base.get("version").unwrap_or(&kernel).to_string();
        let file = format!("{dir}/{ENTRY_PREFIX}{version}.conf");
        let mut contents = format!("{MARKER}\n");
        let mut wrote_options = false;
        for (key, value) in &base.lines {
            match key.as_str() {
                "title" => contents.push_str(&format!("title {value} (MICA anti-cheat profile)\n")),
                "options" if !wrote_options => {
                    contents.push_str(&format!("options {new_options}\n"));
                    wrote_options = true;
                }
                "options" => {}
                // A unique sort key keeps the profile from becoming the
                // default in loaders that sort by it.
                "sort-key" => contents.push_str(&format!("sort-key {value}-mica\n")),
                _ => contents.push_str(&format!("{key} {value}\n")),
            }
        }
        if !wrote_options {
            contents.push_str(&format!("options {new_options}\n"));
        }
        plan.base_entry = Some(format!("{dir}/{}", base.file));
        plan.new_entry = Some((file.clone(), contents));
        plan.automatic = true;
        plan.steps = vec![
            format!("Create {file}: a copy of {} whose command line has {joined}", base.file),
            "Nothing else changes: the existing entries, the default entry, the kernel, and the \
             firmware stay as they are."
                .into(),
        ];
        let entry_name = format!("{ENTRY_PREFIX}{version}.conf");
        plan.use_it = vec![
            "Pick \"(MICA anti-cheat profile)\" in the boot menu when you want to play.".into(),
            format!("With systemd-boot, `sudo bootctl set-oneshot {entry_name}` boots it once on the next restart."),
        ];
        plan.recovery = vec![
            format!("`sudo mica security boot-profile --rollback`, or delete {file}."),
            "If the profile fails to boot, choose your usual entry in the boot menu; it was not \
             modified."
                .into(),
        ];
        plan.warnings.push(format!(
            "The profile names kernel {version}. After a kernel update, run `sudo mica security \
             boot-profile --apply` again to add an entry for the new kernel."
        ));
        return plan;
    }

    if context.exists("/boot/grub/grub.cfg") || context.exists("/boot/grub2/grub.cfg") {
        plan.loader = LoaderKind::GrubGenerated;
        let update = if context.exists("/boot/grub2/grub.cfg") {
            "sudo grub2-mkconfig -o /boot/grub2/grub.cfg"
        } else if profile
            .distro
            .as_ref()
            .is_some_and(|distro| distro.family == mica_probe::Family::Debian)
        {
            "sudo update-grub"
        } else {
            "sudo grub-mkconfig -o /boot/grub/grub.cfg"
        };
        plan.steps = vec![
            "Try it once without changing anything: at the GRUB menu press `e` on your usual \
             entry, append the parameters below to the line starting with `linux`, and press \
             Ctrl+X. The change lasts for that boot only."
                .into(),
            format!("  {joined}"),
            "To keep it: add the parameters to GRUB_CMDLINE_LINUX_DEFAULT in /etc/default/grub \
             (keep a copy of the file first), then run:"
                .into(),
            format!("  {update}"),
            "That changes your default entry. For a separate entry instead, copy your usual \
             menuentry from grub.cfg into /etc/grub.d/40_custom, add the parameters there, and \
             run the same command."
                .into(),
        ];
        if !remove.is_empty() {
            plan.steps.push(format!("Also remove: {}", remove.join(", ")));
        }
        plan.recovery = vec![
            "Restore your copy of /etc/default/grub and run the same command again.".into(),
            "If a boot fails, press `e` at the GRUB menu and delete the parameters for that boot.".into(),
        ];
        plan.warnings.push(
            "MICA does not edit GRUB configuration automatically: generated grub.cfg files \
             differ too much between distributions to change them safely."
                .into(),
        );
        return plan;
    }

    plan.steps.push(format!(
        "MICA could not identify the boot loader. Add these kernel parameters with your boot \
         loader's own tools, ideally to a separate entry: {joined}"
    ));
    plan
}

/// Write the planned entry. Fails unless the plan is automatic and the file
/// does not exist yet. Returns the path written.
pub fn apply(context: &Context, plan: &BootPlan) -> std::io::Result<PathBuf> {
    let Some((file, contents)) = &plan.new_entry else {
        return Err(std::io::Error::other("this plan has no entry for MICA to create"));
    };
    let path = context.path(file);
    if path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{file} already exists; run --rollback first to replace it"),
        ));
    }
    let temporary = path.with_extension("conf.mica-tmp");
    std::fs::write(&temporary, contents)?;
    std::fs::rename(&temporary, &path)?;
    Ok(path)
}

/// Delete every entry MICA created. Files without MICA's marker are never
/// touched. Returns the paths removed.
pub fn rollback(context: &Context) -> std::io::Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    for dir in ENTRY_DIRS {
        for name in context.list(dir) {
            if !name.starts_with(ENTRY_PREFIX) || !name.ends_with(".conf") {
                continue;
            }
            let path = context.path(&format!("{dir}/{name}"));
            if is_ours(&path) {
                std::fs::remove_file(&path)?;
                removed.push(path);
            }
        }
    }
    Ok(removed)
}

fn is_ours(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|text| text.starts_with(MARKER))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mica_probe::{Os, probe_with};

    fn tree(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("mica-boot-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(root: &Path, path: &str, contents: &str) {
        let full = root.join(path.trim_start_matches('/'));
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, contents).unwrap();
    }

    #[test]
    fn edits_parameters() {
        let edited = edit_parameters(
            "root=UUID=1 ro quiet iommu=pt lsm=landlock,yama lockdown=none",
            &["lockdown=integrity".into(), "lsm=landlock,yama,bpf".into()],
            &["iommu".into()],
        );
        assert_eq!(
            edited,
            "root=UUID=1 ro quiet lockdown=integrity lsm=landlock,yama,bpf"
        );
    }

    #[test]
    fn plans_applies_and_rolls_back_a_bls_entry() {
        let root = tree("bls");
        write(&root, "/etc/os-release", "ID=fedora\nVERSION_ID=43\n");
        write(&root, "/proc/sys/kernel/osrelease", "6.17.4-300.fc43.x86_64\n");
        let original = "title Fedora Linux (6.17.4-300.fc43.x86_64) 43\n\
                        version 6.17.4-300.fc43.x86_64\n\
                        linux /vmlinuz-6.17.4-300.fc43.x86_64\n\
                        initrd /initramfs-6.17.4-300.fc43.x86_64.img\n\
                        options root=UUID=1 ro rhgb quiet iommu=pt\n\
                        grub_users $grub_users\n\
                        grub_class fedora\n";
        write(&root, "/boot/loader/entries/abc-6.17.4-300.fc43.x86_64.conf", original);
        write(&root, "/boot/loader/entries/abc-6.16.0-200.fc43.x86_64.conf", "title old\nversion 6.16.0\nlinux /vmlinuz-6.16.0\noptions ro\n");
        let context = Context::fake(&root, &[], Context::no_commands());
        let profile = probe_with(&context, Os::Linux);
        let add = vec![
            "lockdown=integrity".to_string(),
            "sysctl.kernel.kexec_load_disabled=1".to_string(),
            "iommu.passthrough=0".to_string(),
        ];
        let bls = plan(&context, &profile, &add, &["iommu".to_string()]);
        assert_eq!(bls.loader, LoaderKind::BootLoaderSpec);
        assert!(bls.automatic);
        assert!(bls.base_entry.as_deref().unwrap().ends_with("abc-6.17.4-300.fc43.x86_64.conf"));
        let (file, contents) = bls.new_entry.clone().unwrap();
        assert_eq!(file, "/boot/loader/entries/mica-anticheat-6.17.4-300.fc43.x86_64.conf");
        assert!(contents.starts_with(MARKER));
        assert!(contents.contains("title Fedora Linux (6.17.4-300.fc43.x86_64) 43 (MICA anti-cheat profile)\n"));
        assert!(contents.contains(
            "options root=UUID=1 ro rhgb quiet lockdown=integrity sysctl.kernel.kexec_load_disabled=1 iommu.passthrough=0\n"
        ));
        assert!(contents.contains("grub_class fedora\n"));

        let written = apply(&context, &bls).unwrap();
        assert!(written.is_file());
        assert!(apply(&context, &bls).is_err(), "never overwrites");
        let entries = root.join("boot/loader/entries");
        assert_eq!(
            std::fs::read_to_string(entries.join("abc-6.17.4-300.fc43.x86_64.conf")).unwrap(),
            original,
            "the original entry is untouched"
        );
        // A same-prefixed file without MICA's marker is not MICA's.
        std::fs::write(entries.join("mica-anticheat-user.conf"), "title mine\n").unwrap();
        let removed = rollback(&context).unwrap();
        assert_eq!(removed, vec![written]);
        assert!(entries.join("mica-anticheat-user.conf").is_file());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn manual_plans_for_grub_ostree_and_nixos() {
        let root = tree("grub");
        write(&root, "/etc/os-release", "ID=ubuntu\nID_LIKE=debian\n");
        write(&root, "/boot/grub/grub.cfg", "menuentry 'Ubuntu' {}\n");
        let context = Context::fake(&root, &[], Context::no_commands());
        let profile = probe_with(&context, Os::Linux);
        let grub = plan(&context, &profile, &["lockdown=integrity".into()], &[]);
        assert_eq!(grub.loader, LoaderKind::GrubGenerated);
        assert!(!grub.automatic);
        assert!(grub.steps.iter().any(|step| step.contains("sudo update-grub")));
        std::fs::remove_dir_all(&root).unwrap();

        let root = tree("ostree");
        write(&root, "/etc/os-release", "ID=bazzite\nID_LIKE=fedora\n");
        write(&root, "/run/ostree-booted", "");
        let context = Context::fake(&root, &[], Context::no_commands());
        let profile = probe_with(&context, Os::Linux);
        let ostree = plan(&context, &profile, &["lockdown=integrity".into()], &["iommu".into()]);
        assert_eq!(ostree.loader, LoaderKind::RpmOstree);
        assert!(ostree.steps.iter().any(|step| step
            == "  sudo rpm-ostree kargs --append-if-missing=lockdown=integrity --delete-if-present=iommu"));
        std::fs::remove_dir_all(&root).unwrap();

        let root = tree("nixos");
        write(&root, "/etc/os-release", "ID=nixos\n");
        let context = Context::fake(&root, &[], Context::no_commands());
        let profile = probe_with(&context, Os::Linux);
        let nixos = plan(&context, &profile, &["lockdown=integrity".into()], &[]);
        assert_eq!(nixos.loader, LoaderKind::NixOs);
        assert!(nixos.steps[1].contains("\"lockdown=integrity\""));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
