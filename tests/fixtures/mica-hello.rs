//! A small Windows program, and its own "installer", for MICA's end-to-end
//! tests of the Windows backend (docs/VALIDATION.md).
//!
//!     rustc --edition 2024 -O -C target-feature=+crt-static --target x86_64-pc-windows-msvc \
//!         tests/fixtures/mica-hello.rs -o mica-hello-setup.exe
//!
//! Named `*setup*` (or given `--install`), it behaves like an installer: it
//! copies itself to `%ProgramFiles%\MICA Hello\mica-hello.exe` and writes a
//! Start-menu shortcut to that copy. Otherwise it prints where it runs, its
//! arguments, and the Wine version when Wine's ntdll exports one.

use std::ffi::{CStr, c_char, c_void};
use std::path::Path;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleA(name: *const c_char) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// Wine's ntdll exports `wine_get_version`; Windows' does not.
fn wine_version() -> Option<String> {
    // SAFETY: both calls take NUL-terminated strings, and a non-null result
    // of GetProcAddress for this name is Wine's `const char *(void)`.
    unsafe {
        let ntdll = GetModuleHandleA(c"ntdll.dll".as_ptr());
        if ntdll.is_null() {
            return None;
        }
        let address = GetProcAddress(ntdll, c"wine_get_version".as_ptr());
        if address.is_null() {
            return None;
        }
        let function: extern "C" fn() -> *const c_char = std::mem::transmute(address);
        Some(CStr::from_ptr(function()).to_string_lossy().into_owned())
    }
}

/// A Shell Link ([MS-SHLLINK]) whose LinkInfo names `target`.
fn shortcut(target: &str) -> Vec<u8> {
    let mut data = vec![0u8; 0x4C];
    data[0..4].copy_from_slice(&0x4Cu32.to_le_bytes());
    // LinkCLSID 00021401-0000-0000-C000-000000000046
    data[4..20].copy_from_slice(&[
        0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
    ]);
    data[0x14..0x18].copy_from_slice(&0x02u32.to_le_bytes()); // HasLinkInfo
    let header = 28u32;
    let suffix = header + target.len() as u32 + 1;
    data.extend_from_slice(&(suffix + 1).to_le_bytes());
    data.extend_from_slice(&header.to_le_bytes());
    data.extend_from_slice(&1u32.to_le_bytes()); // VolumeIDAndLocalBasePath
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&header.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&suffix.to_le_bytes());
    data.extend_from_slice(target.as_bytes());
    data.extend_from_slice(&[0, 0]);
    data
}

fn install() -> std::io::Result<()> {
    let program_files = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
    let program_data = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into());
    let dir = Path::new(&program_files).join("MICA Hello");
    std::fs::create_dir_all(&dir)?;
    let installed = dir.join("mica-hello.exe");
    std::fs::copy(std::env::current_exe()?, &installed)?;
    let menu = Path::new(&program_data).join(r"Microsoft\Windows\Start Menu\Programs");
    std::fs::create_dir_all(&menu)?;
    std::fs::write(menu.join("MICA Hello.lnk"), shortcut(&installed.to_string_lossy()))?;
    println!("mica-hello: installed {}", installed.display());
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let exe = std::env::current_exe().unwrap_or_default();
    let name = exe
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if args.iter().any(|arg| arg == "--install") || name.contains("setup") {
        if let Err(error) = install() {
            eprintln!("mica-hello: install failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    println!("mica-hello: a Windows x86-64 program is running");
    println!("  exe:  {}", exe.display());
    println!("  args: {:?}", &args[1..]);
    match wine_version() {
        Some(version) => println!("  under Wine {version}"),
        None => println!("  under Windows (no wine_get_version export)"),
    }
}
