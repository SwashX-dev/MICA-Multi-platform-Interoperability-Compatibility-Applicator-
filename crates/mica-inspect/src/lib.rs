//! What is this file, and what does it need to run?
//!
//! `mica-inspect` reads executables, installers, and application bundles
//! without running any of their code, and reports what MICA's decision engine
//! needs to choose an execution backend: the operating system the file was
//! built for, its CPU architectures, the graphics APIs and libraries it links,
//! whether it is an installer, and anti-cheat software shipped beside it.
//!
//! Every parser treats its input as hostile: reads are bounds-checked
//! ([`bytes`]), loops are capped, and a malformed file produces a less
//! detailed report rather than a panic. Detection is by content, never by file
//! extension alone; the extension only refines an otherwise ambiguous result
//! (an OLE compound file named `.msi` is a Windows Installer package).

pub mod anticheat;
mod bundle;
mod bytes;
mod elf;
pub mod lnk;
mod macho;
mod pe;

use std::collections::BTreeSet;
use std::fmt;
use std::io::Read;
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

pub use anticheat::{AntiCheat, AntiCheatScan, AntiCheatSignal, Confidence};

/// Files larger than this are inspected from their first `MAX_READ` bytes.
/// Headers, import tables, and installer markers sit near the start, so a
/// huge game binary is still identified; strings referenced only beyond this
/// point are not seen.
pub const MAX_READ: u64 = 256 << 20;

/// How thoroughly to inspect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// Read at most this many bytes.
    pub max_read: u64,
    /// Compute the SHA-256 of what was read.
    pub hash: bool,
}

impl Options {
    /// Everything `mica inspect` shows.
    pub const FULL: Options = Options {
        max_read: MAX_READ,
        hash: true,
    };
    /// Enough to choose a backend, fast enough to run before every launch:
    /// headers and import tables are in the first megabytes of a program.
    pub const QUICK: Options = Options {
        max_read: 32 << 20,
        hash: false,
    };
}

/// The operating system a file was built for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Platform {
    Windows,
    Linux,
    MacOs,
    /// iOS, iPadOS, and other Apple platforms that are not macOS.
    AppleMobile,
    /// 16-bit DOS and Windows 3.x programs.
    Dos,
    FreeBsd,
    /// A `portable:platform` WebAssembly component, which runs on every
    /// operating system through Portable Host.
    Portable,
    /// Plain WebAssembly or a script: depends on its runtime, not on an OS.
    Any,
    Unknown,
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Platform::Windows => "Windows",
            Platform::Linux => "Linux",
            Platform::MacOs => "macOS",
            Platform::AppleMobile => "iOS/iPadOS",
            Platform::Dos => "DOS/16-bit Windows",
            Platform::FreeBsd => "FreeBSD",
            Platform::Portable => "portable (WebAssembly component)",
            Platform::Any => "any (runtime-dependent)",
            Platform::Unknown => "unknown",
        })
    }
}

/// A CPU architecture an executable contains code for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Arch {
    X86,
    X86_64,
    Arm,
    Aarch64,
    /// Windows ARM64EC: ARM64 code that interoperates with x86-64 code.
    Arm64ec,
    Riscv64,
    PowerPc64,
    Wasm32,
    Other,
}

impl Arch {
    /// The architecture this MICA binary was compiled for.
    pub fn host() -> Arch {
        match std::env::consts::ARCH {
            "x86" => Arch::X86,
            "x86_64" => Arch::X86_64,
            "arm" => Arch::Arm,
            "aarch64" => Arch::Aarch64,
            "riscv64" => Arch::Riscv64,
            "powerpc64" => Arch::PowerPc64,
            _ => Arch::Other,
        }
    }
}

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Arch::X86 => "x86 (32-bit)",
            Arch::X86_64 => "x86-64",
            Arch::Arm => "ARM (32-bit)",
            Arch::Aarch64 => "ARM64",
            Arch::Arm64ec => "ARM64EC",
            Arch::Riscv64 => "RISC-V 64",
            Arch::PowerPc64 => "PowerPC 64",
            Arch::Wasm32 => "WebAssembly",
            Arch::Other => "other",
        })
    }
}

/// What the file is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Application,
    Library,
    Installer,
    Script,
    Archive,
    Shortcut,
    Unknown,
}

