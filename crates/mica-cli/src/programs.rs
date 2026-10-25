//! `mica inspect`, `plan`, `run`, `install`, `open`, and `apps`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use serde::Serialize;

use mica_engine::{Backend, Candidate, Decision, Target};
use mica_inspect::{Format, Installer, Kind, Platform, Report};
use mica_probe::{Family, Os};
use mica_runtime::config::{self, AppRecord, Apps};

use crate::session::Session;
use crate::ui::{self, line, more, para};

/// Exit code when no backend can run the program.
pub const CANNOT_RUN: u8 = 2;

fn format_summary(report: &Report) -> String {
    match &report.format {
        Format::Pe(pe) => format!(
            "Windows PE, {}-bit, {}{}{}",
            pe.bits,
            pe.subsystem,
            if pe.dll { ", DLL" } else { "" },
            if pe.dotnet { ", .NET" } else { "" }
        ),
        Format::Legacy { header } => format!("legacy {header} executable"),
        Format::Elf(elf) => format!(
            "ELF {}-bit {}{}",
            elf.bits,
            elf.elf_type,
            if elf.appimage.is_some() { " (AppImage)" } else { "" }
        ),
        Format::MachO(macho) => format!("Mach-O {}", macho.file_type),
        Format::MachOUniversal { slices } => format!("universal Mach-O ({} architectures)", slices.len()),
        Format::AppBundle(bundle) => format!("macOS app bundle (executable {})", bundle.executable),
        Format::WasmComponent => "WebAssembly component".into(),
        Format::WasmModule => "WebAssembly core module".into(),
        Format::OleCompound => "OLE compound file".into(),
        Format::Script { interpreter } => format!("script for {interpreter}"),
        Format::WindowsScript { language } => format!("Windows {language} script"),
        Format::WindowsShortcut => "Windows shortcut".into(),
        Format::Archive { archive } => archive.clone(),
        Format::Unknown => "unrecognised".into(),
    }
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Application => "application",
        Kind::Library => "library",
        Kind::Installer => "installer",
        Kind::Script => "script",
        Kind::Archive => "archive",
        Kind::Shortcut => "shortcut",
        Kind::Unknown => "unknown",
    }
}

