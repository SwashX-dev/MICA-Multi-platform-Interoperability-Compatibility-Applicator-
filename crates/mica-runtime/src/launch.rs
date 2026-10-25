//! Turning a chosen backend into a concrete command, and running it.
//!
//! Commands are always built as a program plus an argument list: no shell
//! ever parses a path or argument, so file names with spaces, quotes, or `;`
//! are passed through unchanged. Every launch is recorded in MICA's log
//! directory (command, MICA's environment additions, exit status, duration,
//! and the program's error output).

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::steam::{self, CompatTool, SteamInstall};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LaunchPlan {
    /// What runs the program, for people: "Proton (GE-Proton10-25) via umu".
    pub runner: String,
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// Variables MICA adds to the inherited environment.
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
    /// The Wine prefix or bottle the program runs in.
    pub prefix: Option<PathBuf>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Outcome {
    pub code: Option<i32>,
    pub success: bool,
    pub duration: Duration,
    pub log: Option<PathBuf>,
}

/// Quote a word for display in a POSIX shell (display only; never executed).
pub fn shell_quote(word: &str) -> String {
    if !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:,+%@".contains(c))
    {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

impl LaunchPlan {
    pub fn new(runner: &str, program: impl Into<PathBuf>) -> LaunchPlan {
        LaunchPlan {
            runner: runner.into(),
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: None,
            prefix: None,
            notes: Vec::new(),
        }
    }

    pub fn arg(mut self, arg: impl Into<OsString>) -> LaunchPlan {
        self.args.push(arg.into());
        self
    }

    pub fn args<I: IntoIterator<Item = S>, S: Into<OsString>>(mut self, args: I) -> LaunchPlan {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, key: &str, value: impl AsRef<str>) -> LaunchPlan {
        self.env.retain(|(name, _)| name != key);
        self.env.push((key.to_string(), value.as_ref().to_string()));
        self
    }

    /// The command line as a person would type it (for display and logs).
    pub fn display(&self) -> String {
        let mut words: Vec<String> = self
            .env
            .iter()
            .map(|(key, value)| format!("{key}={}", shell_quote(value)))
            .collect();
        words.push(shell_quote(&self.program.to_string_lossy()));
        words.extend(self.args.iter().map(|arg| shell_quote(&arg.to_string_lossy())));
        words.join(" ")
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        for (key, value) in &self.env {
            command.env(key, value);
        }
        if let Some(cwd) = &self.cwd {
            command.current_dir(cwd);
        }
        command
    }

    /// Run the program and wait for it. Its standard output goes to the
    /// terminal; its error output goes to the terminal and the log.
    pub fn run(&self, log_dir: Option<&Path>, label: &str) -> std::io::Result<Outcome> {
        let started = Instant::now();
        let mut log = match log_dir {
            Some(dir) => {
                std::fs::create_dir_all(dir)?;
                let name = format!(
                    "{}-{}.log",
                    crate::config::now_rfc3339().replace(':', ""),
                    label
                        .chars()
                        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                        .take(40)
                        .collect::<String>()
                );
                let path = dir.join(name);
                let mut file = std::fs::File::create(&path)?;
                writeln!(file, "runner:  {}", self.runner)?;
                writeln!(file, "command: {}", self.display())?;
                if let Some(cwd) = &self.cwd {
                    writeln!(file, "cwd:     {}", cwd.display())?;
                }
                writeln!(file, "started: {}\n--- error output ---", crate::config::now_rfc3339())?;
                Some((path, file))
            }
            None => None,
        };
        let mut child = self.command().stderr(Stdio::piped()).spawn()?;
        let mut stderr = child.stderr.take().expect("piped");
        let mut copy = log.as_ref().map(|(_, file)| file.try_clone()).transpose()?;
        let tee = std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match stderr.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        let _ = std::io::stderr().write_all(&buffer[..count]);
                        if let Some(file) = copy.as_mut() {
                            let _ = file.write_all(&buffer[..count]);
                        }
                    }
                }
            }
        });
        let status: ExitStatus = child.wait()?;
        let _ = tee.join();
        let duration = started.elapsed();
        if let Some((_, file)) = log.as_mut() {
            writeln!(
                file,
                "--- end ---\nexit:    {}\nseconds: {:.3}",
                status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "terminated by a signal".into()),
                duration.as_secs_f64()
            )?;
        }
        Ok(Outcome {
            code: status.code(),
            success: status.success(),
            duration,
            log: log.map(|(path, _)| path),
        })
    }

    /// Replace this process with the program (Unix), as a launch wrapper
    /// must so that Steam tracks the game itself. Returns only on failure.
    #[cfg(unix)]
    pub fn exec(&self) -> std::io::Error {
        use std::os::unix::process::CommandExt;
        self.command().exec()
    }
}

