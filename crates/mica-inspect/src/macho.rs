//! Mach-O executables and universal ("fat") binaries (macOS, iOS).
//!
//! Layout references: Apple's `<mach-o/loader.h>` and `<mach-o/fat.h>`.

use crate::bytes::{Endian, c_string, u32_be};
use crate::{Arch, Format, Kind, Linkage, MachODetails, Platform, Report, add_graphics};

const MH_MAGIC: u32 = 0xFEED_FACE;
const MH_MAGIC_64: u32 = 0xFEED_FACF;
const FAT_MAGIC: u32 = 0xCAFE_BABE;
const FAT_MAGIC_64: u32 = 0xCAFE_BABF;
const LC_LOAD_DYLIB: u32 = 0xC;
const LC_LOAD_WEAK_DYLIB: u32 = 0x8000_0018;
const LC_REEXPORT_DYLIB: u32 = 0x8000_001F;
const LC_LAZY_LOAD_DYLIB: u32 = 0x20;
const LC_LOAD_UPWARD_DYLIB: u32 = 0x8000_0023;
const LC_VERSION_MIN_MACOSX: u32 = 0x24;
const LC_VERSION_MIN_IPHONEOS: u32 = 0x25;
const LC_BUILD_VERSION: u32 = 0x32;
const MAX_COMMANDS: usize = 4096;
/// Java class files share 0xCAFEBABE; their "architecture count" is a class
/// file version, 45 or more. Real universal binaries hold a handful.
const MAX_FAT_ARCHES: u32 = 30;

pub(crate) fn is_macho(data: &[u8]) -> bool {
    let little = crate::bytes::u32_le(data, 0);
    let big = u32_be(data, 0);
    matches!(little, Some(MH_MAGIC | MH_MAGIC_64))
        || matches!(big, Some(MH_MAGIC | MH_MAGIC_64))
        || (matches!(big, Some(FAT_MAGIC | FAT_MAGIC_64))
            && u32_be(data, 4).is_some_and(|count| (1..=MAX_FAT_ARCHES).contains(&count)))
}

fn cpu(cputype: u32) -> Arch {
    match cputype {
        7 => Arch::X86,
        0x0100_0007 => Arch::X86_64,
        12 => Arch::Arm,
        0x0100_000C | 0x0200_000C => Arch::Aarch64,
        18 | 0x0100_0012 => Arch::PowerPc64,
        _ => Arch::Other,
    }
}

fn file_type(value: u32) -> &'static str {
    match value {
        1 => "object",
        2 => "executable",
        6 => "dynamic library",
        7 => "dynamic linker",
        8 => "bundle",
        _ => "other",
    }
}

struct Slice {
    details: MachODetails,
    libraries: Vec<String>,
    platform: Platform,
}

pub(crate) fn inspect(data: &[u8], report: &mut Report) {
    let magic = u32_be(data, 0).unwrap_or_default();
    if magic == FAT_MAGIC || magic == FAT_MAGIC_64 {
        let count = u32_be(data, 4).unwrap_or_default().min(MAX_FAT_ARCHES) as usize;
        let wide = magic == FAT_MAGIC_64;
        let entry = if wide { 32 } else { 20 };
        let mut slices = Vec::new();
        for index in 0..count {
            let at = 8 + index * entry;
            let offset = if wide {
                crate::bytes::u64_be(data, at + 8).and_then(|value| usize::try_from(value).ok())
            } else {
                u32_be(data, at + 8).map(|value| value as usize)
            };
            let cputype = u32_be(data, at);
            match (offset, cputype) {
                (Some(offset), Some(cputype)) => match data.get(offset..).and_then(parse_thin) {
                    Some(slice) => slices.push(slice),
                    None => slices.push(Slice {
                        details: MachODetails {
                            machine: cpu(cputype),
                            file_type: "unreadable".into(),
                            min_os: None,
                        },
                        libraries: Vec::new(),
                        platform: Platform::MacOs,
                    }),
                },
                _ => break,
            }
        }
        report.architectures = slices.iter().map(|slice| slice.details.machine).collect();
        if let Some(first) = slices.first() {
            report.platform = first.platform;
            report.kind = kind(&first.details.file_type);
            add_graphics(report, &first.libraries, Linkage::Imported);
            report.libraries = first.libraries.clone();
        }
        report.format = Format::MachOUniversal {
            slices: slices.into_iter().map(|slice| slice.details).collect(),
        };
        return;
    }
    match parse_thin(data) {
        Some(slice) => {
            report.architectures = vec![slice.details.machine];
            report.platform = slice.platform;
            report.kind = kind(&slice.details.file_type);
            add_graphics(report, &slice.libraries, Linkage::Imported);
            report.libraries = slice.libraries;
            report.format = Format::MachO(slice.details);
        }
        None => report.notes.push("truncated Mach-O header".into()),
    }
}

