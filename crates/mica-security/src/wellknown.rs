//! The signing authorities Epic calls "well-known" for EAC Linux v2.
//!
//! Source: Epic Online Services, "Awesome Gaming on Linux Without Cheating"
//! (2026-10-14), FAQ "What keys are considered well-known?":
//! <https://onlineservices.epicgames.com/news/introducing-easy-anti-cheat-linux-v2>.
//! Epic maintains the authoritative list, plans to open-source it, and
//! expects it to grow. MICA matches certificate *names* against Epic's
//! published names; Epic's service matches the actual keys. A name match is
//! therefore a strong hint, not proof, and MICA says so in its output.

use serde::Serialize;

use crate::x509::Certificate;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct WellKnownKey {
    /// The name as Epic published it.
    pub name: &'static str,
    pub used_by: &'static str,
    /// Case-insensitive substrings of a certificate's common name.
    #[serde(skip)]
    matches: &'static [&'static str],
}

pub const EAC_LINUX_V2_LIST_DATE: &str = "2026-10-14";

pub const EAC_LINUX_V2: &[WellKnownKey] = &[
    WellKnownKey {
        name: "Canonical Ltd. Master Certificate Authority",
        used_by: "Ubuntu and derivatives such as Linux Mint",
        matches: &["canonical ltd. master certificate authority"],
    },
    WellKnownKey {
        name: "Fedora Secure Boot CA",
        used_by: "Fedora and derivatives such as Silverblue",
        matches: &["fedora secure boot ca"],
    },
    WellKnownKey {
        name: "Universal Blue",
        used_by: "Bazzite, Bluefin, Aurora, and derivatives",
        matches: &["universal blue", "ublue"],
    },
    WellKnownKey {
        name: "Debian Secure Boot CA",
        used_by: "Debian and derivatives",
        matches: &["debian secure boot ca"],
    },
    WellKnownKey {
        name: "openSUSE Secure Boot CA",
        used_by: "openSUSE and derivatives",
        matches: &["opensuse secure boot ca"],
    },
    WellKnownKey {
        name: "SUSE Linux Enterprise Secure Boot CA",
        used_by: "SUSE Linux Enterprise",
        matches: &["suse linux enterprise secure boot ca"],
    },
    WellKnownKey {
        name: "Rocky Linux Secure Boot Root CA",
        used_by: "Rocky Linux",
        matches: &[
            "rocky linux secure boot root ca",
            "rocky enterprise software foundation secure boot",
        ],
    },
    WellKnownKey {
        name: "Microsoft UEFI CA",
        used_by: "signs non-Microsoft components such as the shim boot loader",
        matches: &["microsoft corporation uefi ca", "microsoft uefi ca"],
    },
    WellKnownKey {
        name: "AlmaLinux Secure Boot CA",
        used_by: "AlmaLinux",
        matches: &["almalinux secure boot ca", "almalinux os foundation"],
    },
];

/// How a certificate relates to the well-known list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "match", rename_all = "kebab-case")]
pub enum KeyMatch {
    /// The certificate *is* a listed authority (by name).
    Listed {
        key: WellKnownKey,
    },
    /// The certificate was issued by a listed authority (by name).
    IssuedByListed {
        key: WellKnownKey,
    },
    NotListed,
}

impl KeyMatch {
    pub fn is_well_known(self) -> bool {
        !matches!(self, KeyMatch::NotListed)
    }
}

fn find(name: Option<&str>) -> Option<WellKnownKey> {
    let name = name?.to_ascii_lowercase();
    EAC_LINUX_V2
        .iter()
        .find(|key| key.matches.iter().any(|needle| name.contains(needle)))
        .copied()
}

pub fn classify(certificate: &Certificate) -> KeyMatch {
    if let Some(key) = find(certificate.subject.common_name.as_deref())
        .or_else(|| find(certificate.subject.organization.as_deref()))
    {
        return KeyMatch::Listed { key };
    }
    match find(certificate.issuer.common_name.as_deref()) {
        Some(key) => KeyMatch::IssuedByListed { key },
        None => KeyMatch::NotListed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::x509::{self, tests::certificate};

    fn cert(subject: &str, issuer: &str) -> Certificate {
        x509::parse(&certificate((subject, "Org"), (issuer, "Org"))).unwrap()
    }

    #[test]
    fn classifies_distribution_and_custom_keys() {
        assert!(matches!(
            classify(&cert("Fedora Secure Boot CA", "Fedora Secure Boot CA")),
            KeyMatch::Listed { key } if key.name == "Fedora Secure Boot CA"
        ));
        assert!(matches!(
            classify(&cert("Microsoft Corporation UEFI CA 2011", "Microsoft Root")),
            KeyMatch::Listed { key } if key.name == "Microsoft UEFI CA"
        ));
        assert!(matches!(
            classify(&cert("Microsoft UEFI CA 2023", "Microsoft Root")),
            KeyMatch::Listed { .. }
        ));
        assert!(matches!(
            classify(&cert(
                "Canonical Ltd. Secure Boot Signing (2022 v1)",
                "Canonical Ltd. Master Certificate Authority"
            )),
            KeyMatch::IssuedByListed { .. }
        ));
        // A DKMS machine-owner key and the Windows boot manager's CA are not
        // on the list.
        assert_eq!(
            classify(&cert(
                "ubuntu Secure Boot Module Signature key",
                "ubuntu Secure Boot Module Signature key"
            )),
            KeyMatch::NotListed
        );
        assert_eq!(
            classify(&cert(
                "Microsoft Windows Production PCA 2011",
                "Microsoft Root Certificate Authority 2010"
            )),
            KeyMatch::NotListed
        );
    }

    #[test]
    fn the_list_matches_epics_announcement() {
        assert_eq!(EAC_LINUX_V2.len(), 9);
        assert!(EAC_LINUX_V2.iter().all(|key| !key.matches.is_empty()));
    }
}
