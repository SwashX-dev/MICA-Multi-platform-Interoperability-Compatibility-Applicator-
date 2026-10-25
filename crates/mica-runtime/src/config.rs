//! MICA's configuration file (`config.toml`) and its list of applications
//! installed through MICA (`apps.toml`).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Which Windows runtime to prefer on Linux and macOS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PreferredRuntime {
    /// umu-launcher with Proton if present, then Proton, then Wine
    /// (Linux); CrossOver, then Wine (macOS).
    #[default]
    Auto,
    Umu,
    Proton,
    Wine,
    Crossover,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// `mica games` reads Steam, Epic, and Heroic libraries. Set to false to
    /// stop MICA from looking at them.
    pub detect_games: bool,
    pub windows: WindowsRuntimeConfig,
    pub vm: VmConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            detect_games: true,
            windows: WindowsRuntimeConfig::default(),
            vm: VmConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WindowsRuntimeConfig {
    pub runtime: PreferredRuntime,
    /// A Proton to use: a directory name (`GE-Proton10-25`,
    /// `Proton - Experimental`) or a full path.
    pub proton: Option<String>,
    /// A specific `wine` executable.
    pub wine: Option<String>,
    /// CrossOver bottle used on macOS.
    pub bottle: Option<String>,
    /// Extra environment for every Windows application, e.g. `DXVK_HUD=fps`.
    pub env: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VmConfig {
    /// A libvirt domain holding the user's own, licensed Windows.
    pub libvirt_domain: Option<String>,
    /// `qemu:///session` (default) or `qemu:///system`.
    pub libvirt_uri: Option<String>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(error) => Err(format!("{}: {error}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        write_atomically(
            path,
            &format!("# MICA configuration. `mica config show` explains each setting.\n{text}"),
        )
    }

    /// Set one dotted key from text, as `mica config set` does.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let optional =
            |value: &str| (!value.is_empty() && value != "none").then(|| value.to_string());
        match key {
            "detect_games" => {
                self.detect_games = value
                    .parse()
                    .map_err(|_| format!("{key} takes true or false"))?;
            }
            "windows.runtime" => {
                self.windows.runtime = toml::Value::String(value.to_string())
                    .try_into()
                    .map_err(|_| format!("{key} takes auto, umu, proton, wine, or crossover"))?;
            }
            "windows.proton" => self.windows.proton = optional(value),
            "windows.wine" => self.windows.wine = optional(value),
            "windows.bottle" => self.windows.bottle = optional(value),
            "vm.libvirt_domain" => self.vm.libvirt_domain = optional(value),
            "vm.libvirt_uri" => self.vm.libvirt_uri = optional(value),
            _ => return Err(format!("unknown setting {key}")),
        }
        Ok(())
    }
}

/// An application installed or registered through MICA.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppRecord {
    pub id: String,
    pub name: String,
    pub executable: PathBuf,
    /// The Wine prefix, for Windows applications.
    #[serde(default)]
    pub prefix: Option<PathBuf>,
    pub backend: String,
    #[serde(default)]
    pub installed_from: Option<PathBuf>,
    pub added: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Apps {
    #[serde(default, rename = "app")]
    pub apps: Vec<AppRecord>,
}

impl Apps {
    pub fn load(path: &Path) -> Result<Apps, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Apps::default()),
            Err(error) => Err(format!("{}: {error}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        write_atomically(
            path,
            &format!("# Applications installed through MICA.\n{text}"),
        )
    }

    pub fn find(&self, query: &str) -> Option<&AppRecord> {
        let lower = query.to_ascii_lowercase();
        self.apps
            .iter()
            .find(|app| app.id == query || app.name.to_ascii_lowercase() == lower)
    }

    /// Add or replace a record by id.
    pub fn upsert(&mut self, record: AppRecord) {
        self.apps.retain(|app| app.id != record.id);
        self.apps.push(record);
        self.apps.sort_by(|a, b| a.name.cmp(&b.name));
    }

    pub fn remove(&mut self, id: &str) -> Option<AppRecord> {
        let index = self.apps.iter().position(|app| app.id == id)?;
        Some(self.apps.remove(index))
    }
}

/// Write via a temporary file and rename, so a crash never leaves a
/// half-written file.
pub fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, contents)?;
    std::fs::rename(&temporary, path)
}

/// A stable, readable id for an application: its name, lowercased and
/// hyphenated, plus 8 hex digits of a hash of its source path.
pub fn app_id(name: &str, source: &Path) -> String {
    use sha2::{Digest, Sha256};
    let mut slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "app" } else { slug };
    let slug: String = slug.chars().take(40).collect();
    let digest = Sha256::digest(source.to_string_lossy().as_bytes());
    format!(
        "{slug}-{}",
        digest[..4]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

/// The current UTC time as RFC 3339 (`2026-10-25T14:03:00Z`).
pub fn now_rfc3339() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    rfc3339(seconds)
}

pub fn rfc3339(seconds: u64) -> String {
    // Howard Hinnant's days-to-civil algorithm.
    let days = (seconds / 86_400) as i64;
    let time = seconds % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time / 3600,
        (time / 60) % 60,
        time % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trips_and_validates() {
        let dir = std::env::temp_dir().join(format!("mica-config-{}", std::process::id()));
        let path = dir.join("config.toml");
        assert_eq!(Config::load(&path).unwrap(), Config::default());
        let mut config = Config::default();
        config.set("windows.runtime", "wine").unwrap();
        config.set("windows.proton", "GE-Proton10-25").unwrap();
        config.set("detect_games", "false").unwrap();
        assert!(config.set("windows.runtime", "magic").is_err());
        assert!(config.set("nope", "1").is_err());
        config.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded, config);
        assert_eq!(loaded.windows.runtime, PreferredRuntime::Wine);
        std::fs::write(&path, "telemetry = true\n").unwrap();
        assert!(Config::load(&path).is_err(), "unknown keys are rejected");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn apps_registry() {
        let mut apps = Apps::default();
        let record = AppRecord {
            id: app_id("Notepad++ 8.6", Path::new("/home/u/Downloads/npp.exe")),
            name: "Notepad++".into(),
            executable: "/p/drive_c/Program Files/Notepad++/notepad++.exe".into(),
            prefix: Some("/p".into()),
            backend: "windows-compat".into(),
            installed_from: Some("/home/u/Downloads/npp.exe".into()),
            added: rfc3339(0),
        };
        assert!(record.id.starts_with("notepad-8-6-"));
        assert_eq!(record.id.len(), "notepad-8-6-".len() + 8);
        apps.upsert(record.clone());
        apps.upsert(record.clone());
        assert_eq!(apps.apps.len(), 1);
        assert_eq!(apps.find("notepad++").unwrap().id, record.id);
        let dir = std::env::temp_dir().join(format!("mica-apps-{}", std::process::id()));
        apps.save(&dir.join("apps.toml")).unwrap();
        assert_eq!(Apps::load(&dir.join("apps.toml")).unwrap(), apps);
        assert!(apps.remove(&record.id).is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn formats_times() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_792_886_580), "2026-10-25T00:03:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
    }
}
