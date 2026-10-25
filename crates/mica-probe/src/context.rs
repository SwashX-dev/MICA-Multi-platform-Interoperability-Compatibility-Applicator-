//! Where probes read from: a filesystem root, the environment, and commands.
//!
//! Production code uses [`Context::live`]. Tests build a [`Context`] over a
//! temporary directory laid out like `/sys`, `/proc`, and `/etc`, with a
//! scripted command runner, so every probe is exercised without the hardware
//! it describes.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Runs a program and returns its standard output, or `None` if it is
/// missing, fails, or times out.
pub type Runner = Box<dyn Fn(&str, &[&str]) -> Option<Vec<u8>> + Send + Sync>;

pub struct Context {
    root: PathBuf,
    env: BTreeMap<String, String>,
    runner: Runner,
    /// Directories searched by [`Context::which`].
    path: Vec<PathBuf>,
}

/// How long any one helper command may run before probing gives up on it.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(8);

impl Context {
    /// The running system: `/`, the process environment, real commands.
    pub fn live() -> Context {
        let env: BTreeMap<String, String> = std::env::vars().collect();
        let path = std::env::var_os("PATH")
            .map(|value| std::env::split_paths(&value).collect())
            .unwrap_or_default();
        Context {
            root: PathBuf::from("/"),
            env,
            runner: Box::new(|program, args| run(program, args, COMMAND_TIMEOUT)),
            path,
        }
    }

    /// A fake system rooted at `root`, for tests.
    pub fn fake(root: &Path, env: &[(&str, &str)], runner: Runner) -> Context {
        let env: BTreeMap<String, String> = env
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        let path = env
            .get("PATH")
            .map(|value| value.split(':').map(|dir| root.join(dir.trim_start_matches('/'))).collect())
            .unwrap_or_default();
        Context {
            root: root.to_path_buf(),
            env,
            runner,
            path,
        }
    }

    /// A runner that knows no commands.
    pub fn no_commands() -> Runner {
        Box::new(|_, _| None)
    }

    /// The real path of an absolute path on the probed system.
    pub fn path(&self, absolute: &str) -> PathBuf {
        if self.root == Path::new("/") {
            PathBuf::from(absolute)
        } else {
            self.root.join(absolute.trim_start_matches('/'))
        }
    }

    pub fn exists(&self, absolute: &str) -> bool {
        self.path(absolute).exists()
    }

    pub fn is_dir(&self, absolute: &str) -> bool {
        self.path(absolute).is_dir()
    }

    pub fn read_bytes(&self, absolute: &str) -> Option<Vec<u8>> {
        // Some sysfs and efivarfs files report a size of 4096 or 0; read
        // rather than trusting metadata, and cap the amount.
        let mut data = Vec::new();
        std::fs::File::open(self.path(absolute))
            .ok()?
            .take(64 << 20)
            .read_to_end(&mut data)
            .ok()?;
        Some(data)
    }

    pub fn read(&self, absolute: &str) -> Option<String> {
        self.read_bytes(absolute)
            .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_string())
    }

    /// Names in a directory, sorted; empty if it cannot be read.
    pub fn list(&self, absolute: &str) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.path(absolute))
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// The final component of a symbolic link's target.
    pub fn link_name(&self, absolute: &str) -> Option<String> {
        let target = std::fs::read_link(self.path(absolute)).ok()?;
        Some(target.file_name()?.to_string_lossy().into_owned())
    }

    /// Whether the file can be opened for reading by this process.
    pub fn readable(&self, absolute: &str) -> bool {
        std::fs::File::open(self.path(absolute)).is_ok()
    }

    /// Whether the file can be opened for reading and writing (device nodes
    /// such as `/dev/kvm`). Opening a device node this way has no effect.
    pub fn read_writable(&self, absolute: &str) -> bool {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.path(absolute))
            .is_ok()
    }

    pub fn env(&self, key: &str) -> Option<&str> {
        self.env.get(key).map(String::as_str).filter(|value| !value.is_empty())
    }

    pub fn home(&self) -> Option<PathBuf> {
        self.env("HOME")
            .or_else(|| self.env("USERPROFILE"))
            .map(|home| {
                if self.root == Path::new("/") {
                    PathBuf::from(home)
                } else {
                    self.path(home)
                }
            })
    }

    /// The full path of a program on `PATH`.
    pub fn which(&self, program: &str) -> Option<PathBuf> {
        let candidates: Vec<String> = if cfg!(windows) && self.root == Path::new("/") {
            vec![
                format!("{program}.exe"),
                format!("{program}.cmd"),
                program.to_string(),
            ]
        } else {
            vec![program.to_string()]
        };
        self.path.iter().find_map(|dir| {
            candidates
                .iter()
                .map(|name| dir.join(name))
                .find(|candidate| candidate.is_file())
        })
    }

    /// Run a helper program, returning its standard output as text. Output
    /// in UTF-16LE (as some Windows tools write) is decoded.
    pub fn run(&self, program: &str, args: &[&str]) -> Option<String> {
        let bytes = (self.runner)(program, args)?;
        Some(decode_output(&bytes))
    }
}

/// Decode command output that may be UTF-8 or UTF-16LE.
pub fn decode_output(bytes: &[u8]) -> String {
    let zeros = bytes.iter().skip(1).step_by(2).filter(|&&byte| byte == 0).count();
    let utf16 = bytes.len() >= 4 && zeros * 2 >= bytes.len() / 2;
    let text = if utf16 {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    text.trim_start_matches('\u{feff}').trim().to_string()
}

/// Run `program` with `args`, wait at most `timeout`, and return its standard
/// output if it exits successfully. The program's standard input is closed.
pub fn run(program: &str, args: &[&str], timeout: Duration) -> Option<Vec<u8>> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stdout.read_to_end(&mut output);
        output
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = reader.join().ok()?;
                return status.success().then_some(output);
            }
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf16_and_utf8() {
        let utf16: Vec<u8> = "\u{feff}Ubuntu\r\n"
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        assert_eq!(decode_output(&utf16), "Ubuntu");
        assert_eq!(decode_output(b"plain text\n"), "plain text");
    }

    #[test]
    fn missing_programs_and_timeouts() {
        assert!(run("mica-no-such-program-xyz", &[], Duration::from_secs(1)).is_none());
    }
}
