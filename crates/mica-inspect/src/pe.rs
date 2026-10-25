//! Windows Portable Executable (PE/COFF) files: `.exe`, `.dll`, installers.
//!
//! Layout references: Microsoft's "PE Format" specification
//! (<https://learn.microsoft.com/windows/win32/debug/pe-format>).

use std::collections::BTreeSet;

use crate::bytes::{c_string, contains, contains_ignore_case, slice, u16_le, u32_le, u64_le};
use crate::{Arch, Format, Installer, Kind, Linkage, PeDetails, Platform, Report, add_graphics};

const IMAGE_FILE_LARGE_ADDRESS_AWARE: u16 = 0x0020;
const IMAGE_FILE_DLL: u16 = 0x2000;
const DIRECTORY_IMPORT: usize = 1;
const DIRECTORY_RESOURCE: usize = 2;
const DIRECTORY_DELAY_IMPORT: usize = 13;
const DIRECTORY_CLR: usize = 14;
/// Caps on loop counts, far above anything a real linker emits.
const MAX_SECTIONS: usize = 96;
const MAX_IMPORTS: usize = 4096;
const MAX_NAME: usize = 260;

struct Section {
    name: String,
    virtual_address: u32,
    virtual_size: u32,
    raw_size: u32,
    raw_offset: u32,
}

struct Image<'a> {
    data: &'a [u8],
    sections: Vec<Section>,
    directories: Vec<(u32, u32)>,
    image_base: u64,
}

impl Image<'_> {
    /// File offset of a relative virtual address, if it lies in a section's
    /// file-backed bytes.
    fn offset(&self, rva: u32) -> Option<usize> {
        self.sections.iter().find_map(|section| {
            let size = section.virtual_size.max(section.raw_size);
            let delta = rva.checked_sub(section.virtual_address)?;
            if delta >= size || delta >= section.raw_size {
                return None;
            }
            let offset = section.raw_offset as usize + delta as usize;
            (offset < self.data.len()).then_some(offset)
        })
    }

    fn directory(&self, index: usize) -> Option<(u32, u32)> {
        self.directories
            .get(index)
            .copied()
            .filter(|&(rva, size)| rva != 0 && size != 0)
    }

    fn name_at(&self, rva: u32) -> Option<String> {
        let name = c_string(self.data, self.offset(rva)?, MAX_NAME)?;
        (!name.is_empty()).then(|| name.to_ascii_lowercase())
    }

    /// DLL names from the import directory.
    fn imports(&self) -> Vec<String> {
        let mut names = Vec::new();
        let Some((rva, _)) = self.directory(DIRECTORY_IMPORT) else {
            return names;
        };
        let Some(start) = self.offset(rva) else {
            return names;
        };
        // IMAGE_IMPORT_DESCRIPTOR: 20 bytes, the DLL name's RVA at offset 12.
        for index in 0..MAX_IMPORTS {
            let entry = start + index * 20;
            let Some(descriptor) = slice(self.data, entry, 20) else {
                break;
            };
            if descriptor.iter().all(|&byte| byte == 0) {
                break;
            }
            let Some(name_rva) = u32_le(descriptor, 12) else {
                break;
            };
            if let Some(name) = self.name_at(name_rva) {
                names.push(name);
            }
        }
        names
    }

    /// DLL names from the delay-load import directory.
    fn delay_imports(&self) -> Vec<String> {
        let mut names = Vec::new();
        let Some((rva, _)) = self.directory(DIRECTORY_DELAY_IMPORT) else {
            return names;
        };
        let Some(start) = self.offset(rva) else {
            return names;
        };
        // IMAGE_DELAYLOAD_DESCRIPTOR: 32 bytes; attributes at 0, the DLL
        // name at 4. Attribute bit 0 clear means the old format, which stores
        // virtual addresses rather than RVAs.
        for index in 0..MAX_IMPORTS {
            let entry = start + index * 32;
            let (Some(attributes), Some(name)) =
                (u32_le(self.data, entry), u32_le(self.data, entry + 4))
            else {
                break;
            };
            if name == 0 {
                break;
            }
            let name_rva = if attributes & 1 == 1 {
                Some(name)
            } else {
                u32::try_from(u64::from(name).wrapping_sub(self.image_base)).ok()
            };
            if let Some(name) = name_rva.and_then(|rva| self.name_at(rva)) {
                names.push(name);
            }
        }
        names
    }

    /// Where the last section's file data ends: anything after it is an
    /// overlay, where installers keep their payload.
    fn overlay_start(&self) -> usize {
        self.sections
            .iter()
            .map(|section| section.raw_offset as usize + section.raw_size as usize)
            .max()
            .unwrap_or(0)
            .min(self.data.len())
    }

    fn resources(&self) -> &[u8] {
        let Some((rva, size)) = self.directory(DIRECTORY_RESOURCE) else {
            return &[];
        };
        let Some(start) = self.offset(rva) else {
            return &[];
        };
        let end = start.saturating_add(size as usize).min(self.data.len());
        &self.data[start..end]
    }
}

