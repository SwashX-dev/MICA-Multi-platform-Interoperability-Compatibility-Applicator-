//! `mica steam`: status, the launch-option wrapper, and the optional
//! compatibility tool.
//!
//! Both integrations use mechanisms Steam documents for users: launch
//! options (`mica steam launch -- %command%`) and `compatibilitytools.d`.
//! MICA never edits Steam's settings or a game's files, and runs the game's
//! own command unchanged after its checks.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use serde::Serialize;

use mica_compat::Route;
use mica_engine::policy;
use mica_runtime::steam;

use crate::session::Session;
use crate::ui::{self, line, more};

#[derive(Serialize)]
struct Status<'a> {
    installs: &'a [steam::SteamInstall],
    protons: &'a [steam::CompatTool],
    anticheat_runtimes: &'a steam::AntiCheatRuntimes,
    mica_tool_installed: Vec<PathBuf>,
}

fn tool_installed(root: &Path) -> bool {
    root.join("compatibilitytools.d")
        .join(steam::MICA_TOOL_DIR)
        .join("compatibilitytool.vdf")
        .is_file()
}

pub fn status(session: &Session, json: bool) -> Result<ExitCode> {
    let runtimes = session.runtimes();
    let status = Status {
        installs: &runtimes.steam,
        protons: &runtimes.protons,
        anticheat_runtimes: &runtimes.anticheat,
        mica_tool_installed: runtimes
            .steam
            .iter()
            .filter(|install| tool_installed(&install.root))
            .map(|install| install.root.clone())
            .collect(),
    };
    if json {
        ui::json(&status)?;
        return Ok(ExitCode::SUCCESS);
    }
    if runtimes.steam.is_empty() {
        println!("Steam was not found. Install it from your distribution or https://store.steampowered.com/about/");
        return Ok(ExitCode::SUCCESS);
    }
    for install in &runtimes.steam {
        line(
            "steam",
            format!("{}{}", install.root.display(), if install.flatpak { " (Flatpak)" } else { "" }),
        );
        for library in &install.libraries {
            more(format!("library {}", library.display()));
        }
        more(format!(
            "MICA compatibility tool: {}",
            if tool_installed(&install.root) { "installed" } else { "not installed" }
        ));
    }
    for (index, proton) in runtimes.protons.iter().enumerate() {
        line(if index == 0 { "proton" } else { "" }, &proton.display_name);
    }
    line(
        "EAC",
        runtimes
            .anticheat
            .eac
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "Proton EasyAntiCheat Runtime not installed (Steam installs it for games that need it)".into()),
    );
    line(
        "BattlEye",
        runtimes
            .anticheat
            .battleye
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "Proton BattlEye Runtime not installed (Steam installs it for games that need it)".into()),
    );
    println!();
    println!("To have MICA check a game each time it starts, set its launch options to:");
    println!("  mica steam launch -- %command%");
    Ok(ExitCode::SUCCESS)
}

/// The game's directory and app ID, from Steam's environment or the command.
fn identify(command: &[OsString]) -> (Option<String>, Option<PathBuf>) {
    let app_id = ["SteamAppId", "STEAM_COMPAT_APP_ID", "SteamGameId"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty() && value != "0"));
    let install = std::env::var_os("STEAM_COMPAT_INSTALL_PATH")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .or_else(|| {
            command
                .iter()
                .map(PathBuf::from)
                .find(|path| {
                    path.components()
                        .any(|part| part.as_os_str().eq_ignore_ascii_case("common"))
                        && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
                })
                .and_then(|path| path.parent().map(Path::to_path_buf))
        });
    (app_id, install)
}

/// Check a Steam game and say what MICA concludes. Returns the lines said.
fn check_game(session: &Session, app_id: Option<&str>, install: Option<&Path>) -> Vec<String> {
    let database = session.database();
    let game = app_id.and_then(|id| database.find_by_steam_app(id));
    let scan = install.and_then(|dir| mica_inspect::anticheat::scan_dir(dir).ok());
    let verdict = policy::assess(database, game, scan.as_ref(), Route::LinuxProton);
    let mut lines = Vec::new();
    let name = game
        .map(|game| game.name.clone())
        .or_else(|| app_id.map(|id| format!("Steam app {id}")))
        .unwrap_or_else(|| "this game".into());
    let anticheats = policy::anticheats(game, scan.as_ref());
    if anticheats.is_empty() {
        lines.push(format!("{name}: no anti-cheat found."));
    } else {
        lines.push(format!(
            "{name}: anti-cheat {}.",
            anticheats.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
        ));
    }
    if verdict.forbidden() {
        lines.push(format!(
            "The publisher does not permit Linux/Proton for {name}; its anti-cheat will likely \
             refuse to start or to connect. MICA starts the game unchanged and does not work \
             around this. {}",
            verdict.reasons.join(" ")
        ));
    }
    lines.extend(verdict.warnings.iter().cloned());
    if let Some(game) = game {
        if game.eac_linux_v2.is_some() {
            lines.push(format!(
                "{} is Epic's EAC Linux v2 pilot: `mica security check --game {}` checks this \
                 system against Epic's requirements.",
                game.name, game.id
            ));
        }
    }
    lines
}

