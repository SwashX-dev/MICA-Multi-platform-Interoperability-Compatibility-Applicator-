//! The TPM measured-boot event log (`binary_bios_measurements`).
//!
//! Firmware and boot loaders append one event per thing they measure into a
//! TPM PCR. MICA reads the log to answer two of Epic's EAC Linux v2 questions
//! without trusting anything else on the system: *is boot measured at all*,
//! and *which certificates verified this boot chain* (the
//! `EV_EFI_VARIABLE_AUTHORITY` events Secure Boot records in PCR 7, and shim
//! records for its own and MOK keys).
//!
//! MICA does not replay the log against the TPM's PCR values or produce any
//! attestation: the log is a readiness diagnostic. Attestation is done by the
//! anti-cheat vendor's own service, with the TPM, and MICA never imitates it.
//!
//! Format: TCG PC Client Platform Firmware Profile Specification, section
//! 10 ("Event Logging"): a SHA-1-format `Spec ID Event03` header event, then
//! crypto-agile `TCG_PCR_EVENT2` records.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::x509::{self, Certificate};

pub const EV_NO_ACTION: u32 = 0x3;
pub const EV_SEPARATOR: u32 = 0x4;
pub const EV_IPL: u32 = 0xD;
pub const EV_EFI_VARIABLE_DRIVER_CONFIG: u32 = 0x8000_0001;
pub const EV_EFI_BOOT_SERVICES_APPLICATION: u32 = 0x8000_0003;
pub const EV_EFI_VARIABLE_AUTHORITY: u32 = 0x8000_00E0;

const MAX_EVENTS: usize = 100_000;
const MAX_EVENT_SIZE: usize = 16 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub pcr: u32,
    pub kind: u32,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventLog {
    /// `true` for a crypto-agile (TPM 2.0) log, `false` for SHA-1 only.
    pub crypto_agile: bool,
    pub events: Vec<Event>,
    /// Bytes at the end that did not form a whole event.
    pub trailing_garbage: bool,
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// Parse a binary event log. `None` if even the first event is unreadable.
pub fn parse(data: &[u8]) -> Option<EventLog> {
    // The first event always uses the SHA-1 layout: pcr, type, 20-byte
    // digest, size, data.
    let first_size = u32_at(data, 28)? as usize;
    let first_data = data.get(32..32usize.checked_add(first_size)?)?;
    let mut events = vec![Event {
        pcr: u32_at(data, 0)?,
        kind: u32_at(data, 4)?,
        data: first_data.to_vec(),
    }];
    let mut at = 32 + first_size;

    let digest_sizes = spec_id_digest_sizes(first_data);
    let crypto_agile = digest_sizes.is_some();
    let mut trailing_garbage = false;
    while at < data.len() && events.len() < MAX_EVENTS {
        let parsed = match &digest_sizes {
            Some(sizes) => agile_event(data, at, sizes),
            None => sha1_event(data, at),
        };
        match parsed {
            Some((event, next)) => {
                events.push(event);
                at = next;
            }
            None => {
                // Firmware pads the log's buffer with 0xFF or zeros.
                trailing_garbage = data[at..].iter().any(|&byte| byte != 0 && byte != 0xFF);
                break;
            }
        }
    }
    Some(EventLog {
        crypto_agile,
        events,
        trailing_garbage,
    })
}

/// Digest sizes by algorithm from a `Spec ID Event03` header, if it is one.
fn spec_id_digest_sizes(data: &[u8]) -> Option<BTreeMap<u16, usize>> {
    if !data.starts_with(b"Spec ID Event03\0") {
        return None;
    }
    // signature(16) platformClass(4) minor(1) major(1) errata(1) uintnSize(1)
    let count = u32_at(data, 24)? as usize;
    let mut sizes = BTreeMap::new();
    for index in 0..count.min(32) {
        let at = 28 + index * 4;
        sizes.insert(u16_at(data, at)?, u16_at(data, at + 2)? as usize);
    }
    Some(sizes)
}

fn sha1_event(data: &[u8], at: usize) -> Option<(Event, usize)> {
    let size = u32_at(data, at + 28)? as usize;
    if size > MAX_EVENT_SIZE {
        return None;
    }
    let start = at + 32;
    let body = data.get(start..start.checked_add(size)?)?;
    Some((
        Event {
            pcr: u32_at(data, at)?,
            kind: u32_at(data, at + 4)?,
            data: body.to_vec(),
        },
        start + size,
    ))
}

fn agile_event(data: &[u8], at: usize, sizes: &BTreeMap<u16, usize>) -> Option<(Event, usize)> {
    let pcr = u32_at(data, at)?;
    let kind = u32_at(data, at + 4)?;
    let count = u32_at(data, at + 8)? as usize;
    if count > 32 || pcr > 64 {
        return None;
    }
    let mut cursor = at + 12;
    for _ in 0..count {
        let algorithm = u16_at(data, cursor)?;
        cursor += 2 + sizes.get(&algorithm)?;
    }
    let size = u32_at(data, cursor)? as usize;
    if size > MAX_EVENT_SIZE {
        return None;
    }
    let start = cursor + 4;
    let body = data.get(start..start.checked_add(size)?)?;
    Some((
        Event {
            pcr,
            kind,
            data: body.to_vec(),
        },
        start + size,
    ))
}

/// A `UEFI_VARIABLE_DATA` structure: which variable, and its contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariableData {
    pub name: String,
    pub data: Vec<u8>,
}

