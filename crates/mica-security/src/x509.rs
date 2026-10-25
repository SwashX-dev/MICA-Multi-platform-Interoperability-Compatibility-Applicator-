//! Just enough DER to name an X.509 certificate's subject and issuer.
//!
//! Certificates come from UEFI variables and the TPM event log, which an
//! attacker with firmware access could shape, so the reader is defensive:
//! every length is checked, nesting is bounded, and anything unexpected
//! yields `None`. Nothing here verifies signatures; MICA reports names, and
//! the anti-cheat vendor's own service performs the cryptographic checks.

use serde::Serialize;
use sha2::{Digest, Sha256};

/// One TLV element: its tag and the bytes of its value.
struct Element<'a> {
    tag: u8,
    value: &'a [u8],
}

/// Read the element at the start of `data`; return it and the rest.
fn element(data: &[u8]) -> Option<(Element<'_>, &[u8])> {
    let tag = *data.first()?;
    let first = *data.get(1)?;
    let (length, header) = if first < 0x80 {
        (first as usize, 2)
    } else {
        let count = (first & 0x7F) as usize;
        // DER forbids the indefinite form (count 0); four bytes is plenty.
        if count == 0 || count > 4 {
            return None;
        }
        let bytes = data.get(2..2 + count)?;
        let length = bytes
            .iter()
            .fold(0usize, |acc, &byte| (acc << 8) | byte as usize);
        (length, 2 + count)
    };
    let value = data.get(header..header.checked_add(length)?)?;
    Some((Element { tag, value }, &data[header + length..]))
}

/// The elements directly inside a constructed value, in order.
fn children(value: &[u8]) -> Option<Vec<Element<'_>>> {
    let mut rest = value;
    let mut out = Vec::new();
    while !rest.is_empty() {
        if out.len() > 64 {
            return None;
        }
        let (child, next) = element(rest)?;
        out.push(child);
        rest = next;
    }
    Some(out)
}

const SEQUENCE: u8 = 0x30;
const SET: u8 = 0x31;
const OID: u8 = 0x06;
const OID_COMMON_NAME: &[u8] = &[0x55, 0x04, 0x03];
const OID_ORGANIZATION: &[u8] = &[0x55, 0x04, 0x0A];
const OID_ORGANIZATIONAL_UNIT: &[u8] = &[0x55, 0x04, 0x0B];

/// The parts of a distinguished name MICA shows and matches.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Name {
    pub common_name: Option<String>,
    pub organization: Option<String>,
    pub organizational_unit: Option<String>,
}

