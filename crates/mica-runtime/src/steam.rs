//! Steam: installations, libraries, installed games, Proton builds, Valve's
//! anti-cheat runtimes, and MICA's optional compatibility tool.
//!
//! Everything here reads Steam's files; nothing is written into Steam except
//! MICA's own directory under `compatibilitytools.d`, and only by
//! `mica steam install-tool`. MICA never edits Steam's configuration, its
//! per-game settings, or any game file.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::vdf::{self, Value};

/// Steam's own directory names for Valve's Proton anti-cheat runtimes, as
/// installed into `steamapps/common`. MICA verifies the layout Proton's Wine
/// loads from (see [`AntiCheatRuntimes`]) rather than trusting the names.
const EAC_RUNTIME_DIR: &str = "Proton EasyAntiCheat Runtime";
const BATTLEYE_RUNTIME_DIR: &str = "Proton BattlEye Runtime";
/// The directory MICA's compatibility tool lives in.
pub const MICA_TOOL_DIR: &str = "mica";
const MICA_TOOL_MARKER: &str = "mica-tool.txt";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SteamInstall {
    pub root: PathBuf,
    /// Installed from Flathub (`com.valvesoftware.Steam`).
    pub flatpak: bool,
    pub libraries: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SteamApp {
    pub app_id: String,
    pub name: String,
    pub install_dir: PathBuf,
    pub library: PathBuf,
    /// Fully installed (StateFlags has the "installed" bit).
    pub installed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CompatTool {
    /// Steam's internal name, e.g. `proton_experimental`, `GE-Proton10-25`.
    pub name: String,
    pub display_name: String,
    pub dir: PathBuf,
    /// The `proton` script, if this is a Proton build.
    pub proton: Option<PathBuf>,
    /// The Steam Linux Runtime the tool runs inside, by app ID.
    pub require_tool_appid: Option<String>,
    /// Installed by the user into compatibilitytools.d (GE-Proton, ...),
    /// rather than by Steam.
    pub custom: bool,
}

/// Valve's Proton runtimes for Easy Anti-Cheat and BattlEye. Proton's Wine
/// adds `$PROTON_EAC_RUNTIME/v2/lib64` and `$PROTON_BATTLEYE_RUNTIME/v1/lib64/wine`
/// to its DLL search path (`set_dll_path` in Valve's Wine,
/// `dlls/ntdll/unix/loader.c`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct AntiCheatRuntimes {
    pub eac: Option<PathBuf>,
    pub battleye: Option<PathBuf>,
}

/// Where Steam may be installed for a user with home directory `home`.
pub fn candidate_roots(home: &Path, os: &str) -> Vec<(PathBuf, bool)> {
    match os {
        "macos" => vec![(home.join("Library/Application Support/Steam"), false)],
        "windows" => vec![
            (PathBuf::from(r"C:\Program Files (x86)\Steam"), false),
            (PathBuf::from(r"C:\Program Files\Steam"), false),
        ],
        _ => vec![
            (home.join(".steam/root"), false),
            (home.join(".steam/steam"), false),
            (home.join(".local/share/Steam"), false),
            (
                home.join(".var/app/com.valvesoftware.Steam/data/Steam"),
                true,
            ),
            (
                home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
                true,
            ),
            (home.join("snap/steam/common/.local/share/Steam"), false),
        ],
    }
}

/// `canonicalize`, without Windows' `\\?\` verbatim prefix when the path is
/// an ordinary drive path, so paths print and compare the way users see them.
pub fn canonical(path: &Path) -> PathBuf {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let text = canonical.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => canonical,
    }
}

/// Every distinct Steam installation among `candidates`.
pub fn find(candidates: &[(PathBuf, bool)]) -> Vec<SteamInstall> {
    let mut found: Vec<SteamInstall> = Vec::new();
    for (root, flatpak) in candidates {
        if !root.join("steamapps").is_dir() {
            continue;
        }
        let canonical = canonical(root);
        if found.iter().any(|install| install.root == canonical) {
            continue;
        }
        found.push(SteamInstall {
            libraries: libraries(&canonical),
            root: canonical,
            flatpak: *flatpak,
        });
    }
    found
}