fn parent_dir(path: &Path) -> Option<PathBuf> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
}

/// Run a program directly.
pub fn native(executable: &Path, args: &[OsString]) -> LaunchPlan {
    let mut plan = LaunchPlan::new("native", executable).args(args.iter().cloned());
    plan.cwd = parent_dir(executable);
    plan
}

/// A portable:platform WebAssembly component, through Portable Host.
pub fn portable_host(host: &Path, component: &Path) -> LaunchPlan {
    LaunchPlan::new("Portable Host (WebAssembly component)", host)
        .arg("play")
        .arg(component)
}

pub fn dosbox(dosbox: &Path, program: &Path) -> LaunchPlan {
    LaunchPlan::new("DOSBox", dosbox).arg(program)
}

/// What a Windows program needs from a Wine-family runtime.
#[derive(Clone, Debug, Default)]
pub struct WindowsTarget {
    pub executable: PathBuf,
    pub args: Vec<OsString>,
    /// Installers run through `msiexec /i` rather than directly.
    pub msi: bool,
    pub prefix: PathBuf,
    /// Steam app ID, when known, for umu's per-game fixes.
    pub steam_app_id: Option<String>,
    /// Set Valve's anti-cheat runtime paths (only when the game ships the
    /// matching anti-cheat and the runtime is installed).
    pub eac_runtime: Option<PathBuf>,
    pub battleye_runtime: Option<PathBuf>,
    pub extra_env: Vec<(String, String)>,
    pub debug: bool,
}

impl WindowsTarget {
    fn program_args(&self) -> Vec<OsString> {
        let mut args = Vec::new();
        if self.msi {
            args.push("msiexec".into());
            args.push("/i".into());
        }
        args.push(self.executable.clone().into_os_string());
        args.extend(self.args.iter().cloned());
        args
    }

    fn apply_common(&self, mut plan: LaunchPlan) -> LaunchPlan {
        plan.cwd = parent_dir(&self.executable);
        plan.prefix = Some(self.prefix.clone());
        if !self.debug {
            plan = plan.env("WINEDEBUG", "-all");
        }
        if let Some(path) = &self.eac_runtime {
            plan = plan.env("PROTON_EAC_RUNTIME", path.to_string_lossy());
            plan.notes.push(
                "Valve's Proton EasyAntiCheat Runtime is enabled for this game; it works only if \
                 the developer enabled EAC for Proton."
                    .into(),
            );
        }
        if let Some(path) = &self.battleye_runtime {
            plan = plan.env("PROTON_BATTLEYE_RUNTIME", path.to_string_lossy());
            plan.notes.push(
                "Valve's Proton BattlEye Runtime is enabled for this game; it works only if the \
                 developer enabled BattlEye for Proton."
                    .into(),
            );
        }
        for (key, value) in &self.extra_env {
            plan = plan.env(key, value);
        }
        plan
    }
}

/// System Wine with a private prefix.
pub fn wine(wine: &Path, target: &WindowsTarget) -> LaunchPlan {
    let plan = LaunchPlan::new("Wine", wine)
        .env("WINEPREFIX", target.prefix.to_string_lossy())
        .args(target.program_args());
    target.apply_common(plan)
}

