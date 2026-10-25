//! Execution runtimes and the user's games, as MICA finds and drives them.
//!
//! * [`discover`]: which runtimes are installed (Wine, umu-launcher, Proton,
//!   CrossOver, the Game Porting Toolkit, WSL, QEMU/libvirt, FEX-Emu, Box64,
//!   Rosetta 2, Portable Host, DOSBox) and Valve's anti-cheat runtimes;
//! * [`steam`] and [`epic`]: libraries and installed games, read-only, plus
//!   MICA's optional Steam compatibility tool;
//! * [`launch`]: concrete, shell-free commands for each backend, run with a
//!   log;
//! * [`packages`]: install plans that go through the system package manager;
//! * [`config`] and [`dirs`]: MICA's own settings, application list, and
//!   directories;
//! * [`desktop`]: optional file-manager integration on Linux;
//! * [`vdf`]: Valve's KeyValues format.

pub mod config;
pub mod desktop;
pub mod dirs;
pub mod discover;
pub mod epic;
pub mod launch;
pub mod packages;
pub mod steam;
pub mod vdf;

pub use config::{AppRecord, Apps, Config, PreferredRuntime};
pub use dirs::Dirs;
pub use discover::{Runtime, RuntimeKind, Runtimes, discover};
pub use launch::{LaunchPlan, Outcome, WindowsTarget};