/// The library folders listed in `steamapps/libraryfolders.vdf`, with the
/// root first.
pub fn libraries(root: &Path) -> Vec<PathBuf> {
    let mut libraries = vec![root.to_path_buf()];
    let text =
        std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")).unwrap_or_default();
    if let Ok(document) = vdf::parse(&text) {
        let folders = document
            .get("libraryfolders")
            .or_else(|| document.get("LibraryFolders"));
        for (_, folder) in folders.map(Value::entries).unwrap_or_default() {
            // Current format: { "path" "..." }; old format: "1" "/path".
            let path = folder.get("path").and_then(Value::text).or_else(|| {
                folder
                    .text()
                    .filter(|text| text.contains('/') || text.contains('\\'))
            });
            if let Some(path) = path {
                let canonical = canonical(Path::new(path));
                if !libraries.contains(&canonical) && canonical.join("steamapps").is_dir() {
                    libraries.push(canonical);
                }
            }
        }
    }
    libraries
}

/// Games and tools installed in every library.
pub fn installed_apps(install: &SteamInstall) -> Vec<SteamApp> {
    let mut apps = Vec::new();
    for library in &install.libraries {
        let steamapps = library.join("steamapps");
        let Ok(entries) = std::fs::read_dir(&steamapps) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            let Ok(document) = vdf::parse(&text) else {
                continue;
            };
            let Some(state) = document.get("AppState") else {
                continue;
            };
            let field = |key: &str| state.get(key).and_then(Value::text).unwrap_or_default();
            if field("appid").is_empty() || field("installdir").is_empty() {
                continue;
            }
            let flags: u32 = field("StateFlags").parse().unwrap_or(0);
            apps.push(SteamApp {
                app_id: field("appid").to_string(),
                name: field("name").to_string(),
                install_dir: steamapps.join("common").join(field("installdir")),
                library: library.clone(),
                installed: flags & 4 != 0,
            });
        }
    }
    apps.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });
    apps
}

fn tool_manifest(dir: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(dir.join("toolmanifest.vdf")).ok()?;
    vdf::parse(&text).ok()?.get("manifest").cloned()
}

/// Proton builds and other compatibility tools: Valve's in each library's
/// `steamapps/common`, the user's in `compatibilitytools.d`.
pub fn compat_tools(install: &SteamInstall) -> Vec<CompatTool> {
    let mut tools = Vec::new();
    let custom_dirs = [
        install.root.join("compatibilitytools.d"),
        PathBuf::from("/usr/share/steam/compatibilitytools.d"),
    ];
    for dir in custom_dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let tool_dir = entry.path();
            let Ok(text) = std::fs::read_to_string(tool_dir.join("compatibilitytool.vdf")) else {
                continue;
            };
            let Ok(document) = vdf::parse(&text) else {
                continue;
            };
            for (name, tool) in document
                .path(&["compatibilitytools", "compat_tools"])
                .map(Value::entries)
                .unwrap_or_default()
            {
                let proton = tool_dir.join("proton");
                tools.push(CompatTool {
                    name: name.clone(),
                    display_name: tool
                        .get("display_name")
                        .and_then(Value::text)
                        .unwrap_or(name)
                        .to_string(),
                    require_tool_appid: tool_manifest(&tool_dir).and_then(|manifest| {
                        manifest
                            .get("require_tool_appid")
                            .and_then(Value::text)
                            .map(str::to_string)
                    }),
                    proton: proton.is_file().then_some(proton),
                    dir: tool_dir.clone(),
                    custom: true,
                });
            }
        }
    }
    for library in &install.libraries {
        let Ok(entries) = std::fs::read_dir(library.join("steamapps/common")) else {
            continue;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            let proton = dir.join("proton");
            if !proton.is_file() {
                continue;
            }
            let display_name = entry.file_name().to_string_lossy().into_owned();
            tools.push(CompatTool {
                name: display_name
                    .to_ascii_lowercase()
                    .replace([' ', '-', '.'], "_"),
                require_tool_appid: tool_manifest(&dir).and_then(|manifest| {
                    manifest
                        .get("require_tool_appid")
                        .and_then(Value::text)
                        .map(str::to_string)
                }),
                display_name,
                proton: Some(proton),
                dir,
                custom: false,
            });
        }
    }
    tools
}