pub(crate) fn inspect(data: &[u8], lower_name: &str, report: &mut Report) {
    let header = u32_le(data, 0x3C).map(|offset| offset as usize);
    let signature = header.and_then(|offset| slice(data, offset, 4));
    match signature {
        Some(b"PE\0\0") => inspect_pe(data, header.unwrap_or_default(), lower_name, report),
        Some(sig) if sig.starts_with(b"NE") || sig.starts_with(b"LE") || sig.starts_with(b"LX") => {
            legacy(report, &String::from_utf8_lossy(&sig[..2]));
            report
                .notes
                .push("a 16-bit Windows or OS/2 program; 64-bit Windows cannot run it".into());
        }
        _ => {
            legacy(report, "MZ");
            report
                .notes
                .push("a DOS program: DOSBox runs these on every platform".into());
        }
    }
}

fn legacy(report: &mut Report, header: &str) {
    report.format = Format::Legacy {
        header: header.into(),
    };
    report.platform = Platform::Dos;
    report.kind = Kind::Application;
    report.architectures = vec![Arch::X86];
}

fn machine(value: u16) -> Arch {
    match value {
        0x014C => Arch::X86,
        0x8664 => Arch::X86_64,
        0x01C0 | 0x01C4 => Arch::Arm,
        0xAA64 => Arch::Aarch64,
        0xA641 => Arch::Arm64ec,
        0x5064 => Arch::Riscv64,
        _ => Arch::Other,
    }
}

fn subsystem(value: u16) -> &'static str {
    match value {
        1 => "native",
        2 => "windows-gui",
        3 => "windows-console",
        9 => "windows-ce",
        10..=12 => "efi",
        14 => "xbox",
        16 => "boot-application",
        _ => "unknown",
    }
}

fn inspect_pe(data: &[u8], pe: usize, lower_name: &str, report: &mut Report) {
    report.platform = Platform::Windows;
    report.kind = Kind::Application;
    let coff = pe + 4;
    let (Some(machine_id), Some(sections), Some(optional_size), Some(characteristics)) = (
        u16_le(data, coff),
        u16_le(data, coff + 2),
        u16_le(data, coff + 16),
        u16_le(data, coff + 18),
    ) else {
        report.notes.push("truncated PE header".into());
        return;
    };
    let arch = machine(machine_id);
    report.architectures = vec![arch];

    let optional = coff + 20;
    let magic = u16_le(data, optional).unwrap_or_default();
    let (bits, directories_at, count_at, image_base) = match magic {
        0x10B => (
            32,
            optional + 96,
            optional + 92,
            u32_le(data, optional + 28).map(u64::from),
        ),
        0x20B => (
            64,
            optional + 112,
            optional + 108,
            u64_le(data, optional + 24),
        ),
        _ => {
            report.notes.push("unrecognised PE optional header".into());
            (0, 0, 0, None)
        }
    };
    let subsystem_id = u16_le(data, optional + 68).unwrap_or_default();
    let directory_count = u32_le(data, count_at).unwrap_or_default().min(16) as usize;
    let directories = (0..directory_count)
        .map_while(|index| {
            let at = directories_at + index * 8;
            Some((u32_le(data, at)?, u32_le(data, at + 4)?))
        })
        .collect();

    let table = optional + optional_size as usize;
    let sections = (0..(sections as usize).min(MAX_SECTIONS))
        .map_while(|index| {
            let at = table + index * 40;
            let raw = slice(data, at, 40)?;
            let end = raw[..8].iter().position(|&byte| byte == 0).unwrap_or(8);
            Some(Section {
                name: String::from_utf8_lossy(&raw[..end]).into_owned(),
                virtual_size: u32_le(raw, 8)?,
                virtual_address: u32_le(raw, 12)?,
                raw_size: u32_le(raw, 16)?,
                raw_offset: u32_le(raw, 20)?,
            })
        })
        .collect();
    let image = Image {
        data,
        sections,
        directories,
        image_base: image_base.unwrap_or_default(),
    };

    let imports = image.imports();
    let delay_loaded = image.delay_imports();
    add_graphics(report, &imports, Linkage::Imported);
    add_graphics(report, &delay_loaded, Linkage::Referenced);
    let referenced: Vec<String> = referenced_dlls(data).into_iter().collect();
    add_graphics(report, &referenced, Linkage::Referenced);

    let dll = characteristics & IMAGE_FILE_DLL != 0;
    let dotnet = image.directory(DIRECTORY_CLR).is_some();
    let resources = image.resources();
    let requires_admin = contains(resources, b"requireAdministrator")
        || contains(resources, &utf16(b"requireAdministrator"));
    if dll {
        report.kind = Kind::Library;
    }
    if dotnet {
        report.notes.push(
            "a .NET assembly: under Wine it needs Wine Mono or Microsoft .NET in the prefix".into(),
        );
    }

    let installer = installer_kind(&image, lower_name);
    if let Some(kind) = installer {
        report.installer = Some(kind);
        report.kind = Kind::Installer;
    }

    let mut libraries = imports;
    libraries.extend(delay_loaded.iter().cloned());
    libraries.sort();
    libraries.dedup();
    report.libraries = libraries;
    report.format = Format::Pe(PeDetails {
        machine: arch,
        bits,
        subsystem: subsystem(subsystem_id).into(),
        dll,
        dotnet,
        large_address_aware: characteristics & IMAGE_FILE_LARGE_ADDRESS_AWARE != 0,
        requires_admin,
        sections: image
            .sections
            .iter()
            .map(|section| section.name.clone())
            .collect(),
        delay_loaded,
    });
}

