//! What every command may need, loaded on first use.

use std::cell::OnceCell;

use anyhow::{Result, anyhow};

use mica_compat::Database;
use mica_probe::{Context, HostProfile, Os};
use mica_runtime::{Config, Dirs, Runtimes};

pub struct Session {
    pub dirs: Dirs,
    pub config: Config,
    pub context: Context,
    host: OnceCell<HostProfile>,
    host_light: OnceCell<HostProfile>,
    runtimes: OnceCell<Runtimes>,
    database: OnceCell<Database>,
}

impl Session {
    pub fn new() -> Result<Session> {
        let dirs = Dirs::current();
        let config = Config::load(&dirs.config_file()).map_err(|error| anyhow!(error))?;
        Ok(Session {
            dirs,
            config,
            context: Context::live(),
            host: OnceCell::new(),
            host_light: OnceCell::new(),
            runtimes: OnceCell::new(),
            database: OnceCell::new(),
        })
    }

    pub fn os(&self) -> Os {
        Os::current()
    }

    pub fn host(&self) -> &HostProfile {
        self.host
            .get_or_init(|| mica_probe::probe_with(&self.context, Os::current()))
    }

    /// Enough of the host to choose a backend, without slow queries; the
    /// full profile when one was already gathered.
    pub fn host_light(&self) -> &HostProfile {
        if let Some(host) = self.host.get() {
            return host;
        }
        self.host_light.get_or_init(|| {
            mica_probe::probe_depth(&self.context, Os::current(), mica_probe::Depth::Light)
        })
    }

    pub fn runtimes(&self) -> &Runtimes {
        self.runtimes.get_or_init(|| {
            mica_runtime::discover(
                &self.context,
                Os::current(),
                self.config.windows.proton.as_deref(),
            )
        })
    }

    /// The built-in database plus the user's own files; problems with the
    /// user's files are printed once.
    pub fn database(&self) -> &Database {
        self.database.get_or_init(|| {
            let (database, problems) = Database::with_local(&self.dirs.compat_dir());
            for problem in problems {
                eprintln!("warning: ignoring {problem}");
            }
            database
        })
    }

    pub fn inputs(&self) -> mica_engine::Inputs<'_> {
        mica_engine::Inputs {
            host: self.host_light(),
            runtimes: self.runtimes(),
            database: self.database(),
            config: &self.config,
        }
    }

    pub fn mica_exe(&self) -> std::path::PathBuf {
        std::env::current_exe().unwrap_or_else(|_| "mica".into())
    }
}