/// A graphics or windowing API the program links or names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GraphicsApi {
    DirectDraw,
    Direct3D8,
    Direct3D9,
    Direct3D10,
    Direct3D11,
    Direct3D12,
    Dxgi,
    OpenGl,
    OpenGlEs,
    Vulkan,
    Metal,
}

impl fmt::Display for GraphicsApi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            GraphicsApi::DirectDraw => "DirectDraw",
            GraphicsApi::Direct3D8 => "Direct3D 8",
            GraphicsApi::Direct3D9 => "Direct3D 9",
            GraphicsApi::Direct3D10 => "Direct3D 10",
            GraphicsApi::Direct3D11 => "Direct3D 11",
            GraphicsApi::Direct3D12 => "Direct3D 12",
            GraphicsApi::Dxgi => "DXGI",
            GraphicsApi::OpenGl => "OpenGL",
            GraphicsApi::OpenGlEs => "OpenGL ES/EGL",
            GraphicsApi::Vulkan => "Vulkan",
            GraphicsApi::Metal => "Metal",
        })
    }
}

/// How the program reaches a graphics API.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Linkage {
    /// Listed in the import table or dynamic section: loaded at startup.
    Imported,
    /// Named in the file's data (for example a DLL name passed to
    /// `LoadLibrary`), so possibly loaded at run time.
    Referenced,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Graphics {
    pub api: GraphicsApi,
    pub linkage: Linkage,
}

/// Installer technologies recognisable from their stub or embedded data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Installer {
    WindowsInstaller,
    Nsis,
    InnoSetup,
    InstallShield,
    WixBurn,
    SevenZipSfx,
    /// Named like an installer (`setup.exe`, `*installer*.exe`), with no
    /// recognised stub.
    LikelyByName,
    DebianPackage,
    RpmPackage,
    AppleDiskImage,
    ApplePackage,
}

impl fmt::Display for Installer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Installer::WindowsInstaller => "Windows Installer package (MSI)",
            Installer::Nsis => "NSIS installer",
            Installer::InnoSetup => "Inno Setup installer",
            Installer::InstallShield => "InstallShield installer",
            Installer::WixBurn => "WiX Burn bundle",
            Installer::SevenZipSfx => "7-Zip self-extracting archive",
            Installer::LikelyByName => "installer (by its name)",
            Installer::DebianPackage => "Debian package",
            Installer::RpmPackage => "RPM package",
            Installer::AppleDiskImage => "Apple disk image",
            Installer::ApplePackage => "Apple installer package",
        })
    }
}

/// Format-specific facts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Format {
    Pe(PeDetails),
    /// A DOS `MZ` program with no PE header, or a 16-bit `NE`/`LE` program.
    Legacy {
        header: String,
    },
    Elf(ElfDetails),
    MachO(MachODetails),
    /// A multi-architecture ("universal") Mach-O file.
    MachOUniversal {
        slices: Vec<MachODetails>,
    },
    AppBundle(BundleDetails),
    WasmComponent,
    WasmModule,
    OleCompound,
    Script {
        interpreter: String,
    },
    WindowsScript {
        language: String,
    },
    WindowsShortcut,
    Archive {
        archive: String,
    },
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PeDetails {
    pub machine: Arch,
    pub bits: u8,
    pub subsystem: String,
    pub dll: bool,
    /// Carries a CLR header: a .NET assembly.
    pub dotnet: bool,
    pub large_address_aware: bool,
    /// The application manifest asks for administrator rights.
    pub requires_admin: bool,
    pub sections: Vec<String>,
    pub delay_loaded: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ElfDetails {
    pub machine: Arch,
    pub bits: u8,
    pub elf_type: String,
    pub os_abi: u8,
    pub interpreter: Option<String>,
    /// An AppImage (type 1 or 2): a self-mounting Linux application.
    pub appimage: Option<u8>,
    pub statically_linked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MachODetails {
    pub machine: Arch,
    pub file_type: String,
    /// Minimum OS version from LC_BUILD_VERSION or LC_VERSION_MIN_*.
    pub min_os: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BundleDetails {
    pub identifier: Option<String>,
    pub executable: String,
    pub inner: Box<Format>,
}

/// Everything inspection learned about one file or bundle.
#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub path: String,
    pub size: u64,
    /// SHA-256 of the bytes read (`None` in [`Options::QUICK`] inspections).
    pub sha256: Option<String>,
    /// Whether only part of the file was read.
    pub truncated: bool,
    pub format: Format,
    pub platform: Platform,
    pub kind: Kind,
    pub architectures: Vec<Arch>,
    pub graphics: Vec<Graphics>,
    /// Imported DLLs, needed shared objects, or linked dylibs, lowercased for
    /// DLLs and as written otherwise.
    pub libraries: Vec<String>,
    pub installer: Option<Installer>,
    pub notes: Vec<String>,
}

impl Report {
    /// The graphics APIs, strongest linkage first, without duplicates.
    pub fn graphics_apis(&self) -> BTreeSet<GraphicsApi> {
        self.graphics.iter().map(|graphics| graphics.api).collect()
    }

    pub fn is_installer(&self) -> bool {
        self.kind == Kind::Installer
    }
}

#[derive(Debug)]
pub enum InspectError {
    Io(std::io::Error),
    NotAFile(String),
}

impl fmt::Display for InspectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InspectError::Io(error) => write!(f, "{error}"),
            InspectError::NotAFile(path) => {
                write!(
                    f,
                    "{path} is neither a regular file nor an application bundle"
                )
            }
        }
    }
}