fn kind(file_type: &str) -> Kind {
    match file_type {
        "executable" => Kind::Application,
        "dynamic library" | "bundle" => Kind::Library,
        _ => Kind::Unknown,
    }
}

fn parse_thin(data: &[u8]) -> Option<Slice> {
    let (endian, wide) = match crate::bytes::u32_le(data, 0)? {
        MH_MAGIC => (Endian::Little, false),
        MH_MAGIC_64 => (Endian::Little, true),
        _ => match u32_be(data, 0)? {
            MH_MAGIC => (Endian::Big, false),
            MH_MAGIC_64 => (Endian::Big, true),
            _ => return None,
        },
    };
    let cputype = endian.u32(data, 4)?;
    let filetype = endian.u32(data, 12)?;
    let ncmds = endian.u32(data, 16)? as usize;
    let mut at = if wide { 32 } else { 28 };
    let mut libraries = Vec::new();
    let mut min_os = None;
    let mut platform = Platform::MacOs;
    for _ in 0..ncmds.min(MAX_COMMANDS) {
        let (Some(command), Some(size)) = (endian.u32(data, at), endian.u32(data, at + 4)) else {
            break;
        };
        if size < 8 {
            break;
        }
        match command {
            LC_LOAD_DYLIB | LC_LOAD_WEAK_DYLIB | LC_REEXPORT_DYLIB | LC_LAZY_LOAD_DYLIB
            | LC_LOAD_UPWARD_DYLIB => {
                let name = endian
                    .u32(data, at + 8)
                    .and_then(|offset| c_string(data, at + offset as usize, size as usize));
                if let Some(name) = name {
                    libraries.push(name);
                }
            }
            LC_BUILD_VERSION => {
                // platform, minos (xxxx.yy.zz), sdk, ...
                if let (Some(id), Some(version)) =
                    (endian.u32(data, at + 8), endian.u32(data, at + 12))
                {
                    platform = match id {
                        1 => Platform::MacOs,
                        6 => Platform::MacOs, // Mac Catalyst
                        _ => Platform::AppleMobile,
                    };
                    min_os = Some(version_string(version));
                }
            }
            LC_VERSION_MIN_MACOSX => min_os = endian.u32(data, at + 8).map(version_string),
            LC_VERSION_MIN_IPHONEOS => {
                platform = Platform::AppleMobile;
                min_os = endian.u32(data, at + 8).map(version_string);
            }
            _ => {}
        }
        at = at.checked_add(size as usize)?;
    }
    Some(Slice {
        details: MachODetails {
            machine: cpu(cputype),
            file_type: file_type(filetype).into(),
            min_os,
        },
        libraries,
        platform,
    })
}