fn print_target(target: &Target) {
    let report = &target.report;
    line("file", target.path.display());
    if let Some(sha256) = &report.sha256 {
        more(format!("sha256 {sha256}"));
    }
    more(format!("{} bytes{}", report.size, if report.truncated { " (partly read)" } else { "" }));
    line("format", format_summary(report));
    line("platform", report.platform);
    line(
        "arch",
        report
            .architectures
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", "),
    );
    line("kind", kind_name(report.kind));
    if let Some(installer) = report.installer {
        line("installer", installer);
    }
    if !report.graphics.is_empty() {
        line(
            "graphics",
            report
                .graphics
                .iter()
                .map(|graphics| format!("{} ({:?})", graphics.api, graphics.linkage).to_lowercase())
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    if !report.libraries.is_empty() {
        let shown: Vec<&str> = report.libraries.iter().take(12).map(String::as_str).collect();
        let rest = report.libraries.len().saturating_sub(shown.len());
        para(
            "libraries",
            &format!(
                "{}{}",
                shown.join(", "),
                if rest > 0 { format!(" (+{rest} more)") } else { String::new() }
            ),
        );
    }
    if let Some(scan) = &target.scan {
        if scan.signals.is_empty() {
            line("anti-cheat", format!("none found ({} files checked)", scan.entries_scanned));
        } else {
            for anticheat in scan.detected() {
                let markers: Vec<&str> = scan
                    .signals
                    .iter()
                    .filter(|signal| signal.anticheat == anticheat)
                    .map(|signal| signal.marker.as_str())
                    .take(3)
                    .collect();
                line("anti-cheat", format!("{anticheat} ({})", markers.join(", ")));
            }
            if scan.detected().contains(&mica_inspect::AntiCheat::EasyAntiCheat) {
                more(format!(
                    "EAC's Proton library (easyanticheat_x64.so): {}",
                    if scan.eac_proton_library { "present" } else { "absent" }
                ));
            }
        }
    }
    if let Some(game) = &target.game {
        line("game", format!("{} — in MICA's compatibility database ({})", game.name, game.id));
    }
    if let Some(id) = &target.steam_app_id {
        line("steam", format!("app {id}"));
    }
    for note in &report.notes {
        para("note", note);
    }
}

pub fn inspect(session: &Session, path: &Path, json: bool) -> Result<ExitCode> {
    let target = mica_engine::analyze_with(path, session.database(), mica_inspect::Options::FULL)
        .with_context(|| format!("cannot inspect {}", path.display()))?;
    if json {
        ui::json(&target)?;
    } else {
        print_target(&target);
    }
    Ok(ExitCode::SUCCESS)
}

fn print_candidate(candidate: &Candidate, chosen: bool) {
    let runner = candidate
        .runner
        .as_ref()
        .map(|runner| format!(" — {runner}"))
        .unwrap_or_default();
    let marker = if chosen {
        "=>"
    } else if candidate.eligible() {
        " +"
    } else {
        " -"
    };
    println!("{marker} {}{runner}", candidate.backend);
    for reason in &candidate.reasons {
        println!("     because  {}", ui::wrap(reason, 15, 100));
    }
    for warning in &candidate.warnings {
        println!("     warning  {}", ui::wrap(warning, 15, 100));
    }
    for blocker in &candidate.blockers {
        let tag = match blocker.kind {
            mica_engine::BlockerKind::Policy if !blocker.overridable => "blocked",
            mica_engine::BlockerKind::Policy => "policy",
            mica_engine::BlockerKind::MissingRuntime => "missing",
            mica_engine::BlockerKind::Architecture => "cpu",
            mica_engine::BlockerKind::Technical => "cannot",
            mica_engine::BlockerKind::NotConfigured => "setup",
        };
        println!("     {tag:<8} {}", ui::wrap(&blocker.message, 15, 100));
    }
    for step in &candidate.setup {
        println!("     to fix   {}", ui::wrap(step, 15, 100));
    }
    if let Some(policy) = &candidate.policy {
        for source in policy.sources.iter().take(4) {
            println!("     source   {source}");
        }
    }
}

fn print_decision(session: &Session, target: &Target, decision: &Decision) {
    let host = session.host_light();
    line(
        "program",
        format!(
            "{} — {} {} for {}",
            target.path.display(),
            target.report.platform,
            kind_name(target.report.kind),
            target
                .report
                .architectures
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );
    line("host", format!("{} ({}, {})", host.os_name, host.os, host.arch));
    if !decision.anticheats.is_empty() {
        line(
            "anti-cheat",
            decision
                .anticheats
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    println!();
    if decision.chosen().is_none() {
        println!("No backend can run this here. What MICA considered:");
    }
    for (index, candidate) in decision.candidates.iter().enumerate() {
        print_candidate(candidate, Some(index) == decision.chosen);
    }
    for note in &decision.notes {
        println!();
        para("note", note);
    }
}

#[derive(Serialize)]
struct PlanOutput<'a> {
    target: &'a Target,
    decision: &'a Decision,
    command: Option<mica_runtime::LaunchPlan>,
}

pub fn plan(session: &Session, path: &Path, backend: Option<Backend>, json: bool) -> Result<ExitCode> {
    let target = mica_engine::analyze(path, session.database())
        .with_context(|| format!("cannot inspect {}", path.display()))?;
    let decision = mica_engine::decide(&session.inputs(), &target, backend);
    let command = decision.chosen().and_then(|candidate| {
        mica_engine::prepare(
            candidate,
            &target,
            &[],
            session.runtimes(),
            &session.dirs,
            None,
            &session.config.windows.env,
        )
        .ok()
    });
    if json {
        ui::json(&PlanOutput {
            target: &target,
            decision: &decision,
            command,
        })?;
    } else {
        print_decision(session, &target, &decision);
        if let Some(command) = command {
            println!();
            line("command", &command.display());
            if let Some(prefix) = &command.prefix {
                line("prefix", prefix.display());
            }
        }
    }
    Ok(if decision.chosen().is_some() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(CANNOT_RUN)
    })
}

/// Resolve `mica run <target>`: a file, or an app installed through MICA.
fn resolve(session: &Session, target: &str) -> Result<(PathBuf, Option<AppRecord>)> {
    let path = PathBuf::from(target);
    if path.exists() {
        return Ok((path, None));
    }
    let apps = Apps::load(&session.dirs.apps_file()).map_err(anyhow::Error::msg)?;
    if let Some(app) = apps.find(target) {
        return Ok((app.executable.clone(), Some(app.clone())));
    }
    if let Some(game) = session.database().find(target) {
        bail!(
            "{target} is not a file. {} is in MICA's compatibility database: run `mica \
             compatibility {}` to see how it can be played here, or `mica run <path to its \
             executable>`.",
            game.name,
            game.id
        );
    }
    bail!("{target} is neither a file nor an application installed through MICA (`mica apps`)")
}

pub fn run(
    session: &Session,
    target_arg: &str,
    backend: Option<Backend>,
    prefix: Option<&str>,
    dry_run: bool,
    args: &[OsString],
) -> Result<ExitCode> {
    let (path, app) = resolve(session, target_arg)?;
    let target = mica_engine::analyze(&path, session.database())
        .with_context(|| format!("cannot inspect {}", path.display()))?;
    let decision = mica_engine::decide(&session.inputs(), &target, backend);
    let Some(candidate) = decision.chosen() else {
        print_decision(session, &target, &decision);
        return Ok(ExitCode::from(CANNOT_RUN));
    };
    let prefix_name = prefix.map(str::to_string).or_else(|| {
        app.as_ref()
            .and_then(|app| app.prefix.as_ref())
            .and_then(|prefix| prefix.file_name())
            .map(|name| name.to_string_lossy().into_owned())
    });
    let plan = mica_engine::prepare(
        candidate,
        &target,
        args,
        session.runtimes(),
        &session.dirs,
        prefix_name.as_deref(),
        &session.config.windows.env,
    )
    .map_err(anyhow::Error::msg)?;
    eprintln!("mica: running with {}", plan.runner);
    for warning in &plan.notes {
        eprintln!("mica: note: {}", ui::wrap(warning, 12, 100));
    }
    if dry_run {
        println!("{}", plan.display());
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(prefix) = &plan.prefix {
        std::fs::create_dir_all(prefix.parent().unwrap_or(prefix))
            .with_context(|| format!("cannot create {}", prefix.display()))?;
    }
    let label = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "program".into());
    let outcome = plan
        .run(Some(&session.dirs.logs()), &label)
        .with_context(|| format!("cannot start {}", plan.program.display()))?;
    if let Some(log) = &outcome.log {
        eprintln!(
            "mica: exited with {} after {:.1} s; log: {}",
            outcome
                .code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "a signal".into()),
            outcome.duration.as_secs_f64(),
            log.display()
        );
    }
    Ok(match outcome.code {
        Some(0) => ExitCode::SUCCESS,
        Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        None => ExitCode::FAILURE,
    })
}

/// Windows paths a shortcut names, mapped into a Wine prefix.
fn prefix_path(prefix: &Path, windows: &str) -> Option<PathBuf> {
    let rest = windows.strip_prefix("C:\\").or_else(|| windows.strip_prefix("c:\\"))?;
    let mut path = prefix.join("drive_c");
    for part in rest.split('\\').filter(|part| !part.is_empty()) {
        path.push(part);
    }
    Some(path)
}

/// Start-menu shortcuts inside a prefix, as (name, target) pairs.
fn shortcuts(prefix: &Path) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    let mut roots = vec![prefix.join("drive_c/ProgramData/Microsoft/Windows/Start Menu/Programs")];
    if let Ok(users) = std::fs::read_dir(prefix.join("drive_c/users")) {
        for user in users.flatten() {
            roots.push(user.path().join("AppData/Roaming/Microsoft/Windows/Start Menu/Programs"));
        }
    }
    let mut stack = roots;
    let mut seen = 0;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > 5000 {
                return found;
            }
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("lnk")) {
                let Ok(data) = std::fs::read(&path) else { continue };
                let Some(shortcut) = mica_inspect::lnk::parse(&data) else { continue };
                if !shortcut.target.to_ascii_lowercase().ends_with(".exe") {
                    continue;
                }
                let lower = shortcut.target.to_ascii_lowercase();
                if lower.contains("uninst") || lower.contains("\\windows\\") {
                    continue;
                }
                if let Some(target) = prefix_path(prefix, &shortcut.target) {
                    let name = path
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    found.push((name, target));
                }
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

pub fn install(session: &Session, installer: &Path, name: Option<&str>, dry_run: bool, yes: bool) -> Result<ExitCode> {
    let target = mica_engine::analyze(installer, session.database())
        .with_context(|| format!("cannot inspect {}", installer.display()))?;
    let report = &target.report;
    let os = session.os();
    let stem = installer
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "app".into());
    let name = name.map(str::to_string).unwrap_or_else(|| {
        stem.trim_end_matches("-setup")
            .trim_end_matches("_setup")
            .trim_end_matches("Setup")
            .trim_end_matches(['-', '_', ' ', '.'])
            .to_string()
    });

    // Linux packages go through the package manager.
    if matches!(report.installer, Some(Installer::DebianPackage | Installer::RpmPackage)) {
        if os != Os::Linux {
            bail!("{} is a Linux package; install it inside Linux (on Windows: inside WSL)", installer.display());
        }
        let family = session.host_light().distro.as_ref().map(|distro| distro.family);
        let absolute = installer.canonicalize()?;
        let command: Vec<String> = match (report.installer, family) {
            (Some(Installer::DebianPackage), Some(Family::Debian)) => {
                vec!["sudo".into(), "apt-get".into(), "install".into(), "-y".into(), absolute.to_string_lossy().into()]
            }
            (Some(Installer::RpmPackage), Some(Family::Fedora)) => {
                vec!["sudo".into(), "dnf".into(), "install".into(), "-y".into(), absolute.to_string_lossy().into()]
            }
            (Some(Installer::RpmPackage), Some(Family::Suse)) => {
                vec!["sudo".into(), "zypper".into(), "install".into(), absolute.to_string_lossy().into()]
            }
            _ => bail!(
                "{} does not match this distribution's package format; look for a package made \
                 for {}",
                installer.display(),
                session.host_light().os_name
            ),
        };
        println!("This package is installed by your system's package manager:");
        println!("  {}", command.join(" "));
        println!("Only install packages from publishers you trust: they run as root.");
        if dry_run || !ui::confirm("Run it now?", yes) {
            return Ok(ExitCode::SUCCESS);
        }
        let status = std::process::Command::new(&command[0]).args(&command[1..]).status()?;
        return Ok(if status.success() { ExitCode::SUCCESS } else { ExitCode::FAILURE });
    }
    if report.installer == Some(Installer::AppleDiskImage) || report.installer == Some(Installer::ApplePackage) {
        if os != Os::MacOs {
            bail!("{} is a macOS installer and needs macOS", installer.display());
        }
        println!("Opening {} with macOS; follow its instructions.", installer.display());
        if !dry_run {
            std::process::Command::new("open").arg(installer).status()?;
        }
        return Ok(ExitCode::SUCCESS);
    }
    if report.platform != Platform::Windows {
        bail!(
            "{} is not an installer MICA recognises ({}); for programs, use `mica run`",
            installer.display(),
            format_summary(report)
        );
    }
    if !report.is_installer() {
        eprintln!(
            "mica: {} does not look like an installer; running it in its own prefix anyway",
            installer.display()
        );
    }

    let decision = mica_engine::decide(&session.inputs(), &target, None);
    let Some(candidate) = decision.chosen() else {
        print_decision(session, &target, &decision);
        return Ok(ExitCode::from(CANNOT_RUN));
    };
    let prefix_name = config::app_id(&name, &installer.canonicalize().unwrap_or(installer.to_path_buf()));
    let plan = mica_engine::prepare(
        candidate,
        &target,
        &[],
        session.runtimes(),
        &session.dirs,
        (candidate.backend == Backend::WindowsCompat).then_some(prefix_name.as_str()),
        &session.config.windows.env,
    )
    .map_err(anyhow::Error::msg)?;
    println!("Installing {name} with {}.", plan.runner);
    if let Some(prefix) = &plan.prefix {
        println!("Its files go into a private prefix: {}", prefix.display());
    }
    if dry_run {
        println!("{}", plan.display());
        return Ok(ExitCode::SUCCESS);
    }
    let outcome = plan.run(Some(&session.dirs.logs()), &format!("install-{name}"))?;
    if !outcome.success {
        eprintln!(
            "mica: the installer exited with {:?}; log: {}",
            outcome.code,
            outcome.log.as_ref().map(|log| log.display().to_string()).unwrap_or_default()
        );
    }
    if os == Os::Windows {
        return Ok(if outcome.success { ExitCode::SUCCESS } else { ExitCode::FAILURE });
    }
    let Some(prefix) = plan.prefix.clone() else {
        return Ok(ExitCode::SUCCESS);
    };
    let found = shortcuts(&prefix);
    if found.is_empty() {
        println!("The installer created no Start-menu shortcuts MICA could follow.");
        println!("Run the installed program with: mica run --prefix {prefix_name} <path to its .exe>");
        return Ok(ExitCode::SUCCESS);
    }
    let mut apps = Apps::load(&session.dirs.apps_file()).map_err(anyhow::Error::msg)?;
    for (shortcut_name, executable) in &found {
        let record = AppRecord {
            id: config::app_id(shortcut_name, executable),
            name: shortcut_name.clone(),
            executable: executable.clone(),
            prefix: Some(prefix.clone()),
            backend: candidate.backend.id().into(),
            installed_from: Some(installer.to_path_buf()),
            added: config::now_rfc3339(),
        };
        println!("added      {} ({}) — run it with: mica run \"{}\"", record.name, record.id, record.name);
        apps.upsert(record);
    }
    apps.save(&session.dirs.apps_file())?;
    Ok(ExitCode::SUCCESS)
}

