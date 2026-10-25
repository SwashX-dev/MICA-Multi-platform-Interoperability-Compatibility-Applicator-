//! `mica games` and `mica compatibility`.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use serde::Serialize;

use mica_compat::{Route, Status};
use mica_engine::compatibility;
use mica_inspect::AntiCheat;
use mica_probe::Os;
use mica_runtime::{epic, steam};

use crate::session::Session;
use crate::ui::{self, line, more, para};

#[derive(Serialize)]
struct GameEntry {
    store: String,
    id: String,
    name: String,
    install_dir: PathBuf,
    anticheat: Vec<AntiCheat>,
    eac_proton_library: bool,
    database_id: Option<String>,
    /// What MICA concludes for this host, in a few words.
    verdict: String,
}

fn verdict(
    session: &Session,
    database_id: Option<&str>,
    anticheat: &[AntiCheat],
    eac_library: bool,
) -> String {
    let os = session.os();
    if let Some(game) = database_id.and_then(|id| session.database().find(id)) {
        let routes = compatibility::routes_for(os);
        if let Some((route, result)) = routes
            .iter()
            .filter_map(|route| game.route(*route).map(|result| (route, result)))
            .find(|(route, _)| **route != Route::Cloud)
        {
            return format!(
                "{} on {route} (`mica compatibility {}`)",
                result.status, game.id
            );
        }
    }
    if os != Os::Linux {
        return String::new();
    }
    match anticheat {
        [] => "no anti-cheat found: Proton should be able to try it".into(),
        _ if anticheat.contains(&AntiCheat::EaJavelin)
            || anticheat.contains(&AntiCheat::RiotVanguard) =>
        {
            format!("{}: the publisher does not permit Linux", anticheat[0])
        }
        [AntiCheat::EasyAntiCheat] if eac_library => {
            "Easy Anti-Cheat with its Proton library: the developer opted in".into()
        }
        [AntiCheat::EasyAntiCheat] => {
            "Easy Anti-Cheat without its Proton library: online play will likely be refused".into()
        }
        _ => format!(
            "{}: depends on the developer enabling it for Proton",
            anticheat
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

pub fn games(session: &Session, json: bool) -> Result<ExitCode> {
    if !session.config.detect_games {
        println!("Game detection is off (`mica config set detect_games true` turns it on).");
        return Ok(ExitCode::SUCCESS);
    }
    let mut entries = Vec::new();
    for install in &session.runtimes().steam {
        for app in steam::installed_apps(install) {
            // Steam's own tools and runtimes are not games.
            let lower = app.name.to_ascii_lowercase();
            if lower.starts_with("proton")
                || lower.starts_with("steam linux runtime")
                || lower.contains("steamworks")
            {
                continue;
            }
            let scan = mica_inspect::anticheat::scan_dir(&app.install_dir).unwrap_or_default();
            let database_id = session
                .database()
                .find_by_steam_app(&app.app_id)
                .map(|game| game.id.clone());
            entries.push(GameEntry {
                store: "steam".into(),
                verdict: verdict(
                    session,
                    database_id.as_deref(),
                    &scan.detected(),
                    scan.eac_proton_library,
                ),
                id: app.app_id,
                name: app.name,
                install_dir: app.install_dir,
                anticheat: scan.detected(),
                eac_proton_library: scan.eac_proton_library,
                database_id,
            });
        }
    }
    let home = session.context.home().unwrap_or_default();
    let program_data = session.context.env("ProgramData").map(PathBuf::from);
    let epic_status = epic::detect(&home, std::env::consts::OS, program_data.as_deref());
    for game in &epic_status.games {
        let scan = mica_inspect::anticheat::scan_dir(&game.install_dir).unwrap_or_default();
        let database_id = session
            .database()
            .find(&game.app_name)
            .map(|found| found.id.clone());
        entries.push(GameEntry {
            store: game.source.clone(),
            verdict: verdict(
                session,
                database_id.as_deref(),
                &scan.detected(),
                scan.eac_proton_library,
            ),
            id: game.app_name.clone(),
            name: game.title.clone(),
            install_dir: game.install_dir.clone(),
            anticheat: scan.detected(),
            eac_proton_library: scan.eac_proton_library,
            database_id,
        });
    }
    if json {
        ui::json(&entries)?;
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(launcher) = &epic_status.launcher {
        line(
            "epic",
            format!("Epic Games Launcher found: {}", launcher.display()),
        );
    }
    if entries.is_empty() {
        println!(
            "No games found in Steam, the Epic Games Launcher, Heroic, or Legendary libraries."
        );
        return Ok(ExitCode::SUCCESS);
    }
    for entry in &entries {
        println!("{} [{} {}]", entry.name, entry.store, entry.id);
        if !entry.anticheat.is_empty() {
            more(format!(
                "anti-cheat: {}",
                entry
                    .anticheat
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !entry.verdict.is_empty() {
            more(ui::wrap(&entry.verdict, 11, 100));
        }
    }
    Ok(ExitCode::SUCCESS)
}

pub fn compatibility(session: &Session, query: &str, json: bool) -> Result<ExitCode> {
    let Some(game) = session.database().find(query) else {
        println!("{query} is not in MICA's compatibility database yet.");
        println!(
            "`mica plan <path to the game's .exe>` analyzes an installed copy: its format, graphics"
        );
        println!("APIs, the anti-cheat beside it, and which backends could run it.");
        return Ok(ExitCode::from(crate::programs::CANNOT_RUN));
    };
    let report = compatibility::report(session.database(), game, session.host());
    if json {
        ui::json(&report)?;
        return Ok(ExitCode::SUCCESS);
    }
    line("game", format!("{} ({})", game.name, game.publisher));
    if !game.anticheat.is_empty() {
        line(
            "anti-cheat",
            game.anticheats()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    line(
        "this host",
        format!("{} ({})", session.host().os_name, session.host().os),
    );
    println!();
    for (route, result) in &report.routes {
        let tag = match result.status {
            Status::VerifiedPlayable => "[PLAYABLE]",
            Status::PartiallyFunctional => "[PARTIAL ]",
            Status::Unverified => "[UNTESTED]",
            Status::BlockedByAntiCheat | Status::BlockedByPublisher => "[BLOCKED ]",
            Status::Unsupported => "[NO      ]",
        };
        println!("{tag} {route}: {}", result.status);
        println!("           publisher: {}", result.authorization);
        println!("           {}", ui::wrap(&result.summary, 11, 100));
        for evidence in &result.evidence {
            println!(
                "           {:?} source, {}: {}",
                evidence.kind,
                evidence.date,
                evidence.url.as_deref().unwrap_or("(direct test)")
            );
        }
        if let Some(test) = result.last_direct_test() {
            println!(
                "           last tested {} on {}",
                test.date,
                test.environment.as_deref().unwrap_or("?")
            );
        }
    }
    if let Some(readiness) = &report.readiness {
        println!();
        line(
            "EAC v2",
            format!(
                "{:?} — `mica security check --game {}` shows every requirement",
                readiness.verdict, game.id
            ),
        );
        for finding in &readiness.findings {
            println!("  {} {}", ui::mark(finding.status), finding.title);
        }
    }
    if !game.notes.is_empty() {
        println!();
        for note in &game.notes {
            para("note", note);
        }
    }
    println!();
    for line_text in &report.recommendation {
        para("what now", line_text);
    }
    Ok(ExitCode::SUCCESS)
}
