//! What publishers and anti-cheat vendors permit for a program on a route.
//!
//! The answer combines the per-game database entry (most specific) with the
//! anti-cheat catalog for every anti-cheat the game ships. A prohibition the
//! publisher stated officially cannot be overridden from the command line;
//! everything weaker becomes a warning.

use serde::Serialize;

use mica_compat::{AntiCheat, Authorization, Database, EvidenceKind, Game, Route};
use mica_inspect::AntiCheatScan;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RouteVerdict {
    pub route: Route,
    pub authorization: Authorization,
    /// The prohibition rests on an official statement: no override.
    pub official_prohibition: bool,
    pub reasons: Vec<String>,
    pub warnings: Vec<String>,
    pub sources: Vec<String>,
}

impl RouteVerdict {
    pub fn forbidden(&self) -> bool {
        self.authorization == Authorization::Forbidden
    }
}

/// The anti-cheat systems relevant to a program: those the database names
/// for the game, plus those found on disk.
pub fn anticheats(game: Option<&Game>, scan: Option<&AntiCheatScan>) -> Vec<AntiCheat> {
    let mut found: Vec<AntiCheat> = game.map(Game::anticheats).unwrap_or_default();
    if let Some(scan) = scan {
        found.extend(scan.detected());
    }
    found.sort();
    found.dedup();
    found
}

/// How much a policy restricts a route, least first.
fn rank(authorization: Authorization) -> u8 {
    match authorization {
        Authorization::Authorized => 0,
        Authorization::Planned => 1,
        Authorization::PerGame => 2,
        Authorization::Unknown => 3,
        Authorization::Forbidden => 4,
    }
}

pub fn assess(
    database: &Database,
    game: Option<&Game>,
    scan: Option<&AntiCheatScan>,
    route: Route,
) -> RouteVerdict {
    let mut verdict = RouteVerdict {
        route,
        authorization: Authorization::Authorized,
        official_prohibition: false,
        reasons: Vec::new(),
        warnings: Vec::new(),
        sources: Vec::new(),
    };
    let anticheats = anticheats(game, scan);

    // The game's own entry is the most specific statement; without one,
    // each anti-cheat's catalog policy narrows what is permitted.
    let game_result = game.and_then(|game| game.route(route));
    if let Some(result) = game_result {
        verdict.authorization = result.authorization;
        verdict.reasons.push(format!(
            "{}: {} — {}",
            game.map(|game| game.name.as_str()).unwrap_or_default(),
            result.status,
            result.summary
        ));
        for evidence in &result.evidence {
            if let Some(url) = &evidence.url {
                verdict.sources.push(url.clone());
            }
        }
        verdict.official_prohibition = result.authorization == Authorization::Forbidden
            && result
                .evidence
                .iter()
                .any(|evidence| evidence.kind == EvidenceKind::Official);
        if result.authorization == Authorization::Forbidden {
            return verdict;
        }
    }
    let game_specific =
        game_result.is_some_and(|result| result.authorization != Authorization::Unknown);
    let narrow = |verdict: &mut RouteVerdict, authorization: Authorization| {
        if !game_specific && rank(authorization) > rank(verdict.authorization) {
            verdict.authorization = authorization;
        }
    };

    for anticheat in anticheats {
        let Some(policy) = database.route_policy(anticheat, route) else {
            if game_result.is_none() {
                verdict.warnings.push(format!(
                    "{anticheat} is present; MICA has no verified policy for it on {route}, so                      online play may be refused."
                ));
                narrow(&mut verdict, Authorization::Unknown);
            }
            continue;
        };
        for source in &policy.sources {
            verdict.sources.push(source.url.clone());
        }
        match policy.authorization {
            Authorization::Forbidden => {
                verdict
                    .reasons
                    .push(format!("{anticheat}: {}", policy.summary));
                if !game_specific {
                    verdict.authorization = Authorization::Forbidden;
                    verdict.official_prohibition |= policy
                        .sources
                        .iter()
                        .any(|source| source.kind == EvidenceKind::Official);
                }
            }
            Authorization::PerGame => {
                if anticheat == AntiCheat::EasyAntiCheat && route == Route::LinuxProton {
                    match scan {
                        Some(scan) if scan.eac_proton_library => verdict.reasons.push(
                            "The game ships easyanticheat_x64.so, the file Valve's documentation                              tells developers to add to enable Easy Anti-Cheat under Proton."
                                .into(),
                        ),
                        Some(_) => verdict.warnings.push(
                            "Easy Anti-Cheat is present without its Proton library                              (easyanticheat_x64.so): the developer has probably not enabled                              EAC for Proton, so online play will likely be refused."
                                .into(),
                        ),
                        None => verdict.warnings.push(
                            "Easy Anti-Cheat works under Proton only for games whose developer                              enabled it."
                                .into(),
                        ),
                    }
                } else {
                    verdict.warnings.push(format!(
                        "{anticheat}: {} Whether this game opted in is not visible to MICA.",
                        policy.summary
                    ));
                }
                narrow(&mut verdict, Authorization::PerGame);
            }
            Authorization::Unknown => {
                verdict
                    .warnings
                    .push(format!("{anticheat}: {}", policy.summary));
                narrow(&mut verdict, Authorization::Unknown);
            }
            Authorization::Planned | Authorization::Authorized => {
                verdict
                    .reasons
                    .push(format!("{anticheat}: {}", policy.summary));
            }
        }
    }
    verdict.sources.sort();
    verdict.sources.dedup();
    verdict
}

