//! Portable Host: runs `portable:platform` application components natively.
//!
//! The crate is organised around one boundary. [`runtime`] owns everything that
//! touches the WebAssembly component: compiling it, granting it capabilities,
//! bounding its resources, and calling it. The other modules are backends that
//! feed an application input and do something with its frames:
//!
//! * [`headless`] drives it from a [`script::Script`] and records a
//!   [`trace::Trace`], with no clock and no display;
//! * `sdl` (feature `sdl`) drives it from the keyboard and draws into a window,
//!   using [`pacer`] to turn real time into fixed simulation ticks.

pub mod headless;
pub mod pacer;
pub mod runtime;
pub mod script;
#[cfg(feature = "sdl")]
pub mod sdl;
pub mod trace;

use sha2::{Digest, Sha256};

/// Lowercase hex SHA-256, the fingerprint format used throughout.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
