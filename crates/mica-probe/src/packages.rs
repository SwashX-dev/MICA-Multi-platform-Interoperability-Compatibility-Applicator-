//! Package managers present on the host.

use std::fmt;

use serde::Serialize;

use crate::{Context, Os};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PackageManager {
    Apt,
    Dnf,
    Pacman,
    Zypper,
    Nix,
    RpmOstree,
    Flatpak,
    Apk,
    Xbps,
    Emerge,
    Eopkg,
    Snap,
    Homebrew,
    MacPorts,
    Winget,
    Scoop,
    Chocolatey,
}

impl PackageManager {
    /// The program whose presence identifies the package manager.
    pub fn program(self) -> &'static str {
        match self {
            PackageManager::Apt => "apt-get",
            PackageManager::Dnf => "dnf",
            PackageManager::Pacman => "pacman",
            PackageManager::Zypper => "zypper",
            PackageManager::Nix => "nix",
            PackageManager::RpmOstree => "rpm-ostree",
            PackageManager::Flatpak => "flatpak",
            PackageManager::Apk => "apk",
            PackageManager::Xbps => "xbps-install",
            PackageManager::Emerge => "emerge",
            PackageManager::Eopkg => "eopkg",
            PackageManager::Snap => "snap",
            PackageManager::Homebrew => "brew",
            PackageManager::MacPorts => "port",
            PackageManager::Winget => "winget",
            PackageManager::Scoop => "scoop",
            PackageManager::Chocolatey => "choco",
        }
    }

    fn candidates(os: Os) -> &'static [PackageManager] {
        match os {
            Os::Linux => &[
                PackageManager::Apt,
                PackageManager::Dnf,
                PackageManager::Pacman,
                PackageManager::Zypper,
                PackageManager::Nix,
                PackageManager::RpmOstree,
                PackageManager::Flatpak,
                PackageManager::Apk,
                PackageManager::Xbps,
                PackageManager::Emerge,
                PackageManager::Eopkg,
                PackageManager::Snap,
                PackageManager::Homebrew,
            ],
            Os::MacOs => &[
                PackageManager::Homebrew,
                PackageManager::MacPorts,
                PackageManager::Nix,
            ],
            Os::Windows => &[
                PackageManager::Winget,
                PackageManager::Scoop,
                PackageManager::Chocolatey,
            ],
            Os::Other => &[],
        }
    }
}

impl fmt::Display for PackageManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PackageManager::Apt => "APT",
            PackageManager::Dnf => "DNF",
            PackageManager::Pacman => "pacman",
            PackageManager::Zypper => "Zypper",
            PackageManager::Nix => "Nix",
            PackageManager::RpmOstree => "rpm-ostree",
            PackageManager::Flatpak => "Flatpak",
            PackageManager::Apk => "apk",
            PackageManager::Xbps => "XBPS",
            PackageManager::Emerge => "Portage",
            PackageManager::Eopkg => "eopkg",
            PackageManager::Snap => "Snap",
            PackageManager::Homebrew => "Homebrew",
            PackageManager::MacPorts => "MacPorts",
            PackageManager::Winget => "WinGet",
            PackageManager::Scoop => "Scoop",
            PackageManager::Chocolatey => "Chocolatey",
        })
    }
}

pub(crate) fn detect(context: &Context, os: Os) -> Vec<PackageManager> {
    PackageManager::candidates(os)
        .iter()
        .copied()
        .filter(|manager| context.which(manager.program()).is_some())
        .collect()
}