/// Installers announce themselves in their stub's section names, resources,
/// or the overlay that carries the payload. The name is the last resort.
fn installer_kind(image: &Image, lower_name: &str) -> Option<Installer> {
    if image
        .sections
        .iter()
        .any(|section| section.name == ".wixburn")
    {
        return Some(Installer::WixBurn);
    }
    let overlay = &image.data[image.overlay_start()..];
    let overlay_head = &overlay[..overlay.len().min(64 << 10)];
    let resources = image.resources();
    // Read-only data and resources of the stub, where the toolkits leave
    // their names; bounded so a large game is not scanned end to end.
    let head = &image.data[..image.data.len().min(4 << 20)];
    let markers: &[(&[u8], Installer)] = &[
        (b"Nullsoft Install System", Installer::Nsis),
        (b"NullsoftInst", Installer::Nsis),
        (b"Inno Setup Setup Data", Installer::InnoSetup),
        (b"InnoSetupLdrWindow", Installer::InnoSetup),
        (b"InstallShield", Installer::InstallShield),
        (b"7zS.sfx", Installer::SevenZipSfx),
        (b"7-Zip SFX", Installer::SevenZipSfx),
    ];
    for (marker, kind) in markers {
        if contains(overlay_head, marker) || contains(resources, marker) || contains(head, marker) {
            return Some(*kind);
        }
    }
    let stem = lower_name.trim_end_matches(".exe");
    let named = stem == "setup"
        || stem.starts_with("setup")
        || stem.ends_with("setup")
        || stem.contains("install")
        || stem.ends_with("-setup")
        || stem.contains("_setup");
    (named && lower_name.ends_with(".exe") && !stem.contains("uninst"))
        .then_some(Installer::LikelyByName)
}

fn utf16(ascii: &[u8]) -> Vec<u8> {
    ascii.iter().flat_map(|&byte| [byte, 0]).collect()
}