impl std::error::Error for InspectError {}

impl From<std::io::Error> for InspectError {
    fn from(error: std::io::Error) -> Self {
        InspectError::Io(error)
    }
}

/// Inspect a file, or a macOS `.app` bundle directory, thoroughly.
pub fn inspect_path(path: &Path) -> Result<Report, InspectError> {
    inspect_path_with(path, Options::FULL)
}

/// Inspect a file or bundle with the given thoroughness.
pub fn inspect_path_with(path: &Path, options: Options) -> Result<Report, InspectError> {
    let metadata = std::fs::metadata(path)?;
    if metadata.is_dir() {
        return match bundle::inspect_bundle(path, options)? {
            Some(report) => Ok(report),
            None => Err(InspectError::NotAFile(path.display().to_string())),
        };
    }
    if !metadata.is_file() {
        return Err(InspectError::NotAFile(path.display().to_string()));
    }
    let mut data = Vec::new();
    std::fs::File::open(path)?
        .take(options.max_read)
        .read_to_end(&mut data)?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut report = inspect_bytes(&name, &data);
    if !options.hash {
        report.sha256 = None;
    }
    report.path = path.display().to_string();
    report.size = metadata.len();
    report.truncated = metadata.len() > options.max_read;
    if report.truncated {
        report.notes.push(format!(
            "only the first {} MiB were inspected{}",
            options.max_read >> 20,
            if options.hash {
                "; the sha256 covers only those bytes"
            } else {
                ""
            }
        ));
    }
    Ok(report)
}

/// Inspect bytes in memory. `name` is the file name, used only to refine
/// otherwise ambiguous formats.
pub fn inspect_bytes(name: &str, data: &[u8]) -> Report {
    let mut report = Report {
        path: name.to_string(),
        size: data.len() as u64,
        sha256: Some(sha256_hex(data)),
        truncated: false,
        format: Format::Unknown,
        platform: Platform::Unknown,
        kind: Kind::Unknown,
        architectures: Vec::new(),
        graphics: Vec::new(),
        libraries: Vec::new(),
        installer: None,
        notes: Vec::new(),
    };
    let lower = name.to_ascii_lowercase();

    if data.starts_with(b"MZ") {
        pe::inspect(data, &lower, &mut report);
    } else if data.starts_with(b"\x7fELF") {
        elf::inspect(data, &mut report);
    } else if macho::is_macho(data) {
        macho::inspect(data, &mut report);
    } else if data.starts_with(b"\0asm") {
        inspect_wasm(data, &mut report);
    } else if data.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
        report.format = Format::OleCompound;
        if lower.ends_with(".msi") || lower.ends_with(".msp") {
            report.platform = Platform::Windows;
            report.kind = Kind::Installer;
            report.installer = Some(Installer::WindowsInstaller);
        } else {
            report
                .notes
                .push("an OLE compound file (Office document, MSI, or similar)".into());
        }
    } else if data.starts_with(&[0x4C, 0, 0, 0, 0x01, 0x14, 0x02, 0]) {
        report.format = Format::WindowsShortcut;
        report.platform = Platform::Windows;
        report.kind = Kind::Shortcut;
    } else if data.starts_with(b"#!") {
        let line = data.split(|&byte| byte == b'\n').next().unwrap_or_default();
        let interpreter = String::from_utf8_lossy(&line[2..]).trim().to_string();
        report.format = Format::Script { interpreter };
        report.platform = Platform::Any;
        report.kind = Kind::Script;
    } else if let Some(archive) = archive_kind(data, &lower) {
        report.format = Format::Archive {
            archive: archive.to_string(),
        };
        report.kind = Kind::Archive;
        match archive {
            "debian package" => {
                report.platform = Platform::Linux;
                report.kind = Kind::Installer;
                report.installer = Some(Installer::DebianPackage);
            }
            "rpm package" => {
                report.platform = Platform::Linux;
                report.kind = Kind::Installer;
                report.installer = Some(Installer::RpmPackage);
            }
            "apple disk image" => {
                report.platform = Platform::MacOs;
                report.kind = Kind::Installer;
                report.installer = Some(Installer::AppleDiskImage);
            }
            "apple installer package" => {
                report.platform = Platform::MacOs;
                report.kind = Kind::Installer;
                report.installer = Some(Installer::ApplePackage);
            }
            "java archive" => report.platform = Platform::Any,
            _ => {}
        }
    } else if let Some(language) = windows_script(&lower) {
        report.format = Format::WindowsScript {
            language: language.to_string(),
        };
        report.platform = Platform::Windows;
        report.kind = Kind::Script;
    }
    report
}