#[cfg(test)]
mod tests {
    use super::*;
    use mica_inspect::{AntiCheatSignal, Confidence};

    fn scan(anticheat: AntiCheat, proton_library: bool) -> AntiCheatScan {
        AntiCheatScan {
            signals: vec![AntiCheatSignal {
                anticheat,
                marker: "x".into(),
                confidence: Confidence::High,
            }],
            eac_proton_library: proton_library,
            entries_scanned: 10,
            truncated: false,
        }
    }

    #[test]
    fn official_game_prohibitions_cannot_be_overridden() {
        let database = Database::builtin();
        let apex = database.find("apex-legends");
        let verdict = assess(&database, apex, None, Route::LinuxProton);
        assert!(verdict.forbidden());
        assert!(verdict.official_prohibition);
        let fortnite = database.find("fortnite");
        let vm = assess(&database, fortnite, None, Route::LinuxVm);
        assert!(vm.official_prohibition);
        assert!(vm.sources.iter().any(|url| url.contains("a25107496")));
    }

    #[test]
    fn unknown_games_are_judged_by_their_anticheat() {
        let database = Database::builtin();
        let vanguard = assess(
            &database,
            None,
            Some(&scan(AntiCheat::RiotVanguard, false)),
            Route::LinuxVm,
        );
        assert!(vanguard.forbidden() && vanguard.official_prohibition);
        // Riot's Linux statement is community-sourced: forbidden, but a warning-level block.
        let proton = assess(
            &database,
            None,
            Some(&scan(AntiCheat::RiotVanguard, false)),
            Route::LinuxProton,
        );
        assert!(proton.forbidden());
        assert!(!proton.official_prohibition);

        let eac_opted_in = assess(
            &database,
            None,
            Some(&scan(AntiCheat::EasyAntiCheat, true)),
            Route::LinuxProton,
        );
        assert_eq!(eac_opted_in.authorization, Authorization::PerGame);
        assert!(eac_opted_in.warnings.is_empty());
        assert!(eac_opted_in.reasons[0].contains("easyanticheat_x64.so"));

        let eac_not = assess(
            &database,
            None,
            Some(&scan(AntiCheat::EasyAntiCheat, false)),
            Route::LinuxProton,
        );
        assert!(eac_not.warnings[0].contains("probably not enabled"));

        let clean = assess(
            &database,
            None,
            Some(&AntiCheatScan::default()),
            Route::LinuxProton,
        );
        assert_eq!(clean.authorization, Authorization::Authorized);
        assert!(clean.warnings.is_empty());

        let gameguard = assess(
            &database,
            None,
            Some(&scan(AntiCheat::NProtectGameGuard, false)),
            Route::LinuxProton,
        );
        assert!(gameguard.warnings[0].contains("no verified policy"));
    }
}
