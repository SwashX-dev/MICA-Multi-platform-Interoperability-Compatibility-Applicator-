//! Linux distribution identity from `os-release(5)`.

use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;

/// Distribution families that share a package manager and packaging
/// conventions. Derivatives are classified through `ID_LIKE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Family {
    Debian,
    Fedora,
    Arch,
    Suse,
    Nix,
    Gentoo,
    Void,
    Alpine,
    Solus,
    Other,
}

impl fmt::Display for Family {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Family::Debian => "Debian/Ubuntu",
            Family::Fedora => "Fedora/RHEL",
            Family::Arch => "Arch",
            Family::Suse => "openSUSE/SUSE",
            Family::Nix => "NixOS",
            Family::Gentoo => "Gentoo",
            Family::Void => "Void",
            Family::Alpine => "Alpine",
            Family::Solus => "Solus",
            Family::Other => "other",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Distro {
    pub id: String,
    pub id_like: Vec<String>,
    pub name: String,
    pub pretty_name: String,
    pub version_id: Option<String>,
    pub variant_id: Option<String>,
    pub family: Family,
}

/// Parse `os-release` text. Returns `None` without an `ID`.
pub fn parse(text: &str) -> Option<Distro> {
    let mut fields = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        fields.insert(key.trim().to_string(), unquote(value.trim()));
    }
    let id = fields.get("ID")?.to_ascii_lowercase();
    let id_like: Vec<String> = fields
        .get("ID_LIKE")
        .map(|like| {
            like.split_whitespace()
                .map(str::to_ascii_lowercase)
                .collect()
        })
        .unwrap_or_default();
    let name = fields.get("NAME").cloned().unwrap_or_else(|| id.clone());
    let version_id = fields.get("VERSION_ID").cloned();
    let pretty_name = fields.get("PRETTY_NAME").cloned().unwrap_or_else(|| {
        format!("{name} {}", version_id.clone().unwrap_or_default())
            .trim()
            .to_string()
    });
    let family = family(&id, &id_like);
    Some(Distro {
        id,
        id_like,
        name,
        pretty_name,
        version_id,
        variant_id: fields.get("VARIANT_ID").cloned(),
        family,
    })
}

fn family(id: &str, id_like: &[String]) -> Family {
    let classify = |id: &str| match id {
        "debian" | "ubuntu" | "linuxmint" | "pop" | "elementary" | "zorin" | "kali"
        | "raspbian" | "neon" | "pika" => Some(Family::Debian),
        "fedora" | "rhel" | "centos" | "rocky" | "almalinux" | "nobara" | "ultramarine"
        | "bazzite" | "bluefin" | "aurora" => Some(Family::Fedora),
        "arch" | "manjaro" | "endeavouros" | "cachyos" | "garuda" | "steamos" | "artix" => {
            Some(Family::Arch)
        }
        "opensuse" | "opensuse-tumbleweed" | "opensuse-leap" | "opensuse-microos"
        | "opensuse-aeon" | "opensuse-kalpa" | "opensuse-slowroll" | "suse" | "sles"
        | "sled" => Some(Family::Suse),
        "nixos" => Some(Family::Nix),
        "gentoo" => Some(Family::Gentoo),
        "void" => Some(Family::Void),
        "alpine" => Some(Family::Alpine),
        "solus" => Some(Family::Solus),
        _ => None,
    };
    classify(id)
        .or_else(|| id_like.iter().find_map(|like| classify(like)))
        .unwrap_or(Family::Other)
}

fn unquote(value: &str) -> String {
    let inner = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|rest| rest.strip_suffix('\'')))
        .unwrap_or(value);
    inner.replace("\\\"", "\"").replace("\\\\", "\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_os_release_files() {
        let fedora = parse(
            "NAME=\"Fedora Linux\"\nVERSION=\"43 (Workstation Edition)\"\nID=fedora\n\
             VERSION_ID=43\nPRETTY_NAME=\"Fedora Linux 43 (Workstation Edition)\"\n\
             VARIANT_ID=workstation\n",
        )
        .unwrap();
        assert_eq!(fedora.family, Family::Fedora);
        assert_eq!(fedora.version_id.as_deref(), Some("43"));
        assert_eq!(fedora.variant_id.as_deref(), Some("workstation"));

        let mint = parse("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\nNAME=\"Linux Mint\"\n").unwrap();
        assert_eq!(mint.family, Family::Debian);

        let derivative = parse("ID=mydistro\nID_LIKE='arch'\n").unwrap();
        assert_eq!(derivative.family, Family::Arch);
        assert_eq!(derivative.id_like, vec!["arch"]);

        let tumbleweed =
            parse("NAME=\"openSUSE Tumbleweed\"\nID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n")
                .unwrap();
        assert_eq!(tumbleweed.family, Family::Suse);

        let nixos = parse("ID=nixos\nVERSION_ID=\"26.05\"\n").unwrap();
        assert_eq!(nixos.family, Family::Nix);
        assert_eq!(nixos.pretty_name, "nixos 26.05");

        assert_eq!(parse("NAME=nothing\n"), None);
        assert_eq!(parse("ID=unknownthing\n").unwrap().family, Family::Other);
    }
}