fn version_string(packed: u32) -> String {
    format!(
        "{}.{}.{}",
        packed >> 16,
        (packed >> 8) & 0xFF,
        packed & 0xFF
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{GraphicsApi, inspect_bytes};

    /// A little-endian 64-bit Mach-O with LC_BUILD_VERSION and dylib loads.
    pub(crate) fn build(cputype: u32, dylibs: &[&str]) -> Vec<u8> {
        let mut commands = Vec::new();
        // LC_BUILD_VERSION: platform macOS, minos 13.0.0, sdk, ntools 0.
        commands.extend_from_slice(&LC_BUILD_VERSION.to_le_bytes());
        commands.extend_from_slice(&24u32.to_le_bytes());
        commands.extend_from_slice(&1u32.to_le_bytes());
        commands.extend_from_slice(&(13u32 << 16).to_le_bytes());
        commands.extend_from_slice(&(14u32 << 16).to_le_bytes());
        commands.extend_from_slice(&0u32.to_le_bytes());
        for name in dylibs {
            let size = (24 + name.len() + 1).div_ceil(8) * 8;
            commands.extend_from_slice(&LC_LOAD_DYLIB.to_le_bytes());
            commands.extend_from_slice(&(size as u32).to_le_bytes());
            commands.extend_from_slice(&24u32.to_le_bytes());
            commands.extend_from_slice(&[0u8; 12]);
            let mut name = name.as_bytes().to_vec();
            name.resize(size - 24, 0);
            commands.extend_from_slice(&name);
        }
        let mut data = Vec::new();
        data.extend_from_slice(&MH_MAGIC_64.to_le_bytes());
        data.extend_from_slice(&cputype.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&2u32.to_le_bytes());
        data.extend_from_slice(&(1 + dylibs.len() as u32).to_le_bytes());
        data.extend_from_slice(&(commands.len() as u32).to_le_bytes());
        data.extend_from_slice(&[0u8; 8]);
        data.extend_from_slice(&commands);
        data
    }

    const METAL: &str = "/System/Library/Frameworks/Metal.framework/Versions/A/Metal";

    #[test]
    fn reads_a_thin_arm64_metal_app() {
        let report = inspect_bytes(
            "Game",
            &build(0x0100_000C, &[METAL, "/usr/lib/libSystem.B.dylib"]),
        );
        assert_eq!(report.platform, Platform::MacOs);
        assert_eq!(report.kind, Kind::Application);
        assert_eq!(report.architectures, vec![Arch::Aarch64]);
        assert!(report.graphics_apis().contains(&GraphicsApi::Metal));
        match &report.format {
            Format::MachO(details) => assert_eq!(details.min_os.as_deref(), Some("13.0.0")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn reads_a_universal_binary() {
        let x86 = build(0x0100_0007, &[METAL]);
        let arm = build(0x0100_000C, &[METAL]);
        let mut fat = Vec::new();
        fat.extend_from_slice(&FAT_MAGIC.to_be_bytes());
        fat.extend_from_slice(&2u32.to_be_bytes());
        let first = 0x1000u32;
        let second = 0x2000u32;
        for (cputype, offset, size) in [
            (0x0100_0007u32, first, x86.len() as u32),
            (0x0100_000C, second, arm.len() as u32),
        ] {
            fat.extend_from_slice(&cputype.to_be_bytes());
            fat.extend_from_slice(&0u32.to_be_bytes());
            fat.extend_from_slice(&offset.to_be_bytes());
            fat.extend_from_slice(&size.to_be_bytes());
            fat.extend_from_slice(&12u32.to_be_bytes());
        }
        fat.resize(first as usize, 0);
        fat.extend_from_slice(&x86);
        fat.resize(second as usize, 0);
        fat.extend_from_slice(&arm);
        let report = inspect_bytes("Universal", &fat);
        assert_eq!(report.architectures, vec![Arch::X86_64, Arch::Aarch64]);
        assert!(matches!(report.format, Format::MachOUniversal { .. }));
    }

    #[test]
    fn java_classes_are_not_universal_binaries() {
        let class = [0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 61];
        assert!(!is_macho(&class));
    }

    #[test]
    fn truncation_does_not_panic() {
        let data = build(0x0100_000C, &[METAL]);
        for cut in 0..data.len() {
            inspect_bytes("cut", &data[..cut]);
        }
    }
}
