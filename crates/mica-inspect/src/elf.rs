//! ELF executables and shared objects (Linux, FreeBSD), including AppImages.
//!
//! Layout references: the System V ABI's ELF chapter and `elf(5)`.

use crate::bytes::{Endian, c_string, slice};
use crate::{Arch, ElfDetails, Format, Kind, Linkage, Platform, Report, add_graphics};

const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const DT_NULL: u64 = 0;
const DT_NEEDED: u64 = 1;
const DT_STRTAB: u64 = 5;
const MAX_SEGMENTS: usize = 256;
const MAX_DYNAMIC: usize = 4096;

struct Segment {
    kind: u32,
    offset: u64,
    vaddr: u64,
    filesz: u64,
}

fn machine(value: u16, bits: u8) -> Arch {
    match value {
        3 => Arch::X86,
        62 => Arch::X86_64,
        40 => Arch::Arm,
        183 => Arch::Aarch64,
        243 if bits == 64 => Arch::Riscv64,
        21 => Arch::PowerPc64,
        _ => Arch::Other,
    }
}

pub(crate) fn inspect(data: &[u8], report: &mut Report) {
    let bits = match data.get(4) {
        Some(1) => 32u8,
        Some(2) => 64,
        _ => {
            report.notes.push("unrecognised ELF class".into());
            return;
        }
    };
    let endian = match data.get(5) {
        Some(1) => Endian::Little,
        Some(2) => Endian::Big,
        _ => {
            report.notes.push("unrecognised ELF byte order".into());
            return;
        }
    };
    let os_abi = data.get(7).copied().unwrap_or_default();
    let (Some(elf_type), Some(machine_id)) = (endian.u16(data, 16), endian.u16(data, 18)) else {
        report.notes.push("truncated ELF header".into());
        return;
    };
    let arch = machine(machine_id, bits);
    report.architectures = vec![arch];
    report.platform = match os_abi {
        9 => Platform::FreeBsd,
        _ => Platform::Linux,
    };

    let (phoff, phentsize, phnum) = if bits == 64 {
        (
            endian.u64(data, 32),
            endian.u16(data, 54),
            endian.u16(data, 56),
        )
    } else {
        (
            endian.u32(data, 28).map(u64::from),
            endian.u16(data, 42),
            endian.u16(data, 44),
        )
    };
    let segments: Vec<Segment> = match (phoff, phentsize, phnum) {
        (Some(phoff), Some(size), Some(count)) if size > 0 => (0..(count as usize).min(MAX_SEGMENTS))
            .map_while(|index| {
                let at = usize::try_from(phoff).ok()? + index * size as usize;
                read_segment(data, at, bits, endian)
            })
            .collect(),
        _ => Vec::new(),
    };

    let interpreter = segments
        .iter()
        .find(|segment| segment.kind == PT_INTERP)
        .and_then(|segment| c_string(data, usize::try_from(segment.offset).ok()?, 4096));
    let needed = needed_libraries(data, &segments, bits, endian);
    let has_dynamic = segments.iter().any(|segment| segment.kind == PT_DYNAMIC);

    let appimage = match slice(data, 8, 3) {
        Some([b'A', b'I', kind @ (1 | 2)]) => Some(*kind),
        _ => None,
    };
    let type_name = match elf_type {
        1 => "relocatable",
        2 => "executable",
        3 if interpreter.is_some() => "position-independent executable",
        3 => "shared object",
        4 => "core dump",
        _ => "unknown",
    };
    report.kind = match type_name {
        "executable" | "position-independent executable" => Kind::Application,
        "shared object" => Kind::Library,
        _ => Kind::Unknown,
    };
    if appimage.is_some() {
        report.kind = Kind::Application;
        report
            .notes
            .push("an AppImage: it runs directly on most Linux distributions (needs FUSE)".into());
    }

    add_graphics(report, &needed, Linkage::Imported);
    report.libraries = needed;
    report.format = Format::Elf(ElfDetails {
        machine: arch,
        bits,
        elf_type: type_name.into(),
        os_abi,
        interpreter,
        appimage,
        statically_linked: !has_dynamic && report.kind == Kind::Application,
    });
}