pub fn variable_data(event: &[u8]) -> Option<VariableData> {
    // VariableName GUID(16), UnicodeNameLength u64, VariableDataLength u64.
    let name_length = usize::try_from(u64::from_le_bytes(event.get(16..24)?.try_into().ok()?)).ok()?;
    let data_length = usize::try_from(u64::from_le_bytes(event.get(24..32)?.try_into().ok()?)).ok()?;
    let name_end = 32usize.checked_add(name_length.checked_mul(2)?)?;
    let units: Vec<u16> = event
        .get(32..name_end)?
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let data = event.get(name_end..name_end.checked_add(data_length)?)?;
    Some(VariableData {
        name: String::from_utf16_lossy(&units),
        data: data.to_vec(),
    })
}

/// A certificate that verified part of this boot, according to the log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Authority {
    pub pcr: u32,
    /// The variable the certificate came from: `db` for firmware Secure
    /// Boot, `Shim` for the key built into shim, `MokList...` for a MOK.
    pub source: String,
    pub certificate: Option<Certificate>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub crypto_agile: bool,
    pub events: usize,
    /// Number of events recorded into each PCR.
    pub events_per_pcr: BTreeMap<u32, usize>,
    /// EFI applications measured into PCR 4 (shim, boot loader, kernel).
    pub boot_applications: usize,
    /// Secure Boot configuration recorded in PCR 7 (SecureBoot, PK, KEK, db, dbx).
    pub secure_boot_config: Vec<String>,
    pub authorities: Vec<Authority>,
    /// Kernel command lines GRUB recorded (`kernel_cmdline: ...`, PCR 8).
    pub kernel_cmdlines: Vec<String>,
    pub trailing_garbage: bool,
}

