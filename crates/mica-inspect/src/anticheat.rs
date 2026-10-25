//! Anti-cheat software shipped in a game's installation directory.
//!
//! Detection only reads file and directory names. It never opens, runs, or
//! modifies anti-cheat components; MICA uses the result to decide which
//! execution backends the publisher permits and to explain why a game may be
//! blocked, never to work around the anti-cheat.

use std::collections::VecDeque;
use std::fmt;
use std::path::Path;

use serde::Serialize;

/// Anti-cheat systems MICA knows about. Not all are detectable from files;
/// the compatibility database names the rest per game.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AntiCheat {
    EasyAntiCheat,
    BattlEye,
    EaJavelin,
    RiotVanguard,
    NProtectGameGuard,
    Xigncode3,
    PunkBuster,
    Mhyprot,
    Faceit,
    Ricochet,
}

impl AntiCheat {
    pub const ALL: [AntiCheat; 10] = [
        AntiCheat::EasyAntiCheat,
        AntiCheat::BattlEye,
        AntiCheat::EaJavelin,
        AntiCheat::RiotVanguard,
        AntiCheat::NProtectGameGuard,
        AntiCheat::Xigncode3,
        AntiCheat::PunkBuster,
        AntiCheat::Mhyprot,
        AntiCheat::Faceit,
        AntiCheat::Ricochet,
    ];

    /// The stable identifier used in the compatibility database.
    pub fn id(self) -> &'static str {
        match self {
            AntiCheat::EasyAntiCheat => "easy-anti-cheat",
            AntiCheat::BattlEye => "battleye",
            AntiCheat::EaJavelin => "ea-javelin",
            AntiCheat::RiotVanguard => "riot-vanguard",
            AntiCheat::NProtectGameGuard => "nprotect-gameguard",
            AntiCheat::Xigncode3 => "xigncode3",
            AntiCheat::PunkBuster => "punkbuster",
            AntiCheat::Mhyprot => "mhyprot",
            AntiCheat::Faceit => "faceit",
            AntiCheat::Ricochet => "ricochet",
        }
    }

    pub fn from_id(id: &str) -> Option<AntiCheat> {
        AntiCheat::ALL
            .into_iter()
            .find(|anticheat| anticheat.id() == id)
    }
}

