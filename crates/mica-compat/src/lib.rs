//! MICA's compatibility database.
//!
//! Two versioned TOML files ship inside MICA (`data/anticheat.toml` and
//! `data/games.toml`): what each anti-cheat permits on each execution route,
//! and per-game results. Users may add their own files (for example their
//! own direct-test results) to MICA's configuration directory; those are
//! loaded on top and marked as local.
//!
//! The loader enforces the rules that keep the database honest (see
//! [`validate`]): a "verified playable" result needs a reproducible direct
//! test, a protected multiplayer game needs that test to have completed a
//! multiplayer session, a publisher-forbidden route can never be rated
//! playable, and every block names its source. Community reports are kept
//! apart from official statements and direct tests.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub use mica_inspect::AntiCheat;

const ANTICHEAT_TOML: &str = include_str!("../data/anticheat.toml");
const GAMES_TOML: &str = include_str!("../data/games.toml");
/// The database schema this MICA reads.
pub const SCHEMA: u32 = 1;

/// A way of running a program on some host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Route {
    WindowsNative,
    /// A native Linux build of the program.
    LinuxNative,
    /// A Windows build through Proton or Wine on Linux.
    LinuxProton,
    /// Windows in a virtual machine on Linux.
    LinuxVm,
    MacosNative,
    /// A Windows build through Wine, CrossOver, or the Game Porting Toolkit.
    MacosWine,
    MacosVm,
    /// A Linux build through WSL on Windows.
    WindowsWsl,
    /// Official cloud streaming: the game runs on someone else's computer.
    Cloud,
}

impl fmt::Display for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Route::WindowsNative => "Windows (native)",
            Route::LinuxNative => "Linux (native build)",
            Route::LinuxProton => "Linux via Proton/Wine",
            Route::LinuxVm => "Linux via a Windows virtual machine",
            Route::MacosNative => "macOS (native build)",
            Route::MacosWine => "macOS via Wine/CrossOver/GPTK",
            Route::MacosVm => "macOS via a Windows virtual machine",
            Route::WindowsWsl => "Windows via WSL",
            Route::Cloud => "cloud streaming (remote)",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    VerifiedPlayable,
    PartiallyFunctional,
    Unverified,
    BlockedByAntiCheat,
    BlockedByPublisher,
    Unsupported,
}