fn report(session: &Session, lines: &[String]) {
    let log = session.dirs.logs().join("steam.log");
    let _ = std::fs::create_dir_all(session.dirs.logs());
    let mut text = format!("{}\n", mica_runtime::config::now_rfc3339());
    for line in lines {
        eprintln!("MICA: {line}");
        text.push_str(&format!("  {line}\n"));
    }
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .and_then(|mut file| std::io::Write::write_all(&mut file, text.as_bytes()));
}

pub fn launch(session: &Session, command: &[OsString]) -> Result<ExitCode> {
    let (app_id, install) = identify(command);
    let lines = check_game(session, app_id.as_deref(), install.as_deref());
    report(session, &lines);
    let (program, args) = command.split_first().context("no command to run")?;
    let mut plan = mica_runtime::LaunchPlan::new("Steam", program).args(args.iter().cloned());
    plan.cwd = None;
    #[cfg(unix)]
    {
        let error = plan.exec();
        bail!("cannot start {}: {error}", PathBuf::from(program).display());
    }
    #[cfg(not(unix))]
    {
        let outcome = plan.run(None, "steam-game")?;
        Ok(match outcome.code {
            Some(0) => ExitCode::SUCCESS,
            Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
            None => ExitCode::FAILURE,
        })
    }
}

pub fn install_tool(session: &Session) -> Result<ExitCode> {
    if session.os() != mica_probe::Os::Linux {
        bail!("Steam compatibility tools exist only on Linux");
    }
    let runtimes = session.runtimes();
    let Some(install) = runtimes.steam.first() else {
        bail!("Steam was not found");
    };
    if install.flatpak {
        println!(
            "Note: Flatpak Steam runs compatibility tools inside its sandbox, where {} may not be \
             visible. The launch-option wrapper (`mica steam launch -- %command%`) needs the same \
             access.",
            session.mica_exe().display()
        );
    }
    let dir = steam::install_tool(&install.root, &session.mica_exe())?;
    line("installed", dir.display());
    println!("Restart Steam. Then, in a game's Properties > Compatibility, choose \"MICA\".");
    println!("MICA checks the game, then runs it with your best Proton (set one with");
    println!("`mica config set windows.proton \"GE-Proton10-25\"`).");
    Ok(ExitCode::SUCCESS)
}

pub fn remove_tool(session: &Session) -> Result<ExitCode> {
    let mut removed = false;
    for install in &session.runtimes().steam {
        if steam::remove_tool(&install.root)? {
            line("removed", install.root.join("compatibilitytools.d").join(steam::MICA_TOOL_DIR).display());
            removed = true;
        }
    }
    if !removed {
        println!("MICA's compatibility tool is not installed.");
    }
    Ok(ExitCode::SUCCESS)
}

/// Steam calls this as `mica steam run <verb> <args...>` for games set to
/// MICA's compatibility tool.
pub fn run(session: &Session, verb: &str, args: &[String]) -> Result<ExitCode> {
    let runtimes = session.runtimes();
    let tool = runtimes
        .protons
        .first()
        .context("MICA's Steam tool needs a Proton build; install Proton from Steam's Tools library")?;
    let install = runtimes
        .steam
        .iter()
        .find(|install| install.libraries.iter().any(|library| tool.dir.starts_with(library)) || tool.dir.starts_with(&install.root))
        .or_else(|| runtimes.steam.first())
        .context("Steam was not found")?;
    let launching = matches!(verb, "run" | "waitforexitandrun");
    let mut scan = None;
    if launching {
        let command: Vec<OsString> = args.iter().map(OsString::from).collect();
        let (app_id, game_dir) = identify(&command);
        scan = game_dir
            .as_deref()
            .and_then(|dir| mica_inspect::anticheat::scan_dir(dir).ok());
        let mut lines = check_game(session, app_id.as_deref(), game_dir.as_deref());
        lines.push(format!("running with {}", tool.display_name));
        report(session, &lines);
    }
    let command = steam::proton_command(install, tool, verb, args).map_err(anyhow::Error::msg)?;
    let (program, rest) = command.split_first().context("empty command")?;
    let mut plan = mica_runtime::LaunchPlan::new("Proton", program).args(rest.iter().cloned());
    // Steam's environment (STEAM_COMPAT_*) is inherited. Valve's anti-cheat
    // runtimes are added only for a game that ships that anti-cheat (and,
    // for EAC, its Proton library), and only if Steam has not set them.
    let detected = scan.as_ref().map(|scan| scan.detected()).unwrap_or_default();
    let eac_opted_in = scan.as_ref().is_some_and(|scan| scan.eac_proton_library);
    if std::env::var_os("PROTON_EAC_RUNTIME").is_none()
        && eac_opted_in
        && detected.contains(&mica_inspect::AntiCheat::EasyAntiCheat)
    {
        if let Some(path) = &runtimes.anticheat.eac {
            plan = plan.env("PROTON_EAC_RUNTIME", path.to_string_lossy());
        }
    }
    if std::env::var_os("PROTON_BATTLEYE_RUNTIME").is_none()
        && detected.contains(&mica_inspect::AntiCheat::BattlEye)
    {
        if let Some(path) = &runtimes.anticheat.battleye {
            plan = plan.env("PROTON_BATTLEYE_RUNTIME", path.to_string_lossy());
        }
    }
    #[cfg(unix)]
    {
        let error = plan.exec();
        bail!("cannot start Proton: {error}");
    }
    #[cfg(not(unix))]
    {
        let _ = plan;
        bail!("Steam compatibility tools exist only on Linux");
    }
}
