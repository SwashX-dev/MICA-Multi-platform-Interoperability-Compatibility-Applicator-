//! UEFI signature databases (`db`, `MokListRT`): lists of `EFI_SIGNATURE_LIST`.
//!
//! Layout: UEFI Specification 2.10, section 32.4.1, "Signature Database".

use serde::Serialize;

use crate::x509::{self, Certificate};

/// EFI_CERT_X509_GUID (a5c059a1-94e4-4aa7-87b5-ab155c2bf072), as stored.
const CERT_X509: [u8; 16] = [
    0xA1, 0x59, 0xC0, 0xA5, 0xE4, 0x94, 0xA7, 0x4A, 0x87, 0xB5, 0xAB, 0x15, 0x5C, 0x2B, 0xF0, 0x72,
];
/// EFI_CERT_SHA256_GUID (c1c41626-504c-4092-aca9-41f936934328), as stored.
const CERT_SHA256: [u8; 16] = [
    0x26, 0x16, 0xC4, 0xC1, 0x4C, 0x50, 0x92, 0x40, 0xAC, 0xA9, 0x41, 0xF9, 0x36, 0x93, 0x43, 0x28,
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Entry {
    Certificate(Certificate),
    /// An X.509 entry whose DER MICA could not read.
    UnreadableCertificate,
    /// A trusted (in `db`) or revoked image hash.
    Sha256,
    Other,
}

/// Every entry in a sequence of signature lists. Malformed lists end the
/// walk; what was read before them is kept.
pub fn parse_signature_lists(data: &[u8]) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut rest = data;
    let read = |bytes: &[u8], at: usize| -> Option<usize> {
        Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize)
    };
    for _ in 0..1024 {
        if rest.len() < 28 {
            break;
        }
        let kind: [u8; 16] = rest[..16].try_into().expect("16 bytes");
        let (Some(list_size), Some(header_size), Some(entry_size)) =
            (read(rest, 16), read(rest, 20), read(rest, 24))
        else {
            break;
        };
        if list_size < 28 || list_size > rest.len() || entry_size <= 16 {
            break;
        }
        let Some(body) = 28usize
            .checked_add(header_size)
            .and_then(|start| rest.get(start..list_size))
        else {
            break;
        };
        for signature in body.chunks_exact(entry_size) {
            // Each EFI_SIGNATURE_DATA: 16-byte owner GUID, then the data.
            let data = &signature[16..];
            entries.push(match kind {
                CERT_X509 => x509::parse(data)
                    .map(Entry::Certificate)
                    .unwrap_or(Entry::UnreadableCertificate),
                CERT_SHA256 => Entry::Sha256,
                _ => Entry::Other,
            });
        }
        rest = &rest[list_size..];
    }
    entries
}

/// The certificates in a signature database.
pub fn certificates(data: &[u8]) -> Vec<Certificate> {
    parse_signature_lists(data)
        .into_iter()
        .filter_map(|entry| match entry {
            Entry::Certificate(certificate) => Some(certificate),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::x509::tests::certificate;

    /// One EFI_SIGNATURE_LIST holding one X.509 certificate.
    pub(crate) fn x509_list(der: &[u8]) -> Vec<u8> {
        let entry_size = 16 + der.len();
        let mut list = CERT_X509.to_vec();
        list.extend_from_slice(&((28 + entry_size) as u32).to_le_bytes());
        list.extend_from_slice(&0u32.to_le_bytes());
        list.extend_from_slice(&(entry_size as u32).to_le_bytes());
        list.extend_from_slice(&[0x77; 16]);
        list.extend_from_slice(der);
        list
    }

    fn sha256_list(count: usize) -> Vec<u8> {
        let entry_size = 16 + 32;
        let mut list = CERT_SHA256.to_vec();
        list.extend_from_slice(&((28 + entry_size * count) as u32).to_le_bytes());
        list.extend_from_slice(&0u32.to_le_bytes());
        list.extend_from_slice(&(entry_size as u32).to_le_bytes());
        list.extend(std::iter::repeat_n(0xAB, entry_size * count));
        list
    }

    #[test]
    fn reads_certificates_and_hashes() {
        let mut db = x509_list(&certificate(
            ("Microsoft Corporation UEFI CA 2011", "Microsoft Corporation"),
            ("Microsoft Corporation Third Party Marketplace Root", "Microsoft Corporation"),
        ));
        db.extend(sha256_list(3));
        db.extend(x509_list(&certificate(
            ("My Own Key", "Me"),
            ("My Own Key", "Me"),
        )));
        let entries = parse_signature_lists(&db);
        assert_eq!(entries.len(), 5);
        assert_eq!(entries.iter().filter(|entry| **entry == Entry::Sha256).count(), 3);
        let names: Vec<String> = certificates(&db)
            .into_iter()
            .map(|certificate| certificate.subject.common_name.unwrap())
            .collect();
        assert_eq!(names, vec!["Microsoft Corporation UEFI CA 2011", "My Own Key"]);
    }

    #[test]
    fn stops_safely_on_malformed_lists() {
        let mut db = x509_list(&certificate(("a", "b"), ("a", "b")));
        db.extend_from_slice(&[0xFF; 40]);
        assert_eq!(certificates(&db).len(), 1);
        for cut in 0..db.len() {
            let _ = parse_signature_lists(&db[..cut]);
        }
        assert!(parse_signature_lists(&[]).is_empty());
    }
}