/// The desktop entry's handler: installers install, everything else runs.
/// Errors go to a desktop notification as well as the log.
pub fn open(session: &Session, path: &Path) -> Result<ExitCode> {
    let report = mica_inspect::inspect_path(path)?;
    let result = if report.is_installer() {
        install(session, path, None, false, true)
    } else {
        run(session, &path.to_string_lossy(), None, None, false, &[])
    };
    let failed = match &result {
        Ok(code) => *code != ExitCode::SUCCESS,
        Err(_) => true,
    };
    if failed && session.os() == Os::Linux {
        let message = match &result {
            Err(error) => format!("{error:#}"),
            Ok(_) => format!(
                "{} could not be run; open a terminal and run `mica plan \"{}\"` for details.",
                path.display(),
                path.display()
            ),
        };
        let _ = std::process::Command::new("notify-send")
            .args(["--app-name=MICA", "MICA could not open this file"])
            .arg(message)
            .status();
    }
    result
}

pub fn apps_list(session: &Session, json: bool) -> Result<ExitCode> {
    let apps = Apps::load(&session.dirs.apps_file()).map_err(anyhow::Error::msg)?;
    if json {
        ui::json(&apps)?;
        return Ok(ExitCode::SUCCESS);
    }
    if apps.apps.is_empty() {
        println!("No applications installed through MICA yet. Install one with: mica install <installer>");
    }
    for app in &apps.apps {
        println!("{}  (id {})", app.name, app.id);
        more(app.executable.display());
    }
    Ok(ExitCode::SUCCESS)
}