/// DLL names that appear in the file's data, in ASCII or UTF-16LE: candidates
/// for `LoadLibrary`. One pass over the bytes, looking for ".dll".
pub(crate) fn referenced_dlls(data: &[u8]) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let interesting = |name: &str| crate::graphics_for_library(name).is_some();
    let is_name_byte = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-';

    let mut index = 0;
    while index + 4 <= data.len() {
        // ASCII: "name.dll"
        if data[index] == b'.' && data[index + 1..index + 4].eq_ignore_ascii_case(b"dll") {
            let mut start = index;
            while start > 0 && index - start < 32 && is_name_byte(data[start - 1]) {
                start -= 1;
            }
            if start < index {
                let name = String::from_utf8_lossy(&data[start..index + 4]).to_ascii_lowercase();
                if interesting(&name) {
                    found.insert(name);
                }
            }
        }
        // UTF-16LE: "n\0a\0m\0e\0.\0d\0l\0l\0"
        if index + 8 <= data.len()
            && data[index] == b'.'
            && data[index + 1] == 0
            && contains_ignore_case(&[data[index + 2], data[index + 4], data[index + 6]], b"dll")
            && data[index + 3] == 0
            && data[index + 5] == 0
            && data[index + 7] == 0
        {
            let mut start = index;
            let mut name = Vec::new();
            while start >= 2
                && name.len() < 32
                && data[start - 1] == 0
                && is_name_byte(data[start - 2])
            {
                start -= 2;
                name.push(data[start]);
            }
            name.reverse();
            if !name.is_empty() {
                let name = format!("{}.dll", String::from_utf8_lossy(&name)).to_ascii_lowercase();
                if interesting(&name) {
                    found.insert(name);
                }
            }
        }
        index += 1;
    }
    found
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{GraphicsApi, inspect_bytes};

    /// Options for a synthetic PE image built by [`build`].
    pub(crate) struct Spec<'a> {
        pub machine: u16,
        pub pe32_plus: bool,
        pub subsystem: u16,
        pub dll: bool,
        pub imports: &'a [&'a str],
        pub delay_imports: &'a [&'a str],
        pub dotnet: bool,
        pub section_names: &'a [&'a str],
        pub overlay: &'a [u8],
    }

    impl Default for Spec<'_> {
        fn default() -> Self {
            Spec {
                machine: 0x8664,
                pe32_plus: true,
                subsystem: 2,
                dll: false,
                imports: &["kernel32.dll"],
                delay_imports: &[],
                dotnet: false,
                section_names: &[],
                overlay: &[],
            }
        }
    }

    /// A minimal but well-formed PE image: DOS header, PE header, optional
    /// header with 16 data directories, one `.idata` section holding the
    /// import and delay-import descriptors and the DLL names, any extra
    /// (empty) sections, and an overlay.
    pub(crate) fn build(spec: &Spec) -> Vec<u8> {
        const PE: usize = 0x80;
        const FILE_ALIGN: usize = 0x200;
        const SECTION_RVA: u32 = 0x1000;
        let optional_size: usize = if spec.pe32_plus { 240 } else { 224 };
        let section_count = 1 + spec.section_names.len();
        let headers_end = PE + 24 + optional_size + 40 * section_count;
        let raw_offset = headers_end.div_ceil(FILE_ALIGN) * FILE_ALIGN;

        // Section contents: descriptors first, then names.
        let mut idata =
            vec![0u8; 20 * (spec.imports.len() + 1) + 32 * (spec.delay_imports.len() + 1)];
        let delay_at = 20 * (spec.imports.len() + 1);
        for (index, name) in spec.imports.iter().enumerate() {
            let rva = SECTION_RVA + idata.len() as u32;
            idata.extend_from_slice(name.as_bytes());
            idata.push(0);
            idata[index * 20 + 12..index * 20 + 16].copy_from_slice(&rva.to_le_bytes());
        }
        for (index, name) in spec.delay_imports.iter().enumerate() {
            let rva = SECTION_RVA + idata.len() as u32;
            idata.extend_from_slice(name.as_bytes());
            idata.push(0);
            let at = delay_at + index * 32;
            idata[at..at + 4].copy_from_slice(&1u32.to_le_bytes());
            idata[at + 4..at + 8].copy_from_slice(&rva.to_le_bytes());
        }
        let raw_size = idata.len().div_ceil(FILE_ALIGN) * FILE_ALIGN;
        idata.resize(raw_size, 0);

        let mut data = vec![0u8; raw_offset];
        data[0..2].copy_from_slice(b"MZ");
        data[0x3C..0x40].copy_from_slice(&(PE as u32).to_le_bytes());
        data[PE..PE + 4].copy_from_slice(b"PE\0\0");
        let coff = PE + 4;
        let characteristics: u16 = 0x0002 | 0x0020 | if spec.dll { IMAGE_FILE_DLL } else { 0 };
        data[coff..coff + 2].copy_from_slice(&spec.machine.to_le_bytes());
        data[coff + 2..coff + 4].copy_from_slice(&(section_count as u16).to_le_bytes());
        data[coff + 16..coff + 18].copy_from_slice(&(optional_size as u16).to_le_bytes());
        data[coff + 18..coff + 20].copy_from_slice(&characteristics.to_le_bytes());
        let optional = coff + 20;
        let magic: u16 = if spec.pe32_plus { 0x20B } else { 0x10B };
        data[optional..optional + 2].copy_from_slice(&magic.to_le_bytes());
        data[optional + 68..optional + 70].copy_from_slice(&spec.subsystem.to_le_bytes());
        let (count_at, directories_at) = if spec.pe32_plus {
            (optional + 108, optional + 112)
        } else {
            (optional + 92, optional + 96)
        };
        data[count_at..count_at + 4].copy_from_slice(&16u32.to_le_bytes());
        let mut directory = |index: usize, rva: u32, size: u32| {
            let at = directories_at + index * 8;
            data[at..at + 4].copy_from_slice(&rva.to_le_bytes());
            data[at + 4..at + 8].copy_from_slice(&size.to_le_bytes());
        };
        if !spec.imports.is_empty() {
            directory(
                DIRECTORY_IMPORT,
                SECTION_RVA,
                20 * (spec.imports.len() as u32 + 1),
            );
        }
        if !spec.delay_imports.is_empty() {
            directory(DIRECTORY_DELAY_IMPORT, SECTION_RVA + delay_at as u32, 32);
        }
        if spec.dotnet {
            directory(DIRECTORY_CLR, SECTION_RVA, 72);
        }

        let table = optional + optional_size;
        let mut section = |index: usize, name: &str, rva: u32, size: usize, offset: usize| {
            let at = table + index * 40;
            let bytes = name.as_bytes();
            data[at..at + bytes.len().min(8)].copy_from_slice(&bytes[..bytes.len().min(8)]);
            data[at + 8..at + 12].copy_from_slice(&(size as u32).to_le_bytes());
            data[at + 12..at + 16].copy_from_slice(&rva.to_le_bytes());
            data[at + 16..at + 20].copy_from_slice(&(size as u32).to_le_bytes());
            data[at + 20..at + 24].copy_from_slice(&(offset as u32).to_le_bytes());
        };
        section(0, ".idata", SECTION_RVA, raw_size, raw_offset);
        for (index, name) in spec.section_names.iter().enumerate() {
            section(
                index + 1,
                name,
                SECTION_RVA + 0x10000 * (index as u32 + 1),
                0,
                0,
            );
        }
        data.extend_from_slice(&idata);
        data.extend_from_slice(spec.overlay);
        data
    }

    fn details(report: &Report) -> &PeDetails {
        match &report.format {
            Format::Pe(details) => details,
            other => panic!("not a PE: {other:?}"),
        }
    }

    #[test]
    fn reads_a_64_bit_direct3d_11_game() {
        let image = build(&Spec {
            imports: &["KERNEL32.dll", "d3d11.dll", "dxgi.dll", "XINPUT1_4.dll"],
            ..Spec::default()
        });
        let report = inspect_bytes("Game.exe", &image);
        assert_eq!(report.platform, Platform::Windows);
        assert_eq!(report.kind, Kind::Application);
        assert_eq!(report.architectures, vec![Arch::X86_64]);
        assert_eq!(
            report.libraries,
            vec!["d3d11.dll", "dxgi.dll", "kernel32.dll", "xinput1_4.dll"]
        );
        let apis = report.graphics_apis();
        assert!(apis.contains(&GraphicsApi::Direct3D11));
        assert!(apis.contains(&GraphicsApi::Dxgi));
        let pe = details(&report);
        assert_eq!(pe.bits, 64);
        assert_eq!(pe.subsystem, "windows-gui");
        assert!(!pe.dll && !pe.dotnet);
        assert!(pe.large_address_aware);
    }

    #[test]
    fn reads_32_bit_console_and_dll_and_dotnet() {
        let console = inspect_bytes(
            "tool.exe",
            &build(&Spec {
                machine: 0x014C,
                pe32_plus: false,
                subsystem: 3,
                imports: &["d3d9.dll"],
                ..Spec::default()
            }),
        );
        assert_eq!(console.architectures, vec![Arch::X86]);
        assert_eq!(details(&console).bits, 32);
        assert_eq!(details(&console).subsystem, "windows-console");
        assert!(console.graphics_apis().contains(&GraphicsApi::Direct3D9));

        let dll = inspect_bytes(
            "plugin.dll",
            &build(&Spec {
                dll: true,
                ..Spec::default()
            }),
        );
        assert_eq!(dll.kind, Kind::Library);

        let dotnet = inspect_bytes(
            "App.exe",
            &build(&Spec {
                imports: &["mscoree.dll"],
                dotnet: true,
                ..Spec::default()
            }),
        );
        assert!(details(&dotnet).dotnet);
        assert!(dotnet.notes.iter().any(|note| note.contains(".NET")));

        let arm = inspect_bytes(
            "arm.exe",
            &build(&Spec {
                machine: 0xAA64,
                ..Spec::default()
            }),
        );
        assert_eq!(arm.architectures, vec![Arch::Aarch64]);
    }

    #[test]
    fn delay_loaded_and_referenced_apis() {
        let mut overlay = b"....LoadLibrary(\"vulkan-1.dll\")....".to_vec();
        overlay.extend_from_slice(&utf16(b"D3D12.DLL"));
        let report = inspect_bytes(
            "engine.exe",
            &build(&Spec {
                delay_imports: &["d3d11.dll"],
                overlay: &overlay,
                ..Spec::default()
            }),
        );
        assert_eq!(details(&report).delay_loaded, vec!["d3d11.dll"]);
        let apis = report.graphics_apis();
        assert!(apis.contains(&GraphicsApi::Direct3D11));
        assert!(apis.contains(&GraphicsApi::Vulkan));
        assert!(apis.contains(&GraphicsApi::Direct3D12));
        assert!(
            report
                .graphics
                .iter()
                .all(|graphics| graphics.linkage == Linkage::Referenced)
        );
    }

    #[test]
    fn recognises_installers() {
        let nsis = inspect_bytes(
            "thing.exe",
            &build(&Spec {
                overlay: b"\xef\xbe\xad\xdeNullsoftInst....",
                ..Spec::default()
            }),
        );
        assert_eq!(nsis.installer, Some(Installer::Nsis));
        assert_eq!(nsis.kind, Kind::Installer);

        let inno = inspect_bytes(
            "x.exe",
            &build(&Spec {
                overlay: b"Inno Setup Setup Data (6.2.0)",
                ..Spec::default()
            }),
        );
        assert_eq!(inno.installer, Some(Installer::InnoSetup));

        let burn = inspect_bytes(
            "bundle.exe",
            &build(&Spec {
                section_names: &[".wixburn"],
                ..Spec::default()
            }),
        );
        assert_eq!(burn.installer, Some(Installer::WixBurn));

        let named = inspect_bytes("MyGame_Setup.exe", &build(&Spec::default()));
        assert_eq!(named.installer, Some(Installer::LikelyByName));

        let uninstaller = inspect_bytes("uninstall.exe", &build(&Spec::default()));
        assert_eq!(uninstaller.installer, None);
        let game = inspect_bytes("Game.exe", &build(&Spec::default()));
        assert_eq!(game.installer, None);
    }

    #[test]
    fn dos_and_16_bit_programs() {
        let mut dos = vec![0u8; 0x40];
        dos[0..2].copy_from_slice(b"MZ");
        let report = inspect_bytes("GAME.EXE", &dos);
        assert_eq!(report.platform, Platform::Dos);
        assert!(report.notes.iter().any(|note| note.contains("DOSBox")));

        let mut ne = vec![0u8; 0x90];
        ne[0..2].copy_from_slice(b"MZ");
        ne[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        ne[0x80..0x82].copy_from_slice(b"NE");
        let report = inspect_bytes("WIN16.EXE", &ne);
        assert_eq!(
            report.format,
            Format::Legacy {
                header: "NE".into()
            }
        );
    }

    #[test]
    fn hostile_offsets_do_not_panic() {
        let mut image = build(&Spec {
            imports: &["d3d11.dll"],
            ..Spec::default()
        });
        // Point e_lfanew, the import directory, and a section past the end.
        let mut broken = image.clone();
        broken[0x3C..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        inspect_bytes("a.exe", &broken);
        let directories_at = 0x80 + 24 + 112;
        image[directories_at + 8..directories_at + 12]
            .copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
        inspect_bytes("b.exe", &image);
        for cut in [0x40, 0x84, 0x98, 0x100, 0x200, 0x300] {
            inspect_bytes("c.exe", &image[..cut.min(image.len())]);
        }
    }
}