fn read_segment(data: &[u8], at: usize, bits: u8, endian: Endian) -> Option<Segment> {
    if bits == 64 {
        Some(Segment {
            kind: endian.u32(data, at)?,
            offset: endian.u64(data, at + 8)?,
            vaddr: endian.u64(data, at + 16)?,
            filesz: endian.u64(data, at + 32)?,
        })
    } else {
        Some(Segment {
            kind: endian.u32(data, at)?,
            offset: endian.u32(data, at + 4)?.into(),
            vaddr: endian.u32(data, at + 8)?.into(),
            filesz: endian.u32(data, at + 16)?.into(),
        })
    }
}

/// `DT_NEEDED` entries of the dynamic segment. The string table is named by a
/// virtual address, which the `PT_LOAD` segments map back to a file offset.
fn needed_libraries(data: &[u8], segments: &[Segment], bits: u8, endian: Endian) -> Vec<String> {
    let Some(dynamic) = segments.iter().find(|segment| segment.kind == PT_DYNAMIC) else {
        return Vec::new();
    };
    let entry_size = if bits == 64 { 16 } else { 8 };
    let Ok(start) = usize::try_from(dynamic.offset) else {
        return Vec::new();
    };
    let count = (dynamic.filesz as usize / entry_size).min(MAX_DYNAMIC);
    let mut needed_offsets = Vec::new();
    let mut strtab = None;
    for index in 0..count {
        let at = start + index * entry_size;
        let entry = if bits == 64 {
            endian.u64(data, at).zip(endian.u64(data, at + 8))
        } else {
            endian
                .u32(data, at)
                .zip(endian.u32(data, at + 4))
                .map(|(tag, value)| (u64::from(tag), u64::from(value)))
        };
        let Some((tag, value)) = entry else { break };
        match tag {
            DT_NULL => break,
            DT_NEEDED => needed_offsets.push(value),
            DT_STRTAB => strtab = Some(value),
            _ => {}
        }
    }
    let Some(strtab) = strtab.and_then(|address| file_offset(segments, address)) else {
        return Vec::new();
    };
    needed_offsets
        .into_iter()
        .filter_map(|offset| {
            let at = strtab.checked_add(usize::try_from(offset).ok()?)?;
            c_string(data, at, 4096)
        })
        .collect()
}