impl fmt::Display for AntiCheat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AntiCheat::EasyAntiCheat => "Easy Anti-Cheat",
            AntiCheat::BattlEye => "BattlEye",
            AntiCheat::EaJavelin => "EA Javelin Anticheat",
            AntiCheat::RiotVanguard => "Riot Vanguard",
            AntiCheat::NProtectGameGuard => "nProtect GameGuard",
            AntiCheat::Xigncode3 => "XIGNCODE3",
            AntiCheat::PunkBuster => "PunkBuster",
            AntiCheat::Mhyprot => "mhyprot (HoYoverse)",
            AntiCheat::Faceit => "FACEIT",
            AntiCheat::Ricochet => "RICOCHET",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Confidence {
    /// A file name only this anti-cheat ships.
    High,
    /// A plausible but not unique name.
    Medium,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AntiCheatSignal {
    pub anticheat: AntiCheat,
    /// Path of the marker, relative to the scanned directory.
    pub marker: String,
    pub confidence: Confidence,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct AntiCheatScan {
    pub signals: Vec<AntiCheatSignal>,
    /// `easyanticheat_x64.so` ships beside the Windows EAC library. Valve's
    /// Proton documentation tells developers to add exactly this file to
    /// enable EAC under Proton, so its presence means the developer opted in.
    pub eac_proton_library: bool,
    pub entries_scanned: usize,
    /// The scan stopped at its entry limit before covering every file.
    pub truncated: bool,
}

impl AntiCheatScan {
    /// Distinct anti-cheat systems found, strongest evidence first.
    pub fn detected(&self) -> Vec<AntiCheat> {
        let mut found: Vec<AntiCheat> =
            self.signals.iter().map(|signal| signal.anticheat).collect();
        found.sort();
        found.dedup();
        found
    }
}

enum Marker {
    File(&'static str),
    Dir(&'static str),
    Prefix(&'static str),
}

const MARKERS: &[(Marker, AntiCheat, Confidence)] = &[
    (
        Marker::Dir("easyanticheat"),
        AntiCheat::EasyAntiCheat,
        Confidence::High,
    ),
    (
        Marker::File("easyanticheat_x64.dll"),
        AntiCheat::EasyAntiCheat,
        Confidence::High,
    ),
    (
        Marker::File("easyanticheat_x86.dll"),
        AntiCheat::EasyAntiCheat,
        Confidence::High,
    ),
    (
        Marker::File("easyanticheat_x64.so"),
        AntiCheat::EasyAntiCheat,
        Confidence::High,
    ),
    (
        Marker::File("easyanticheat_setup.exe"),
        AntiCheat::EasyAntiCheat,
        Confidence::High,
    ),
    (
        Marker::File("easyanticheat_eos_setup.exe"),
        AntiCheat::EasyAntiCheat,
        Confidence::High,
    ),
    (
        Marker::File("start_protected_game.exe"),
        AntiCheat::EasyAntiCheat,
        Confidence::Medium,
    ),
    (
        Marker::Dir("battleye"),
        AntiCheat::BattlEye,
        Confidence::High,
    ),
    (
        Marker::File("beservice.exe"),
        AntiCheat::BattlEye,
        Confidence::High,
    ),
    (
        Marker::File("beservice_x64.exe"),
        AntiCheat::BattlEye,
        Confidence::High,
    ),
    (
        Marker::File("beclient.dll"),
        AntiCheat::BattlEye,
        Confidence::High,
    ),
    (
        Marker::File("beclient_x64.dll"),
        AntiCheat::BattlEye,
        Confidence::High,
    ),
    (
        Marker::File("eaanticheat.gameservicelauncher.exe"),
        AntiCheat::EaJavelin,
        Confidence::High,
    ),
    (
        Marker::File("eaanticheat.installer.exe"),
        AntiCheat::EaJavelin,
        Confidence::High,
    ),
    (
        Marker::Prefix("eaanticheat."),
        AntiCheat::EaJavelin,
        Confidence::Medium,
    ),
    (
        Marker::File("vgk.sys"),
        AntiCheat::RiotVanguard,
        Confidence::High,
    ),
    (
        Marker::File("vgc.exe"),
        AntiCheat::RiotVanguard,
        Confidence::High,
    ),
    (
        Marker::Dir("gameguard"),
        AntiCheat::NProtectGameGuard,
        Confidence::High,
    ),
    (
        Marker::File("gamemon.des"),
        AntiCheat::NProtectGameGuard,
        Confidence::High,
    ),
    (
        Marker::File("gameguard.des"),
        AntiCheat::NProtectGameGuard,
        Confidence::High,
    ),
    (
        Marker::Dir("xigncode"),
        AntiCheat::Xigncode3,
        Confidence::High,
    ),
    (
        Marker::File("x3.xem"),
        AntiCheat::Xigncode3,
        Confidence::High,
    ),
    (
        Marker::File("pbsvc.exe"),
        AntiCheat::PunkBuster,
        Confidence::Medium,
    ),
    (
        Marker::File("pbcl.dll"),
        AntiCheat::PunkBuster,
        Confidence::Medium,
    ),
    (
        Marker::File("mhyprot2.sys"),
        AntiCheat::Mhyprot,
        Confidence::High,
    ),
    (
        Marker::File("mhyprot3.sys"),
        AntiCheat::Mhyprot,
        Confidence::High,
    ),
];

/// Most directory entries examined per scan; a game folder rarely has more.
pub const MAX_ENTRIES: usize = 50_000;
/// How deep below the scanned directory to look.
pub const MAX_DEPTH: usize = 4;

/// Look for anti-cheat components in a game's installation directory.
/// Symbolic links are not followed.
pub fn scan_dir(root: &Path) -> std::io::Result<AntiCheatScan> {
    let mut scan = AntiCheatScan::default();
    let mut queue = VecDeque::from([(root.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            // The top directory must be readable; unreadable subdirectories
            // are skipped.
            Err(error) if depth == 0 => return Err(error),
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            if scan.entries_scanned >= MAX_ENTRIES {
                scan.truncated = true;
                return Ok(scan);
            }
            scan.entries_scanned += 1;
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            for (marker, anticheat, confidence) in MARKERS {
                let hit = match marker {
                    Marker::File(file) => file_type.is_file() && name == *file,
                    Marker::Dir(dir) => file_type.is_dir() && name == *dir,
                    Marker::Prefix(prefix) => file_type.is_file() && name.starts_with(prefix),
                };
                if hit {
                    let duplicate = scan
                        .signals
                        .iter()
                        .any(|signal| signal.marker == relative && signal.anticheat == *anticheat);
                    if !duplicate {
                        scan.signals.push(AntiCheatSignal {
                            anticheat: *anticheat,
                            marker: relative.clone(),
                            confidence: *confidence,
                        });
                    }
                }
            }
            if file_type.is_file() && name == "easyanticheat_x64.so" {
                scan.eac_proton_library = true;
            }
            if file_type.is_dir() && depth + 1 < MAX_DEPTH {
                queue.push_back((path, depth + 1));
            }
        }
    }
    scan.signals.sort_by(|a, b| {
        (a.anticheat, a.confidence, &a.marker).cmp(&(b.anticheat, b.confidence, &b.marker))
    });
    Ok(scan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mica-ac-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(root: &Path, relative: &str) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"").unwrap();
    }

    #[test]
    fn finds_eac_and_the_proton_opt_in() {
        let root = temp("eac");
        touch(&root, "Game.exe");
        touch(&root, "EasyAntiCheat/EasyAntiCheat_EOS_Setup.exe");
        touch(&root, "EasyAntiCheat/easyanticheat_x64.so");
        touch(&root, "start_protected_game.exe");
        let scan = scan_dir(&root).unwrap();
        assert_eq!(scan.detected(), vec![AntiCheat::EasyAntiCheat]);
        assert!(scan.eac_proton_library);
        assert!(
            scan.signals
                .iter()
                .any(|signal| signal.marker == "EasyAntiCheat")
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn finds_battleye_and_javelin_without_proton_marker() {
        let root = temp("be");
        touch(&root, "BattlEye/BEClient_x64.dll");
        touch(&root, "Binaries/Win64/EAAntiCheat.GameServiceLauncher.exe");
        let scan = scan_dir(&root).unwrap();
        assert_eq!(
            scan.detected(),
            vec![AntiCheat::BattlEye, AntiCheat::EaJavelin]
        );
        assert!(!scan.eac_proton_library);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn clean_games_and_depth_limits() {
        let root = temp("clean");
        touch(&root, "game.exe");
        touch(&root, "a/b/c/d/e/EasyAntiCheat_x64.dll");
        let scan = scan_dir(&root).unwrap();
        assert!(scan.detected().is_empty(), "{:?}", scan.signals);
        assert!(scan_dir(&root.join("missing")).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn ids_round_trip() {
        for anticheat in AntiCheat::ALL {
            assert_eq!(AntiCheat::from_id(anticheat.id()), Some(anticheat));
        }
        assert_eq!(AntiCheat::from_id("nope"), None);
    }
}
