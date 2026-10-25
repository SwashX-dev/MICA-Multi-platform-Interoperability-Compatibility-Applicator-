//! Inspection of real binaries that ship with each operating system, and a
//! mutation test: thousands of corrupted copies of real and synthetic files
//! must never make a parser panic.
//!
//! Each real-binary test runs only on the system that has the file and prints
//! which file it read, so test logs show what was actually exercised.

use std::path::Path;

#[allow(unused_imports)]
use mica_inspect::{Arch, Format, Kind, Platform, inspect_bytes, inspect_path};

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn first_existing(paths: &[&str]) -> Option<&'static Path> {
    paths
        .iter()
        .map(|path| Path::new(*path))
        .find(|path| path.is_file())
        .map(|path| Box::leak(path.to_path_buf().into_boxed_path()) as &'static Path)
}

#[test]
#[cfg(windows)]
fn windows_system_binaries() {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let notepad = Path::new(&root).join(r"System32\notepad.exe");
    let report = inspect_path(&notepad).unwrap();
    println!("{}: {:?}", notepad.display(), report.format);
    assert_eq!(report.platform, Platform::Windows);
    assert_eq!(report.kind, Kind::Application);
    assert!(
        report
            .libraries
            .iter()
            .any(|library| library.contains("kernel32") || library.starts_with("api-ms-win"))
    );

    let d3d11 = Path::new(&root).join(r"System32\d3d11.dll");
    if d3d11.is_file() {
        let report = inspect_path(&d3d11).unwrap();
        assert_eq!(report.kind, Kind::Library);
        assert!(
            report
                .libraries
                .iter()
                .any(|library| library == "dxgi.dll" || library.starts_with("api-ms-win"))
        );
    }
}

#[test]
#[cfg(target_os = "linux")]
fn linux_system_binaries() {
    let Some(shell) = first_existing(&["/bin/sh", "/usr/bin/sh", "/bin/bash"]) else {
        return;
    };
    let path = shell.canonicalize().unwrap();
    let report = inspect_path(&path).unwrap();
    println!(
        "{}: {:?} {:?}",
        path.display(),
        report.format,
        report.libraries
    );
    assert_eq!(report.platform, Platform::Linux);
    assert_eq!(report.kind, Kind::Application);
    assert_eq!(report.architectures, vec![Arch::host()]);
    match report.format {
        Format::Elf(details) => {
            if !details.statically_linked {
                assert!(details.interpreter.is_some());
                assert!(
                    report
                        .libraries
                        .iter()
                        .any(|library| library.starts_with("libc"))
                );
            }
        }
        other => panic!("{other:?}"),
    }
}

#[test]
#[cfg(target_os = "macos")]
fn macos_system_binaries() {
    let ls = Path::new("/bin/ls");
    let report = inspect_path(ls).unwrap();
    println!("{}: {:?}", ls.display(), report.format);
    assert_eq!(report.platform, Platform::MacOs);
    assert_eq!(report.kind, Kind::Application);
    assert!(
        report.architectures.contains(&Arch::Aarch64)
            || report.architectures.contains(&Arch::X86_64)
    );
    let metal = Path::new("/System/Applications/Chess.app");
    if metal.is_dir() {
        let report = inspect_path(metal).unwrap();
        println!("{}: {:?}", metal.display(), report.graphics);
        assert!(matches!(report.format, Format::AppBundle(_)));
    }
}

/// A small deterministic generator, so a failure is reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn mutate(seed: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut data = seed.to_vec();
    if data.is_empty() {
        return data;
    }
    for _ in 0..(1 + rng.next() % 16) {
        let at = (rng.next() as usize) % data.len();
        match rng.next() % 4 {
            0 => data[at] = rng.next() as u8,
            1 => data[at] = 0xFF,
            2 => {
                let value = (rng.next() as u32).to_le_bytes();
                let end = (at + 4).min(data.len());
                data[at..end].copy_from_slice(&value[..end - at]);
            }
            _ => data.truncate(at.max(1)),
        }
    }
    data
}

#[test]
fn corrupted_inputs_never_panic() {
    let mut seeds: Vec<Vec<u8>> = vec![
        b"MZ".to_vec(),
        b"\x7fELF\x02\x01\x01".to_vec(),
        vec![0xCF, 0xFA, 0xED, 0xFE, 0x0C, 0, 0, 1],
        vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 2],
        b"\0asm\x0d\0\x01\0".to_vec(),
    ];
    // Real system files make richer seeds where they exist.
    for path in [
        r"C:\Windows\System32\notepad.exe",
        "/bin/sh",
        "/bin/ls",
        "/usr/bin/env",
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            seeds.push(bytes[..bytes.len().min(256 << 10)].to_vec());
        }
    }
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut runs = 0;
    for seed in &seeds {
        for _ in 0..400 {
            let data = mutate(seed, &mut rng);
            let report = inspect_bytes("fuzz.exe", &data);
            // Whatever it decides, the report must be self-consistent.
            if report.kind == Kind::Installer {
                assert!(report.installer.is_some());
            }
            runs += 1;
        }
    }
    println!(
        "{runs} corrupted inputs inspected from {} seeds",
        seeds.len()
    );
}
