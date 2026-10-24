//! Shared helpers for the integration tests.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use portable_host::headless;
use portable_host::runtime::{App, AppComponent, Limits, Runtime};
use portable_host::script::Script;
use portable_host::trace::Trace;

pub struct Guest {
    pub runtime: Runtime,
    pub component: AppComponent,
}

/// The moving-square component, compiled once per test binary. Its path comes
/// from `PORTABLE_GUEST`, which `.cargo/config.toml` (or Nix) sets.
pub fn guest() -> &'static Guest {
    static GUEST: OnceLock<Guest> = OnceLock::new();
    GUEST.get_or_init(|| {
        let path = std::env::var_os("PORTABLE_GUEST")
            .map(PathBuf::from)
            .expect("PORTABLE_GUEST is not set; run the tests through cargo");
        let runtime = Runtime::new().expect("engine");
        let component = runtime.load_file(&path).unwrap_or_else(|error| {
            panic!("{error:?}\n\nBuild the guest component first: `just guest`")
        });
        Guest { runtime, component }
    })
}

pub fn instantiate() -> App {
    let guest = guest();
    guest
        .runtime
        .instantiate(&guest.component, Limits::default())
        .expect("instantiate moving-square")
}

pub fn replay(script: &Script) -> Trace {
    headless::replay(&mut instantiate(), script).expect("replay")
}

pub fn replay_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/replay")
}

pub fn script(name: &str) -> Script {
    let path = replay_dir().join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Script::parse(&text).unwrap()
}
