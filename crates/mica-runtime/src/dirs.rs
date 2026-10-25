//! Where MICA keeps its own files. Nothing outside these directories is
//! written without an explicit command and confirmation.
//!
//! | | Linux and other Unix | macOS | Windows |
//! | --- | --- | --- | --- |
//! | config | `$XDG_CONFIG_HOME/mica` | `~/Library/Application Support/MICA` | `%APPDATA%\MICA` |
//! | data (prefixes, app list) | `$XDG_DATA_HOME/mica` | `~/Library/Application Support/MICA` | `%LOCALAPPDATA%\MICA` |
//! | state (logs, undo records) | `$XDG_STATE_HOME/mica` | `~/Library/Logs/MICA` | `%LOCALAPPDATA%\MICA\state` |
//! | cache | `$XDG_CACHE_HOME/mica` | `~/Library/Caches/MICA` | `%LOCALAPPDATA%\MICA\cache` |
//!
//! `MICA_HOME` overrides all four with `config/`, `data/`, `state/`, and
//! `cache/` beneath it (portable installs and tests).

use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Dirs {
    pub config: PathBuf,
    pub data: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
}

impl Dirs {
    /// MICA's directories for the current user, from the environment.
    pub fn current() -> Dirs {
        let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty()).map(PathBuf::from);
        Dirs::resolve(std::env::consts::OS, &var)
    }

    /// Resolve the directories for `os` given an environment lookup.
    pub fn resolve(os: &str, var: &dyn Fn(&str) -> Option<PathBuf>) -> Dirs {
        if let Some(home) = var("MICA_HOME") {
            return Dirs::under(&home);
        }
        let home = var("HOME")
            .or_else(|| var("USERPROFILE"))
            .unwrap_or_else(|| PathBuf::from("."));
        match os {
            "windows" => {
                let roaming = var("APPDATA").unwrap_or_else(|| home.join(r"AppData\Roaming"));
                let local = var("LOCALAPPDATA").unwrap_or_else(|| home.join(r"AppData\Local"));
                Dirs {
                    config: roaming.join("MICA"),
                    data: local.join("MICA"),
                    state: local.join("MICA").join("state"),
                    cache: local.join("MICA").join("cache"),
                }
            }
            "macos" => {
                let library = home.join("Library");
                Dirs {
                    config: library.join("Application Support").join("MICA"),
                    data: library.join("Application Support").join("MICA"),
                    state: library.join("Logs").join("MICA"),
                    cache: library.join("Caches").join("MICA"),
                }
            }
            _ => Dirs {
                config: var("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config")).join("mica"),
                data: var("XDG_DATA_HOME")
                    .unwrap_or_else(|| home.join(".local").join("share"))
                    .join("mica"),
                state: var("XDG_STATE_HOME")
                    .unwrap_or_else(|| home.join(".local").join("state"))
                    .join("mica"),
                cache: var("XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache")).join("mica"),
            },
        }
    }

    pub fn under(root: &Path) -> Dirs {
        Dirs {
            config: root.join("config"),
            data: root.join("data"),
            state: root.join("state"),
            cache: root.join("cache"),
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.config.join("config.toml")
    }

    /// The user's own compatibility-database files.
    pub fn compat_dir(&self) -> PathBuf {
        self.config.join("compat.d")
    }

    pub fn prefixes(&self) -> PathBuf {
        self.data.join("prefixes")
    }

    pub fn apps_file(&self) -> PathBuf {
        self.data.join("apps.toml")
    }

    pub fn logs(&self) -> PathBuf {
        self.state.join("logs")
    }

    /// Records of system changes MICA made, for `--rollback`.
    pub fn undo(&self) -> PathBuf {
        self.state.join("undo")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<PathBuf> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| PathBuf::from(value))
        }
    }

    #[test]
    fn follows_each_platforms_conventions() {
        let linux = Dirs::resolve("linux", &lookup(&[("HOME", "/home/u"), ("XDG_STATE_HOME", "/st")]));
        assert_eq!(linux.config, PathBuf::from("/home/u/.config/mica"));
        assert_eq!(linux.data, PathBuf::from("/home/u/.local/share/mica"));
        assert_eq!(linux.state, PathBuf::from("/st/mica"));
        assert_eq!(linux.prefixes(), PathBuf::from("/home/u/.local/share/mica/prefixes"));

        let mac = Dirs::resolve("macos", &lookup(&[("HOME", "/Users/u")]));
        assert_eq!(mac.cache, PathBuf::from("/Users/u/Library/Caches/MICA"));

        let windows = Dirs::resolve(
            "windows",
            &lookup(&[("APPDATA", r"C:\Users\u\AppData\Roaming"), ("LOCALAPPDATA", r"C:\L")]),
        );
        assert_eq!(windows.config, PathBuf::from(r"C:\Users\u\AppData\Roaming").join("MICA"));
        assert_eq!(windows.data, PathBuf::from(r"C:\L").join("MICA"));

        let portable = Dirs::resolve("linux", &lookup(&[("MICA_HOME", "/opt/m"), ("HOME", "/h")]));
        assert_eq!(portable.config, PathBuf::from("/opt/m/config"));
    }
}