fn inspect_wasm(data: &[u8], report: &mut Report) {
    report.architectures = vec![Arch::Wasm32];
    // Bytes 4..8 are the version and layer; layer 1 is a component.
    match data.get(6..8) {
        Some([1, 0]) => {
            report.format = Format::WasmComponent;
            report.platform = Platform::Portable;
            report.kind = Kind::Application;
        }
        Some([0, 0]) => {
            report.format = Format::WasmModule;
            report.platform = Platform::Any;
            report.notes.push(
                "a core WebAssembly module: it needs a runtime such as wasmtime; Portable Host \
                 runs only components"
                    .into(),
            );
        }
        _ => report.notes.push("unrecognised WebAssembly layer".into()),
    }
}

fn archive_kind(data: &[u8], lower_name: &str) -> Option<&'static str> {
    if data.starts_with(b"PK\x03\x04") {
        return Some(if lower_name.ends_with(".jar") {
            "java archive"
        } else {
            "zip"
        });
    }
    if data.starts_with(b"!<arch>\ndebian-binary") {
        return Some("debian package");
    }
    if data.starts_with(&[0xED, 0xAB, 0xEE, 0xDB]) {
        return Some("rpm package");
    }
    if data.starts_with(b"xar!") {
        return Some("apple installer package");
    }
    // UDIF disk images end with a 512-byte "koly" trailer.
    if data.len() >= 512 && data[data.len() - 512..].starts_with(b"koly") {
        return Some("apple disk image");
    }
    let table: &[(&[u8], &str)] = &[
        (&[0x1F, 0x8B], "gzip"),
        (&[0xFD, b'7', b'z', b'X', b'Z', 0], "xz"),
        (&[0x28, 0xB5, 0x2F, 0xFD], "zstd"),
        (&[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C], "7-zip"),
        (b"Rar!\x1a\x07", "rar"),
        (b"BZh", "bzip2"),
    ];
    table
        .iter()
        .find(|(magic, _)| data.starts_with(magic))
        .map(|(_, kind)| *kind)
}

fn windows_script(lower_name: &str) -> Option<&'static str> {
    [
        (".bat", "batch"),
        (".cmd", "batch"),
        (".ps1", "powershell"),
        (".vbs", "vbscript"),
    ]
    .iter()
    .find(|(extension, _)| lower_name.ends_with(extension))
    .map(|(_, language)| *language)
}

