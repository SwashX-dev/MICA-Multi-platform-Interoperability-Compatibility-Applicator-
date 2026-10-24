//! Headless backend: scripted input in, a trace of frames out.
//!
//! It reads no clock and opens no display. Tick N always receives the script's
//! input for tick N, so a replay produces the same trace on every run and on
//! every machine.

use anyhow::Result;

use crate::runtime::{App, Input};
use crate::script::Script;
use crate::trace::Trace;

/// Run every tick of `script`, rendering after each one.
pub fn replay(app: &mut App, script: &Script) -> Result<Trace> {
    let mut trace = Trace::new();
    for held in script.inputs() {
        let tick = app.ticks();
        app.update(Input { held })?;
        let frame = app.render()?;
        trace.record(tick, held, &frame);
    }
    Ok(trace)
}