impl EventLog {
    pub fn summary(&self) -> Summary {
        let mut summary = Summary {
            crypto_agile: self.crypto_agile,
            events: self.events.len(),
            trailing_garbage: self.trailing_garbage,
            ..Summary::default()
        };
        for event in &self.events {
            if event.kind != EV_NO_ACTION {
                *summary.events_per_pcr.entry(event.pcr).or_default() += 1;
            }
            match event.kind {
                EV_EFI_BOOT_SERVICES_APPLICATION if event.pcr == 4 => {
                    summary.boot_applications += 1;
                }
                EV_EFI_VARIABLE_DRIVER_CONFIG if event.pcr == 7 => {
                    if let Some(variable) = variable_data(&event.data) {
                        summary.secure_boot_config.push(variable.name);
                    }
                }
                EV_EFI_VARIABLE_AUTHORITY => {
                    if let Some(variable) = variable_data(&event.data) {
                        summary.authorities.push(Authority {
                            pcr: event.pcr,
                            source: variable.name,
                            certificate: authority_certificate(&variable.data),
                        });
                    }
                }
                EV_IPL => {
                    let text = String::from_utf8_lossy(&event.data);
                    let text = text.trim_end_matches('\0');
                    if let Some(cmdline) = text.strip_prefix("kernel_cmdline: ") {
                        summary.kernel_cmdlines.push(cmdline.to_string());
                    }
                }
                _ => {}
            }
        }
        summary
    }
}

