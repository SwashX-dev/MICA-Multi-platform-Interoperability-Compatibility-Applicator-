//! `mica compatibility <game>`: what MICA knows about a game on this host,
//! and what the user can legitimately do today.

use serde::Serialize;

use mica_compat::{AntiCheatPolicy, Authorization, Database, Game, Route, RouteResult, Status};
use mica_probe::{HostProfile, Os};
use mica_security::{Policy, Readiness, Verdict, eac_linux_v2};

#[derive(Clone, Debug, Serialize)]
pub struct GameReport {
    pub game: Game,
    pub host: Os,
    pub routes: Vec<(Route, RouteResult)>,
    pub anticheat: Vec<AntiCheatPolicy>,
    pub readiness: Option<Readiness>,
    pub recommendation: Vec<String>,
}

/// The routes that matter on a host, in the order MICA would try them.
pub fn routes_for(os: Os) -> &'static [Route] {
    match os {
        Os::Linux => &[
            Route::LinuxNative,
            Route::LinuxProton,
            Route::LinuxVm,
            Route::Cloud,
        ],
        Os::MacOs => &[
            Route::MacosNative,
            Route::MacosWine,
            Route::MacosVm,
            Route::Cloud,
        ],
        Os::Windows => &[Route::WindowsNative, Route::Cloud],
        Os::Other => &[Route::Cloud],
    }
}

pub fn report(database: &Database, game: &Game, host: &HostProfile) -> GameReport {
    let routes: Vec<(Route, RouteResult)> = routes_for(host.os)
        .iter()
        .filter_map(|route| game.route(*route).map(|result| (*route, result.clone())))
        .collect();
    let anticheat = game
        .anticheats()
        .into_iter()
        .filter_map(|anticheat| database.anticheat_policy(anticheat).cloned())
        .collect();
    let forbid_vm = game
        .route(Route::LinuxVm)
        .is_some_and(|route| route.authorization == Authorization::Forbidden);
    let readiness = (game.eac_linux_v2.is_some() && host.os == Os::Linux).then(|| {
        eac_linux_v2(
            host,
            Policy {
                forbid_virtual_machines: forbid_vm,
            },
        )
    });

    let mut recommendation = Vec::new();
    let playable = routes
        .iter()
        .find(|(_, result)| result.status == Status::VerifiedPlayable);
    if let Some((route, result)) = playable {
        recommendation.push(format!("Verified playable on {route}: {}", result.summary));
    } else if host.os == Os::Windows {
        recommendation.push(format!(
            "{} is a Windows game: install it from its official store and play natively.",
            game.name
        ));
    } else {
        let planned = routes
            .iter()
            .find(|(_, result)| result.authorization == Authorization::Planned);
        match planned {
            Some((_, result)) => recommendation.push(format!(
                "Not playable on {} today. Planned: {}",
                host.os, result.summary
            )),
            None => recommendation.push(format!(
                "{} does not permit {} on {} today, and MICA will not work around that.",
                game.publisher, game.name, host.os
            )),
        }
        if let Some(readiness) = &readiness {
            recommendation.push(match readiness.verdict {
                Verdict::Ready => {
                    "This system meets every EAC Linux v2 requirement MICA can check, \
                    so it should be ready when Epic enables the game on Linux."
                        .into()
                }
                Verdict::Incomplete => "Some EAC Linux v2 requirements could not be checked; run \
                    `sudo mica security check` for the full picture."
                    .into(),
                Verdict::NotReady => "This system does not yet meet Epic's EAC Linux v2 \
                    requirements; `mica security check` lists what to change and how."
                    .into(),
                Verdict::NotApplicable => String::new(),
            });
        }
        let mut options = vec![
            "install Windows on its own partition or drive (dual boot) and play there".to_string(),
        ];
        if let Some(cloud) = game.route(Route::Cloud)
            && !cloud.providers.is_empty()
        {
            options.push(format!(
                "play through {} (cloud streaming: the game runs remotely)",
                cloud.providers.join(", ")
            ));
        }
        recommendation.push(format!(
            "Officially supported options today: {}.",
            options.join("; or ")
        ));
    }
    recommendation.retain(|line| !line.is_empty());
    GameReport {
        game: game.clone(),
        host: host.os,
        routes,
        anticheat,
        readiness,
        recommendation,
    }
}