/// The graphics API a library name implies, if any. Handles Windows DLL
/// names, Linux sonames, and macOS framework paths.
pub(crate) fn graphics_for_library(library: &str) -> Option<GraphicsApi> {
    let lower = library.to_ascii_lowercase();
    let base = lower.rsplit(['/', '\\']).next().unwrap_or(&lower);
    let api = match base {
        "ddraw.dll" => GraphicsApi::DirectDraw,
        "d3d8.dll" => GraphicsApi::Direct3D8,
        "d3d9.dll" => GraphicsApi::Direct3D9,
        "d3d10.dll" | "d3d10_1.dll" | "d3d10core.dll" => GraphicsApi::Direct3D10,
        "d3d11.dll" => GraphicsApi::Direct3D11,
        "d3d12.dll" | "d3d12core.dll" => GraphicsApi::Direct3D12,
        "dxgi.dll" => GraphicsApi::Dxgi,
        "opengl32.dll" => GraphicsApi::OpenGl,
        "vulkan-1.dll" => GraphicsApi::Vulkan,
        "metal" => GraphicsApi::Metal,
        "opengl" => GraphicsApi::OpenGl,
        _ if base.starts_with("libvulkan.so") || base.starts_with("libmoltenvk") => {
            GraphicsApi::Vulkan
        }
        _ if base.starts_with("libgl.so")
            || base.starts_with("libglx.so")
            || base.starts_with("libopengl.so") =>
        {
            GraphicsApi::OpenGl
        }
        _ if base.starts_with("libegl.so") || base.starts_with("libglesv2.so") => {
            GraphicsApi::OpenGlEs
        }
        _ => return None,
    };
    Some(api)
}

/// Record the graphics APIs implied by a set of library names.
pub(crate) fn add_graphics(report: &mut Report, libraries: &[String], linkage: Linkage) {
    for library in libraries {
        if let Some(api) = graphics_for_library(library) {
            let entry = Graphics { api, linkage };
            let known = report
                .graphics
                .iter()
                .any(|existing| existing.api == api && existing.linkage <= linkage);
            if !known {
                report.graphics.push(entry);
            }
        }
    }
    report.graphics.sort();
    report.graphics.dedup_by_key(|graphics| graphics.api);
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_simple_formats() {
        let component = inspect_bytes("app.wasm", b"\0asm\x0d\0\x01\0");
        assert_eq!(component.format, Format::WasmComponent);
        assert_eq!(component.platform, Platform::Portable);

        let module = inspect_bytes("lib.wasm", b"\0asm\x01\0\0\0");
        assert_eq!(module.format, Format::WasmModule);

        let msi = inspect_bytes(
            "Setup.MSI",
            &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0],
        );
        assert_eq!(msi.installer, Some(Installer::WindowsInstaller));
        assert_eq!(msi.platform, Platform::Windows);

        let doc = inspect_bytes(
            "report.doc",
            &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1],
        );
        assert_eq!(doc.installer, None);

        let script = inspect_bytes("run.sh", b"#!/bin/sh -e\necho hi\n");
        assert_eq!(
            script.format,
            Format::Script {
                interpreter: "/bin/sh -e".into()
            }
        );

        let batch = inspect_bytes("go.BAT", b"@echo off\r\n");
        assert_eq!(batch.platform, Platform::Windows);
        assert_eq!(batch.kind, Kind::Script);

        let deb = inspect_bytes("x.deb", b"!<arch>\ndebian-binary   ");
        assert_eq!(deb.installer, Some(Installer::DebianPackage));

        let jar = inspect_bytes("tool.jar", b"PK\x03\x04rest");
        assert_eq!(
            jar.format,
            Format::Archive {
                archive: "java archive".into()
            }
        );

        let unknown = inspect_bytes("notes.txt", b"just text");
        assert_eq!(unknown.format, Format::Unknown);
        assert_eq!(unknown.kind, Kind::Unknown);
    }

    #[test]
    fn library_names_map_to_graphics_apis() {
        assert_eq!(
            graphics_for_library("D3D12.dll"),
            Some(GraphicsApi::Direct3D12)
        );
        assert_eq!(
            graphics_for_library("libvulkan.so.1"),
            Some(GraphicsApi::Vulkan)
        );
        assert_eq!(
            graphics_for_library("/System/Library/Frameworks/Metal.framework/Versions/A/Metal"),
            Some(GraphicsApi::Metal)
        );
        assert_eq!(graphics_for_library("kernel32.dll"), None);
    }

    #[test]
    fn imported_beats_referenced() {
        let mut report = inspect_bytes("x", b"");
        add_graphics(&mut report, &["d3d11.dll".into()], Linkage::Referenced);
        add_graphics(&mut report, &["d3d11.dll".into()], Linkage::Imported);
        assert_eq!(
            report.graphics,
            vec![Graphics {
                api: GraphicsApi::Direct3D11,
                linkage: Linkage::Imported
            }]
        );
    }
}