/// Firmware records an `EFI_SIGNATURE_DATA` (16-byte owner GUID, then the
/// certificate); shim records the bare certificate. Accept both.
fn authority_certificate(data: &[u8]) -> Option<Certificate> {
    if data.first() == Some(&0x30) {
        if let Some(certificate) = x509::parse(data) {
            return Some(certificate);
        }
    }
    x509::parse(data.get(16..)?)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::x509::tests::certificate;

    pub(crate) struct LogBuilder {
        data: Vec<u8>,
    }

    impl LogBuilder {
        /// A crypto-agile log with SHA-256 (0x000B, 32 bytes) only.
        pub(crate) fn new() -> LogBuilder {
            let mut spec = b"Spec ID Event03\0".to_vec();
            spec.extend_from_slice(&0u32.to_le_bytes());
            spec.extend_from_slice(&[0, 2, 0, 2]);
            spec.extend_from_slice(&1u32.to_le_bytes());
            spec.extend_from_slice(&0x000Bu16.to_le_bytes());
            spec.extend_from_slice(&32u16.to_le_bytes());
            spec.push(0);
            let mut data = Vec::new();
            data.extend_from_slice(&0u32.to_le_bytes());
            data.extend_from_slice(&EV_NO_ACTION.to_le_bytes());
            data.extend_from_slice(&[0; 20]);
            data.extend_from_slice(&(spec.len() as u32).to_le_bytes());
            data.extend_from_slice(&spec);
            LogBuilder { data }
        }

        pub(crate) fn event(mut self, pcr: u32, kind: u32, body: &[u8]) -> LogBuilder {
            self.data.extend_from_slice(&pcr.to_le_bytes());
            self.data.extend_from_slice(&kind.to_le_bytes());
            self.data.extend_from_slice(&1u32.to_le_bytes());
            self.data.extend_from_slice(&0x000Bu16.to_le_bytes());
            self.data.extend_from_slice(&[0x5A; 32]);
            self.data.extend_from_slice(&(body.len() as u32).to_le_bytes());
            self.data.extend_from_slice(body);
            self
        }

        pub(crate) fn variable(self, pcr: u32, kind: u32, name: &str, value: &[u8]) -> LogBuilder {
            let units: Vec<u8> = name.encode_utf16().flat_map(|unit| unit.to_le_bytes()).collect();
            let mut body = vec![0x11; 16];
            body.extend_from_slice(&(name.encode_utf16().count() as u64).to_le_bytes());
            body.extend_from_slice(&(value.len() as u64).to_le_bytes());
            body.extend_from_slice(&units);
            body.extend_from_slice(value);
            self.event(pcr, kind, &body)
        }

        pub(crate) fn build(self) -> Vec<u8> {
            self.data
        }
    }

    /// The log of a typical shim → GRUB → Fedora kernel Secure Boot.
    pub(crate) fn fedora_boot() -> Vec<u8> {
        let microsoft = certificate(
            ("Microsoft Corporation UEFI CA 2011", "Microsoft Corporation"),
            ("Microsoft Corporation Third Party Marketplace Root", "Microsoft Corporation"),
        );
        let mut signature_data = vec![0x22; 16];
        signature_data.extend_from_slice(&microsoft);
        let fedora = certificate(
            ("Fedora Secure Boot CA", "Fedora Project"),
            ("Fedora Secure Boot CA", "Fedora Project"),
        );
        LogBuilder::new()
            .variable(7, EV_EFI_VARIABLE_DRIVER_CONFIG, "SecureBoot", &[1])
            .variable(7, EV_EFI_VARIABLE_DRIVER_CONFIG, "PK", &[1, 2])
            .variable(7, EV_EFI_VARIABLE_DRIVER_CONFIG, "db", &[3])
            .event(7, EV_SEPARATOR, &[0; 4])
            .variable(7, EV_EFI_VARIABLE_AUTHORITY, "db", &signature_data)
            .event(4, EV_EFI_BOOT_SERVICES_APPLICATION, &[0; 40])
            .variable(7, EV_EFI_VARIABLE_AUTHORITY, "Shim", &fedora)
            .event(4, EV_EFI_BOOT_SERVICES_APPLICATION, &[0; 40])
            .event(8, EV_IPL, b"kernel_cmdline: /vmlinuz-6.17.4 root=UUID=1 ro lockdown=integrity\0")
            .event(4, EV_EFI_BOOT_SERVICES_APPLICATION, &[0; 40])
            .build()
    }

    #[test]
    fn summarises_a_secure_boot_log() {
        let log = parse(&fedora_boot()).unwrap();
        assert!(log.crypto_agile);
        let summary = log.summary();
        assert_eq!(summary.boot_applications, 3);
        assert_eq!(summary.secure_boot_config, vec!["SecureBoot", "PK", "db"]);
        let names: Vec<(String, String)> = summary
            .authorities
            .iter()
            .map(|authority| {
                (
                    authority.source.clone(),
                    authority.certificate.as_ref().unwrap().subject.display(),
                )
            })
            .collect();
        assert_eq!(
            names,
            vec![
                (
                    "db".to_string(),
                    "Microsoft Corporation UEFI CA 2011, Microsoft Corporation".to_string()
                ),
                (
                    "Shim".to_string(),
                    "Fedora Secure Boot CA, Fedora Project".to_string()
                ),
            ]
        );
        assert_eq!(
            summary.kernel_cmdlines,
            vec!["/vmlinuz-6.17.4 root=UUID=1 ro lockdown=integrity"]
        );
        assert_eq!(summary.events_per_pcr[&4], 3);
        assert!(!summary.trailing_garbage);
    }

    #[test]
    fn padding_garbage_and_truncation() {
        let mut log = fedora_boot();
        log.extend_from_slice(&[0xFF; 64]);
        assert!(!parse(&log).unwrap().trailing_garbage);
        log.extend_from_slice(&[1, 2, 3]);
        assert!(parse(&log).unwrap().trailing_garbage);
        for cut in 0..log.len() {
            if let Some(parsed) = parse(&log[..cut]) {
                let _ = parsed.summary();
            }
        }
        assert!(parse(&[]).is_none());
    }

    #[test]
    fn sha1_only_logs() {
        let mut data = Vec::new();
        for (pcr, kind, body) in [(0u32, 8u32, b"version".as_slice()), (4, EV_EFI_BOOT_SERVICES_APPLICATION, &[0u8; 8])] {
            data.extend_from_slice(&pcr.to_le_bytes());
            data.extend_from_slice(&kind.to_le_bytes());
            data.extend_from_slice(&[0; 20]);
            data.extend_from_slice(&(body.len() as u32).to_le_bytes());
            data.extend_from_slice(body);
        }
        let log = parse(&data).unwrap();
        assert!(!log.crypto_agile);
        assert_eq!(log.summary().boot_applications, 1);
    }
}