/// Proton builds, best first: an explicit choice, else Proton Experimental,
/// else the highest-numbered GE-Proton or Proton.
pub fn rank_protons(tools: &[CompatTool], preferred: Option<&str>) -> Vec<CompatTool> {
    let mut protons: Vec<CompatTool> = tools
        .iter()
        .filter(|tool| tool.proton.is_some())
        .cloned()
        .collect();
    let numbers = |name: &str| -> Vec<u32> {
        name.split(|c: char| !c.is_ascii_digit())
            .filter_map(|part| part.parse().ok())
            .collect()
    };
    protons.sort_by(|a, b| {
        let key = |tool: &CompatTool| {
            let name = tool.display_name.as_str();
            let chosen = preferred.is_some_and(|wanted| {
                name == wanted || tool.name == wanted || tool.dir == Path::new(wanted)
            });
            (chosen, name.contains("Experimental"), numbers(name))
        };
        key(b).cmp(&key(a))
    });
    protons
}

/// Valve's anti-cheat runtimes, if Steam has installed them.
pub fn anticheat_runtimes(install: &SteamInstall) -> AntiCheatRuntimes {
    let mut runtimes = AntiCheatRuntimes::default();
    for library in &install.libraries {
        let common = library.join("steamapps/common");
        let eac = common.join(EAC_RUNTIME_DIR);
        if runtimes.eac.is_none() && eac.join("v2").is_dir() {
            runtimes.eac = Some(eac);
        }
        let battleye = common.join(BATTLEYE_RUNTIME_DIR);
        if runtimes.battleye.is_none() && battleye.join("v1").is_dir() {
            runtimes.battleye = Some(battleye);
        }
    }
    runtimes
}

/// The installation directory of a Steam tool by app ID (for example the
/// Steam Linux Runtime a Proton build requires).
pub fn app_dir(install: &SteamInstall, app_id: &str) -> Option<PathBuf> {
    installed_apps(install)
        .into_iter()
        .find(|app| app.app_id == app_id && app.install_dir.is_dir())
        .map(|app| app.install_dir)
}

/// The command Steam itself would run for a Proton tool: Proton inside its
/// Steam Linux Runtime container when the tool requires one.
pub fn proton_command(
    install: &SteamInstall,
    tool: &CompatTool,
    verb: &str,
    args: &[String],
) -> Result<Vec<String>, String> {
    let proton = tool
        .proton
        .as_ref()
        .ok_or_else(|| format!("{} is not a Proton build", tool.display_name))?;
    let mut command = Vec::new();
    if let Some(app_id) = &tool.require_tool_appid {
        let runtime = app_dir(install, app_id).ok_or_else(|| {
            format!(
                "{} needs Steam tool {app_id} (a Steam Linux Runtime); install it from your \
                 Steam library (Tools)",
                tool.display_name
            )
        })?;
        command.push(
            runtime
                .join("_v2-entry-point")
                .to_string_lossy()
                .into_owned(),
        );
        command.push(format!("--verb={verb}"));
        command.push("--".into());
    }
    command.push(proton.to_string_lossy().into_owned());
    command.push(verb.to_string());
    command.extend(args.iter().cloned());
    Ok(command)
}