/// Proton outside Steam through umu-launcher, which provides the Steam
/// Linux Runtime and Proton's expected environment. Without a chosen Proton,
/// umu fetches its own UMU-Proton build.
pub fn umu(umu: &Path, proton: Option<&CompatTool>, target: &WindowsTarget) -> LaunchPlan {
    let game_id = target
        .steam_app_id
        .as_ref()
        .map(|id| format!("umu-{id}"))
        .unwrap_or_else(|| "umu-default".into());
    let mut plan = LaunchPlan::new(
        &match proton {
            Some(tool) => format!("Proton ({}) via umu-launcher", tool.display_name),
            None => "Proton (UMU-Proton, fetched by umu-launcher) via umu-launcher".into(),
        },
        umu,
    )
    .env("WINEPREFIX", target.prefix.to_string_lossy())
    .env("GAMEID", game_id);
    if let Some(tool) = proton {
        plan = plan.env("PROTONPATH", tool.dir.to_string_lossy());
    }
    target.apply_common(plan.args(target.program_args()))
}

/// Proton directly, the way Steam starts it, without umu. Proton keeps its
/// prefix in `<compat data>/pfx`.
pub fn proton(
    install: &SteamInstall,
    tool: &CompatTool,
    target: &WindowsTarget,
) -> Result<LaunchPlan, String> {
    let mut args: Vec<String> = Vec::new();
    if target.msi {
        args.extend(["msiexec".to_string(), "/i".to_string()]);
    }
    args.push(target.executable.to_string_lossy().into_owned());
    args.extend(target.args.iter().map(|arg| arg.to_string_lossy().into_owned()));
    let command = steam::proton_command(install, tool, "waitforexitandrun", &args)?;
    let (program, rest) = command.split_first().expect("non-empty");
    let plan = LaunchPlan::new(&format!("Proton ({})", tool.display_name), program)
        .args(rest.iter().cloned())
        .env("STEAM_COMPAT_DATA_PATH", target.prefix.to_string_lossy())
        .env("STEAM_COMPAT_CLIENT_INSTALL_PATH", install.root.to_string_lossy());
    let mut plan = target.apply_common(plan);
    plan.prefix = Some(target.prefix.join("pfx"));
    Ok(plan)
}

/// CrossOver's command-line Wine with a named bottle (macOS).
pub fn crossover(wine: &Path, bottle: &str, target: &WindowsTarget) -> LaunchPlan {
    let plan = LaunchPlan::new(&format!("CrossOver (bottle \"{bottle}\")"), wine)
        .arg("--bottle")
        .arg(bottle)
        .args(target.program_args());
    let mut plan = target.apply_common(plan);
    plan.prefix = None;
    plan
}

/// A Linux program on Windows, through WSL.
pub fn wsl(wsl: &Path, program: &Path, args: &[OsString]) -> LaunchPlan {
    let linux = windows_to_wsl_path(&program.to_string_lossy());
    LaunchPlan::new("Windows Subsystem for Linux", wsl)
        .arg("--exec")
        .arg(linux)
        .args(args.iter().cloned())
}

/// An x86 Linux program on ARM64 Linux through FEX-Emu or Box64.
pub fn translated(translator: &Path, name: &str, program: &Path, args: &[OsString]) -> LaunchPlan {
    LaunchPlan::new(&format!("{name} (x86 translation)"), translator)
        .arg(program)
        .args(args.iter().cloned())
}

/// Open a web page with the desktop's handler (cloud gaming providers).
pub fn open_url(url: &str) -> LaunchPlan {
    match std::env::consts::OS {
        "windows" => LaunchPlan::new("web browser", "cmd").args(["/C", "start", "", url]),
        "macos" => LaunchPlan::new("web browser", "open").arg(url),
        _ => LaunchPlan::new("web browser", "xdg-open").arg(url),
    }
}