impl Status {
    pub fn is_blocked(self) -> bool {
        matches!(
            self,
            Status::BlockedByAntiCheat | Status::BlockedByPublisher | Status::Unsupported
        )
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Status::VerifiedPlayable => "Verified playable",
            Status::PartiallyFunctional => "Partially functional",
            Status::Unverified => "Unverified",
            Status::BlockedByAntiCheat => "Blocked by anti-cheat",
            Status::BlockedByPublisher => "Blocked by publisher",
            Status::Unsupported => "Unsupported",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Authorization {
    Authorized,
    PerGame,
    Planned,
    Forbidden,
    Unknown,
}

impl fmt::Display for Authorization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Authorization::Authorized => "authorized by the publisher",
            Authorization::PerGame => "decided per game",
            Authorization::Planned => "planned by the publisher",
            Authorization::Forbidden => "forbidden by the publisher",
            Authorization::Unknown => "no publisher statement found",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceKind {
    /// A publisher, vendor, or platform-holder statement.
    Official,
    /// A reproducible test, with the environment it ran in.
    DirectTest,
    /// A report MICA could not confirm.
    Community,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub kind: EvidenceKind,
    #[serde(default)]
    pub url: Option<String>,
    pub date: String,
    #[serde(default)]
    pub summary: Option<String>,
    /// For direct tests: OS, hardware, runtime versions.
    #[serde(default)]
    pub environment: Option<String>,
    /// For direct tests of multiplayer games: a protected online session
    /// was completed.
    #[serde(default)]
    pub multiplayer_session: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteResult {
    pub status: Status,
    pub authorization: Authorization,
    pub summary: String,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    pub issues: Vec<String>,
    #[serde(default)]
    pub workarounds: Vec<String>,
    /// Cloud services the publisher names (for [`Route::Cloud`]).
    #[serde(default)]
    pub providers: Vec<String>,
}

impl RouteResult {
    /// The most recent direct test, if any.
    pub fn last_direct_test(&self) -> Option<&Evidence> {
        self.evidence
            .iter()
            .filter(|evidence| evidence.kind == EvidenceKind::DirectTest)
            .max_by(|a, b| a.date.cmp(&b.date))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Game {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub publisher: String,
    #[serde(default)]
    pub anticheat: Vec<String>,
    #[serde(default)]
    pub multiplayer: bool,
    #[serde(default)]
    pub executables: Vec<String>,
    /// Store identifiers: `steam` app ID, `epic` app name.
    #[serde(default)]
    pub stores: BTreeMap<String, String>,
    /// Where the game stands with EAC Linux v2: "pilot", "announced", ...
    #[serde(default)]
    pub eac_linux_v2: Option<String>,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub routes: BTreeMap<Route, RouteResult>,
    /// Loaded from the user's own files rather than shipped with MICA.
    #[serde(default, skip_deserializing)]
    pub local: bool,
}

impl Game {
    pub fn route(&self, route: Route) -> Option<&RouteResult> {
        self.routes.get(&route)
    }

    pub fn anticheats(&self) -> Vec<AntiCheat> {
        self.anticheat
            .iter()
            .filter_map(|id| AntiCheat::from_id(id))
            .collect()
    }

    /// Multiplayer behind anti-cheat: the strictest evidence rules apply.
    pub fn protected_multiplayer(&self) -> bool {
        self.multiplayer && !self.anticheat.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub kind: EvidenceKind,
    pub url: String,
    pub date: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutePolicy {
    pub authorization: Authorization,
    pub summary: String,
    #[serde(default)]
    pub sources: Vec<Source>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AntiCheatPolicy {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub summary: String,
    #[serde(default)]
    pub routes: BTreeMap<Route, RoutePolicy>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AntiCheatFile {
    schema: u32,
    updated: String,
    #[serde(default)]
    anticheat: Vec<AntiCheatPolicy>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GamesFile {
    schema: u32,
    #[serde(default)]
    updated: Option<String>,
    #[serde(default)]
    game: Vec<Game>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Database {
    pub updated: String,
    pub anticheat: Vec<AntiCheatPolicy>,
    pub games: Vec<Game>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseError(pub String);

impl fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DatabaseError {}

fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

impl Database {
    /// The database shipped with this MICA.
    pub fn builtin() -> Database {
        Database::from_sources(ANTICHEAT_TOML, &[GAMES_TOML])
            .expect("the built-in compatibility database is valid (tested)")
    }

    /// The built-in database plus every `*.toml` game file in `dir`, which
    /// may add games or replace built-in entries by id. Invalid files are
    /// reported, not loaded.
    pub fn with_local(dir: &Path) -> (Database, Vec<String>) {
        let mut database = Database::builtin();
        let mut problems = Vec::new();
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        for path in files {
            let loaded = std::fs::read_to_string(&path)
                .map_err(|error| DatabaseError(error.to_string()))
                .and_then(|text| parse_games(&text))
                .and_then(|games| {
                    for game in &games {
                        validate_game(game, &database.anticheat)?;
                    }
                    Ok(games)
                });
            match loaded {
                Ok(games) => {
                    for mut game in games {
                        game.local = true;
                        database.games.retain(|existing| existing.id != game.id);
                        database.games.push(game);
                    }
                }
                Err(error) => problems.push(format!("{}: {error}", path.display())),
            }
        }
        (database, problems)
    }

    pub fn from_sources(anticheat: &str, games: &[&str]) -> Result<Database, DatabaseError> {
        let anticheat_file: AntiCheatFile = toml::from_str(anticheat)
            .map_err(|error| DatabaseError(format!("anticheat: {error}")))?;
        if anticheat_file.schema != SCHEMA {
            return Err(DatabaseError(format!(
                "anticheat: schema {} is not {SCHEMA}",
                anticheat_file.schema
            )));
        }
        let mut all = Vec::new();
        for text in games {
            all.extend(parse_games(text)?);
        }
        let database = Database {
            updated: anticheat_file.updated,
            anticheat: anticheat_file.anticheat,
            games: all,
        };
        validate(&database)?;
        Ok(database)
    }

    /// Find a game by id, name, alias, executable name, or store id.
    pub fn find(&self, query: &str) -> Option<&Game> {
        let wanted = normalize(query);
        let exe = query.to_ascii_lowercase();
        self.games.iter().find(|game| {
            normalize(&game.id) == wanted
                || normalize(&game.name) == wanted
                || game.aliases.iter().any(|alias| normalize(alias) == wanted)
                || game
                    .executables
                    .iter()
                    .any(|name| name.to_ascii_lowercase() == exe)
                || game.stores.values().any(|id| id == query)
        })
    }

    /// The game whose executable has this file name, if any.
    pub fn find_by_executable(&self, file_name: &str) -> Option<&Game> {
        let lower = file_name.to_ascii_lowercase();
        self.games.iter().find(|game| {
            game.executables
                .iter()
                .any(|name| name.to_ascii_lowercase() == lower)
        })
    }

    pub fn find_by_steam_app(&self, app_id: &str) -> Option<&Game> {
        self.games
            .iter()
            .find(|game| game.stores.get("steam").map(String::as_str) == Some(app_id))
    }

    pub fn anticheat_policy(&self, anticheat: AntiCheat) -> Option<&AntiCheatPolicy> {
        self.anticheat
            .iter()
            .find(|policy| policy.id == anticheat.id())
    }

    /// What an anti-cheat permits on a route, if the catalog says.
    pub fn route_policy(&self, anticheat: AntiCheat, route: Route) -> Option<&RoutePolicy> {
        self.anticheat_policy(anticheat)?.routes.get(&route)
    }
}

fn parse_games(text: &str) -> Result<Vec<Game>, DatabaseError> {
    let file: GamesFile =
        toml::from_str(text).map_err(|error| DatabaseError(format!("games: {error}")))?;
    if file.schema != SCHEMA {
        return Err(DatabaseError(format!(
            "games: schema {} is not {SCHEMA}",
            file.schema
        )));
    }
    let _ = file.updated;
    Ok(file.game)
}

/// The database's honesty rules.
pub fn validate(database: &Database) -> Result<(), DatabaseError> {
    for policy in &database.anticheat {
        if AntiCheat::from_id(&policy.id).is_none() {
            return Err(DatabaseError(format!(
                "anti-cheat {}: unknown id (add it to mica-inspect's AntiCheat)",
                policy.id
            )));
        }
        for (route, rule) in &policy.routes {
            if rule.authorization == Authorization::Forbidden && rule.sources.is_empty() {
                return Err(DatabaseError(format!(
                    "anti-cheat {} on {route}: \"forbidden\" needs a source",
                    policy.id
                )));
            }
        }
    }
    for game in &database.games {
        validate_game(game, &database.anticheat)?;
    }
    let mut ids: Vec<&str> = database.games.iter().map(|game| game.id.as_str()).collect();
    ids.sort_unstable();
    if let Some(pair) = ids.windows(2).find(|pair| pair[0] == pair[1]) {
        return Err(DatabaseError(format!("game {} appears twice", pair[0])));
    }
    Ok(())
}

fn validate_game(game: &Game, anticheat: &[AntiCheatPolicy]) -> Result<(), DatabaseError> {
    let fail = |route: Option<Route>, message: String| {
        Err(DatabaseError(match route {
            Some(route) => format!("game {} on {route}: {message}", game.id),
            None => format!("game {}: {message}", game.id),
        }))
    };
    for id in &game.anticheat {
        if !anticheat.iter().any(|policy| &policy.id == id) {
            return fail(None, format!("unknown anti-cheat {id}"));
        }
    }
    for (route, result) in &game.routes {
        let route = Some(*route);
        if result.status == Status::VerifiedPlayable {
            let Some(test) = result.last_direct_test() else {
                return fail(route, "verified-playable needs a direct-test result".into());
            };
            if test.environment.is_none() {
                return fail(route, "a direct test must record its environment".into());
            }
            if game.protected_multiplayer() && !test.multiplayer_session {
                return fail(
                    route,
                    "a protected multiplayer game is verified playable only after a direct test \
                     completed a multiplayer session"
                        .into(),
                );
            }
            if result.authorization == Authorization::Forbidden {
                return fail(
                    route,
                    "a route the publisher forbids cannot be rated playable".into(),
                );
            }
        }
        if result.status.is_blocked()
            && result.status != Status::Unsupported
            && result.evidence.is_empty()
        {
            return fail(route, "a block must cite its source".into());
        }
        for evidence in &result.evidence {
            if evidence.kind != EvidenceKind::DirectTest && evidence.url.is_none() {
                return fail(route, "official and community evidence need a URL".into());
            }
            if evidence.kind == EvidenceKind::DirectTest && evidence.environment.is_none() {
                return fail(route, "a direct test must record its environment".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builtin_database_loads_and_is_valid() {
        let database = Database::builtin();
        assert!(database.games.len() >= 4);
        assert!(database.anticheat.len() >= 10);
        for anticheat in AntiCheat::ALL {
            assert!(
                database.anticheat_policy(anticheat).is_some(),
                "{anticheat:?} has no catalog entry"
            );
        }
        // No game is rated playable without a direct test in this release.
        for game in &database.games {
            for (route, result) in &game.routes {
                assert_ne!(
                    result.status,
                    Status::VerifiedPlayable,
                    "{} on {route}",
                    game.id
                );
            }
        }
    }

    #[test]
    fn flagship_games_say_what_the_publishers_say() {
        let database = Database::builtin();
        let fortnite = database.find("Fortnite").unwrap();
        assert_eq!(fortnite.anticheats(), vec![AntiCheat::EasyAntiCheat]);
        assert_eq!(fortnite.eac_linux_v2.as_deref(), Some("pilot"));
        let linux = fortnite.route(Route::LinuxNative).unwrap();
        assert_eq!(linux.status, Status::BlockedByPublisher);
        assert_eq!(linux.authorization, Authorization::Planned);
        assert_eq!(
            fortnite.route(Route::LinuxVm).unwrap().authorization,
            Authorization::Forbidden
        );
        assert_eq!(
            fortnite.route(Route::Cloud).unwrap().providers,
            vec!["GeForce NOW", "Amazon Luna", "Xbox Cloud Gaming"]
        );

        let apex = database.find("apex").unwrap();
        assert_eq!(apex.anticheats(), vec![AntiCheat::EaJavelin]);
        assert!(apex.eac_linux_v2.is_none(), "Javelin is not EAC");
        assert_eq!(
            apex.route(Route::LinuxProton).unwrap().status,
            Status::BlockedByAntiCheat
        );
        assert_eq!(
            database.find_by_steam_app("1172470").unwrap().id,
            "apex-legends"
        );
        assert_eq!(database.find("r5apex.exe").unwrap().id, "apex-legends");

        let valorant = database.find("VALORANT").unwrap();
        assert_eq!(
            valorant.route(Route::LinuxVm).unwrap().authorization,
            Authorization::Forbidden
        );
        assert_eq!(
            database
                .route_policy(AntiCheat::RiotVanguard, Route::LinuxVm)
                .unwrap()
                .authorization,
            Authorization::Forbidden
        );
        assert_eq!(
            database
                .route_policy(AntiCheat::EasyAntiCheat, Route::LinuxProton)
                .unwrap()
                .authorization,
            Authorization::PerGame
        );
        assert!(database.find("not a game").is_none());
    }

    const POLICY: &str = r#"
        schema = 1
        updated = "2026-10-25"
        [[anticheat]]
        id = "easy-anti-cheat"
        name = "EAC"
        vendor = "Epic"
        summary = "s"
    "#;

    fn game_with(route: &str) -> String {
        format!(
            r#"schema = 1
            [[game]]
            id = "test"
            name = "Test"
            publisher = "P"
            anticheat = ["easy-anti-cheat"]
            multiplayer = true
            [game.routes.linux-proton]
            {route}"#
        )
    }

    #[test]
    fn verified_playable_needs_a_multiplayer_direct_test() {
        let no_test = game_with(
            r#"status = "verified-playable"
            authorization = "per-game"
            summary = "it launched""#,
        );
        let error = Database::from_sources(POLICY, &[&no_test]).unwrap_err();
        assert!(error.0.contains("needs a direct-test"), "{error}");

        let menu_only = game_with(
            r#"status = "verified-playable"
            authorization = "per-game"
            summary = "reached the menu"
            evidence = [{ kind = "direct-test", date = "2026-10-25", environment = "Fedora 43, RTX 4060" }]"#,
        );
        let error = Database::from_sources(POLICY, &[&menu_only]).unwrap_err();
        assert!(error.0.contains("multiplayer session"), "{error}");

        let played = game_with(
            r#"status = "verified-playable"
            authorization = "per-game"
            summary = "played three online matches"
            evidence = [{ kind = "direct-test", date = "2026-10-25", environment = "Fedora 43, RTX 4060", multiplayer_session = true }]"#,
        );
        let database = Database::from_sources(POLICY, &[&played]).unwrap();
        assert_eq!(
            database
                .find("test")
                .unwrap()
                .route(Route::LinuxProton)
                .unwrap()
                .status,
            Status::VerifiedPlayable
        );

        let forbidden = game_with(
            r#"status = "verified-playable"
            authorization = "forbidden"
            summary = "x"
            evidence = [{ kind = "direct-test", date = "2026-10-25", environment = "e", multiplayer_session = true }]"#,
        );
        assert!(Database::from_sources(POLICY, &[&forbidden]).is_err());
    }

    #[test]
    fn blocks_need_sources_and_ids_must_be_known() {
        let unsourced = game_with(
            r#"status = "blocked-by-anti-cheat"
            authorization = "forbidden"
            summary = "x""#,
        );
        assert!(Database::from_sources(POLICY, &[&unsourced]).is_err());
        let unknown = game_with(
            r#"status = "unverified"
            authorization = "unknown"
            summary = "x""#,
        )
        .replace("easy-anti-cheat\"]", "made-up\"]");
        assert!(Database::from_sources(POLICY, &[&unknown]).is_err());
        let typo = game_with(
            r#"status = "unverified"
            authorization = "unknown"
            summary = "x"
            stauts = "oops""#,
        );
        assert!(
            Database::from_sources(POLICY, &[&typo]).is_err(),
            "unknown fields are errors"
        );
    }

    #[test]
    fn local_files_override_and_bad_ones_are_reported() {
        let dir = std::env::temp_dir().join(format!("mica-compat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("mine.toml"),
            r#"schema = 1
            [[game]]
            id = "fortnite"
            name = "Fortnite (my notes)"
            publisher = "Epic Games"
            "#,
        )
        .unwrap();
        std::fs::write(dir.join("broken.toml"), "schema = 1\n[[game]]\nid = 3\n").unwrap();
        let (database, problems) = Database::with_local(&dir);
        let fortnite = database.find("fortnite").unwrap();
        assert!(fortnite.local);
        assert_eq!(fortnite.name, "Fortnite (my notes)");
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("broken.toml"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
