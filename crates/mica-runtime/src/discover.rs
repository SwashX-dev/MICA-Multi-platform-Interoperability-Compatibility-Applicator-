//! Which execution runtimes are installed on this host.
//!
//! MICA coordinates mature runtimes rather than reimplementing them: Wine
//! and Proton (with DXVK and VKD3D-Proton inside Proton) for Windows
//! programs on Linux, umu-launcher to run Proton outside Steam, CrossOver
//! and the Game Porting Toolkit on macOS, WSL on Windows, QEMU/KVM and
//! libvirt for virtual machines, FEX-Emu and Box64 for x86 code on ARM64
//! Linux, Rosetta 2 on Apple Silicon, and Portable Host for portable
//! WebAssembly components.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Serialize;

use mica_probe::{Context, Os};

use crate::steam::{self, SteamInstall};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeKind {
    Wine,
    Umu,
    Proton,
    Crossover,
    GamePortingToolkit,
    Wsl,
    Qemu,
    Libvirt,
    Fex,
    Box64,
    Rosetta,
    Lima,
    PortableHost,
    DosBox,
    Moonlight,
    SteamLinuxRuntime,
    EacRuntime,
    BattlEyeRuntime,
}

impl fmt::Display for RuntimeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            RuntimeKind::Wine => "Wine",
            RuntimeKind::Umu => "umu-launcher",
            RuntimeKind::Proton => "Proton",
            RuntimeKind::Crossover => "CrossOver",
            RuntimeKind::GamePortingToolkit => "Game Porting Toolkit",
            RuntimeKind::Wsl => "Windows Subsystem for Linux",
            RuntimeKind::Qemu => "QEMU",
            RuntimeKind::Libvirt => "libvirt",
            RuntimeKind::Fex => "FEX-Emu",
            RuntimeKind::Box64 => "Box64",
            RuntimeKind::Rosetta => "Rosetta 2",
            RuntimeKind::Lima => "Lima",
            RuntimeKind::PortableHost => "Portable Host",
            RuntimeKind::DosBox => "DOSBox",
            RuntimeKind::Moonlight => "Moonlight (game streaming client)",
            RuntimeKind::SteamLinuxRuntime => "Steam Linux Runtime",
            RuntimeKind::EacRuntime => "Proton EasyAntiCheat Runtime",
            RuntimeKind::BattlEyeRuntime => "Proton BattlEye Runtime",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Runtime {
    pub kind: RuntimeKind,
    pub name: String,
    pub path: PathBuf,
    pub version: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Runtimes {
    pub found: Vec<Runtime>,
    pub steam: Vec<SteamInstall>,
    /// Proton builds, best first.
    pub protons: Vec<steam::CompatTool>,
    pub anticheat: steam::AntiCheatRuntimes,
}

impl Runtimes {
    pub fn first(&self, kind: RuntimeKind) -> Option<&Runtime> {
        self.found.iter().find(|runtime| runtime.kind == kind)
    }

    pub fn has(&self, kind: RuntimeKind) -> bool {
        self.first(kind).is_some()
    }
}

fn program(
    context: &Context,
    kind: RuntimeKind,
    names: &[&str],
    version_args: Option<&[&str]>,
) -> Option<Runtime> {
    names.iter().find_map(|name| {
        let path = context.which(name)?;
        let version = version_args.and_then(|args| {
            context
                .run(&path.to_string_lossy(), args)
                .and_then(|output| output.lines().next().map(|line| line.trim().to_string()))
                .filter(|line| !line.is_empty())
        });
        Some(Runtime {
            kind,
            name: name.to_string(),
            path,
            version,
        })
    })
}

fn existing(kind: RuntimeKind, name: &str, path: &Path) -> Option<Runtime> {
    path.exists().then(|| Runtime {
        kind,
        name: name.into(),
        path: path.to_path_buf(),
        version: None,
    })
}

/// Discover runtimes. `preferred_proton` ranks a chosen Proton first.
pub fn discover(context: &Context, os: Os, preferred_proton: Option<&str>) -> Runtimes {
    let mut runtimes = Runtimes::default();
    let found = &mut runtimes.found;
    let home = context.home().unwrap_or_default();
    let mut push = |runtime: Option<Runtime>| {
        if let Some(runtime) = runtime {
            found.push(runtime);
        }
    };
    push(program(
        context,
        RuntimeKind::PortableHost,
        &["portable-host"],
        None,
    ));
    push(program(
        context,
        RuntimeKind::DosBox,
        &["dosbox-staging", "dosbox-x", "dosbox"],
        None,
    ));
    push(program(
        context,
        RuntimeKind::Moonlight,
        &["moonlight", "moonlight-qt"],
        None,
    ));
    match os {
        Os::Linux => {
            push(program(
                context,
                RuntimeKind::Wine,
                &["wine", "wine64"],
                Some(&["--version"]),
            ));
            push(program(context, RuntimeKind::Umu, &["umu-run"], None));
            push(program(
                context,
                RuntimeKind::Qemu,
                &["qemu-system-x86_64", "qemu-system-aarch64"],
                None,
            ));
            push(program(context, RuntimeKind::Libvirt, &["virsh"], None));
            push(program(
                context,
                RuntimeKind::Fex,
                &["FEXInterpreter", "FEXBash"],
                None,
            ));
            push(program(context, RuntimeKind::Box64, &["box64"], None));
            runtimes.steam = steam::find(&steam::candidate_roots(&home, "linux"));
        }
        Os::MacOs => {
            push(program(
                context,
                RuntimeKind::Wine,
                &["wine", "wine64"],
                Some(&["--version"]),
            ));
            push(existing(
                RuntimeKind::Crossover,
                "CrossOver",
                Path::new("/Applications/CrossOver.app/Contents/SharedSupport/CrossOver/bin/wine"),
            ));
            for path in [
                "/usr/local/opt/game-porting-toolkit/bin/wine64",
                "/opt/homebrew/opt/game-porting-toolkit/bin/wine64",
            ] {
                push(existing(
                    RuntimeKind::GamePortingToolkit,
                    "Game Porting Toolkit",
                    Path::new(path),
                ));
            }
            push(existing(
                RuntimeKind::Rosetta,
                "Rosetta 2",
                Path::new("/Library/Apple/usr/libexec/oah/libRosettaRuntime"),
            ));
            push(program(context, RuntimeKind::Lima, &["limactl"], None));
            push(program(
                context,
                RuntimeKind::Qemu,
                &["qemu-system-aarch64", "qemu-system-x86_64"],
                None,
            ));
            runtimes.steam = steam::find(&steam::candidate_roots(&home, "macos"));
        }
        Os::Windows => {
            let system_root = context.env("SystemRoot").unwrap_or(r"C:\Windows");
            let wsl = Path::new(system_root).join(r"System32\wsl.exe");
            if wsl.is_file() {
                // `wsl -l -q` lists installed distributions (UTF-16 output).
                let distributions = context
                    .run(&wsl.to_string_lossy(), &["-l", "-q"])
                    .map(|text| {
                        text.lines()
                            .map(|line| line.trim().trim_matches('\0').to_string())
                            .filter(|line| !line.is_empty())
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .filter(|list| !list.is_empty());
                push(Some(Runtime {
                    kind: RuntimeKind::Wsl,
                    name: "wsl".into(),
                    path: wsl,
                    version: distributions.map(|list| format!("distributions: {list}")),
                }));
            }
            push(program(
                context,
                RuntimeKind::Qemu,
                &["qemu-system-x86_64"],
                None,
            ));
            let mut roots = steam::candidate_roots(&home, "windows");
            if let Some(path) = windows_steam_path(context) {
                roots.insert(0, (path, false));
            }
            runtimes.steam = steam::find(&roots);
        }
        Os::Other => {}
    }
    let tools: Vec<steam::CompatTool> = runtimes
        .steam
        .iter()
        .flat_map(steam::compat_tools)
        .collect();
    runtimes.protons = steam::rank_protons(&tools, preferred_proton);
    for proton in &runtimes.protons {
        runtimes.found.push(Runtime {
            kind: RuntimeKind::Proton,
            name: proton.display_name.clone(),
            path: proton.dir.clone(),
            version: std::fs::read_to_string(proton.dir.join("version"))
                .ok()
                .map(|text| text.trim().to_string()),
        });
    }
    for install in &runtimes.steam {
        let anticheat = steam::anticheat_runtimes(install);
        if runtimes.anticheat.eac.is_none() {
            runtimes.anticheat.eac = anticheat.eac;
        }
        if runtimes.anticheat.battleye.is_none() {
            runtimes.anticheat.battleye = anticheat.battleye;
        }
        if let Some(path) = steam::app_dir(install, "1628350") {
            runtimes.found.push(Runtime {
                kind: RuntimeKind::SteamLinuxRuntime,
                name: "Steam Linux Runtime 3.0 (sniper)".into(),
                path,
                version: None,
            });
        }
    }
    if let Some(path) = &runtimes.anticheat.eac {
        runtimes.found.push(Runtime {
            kind: RuntimeKind::EacRuntime,
            name: "Proton EasyAntiCheat Runtime".into(),
            path: path.clone(),
            version: None,
        });
    }
    if let Some(path) = &runtimes.anticheat.battleye {
        runtimes.found.push(Runtime {
            kind: RuntimeKind::BattlEyeRuntime,
            name: "Proton BattlEye Runtime".into(),
            path: path.clone(),
            version: None,
        });
    }
    runtimes
}

/// Steam's install path from the registry (`HKCU\Software\Valve\Steam`).
fn windows_steam_path(context: &Context) -> Option<PathBuf> {
    let output = context.run(
        "reg",
        &["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"],
    )?;
    let line = output.lines().find(|line| line.contains("SteamPath"))?;
    let value = line.split("REG_SZ").nth(1)?.trim();
    Some(PathBuf::from(value.replace('/', "\\")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_programs_on_path_and_steam_tools() {
        let (home, _) = crate::steam::tests::fake_steam("discover");
        let bin = home.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for name in ["wine", "umu-run", "virsh"] {
            std::fs::write(bin.join(name), "").unwrap();
        }
        let context = Context::fake(
            Path::new("/"),
            &[
                ("PATH", &bin.to_string_lossy()),
                ("HOME", &home.to_string_lossy()),
            ],
            Box::new(|program, args| {
                (program.ends_with("wine") && args == ["--version"])
                    .then(|| b"wine-11.0\n".to_vec())
            }),
        );
        let runtimes = discover(&context, Os::Linux, None);
        assert_eq!(
            runtimes
                .first(RuntimeKind::Wine)
                .unwrap()
                .version
                .as_deref(),
            Some("wine-11.0")
        );
        assert!(runtimes.has(RuntimeKind::Umu));
        assert!(runtimes.has(RuntimeKind::Libvirt));
        assert!(!runtimes.has(RuntimeKind::Qemu));
        assert_eq!(runtimes.protons[0].display_name, "Proton - Experimental");
        assert!(runtimes.has(RuntimeKind::EacRuntime));
        assert!(runtimes.has(RuntimeKind::SteamLinuxRuntime));
        std::fs::remove_dir_all(&home).unwrap();
    }
}