pub fn apps_remove(session: &Session, query: &str, delete_prefix: bool, yes: bool) -> Result<ExitCode> {
    let path = session.dirs.apps_file();
    let mut apps = Apps::load(&path).map_err(anyhow::Error::msg)?;
    let Some(app) = apps.find(query).cloned() else {
        bail!("no application {query}; `mica apps` lists them");
    };
    apps.remove(&app.id);
    apps.save(&path)?;
    println!("removed    {} from MICA's list", app.name);
    if delete_prefix {
        if let Some(prefix) = &app.prefix {
            let shared = apps.apps.iter().any(|other| other.prefix.as_ref() == Some(prefix));
            if shared {
                println!("kept       {} (other applications use it)", prefix.display());
            } else if prefix.starts_with(session.dirs.prefixes())
                && ui::confirm(&format!("Delete {} and everything installed in it?", prefix.display()), yes)
            {
                std::fs::remove_dir_all(prefix)?;
                println!("deleted    {}", prefix.display());
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_shortcut_targets_into_prefixes() {
        let prefix = Path::new("/p");
        assert_eq!(
            prefix_path(prefix, r"C:\Program Files\App\app.exe"),
            Some(PathBuf::from("/p/drive_c/Program Files/App/app.exe"))
        );
        assert_eq!(prefix_path(prefix, r"D:\elsewhere.exe"), None);
    }
}
