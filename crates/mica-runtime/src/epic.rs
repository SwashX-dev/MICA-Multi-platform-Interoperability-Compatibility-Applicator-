//! Epic Games: the Epic Games Launcher (Windows manifests, the Linux
//! Preview), and games installed through Heroic or Legendary on Linux.
//!
//! MICA only reads launcher files. It never signs in, never reads or stores
//! account credentials or tokens, and never starts a protected game around
//! the launcher.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EpicGame {
    pub app_name: String,
    pub title: String,
    pub install_dir: PathBuf,
    pub executable: Option<PathBuf>,
    /// Which program installed it: `epic-launcher`, `heroic`, `legendary`.
    pub source: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct EpicStatus {
    /// Where the Epic Games Launcher (or its Linux Preview) was found.
    pub launcher: Option<PathBuf>,
    pub games: Vec<EpicGame>,
}

/// The fields MICA reads from a Windows launcher manifest (`*.item`).
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LauncherItem {
    #[serde(default)]
    app_name: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    install_location: String,
    #[serde(default)]
    launch_executable: String,
}

/// The fields MICA reads from Legendary's `installed.json`.
#[derive(Deserialize)]
struct LegendaryGame {
    #[serde(default)]
    title: String,
    #[serde(default)]
    install_path: String,
    #[serde(default)]
    executable: String,
}

pub fn launcher_manifests(dir: &Path) -> Vec<EpicGame> {
    let mut games = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return games;
    };
    for entry in entries.flatten() {
        if entry.path().extension().is_none_or(|ext| ext != "item") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Ok(item) = serde_json::from_str::<LauncherItem>(&text) else {
            continue;
        };
        if item.install_location.is_empty() {
            continue;
        }
        let install_dir = PathBuf::from(&item.install_location);
        games.push(EpicGame {
            app_name: item.app_name,
            title: item.display_name,
            executable: (!item.launch_executable.is_empty())
                .then(|| install_dir.join(&item.launch_executable)),
            install_dir,
            source: "epic-launcher".into(),
        });
    }
    games.sort_by(|a, b| a.title.cmp(&b.title));
    games
}

pub fn legendary_installed(file: &Path, source: &str) -> Vec<EpicGame> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    let Ok(map) = serde_json::from_str::<std::collections::BTreeMap<String, LegendaryGame>>(&text)
    else {
        return Vec::new();
    };
    map.into_iter()
        .filter(|(_, game)| !game.install_path.is_empty())
        .map(|(app_name, game)| {
            let install_dir = PathBuf::from(&game.install_path);
            EpicGame {
                app_name,
                title: game.title,
                executable: (!game.executable.is_empty())
                    .then(|| install_dir.join(&game.executable)),
                install_dir,
                source: source.into(),
            }
        })
        .collect()
}

/// Look for the launcher and installed games for a user.
pub fn detect(home: &Path, os: &str, program_data: Option<&Path>) -> EpicStatus {
    let mut status = EpicStatus::default();
    match os {
        "windows" => {
            let data = program_data
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
            let manifests = data.join(r"Epic\EpicGamesLauncher\Data\Manifests");
            status.games = launcher_manifests(&manifests);
            for candidate in [
                PathBuf::from(
                    r"C:\Program Files (x86)\Epic Games\Launcher\Portal\Binaries\Win64\EpicGamesLauncher.exe",
                ),
                PathBuf::from(
                    r"C:\Program Files\Epic Games\Launcher\Portal\Binaries\Win64\EpicGamesLauncher.exe",
                ),
            ] {
                if candidate.is_file() {
                    status.launcher = Some(candidate);
                    break;
                }
            }
        }
        "macos" => {
            let app = PathBuf::from("/Applications/Epic Games Launcher.app");
            status.launcher = app.is_dir().then_some(app);
        }
        _ => {
            for (file, source) in [
                (home.join(".config/heroic/legendaryConfig/legendary/installed.json"), "heroic"),
                (
                    home.join(".var/app/com.heroicgameslauncher.hgl/config/heroic/legendaryConfig/legendary/installed.json"),
                    "heroic",
                ),
                (home.join(".config/legendary/installed.json"), "legendary"),
            ] {
                for game in legendary_installed(&file, source) {
                    if !status.games.iter().any(|known| known.install_dir == game.install_dir) {
                        status.games.push(game);
                    }
                }
            }
            status.launcher = linux_launcher(home);
        }
    }
    status
}

/// The Linux Preview ships as an AppImage or a .deb (Epic Games support,
/// "What is the Epic Games Launcher Preview?"). Its install location is not
/// documented, so MICA looks for a desktop entry that names it.
fn linux_launcher(home: &Path) -> Option<PathBuf> {
    let dirs = [
        home.join(".local/share/applications"),
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/usr/local/share/applications"),
    ];
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "desktop") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            if text
                .lines()
                .any(|line| line.starts_with("Name=") && line.contains("Epic Games Launcher"))
            {
                return Some(path);
            }
        }
    }
    let applications = home.join("Applications");
    std::fs::read_dir(&applications)
        .ok()?
        .flatten()
        .find_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            (name.contains("epic") && name.ends_with(".appimage")).then(|| entry.path())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_windows_manifests_and_heroic() {
        let root = std::env::temp_dir().join(format!("mica-epic-{}", std::process::id()));
        let manifests = root.join(r"Epic\EpicGamesLauncher\Data\Manifests");
        std::fs::create_dir_all(&manifests).unwrap();
        std::fs::write(
            manifests.join("ABC.item"),
            r#"{"FormatVersion":0,"AppName":"Fortnite","DisplayName":"Fortnite","InstallLocation":"C:\\Program Files\\Epic Games\\Fortnite","LaunchExecutable":"FortniteGame/Binaries/Win64/FortniteLauncher.exe"}"#,
        )
        .unwrap();
        std::fs::write(manifests.join("broken.item"), "{").unwrap();
        let status = detect(&root, "windows", Some(&root));
        assert_eq!(status.games.len(), 1);
        assert_eq!(status.games[0].app_name, "Fortnite");
        assert!(
            status.games[0]
                .executable
                .as_ref()
                .unwrap()
                .ends_with("FortniteLauncher.exe")
        );

        let heroic = root.join(".config/heroic/legendaryConfig/legendary");
        std::fs::create_dir_all(&heroic).unwrap();
        std::fs::write(
            heroic.join("installed.json"),
            r#"{"Sugar":{"title":"Rocket League","install_path":"/home/u/Games/Heroic/rocketleague","executable":"Binaries/Win64/RocketLeague.exe","platform":"Windows"}}"#,
        )
        .unwrap();
        let apps = root.join(".local/share/applications");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::write(
            apps.join("epic.desktop"),
            "[Desktop Entry]\nName=Epic Games Launcher\n",
        )
        .unwrap();
        let status = detect(&root, "linux", None);
        assert_eq!(status.games[0].title, "Rocket League");
        assert_eq!(status.games[0].source, "heroic");
        assert!(status.launcher.is_some());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