/// Install MICA's compatibility tool into `<steam>/compatibilitytools.d/mica`.
/// Steam lists it as "MICA" in each game's Compatibility settings once
/// restarted; nothing is selected for any game.
pub fn install_tool(root: &Path, mica: &Path) -> std::io::Result<PathBuf> {
    let dir = root.join("compatibilitytools.d").join(MICA_TOOL_DIR);
    if dir.exists() && !dir.join(MICA_TOOL_MARKER).is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} exists and was not created by MICA", dir.display()),
        ));
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join(MICA_TOOL_MARKER),
        "Created by `mica steam install-tool`. `mica steam remove-tool` deletes this directory.\n",
    )?;
    let tool = vdf::object(&[
        ("install_path", "."),
        (
            "display_name",
            "MICA (checks anti-cheat policy, then runs Proton)",
        ),
        ("from_oslist", "windows"),
        ("to_oslist", "linux"),
    ]);
    std::fs::write(
        dir.join("compatibilitytool.vdf"),
        vdf::write(&[(
            "compatibilitytools".into(),
            Value::Object(vec![(
                "compat_tools".into(),
                Value::Object(vec![("mica".into(), tool)]),
            )]),
        )]),
    )?;
    // No require_tool_appid: the wrapper runs on the host and starts the
    // chosen Proton inside its own Steam Linux Runtime, as Steam would.
    let manifest = vdf::object(&[
        ("version", "2"),
        ("commandline", "/mica-steam %verb%"),
        ("use_sessions", "1"),
    ]);
    std::fs::write(
        dir.join("toolmanifest.vdf"),
        vdf::write(&[("manifest".into(), manifest)]),
    )?;
    let script = dir.join("mica-steam");
    let quoted = mica.to_string_lossy().replace('\'', "'\\''");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n# Created by MICA: Steam runs this for games set to the \"MICA\" \
             compatibility tool.\nexec '{quoted}' steam run \"$@\"\n"
        ),
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(dir)
}