fn file_offset(segments: &[Segment], address: u64) -> Option<usize> {
    segments
        .iter()
        .filter(|segment| segment.kind == PT_LOAD)
        .find_map(|segment| {
            let delta = address.checked_sub(segment.vaddr)?;
            if delta >= segment.filesz {
                return None;
            }
            usize::try_from(segment.offset.checked_add(delta)?).ok()
        })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{GraphicsApi, inspect_bytes};

    /// A minimal little-endian ELF64 executable with a PT_INTERP, one PT_LOAD
    /// covering the whole file, and a PT_DYNAMIC listing `needed`.
    pub(crate) fn build(machine: u16, interpreter: Option<&str>, needed: &[&str]) -> Vec<u8> {
        let mut data = vec![0u8; 64 + 3 * 56];
        data[0..4].copy_from_slice(b"\x7fELF");
        data[4] = 2;
        data[5] = 1;
        data[6] = 1;
        data[16..18].copy_from_slice(&3u16.to_le_bytes());
        data[18..20].copy_from_slice(&machine.to_le_bytes());
        data[32..40].copy_from_slice(&64u64.to_le_bytes());
        data[54..56].copy_from_slice(&56u16.to_le_bytes());
        data[56..58].copy_from_slice(&3u16.to_le_bytes());

        // String table: "\0" then each name.
        let strtab_at = data.len();
        let mut strtab = vec![0u8];
        let mut offsets = Vec::new();
        for name in needed {
            offsets.push(strtab.len() as u64);
            strtab.extend_from_slice(name.as_bytes());
            strtab.push(0);
        }
        data.extend_from_slice(&strtab);
        let interp_at = data.len();
        if let Some(path) = interpreter {
            data.extend_from_slice(path.as_bytes());
            data.push(0);
        }
        let dynamic_at = data.len();
        for offset in &offsets {
            data.extend_from_slice(&DT_NEEDED.to_le_bytes());
            data.extend_from_slice(&offset.to_le_bytes());
        }
        // The file is loaded at virtual address 0x400000.
        data.extend_from_slice(&DT_STRTAB.to_le_bytes());
        data.extend_from_slice(&(0x400000 + strtab_at as u64).to_le_bytes());
        data.extend_from_slice(&[0u8; 16]);
        let total = data.len() as u64;

        let mut header = |index: usize, kind: u32, offset: u64, vaddr: u64, size: u64| {
            let at = 64 + index * 56;
            data[at..at + 4].copy_from_slice(&kind.to_le_bytes());
            data[at + 8..at + 16].copy_from_slice(&offset.to_le_bytes());
            data[at + 16..at + 24].copy_from_slice(&vaddr.to_le_bytes());
            data[at + 32..at + 40].copy_from_slice(&size.to_le_bytes());
            data[at + 40..at + 48].copy_from_slice(&size.to_le_bytes());
        };
        header(0, PT_LOAD, 0, 0x400000, total);
        if interpreter.is_some() {
            header(1, PT_INTERP, interp_at as u64, 0, 0);
        }
        header(
            2,
            PT_DYNAMIC,
            dynamic_at as u64,
            0x400000 + dynamic_at as u64,
            total - dynamic_at as u64,
        );
        data
    }

    #[test]
    fn reads_a_vulkan_game() {
        let elf = build(
            62,
            Some("/lib64/ld-linux-x86-64.so.2"),
            &["libvulkan.so.1", "libSDL3.so.0", "libc.so.6"],
        );
        let report = inspect_bytes("game", &elf);
        assert_eq!(report.platform, Platform::Linux);
        assert_eq!(report.kind, Kind::Application);
        assert_eq!(report.architectures, vec![Arch::X86_64]);
        assert_eq!(
            report.libraries,
            vec!["libvulkan.so.1", "libSDL3.so.0", "libc.so.6"]
        );
        assert!(report.graphics_apis().contains(&GraphicsApi::Vulkan));
        match &report.format {
            Format::Elf(details) => {
                assert_eq!(details.bits, 64);
                assert_eq!(
                    details.interpreter.as_deref(),
                    Some("/lib64/ld-linux-x86-64.so.2")
                );
                assert_eq!(details.elf_type, "position-independent executable");
                assert!(!details.statically_linked);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn shared_objects_arm64_and_appimages() {
        let library = inspect_bytes("libfoo.so", &build(183, None, &["libGL.so.1"]));
        assert_eq!(library.kind, Kind::Library);
        assert_eq!(library.architectures, vec![Arch::Aarch64]);
        assert!(library.graphics_apis().contains(&GraphicsApi::OpenGl));

        let mut appimage = build(62, Some("/lib64/ld-linux-x86-64.so.2"), &[]);
        appimage[8..11].copy_from_slice(b"AI\x02");
        let report = inspect_bytes("Tool.AppImage", &appimage);
        match &report.format {
            Format::Elf(details) => assert_eq!(details.appimage, Some(2)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn truncated_and_hostile_files_do_not_panic() {
        let elf = build(62, Some("/lib/ld.so"), &["libvulkan.so.1"]);
        for cut in 0..elf.len() {
            inspect_bytes("cut", &elf[..cut]);
        }
        let mut hostile = elf.clone();
        hostile[32..40].copy_from_slice(&u64::MAX.to_le_bytes());
        inspect_bytes("hostile", &hostile);
        hostile[4] = 9;
        assert!(inspect_bytes("class", &hostile).notes[0].contains("class"));
    }
}