impl Name {
    /// A one-line rendering: `CN, O` or whichever parts exist.
    pub fn display(&self) -> String {
        let parts: Vec<&str> = [&self.common_name, &self.organization]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect();
        if parts.is_empty() {
            "(unnamed)".into()
        } else {
            parts.join(", ")
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Certificate {
    pub subject: Name,
    pub issuer: Name,
    pub not_after: Option<String>,
    /// SHA-256 of the whole DER certificate, lowercase hex.
    pub sha256: String,
}

fn string_value(element: &Element) -> Option<String> {
    match element.tag {
        // UTF8String, PrintableString, IA5String, T61String, VisibleString
        0x0C | 0x13 | 0x16 | 0x14 | 0x1A => {
            Some(String::from_utf8_lossy(element.value).into_owned())
        }
        // BMPString: UTF-16BE
        0x1E => {
            let units: Vec<u16> = element
                .value
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            Some(String::from_utf16_lossy(&units))
        }
        _ => None,
    }
}

fn name(value: &[u8]) -> Option<Name> {
    let mut name = Name::default();
    for rdn in children(value)? {
        if rdn.tag != SET {
            return None;
        }
        for attribute in children(rdn.value)? {
            if attribute.tag != SEQUENCE {
                return None;
            }
            let parts = children(attribute.value)?;
            let (Some(oid), Some(value)) = (parts.first(), parts.get(1)) else {
                return None;
            };
            if oid.tag != OID {
                return None;
            }
            let text = string_value(value);
            match oid.value {
                OID_COMMON_NAME => name.common_name = text,
                OID_ORGANIZATION => name.organization = text,
                OID_ORGANIZATIONAL_UNIT => name.organizational_unit = text,
                _ => {}
            }
        }
    }
    Some(name)
}

/// Parse a DER X.509 certificate's names. Trailing bytes after the
/// certificate are ignored (EFI signature entries may be padded).
pub fn parse(der: &[u8]) -> Option<Certificate> {
    let (certificate, _) = element(der)?;
    if certificate.tag != SEQUENCE {
        return None;
    }
    let length = der.len() - element(der)?.1.len();
    let parts = children(certificate.value)?;
    let tbs = parts.first().filter(|tbs| tbs.tag == SEQUENCE)?;
    let fields = children(tbs.value)?;
    // [0] version is optional; skip it when present.
    let offset = usize::from(fields.first()?.tag == 0xA0);
    let issuer = fields
        .get(offset + 2)
        .filter(|field| field.tag == SEQUENCE)?;
    let validity = fields
        .get(offset + 3)
        .filter(|field| field.tag == SEQUENCE)?;
    let subject = fields
        .get(offset + 4)
        .filter(|field| field.tag == SEQUENCE)?;
    let not_after = children(validity.value).and_then(|times| {
        times
            .get(1)
            .map(|time| String::from_utf8_lossy(time.value).into_owned())
    });
    Some(Certificate {
        subject: name(subject.value)?,
        issuer: name(issuer.value)?,
        not_after,
        sha256: Sha256::digest(&der[..length])
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn tlv(tag: u8, value: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        if value.len() < 0x80 {
            out.push(value.len() as u8);
        } else if value.len() < 0x100 {
            out.extend_from_slice(&[0x81, value.len() as u8]);
        } else {
            out.extend_from_slice(&[0x82, (value.len() >> 8) as u8, value.len() as u8]);
        }
        out.extend_from_slice(value);
        out
    }

    fn name_der(common_name: &str, organization: &str) -> Vec<u8> {
        let attribute = |oid: &[u8], text: &str| {
            tlv(
                SET,
                &tlv(
                    SEQUENCE,
                    &[tlv(OID, oid), tlv(0x0C, text.as_bytes())].concat(),
                ),
            )
        };
        tlv(
            SEQUENCE,
            &[
                attribute(OID_ORGANIZATION, organization),
                attribute(OID_COMMON_NAME, common_name),
            ]
            .concat(),
        )
    }

    /// A structurally valid certificate with a fake key and signature.
    pub(crate) fn certificate(subject: (&str, &str), issuer: (&str, &str)) -> Vec<u8> {
        let algorithm = tlv(
            SEQUENCE,
            &tlv(OID, &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 1, 1, 11]),
        );
        let validity = tlv(
            SEQUENCE,
            &[tlv(0x17, b"120101000000Z"), tlv(0x17, b"420101000000Z")].concat(),
        );
        let tbs = tlv(
            SEQUENCE,
            &[
                tlv(0xA0, &tlv(0x02, &[2])),
                tlv(0x02, &[0x01, 0x23]),
                algorithm.clone(),
                name_der(issuer.0, issuer.1),
                validity,
                name_der(subject.0, subject.1),
                tlv(
                    SEQUENCE,
                    &[algorithm.clone(), tlv(0x03, &[0, 1, 2, 3])].concat(),
                ),
            ]
            .concat(),
        );
        tlv(SEQUENCE, &[tbs, algorithm, tlv(0x03, &[0, 9, 9])].concat())
    }

    #[test]
    fn reads_subject_and_issuer() {
        let der = certificate(
            ("Fedora Secure Boot CA", "Fedora Project"),
            ("Fedora Secure Boot CA", "Fedora Project"),
        );
        let mut padded = der.clone();
        padded.extend_from_slice(&[0, 0, 0]);
        let certificate = parse(&padded).unwrap();
        assert_eq!(
            certificate.subject.common_name.as_deref(),
            Some("Fedora Secure Boot CA")
        );
        assert_eq!(
            certificate.issuer.organization.as_deref(),
            Some("Fedora Project")
        );
        assert_eq!(certificate.not_after.as_deref(), Some("420101000000Z"));
        assert_eq!(
            certificate.sha256,
            parse(&der).unwrap().sha256,
            "padding is not hashed"
        );
        assert_eq!(
            certificate.subject.display(),
            "Fedora Secure Boot CA, Fedora Project"
        );
    }

    #[test]
    fn rejects_garbage_without_panicking() {
        assert!(parse(&[]).is_none());
        assert!(parse(&[0x30, 0x80]).is_none(), "indefinite length");
        assert!(parse(&[0x30, 0x85, 1, 2, 3, 4, 5]).is_none());
        assert!(parse(&[0x30, 0x05, 1, 2]).is_none());
        let der = certificate(("a", "b"), ("c", "d"));
        for cut in 0..der.len() {
            let _ = parse(&der[..cut]);
        }
        for index in 0..der.len() {
            let mut broken = der.clone();
            broken[index] ^= 0xFF;
            let _ = parse(&broken);
        }
    }

    #[test]
    fn bmp_strings() {
        let value = Element {
            tag: 0x1E,
            value: &[0, b'H', 0, b'i'],
        };
        assert_eq!(string_value(&value).as_deref(), Some("Hi"));
    }
}