/// Remove MICA's compatibility tool, if MICA created it.
pub fn remove_tool(root: &Path) -> std::io::Result<bool> {
    let dir = root.join("compatibilitytools.d").join(MICA_TOOL_DIR);
    if !dir.join(MICA_TOOL_MARKER).is_file() {
        return Ok(false);
    }
    std::fs::remove_dir_all(dir)?;
    Ok(true)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn fake_steam(name: &str) -> (PathBuf, PathBuf) {
        let home = std::env::temp_dir().join(format!("mica-steam-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let root = home.join(".local/share/Steam");
        let second = home.join("games/SteamLibrary");
        let write = |path: PathBuf, text: &str| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            root.join("steamapps/libraryfolders.vdf"),
            &format!(
                "\"libraryfolders\"\n{{\n\"0\"\n{{\n\"path\" \"{}\"\n}}\n\"1\"\n{{\n\"path\" \"{}\"\n}}\n}}\n",
                root.display().to_string().replace('\\', "\\\\"),
                second.display().to_string().replace('\\', "\\\\")
            ),
        );
        let manifest = |id: &str, name: &str, dir: &str, flags: &str| {
            format!(
                "\"AppState\"\n{{\n\"appid\" \"{id}\"\n\"name\" \"{name}\"\n\"installdir\" \"{dir}\"\n\"StateFlags\" \"{flags}\"\n}}\n"
            )
        };
        write(
            second.join("steamapps/appmanifest_1172470.acf"),
            &manifest("1172470", "Apex Legends", "Apex Legends", "4"),
        );
        write(
            second.join("steamapps/common/Apex Legends/EAAntiCheat.GameServiceLauncher.exe"),
            "",
        );
        write(
            root.join("steamapps/appmanifest_1628350.acf"),
            &manifest(
                "1628350",
                "Steam Linux Runtime 3.0 (sniper)",
                "SteamLinuxRuntime_sniper",
                "4",
            ),
        );
        write(
            root.join("steamapps/common/SteamLinuxRuntime_sniper/_v2-entry-point"),
            "",
        );
        write(root.join("steamapps/common/Proton 9.0 (Beta)/proton"), "");
        write(
            root.join("steamapps/common/Proton 9.0 (Beta)/toolmanifest.vdf"),
            "\"manifest\" { \"version\" \"2\" \"commandline\" \"/proton %verb%\" \"require_tool_appid\" \"1628350\" }",
        );
        write(
            root.join("steamapps/common/Proton - Experimental/proton"),
            "",
        );
        write(
            root.join(
                "steamapps/common/Proton EasyAntiCheat Runtime/v2/lib64/easyanticheat_x64.so",
            ),
            "",
        );
        write(root.join("compatibilitytools.d/GE-Proton10-25/proton"), "");
        write(
            root.join("compatibilitytools.d/GE-Proton10-25/compatibilitytool.vdf"),
            "\"compatibilitytools\" { \"compat_tools\" { \"GE-Proton10-25\" { \"install_path\" \".\" \"display_name\" \"GE-Proton10-25\" \"from_oslist\" \"windows\" \"to_oslist\" \"linux\" } } }",
        );
        (home, root)
    }

    #[test]
    fn reads_libraries_apps_tools_and_runtimes() {
        let (home, _) = fake_steam("read");
        let installs = find(&candidate_roots(&home, "linux"));
        assert_eq!(installs.len(), 1, "one installation, found once");
        let install = &installs[0];
        assert_eq!(install.libraries.len(), 2);

        let apps = installed_apps(install);
        let apex = apps.iter().find(|app| app.app_id == "1172470").unwrap();
        assert!(apex.installed);
        assert!(apex.install_dir.ends_with("steamapps/common/Apex Legends"));

        let tools = compat_tools(install);
        let ranked = rank_protons(&tools, None);
        assert_eq!(ranked[0].display_name, "Proton - Experimental");
        assert_eq!(ranked[1].display_name, "GE-Proton10-25");
        let chosen = rank_protons(&tools, Some("Proton 9.0 (Beta)"));
        assert_eq!(chosen[0].display_name, "Proton 9.0 (Beta)");
        assert_eq!(chosen[0].require_tool_appid.as_deref(), Some("1628350"));

        let command = proton_command(
            install,
            &chosen[0],
            "waitforexitandrun",
            &["game.exe".into()],
        )
        .unwrap();
        assert!(command[0].ends_with("_v2-entry-point"));
        assert_eq!(command[1], "--verb=waitforexitandrun");
        assert_eq!(command[2], "--");
        assert!(command[3].ends_with("proton"));
        assert_eq!(&command[4..], ["waitforexitandrun", "game.exe"]);

        let runtimes = anticheat_runtimes(install);
        assert!(runtimes.eac.is_some());
        assert!(runtimes.battleye.is_none());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn installs_and_removes_only_its_own_tool() {
        let (home, root) = fake_steam("tool");
        let dir = install_tool(&root, Path::new("/usr/bin/mica")).unwrap();
        let installs = find(&candidate_roots(&home, "linux"));
        let tools = compat_tools(&installs[0]);
        let mica = tools.iter().find(|tool| tool.name == "mica").unwrap();
        assert!(mica.custom);
        assert!(mica.proton.is_none(), "MICA's tool is not itself a Proton");
        let script = std::fs::read_to_string(dir.join("mica-steam")).unwrap();
        assert!(script.contains("exec '/usr/bin/mica' steam run \"$@\""));
        assert!(remove_tool(&root).unwrap());
        assert!(!dir.exists());
        // A directory someone else made is left alone.
        std::fs::create_dir_all(&dir).unwrap();
        assert!(install_tool(&root, Path::new("/usr/bin/mica")).is_err());
        assert!(!remove_tool(&root).unwrap());
        std::fs::remove_dir_all(&home).unwrap();
    }
}