/// `C:\Users\me\a b\app` → `/mnt/c/Users/me/a b/app`, as WSL mounts drives
/// by default. `\\wsl$\Distro\path` and `\\wsl.localhost\Distro\path` map to
/// `/path`. Anything else is returned with slashes flipped.
pub fn windows_to_wsl_path(path: &str) -> String {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    for prefix in [r"\\wsl$\", r"\\wsl.localhost\"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            let inside = rest.split_once('\\').map(|(_, inner)| inner).unwrap_or("");
            return format!("/{}", inside.replace('\\', "/"));
        }
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        let drive = (bytes[0] as char).to_ascii_lowercase();
        let rest = path[2..].replace('\\', "/");
        return format!("/mnt/{drive}{}", if rest.starts_with('/') { rest } else { format!("/{rest}") });
    }
    path.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> WindowsTarget {
        WindowsTarget {
            executable: "/games/My Game/game.exe".into(),
            args: vec!["-windowed".into()],
            prefix: "/home/u/.local/share/mica/prefixes/my-game-1234".into(),
            ..WindowsTarget::default()
        }
    }

    #[test]
    fn builds_wine_and_umu_commands() {
        let plan = wine(Path::new("/usr/bin/wine"), &target());
        assert_eq!(
            plan.display(),
            "WINEPREFIX=/home/u/.local/share/mica/prefixes/my-game-1234 WINEDEBUG=-all \
             /usr/bin/wine '/games/My Game/game.exe' -windowed"
        );
        assert_eq!(plan.cwd.as_deref(), Some(Path::new("/games/My Game")));

        let mut with_eac = target();
        with_eac.eac_runtime = Some("/steam/common/Proton EasyAntiCheat Runtime".into());
        with_eac.steam_app_id = Some("1172470".into());
        let plan = umu(Path::new("/usr/bin/umu-run"), None, &with_eac);
        assert!(plan.env.contains(&("GAMEID".into(), "umu-1172470".into())));
        assert!(plan.env.iter().any(|(key, _)| key == "PROTON_EAC_RUNTIME"));
        assert!(!plan.env.iter().any(|(key, _)| key == "PROTONPATH"));
        assert!(plan.runner.contains("UMU-Proton"));

        let mut msi = target();
        msi.executable = "/dl/setup.msi".into();
        msi.args.clear();
        msi.msi = true;
        let plan = wine(Path::new("wine"), &msi);
        assert_eq!(plan.args, vec![OsString::from("msiexec"), "/i".into(), "/dl/setup.msi".into()]);
    }

    #[test]
    fn builds_proton_through_the_steam_runtime() {
        let (home, _) = crate::steam::tests::fake_steam("launch");
        let installs = crate::steam::find(&crate::steam::candidate_roots(&home, "linux"));
        let tools = crate::steam::compat_tools(&installs[0]);
        let tool = crate::steam::rank_protons(&tools, Some("Proton 9.0 (Beta)")).remove(0);
        let plan = proton(&installs[0], &tool, &target()).unwrap();
        assert!(plan.program.ends_with("_v2-entry-point"));
        assert_eq!(plan.args[0], OsString::from("--verb=waitforexitandrun"));
        assert!(plan.env.iter().any(|(key, _)| key == "STEAM_COMPAT_DATA_PATH"));
        assert!(plan.prefix.as_ref().unwrap().ends_with("pfx"));
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn translates_paths_for_wsl() {
        assert_eq!(windows_to_wsl_path(r"C:\Users\me\a b\app"), "/mnt/c/Users/me/a b/app");
        assert_eq!(windows_to_wsl_path(r"\\?\D:\games\x"), "/mnt/d/games/x");
        assert_eq!(windows_to_wsl_path(r"\\wsl$\Ubuntu\home\me\app"), "/home/me/app");
        assert_eq!(windows_to_wsl_path(r"\\wsl.localhost\kali-linux\usr\bin\ls"), "/usr/bin/ls");
        let plan = wsl(Path::new(r"C:\Windows\System32\wsl.exe"), Path::new(r"C:\tools\hello"), &["x".into()]);
        assert_eq!(plan.args, vec![OsString::from("--exec"), "/mnt/c/tools/hello".into(), "x".into()]);
    }

    #[test]
    fn quoting_is_for_display_only() {
        assert_eq!(shell_quote("plain"), "plain");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("a;rm -rf"), "'a;rm -rf'");
    }

    #[test]
    fn runs_and_logs_a_real_program() {
        let dir = std::env::temp_dir().join(format!("mica-launch-{}", std::process::id()));
        let plan = if cfg!(windows) {
            LaunchPlan::new("native", "cmd").args(["/C", "echo problem 1>&2 & exit 3"])
        } else {
            LaunchPlan::new("native", "sh").args(["-c", "echo problem >&2; exit 3"])
        };
        let outcome = plan.run(Some(&dir), "test program").unwrap();
        assert_eq!(outcome.code, Some(3));
        assert!(!outcome.success);
        let log = std::fs::read_to_string(outcome.log.unwrap()).unwrap();
        assert!(log.contains("problem"));
        assert!(log.contains("exit:    3"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
