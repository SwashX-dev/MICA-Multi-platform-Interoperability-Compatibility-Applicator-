//! Fixed-timestep scheduling: turns real elapsed time into whole ticks.
//!
//! The simulation only ever advances in whole ticks, so *what* it computes does
//! not depend on frame rate, timing jitter, or the machine's speed. The clock
//! decides only *when* ticks run.

use std::time::Duration;

const NANOS_PER_SECOND: u128 = 1_000_000_000;

#[derive(Debug, Clone)]
pub struct Pacer {
    ticks_per_second: u128,
    /// Elapsed time not yet spent on ticks, in nanoseconds multiplied by the
    /// tick rate, so tick boundaries stay exact even though one tick is not a
    /// whole number of nanoseconds (1 s / 60 is not).
    carry: u128,
    max_catch_up: u32,
}

impl Pacer {
    pub fn new(ticks_per_second: u32) -> Self {
        assert!(ticks_per_second > 0, "tick rate must be positive");
        Self {
            ticks_per_second: ticks_per_second.into(),
            carry: 0,
            max_catch_up: (ticks_per_second / 4).max(1),
        }
    }

    /// Account for `elapsed` real time and return how many ticks are now due.
    ///
    /// After a long stall (a debugger break, a dragged window) it returns at
    /// most a quarter second of ticks and drops the rest of the backlog, so the
    /// simulation slows down rather than freezing while it catches up.
    pub fn advance(&mut self, elapsed: Duration) -> u32 {
        self.carry += elapsed.as_nanos() * self.ticks_per_second;
        let due = self.carry / NANOS_PER_SECOND;
        self.carry %= NANOS_PER_SECOND;
        match u32::try_from(due) {
            Ok(due) if due <= self.max_catch_up => due,
            _ => self.max_catch_up,
        }
    }

    /// Forget partially elapsed time, for example when resuming from a pause.
    pub fn reset(&mut self) {
        self.carry = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticks_for(pacer: &mut Pacer, frame: Duration, frames: u32) -> u64 {
        (0..frames).map(|_| u64::from(pacer.advance(frame))).sum()
    }

    #[test]
    fn tick_count_is_independent_of_frame_rate() {
        // Whatever the display refresh rate, one simulated second is 60 ticks.
        for fps in [24, 30, 59, 60, 75, 120, 144, 165, 240] {
            let mut pacer = Pacer::new(60);
            let frame = Duration::from_nanos(1_000_000_000 / fps);
            let frames = fps as u32 * 10;
            let elapsed = frame.as_nanos() * u128::from(frames);
            let expected = (elapsed * 60 / NANOS_PER_SECOND) as u64;
            assert_eq!(
                ticks_for(&mut pacer, frame, frames),
                expected,
                "at {fps} fps"
            );
            assert!((599..=600).contains(&expected));
        }
    }

    #[test]
    fn exact_rate_has_no_drift() {
        let mut pacer = Pacer::new(60);
        // 1/60 s is 16_666_666.67 ns. Alternating 16_666_667 and 16_666_666 ns
        // frames sum to exact seconds; the pacer must yield exactly one tick each.
        for i in 0..6000 {
            let nanos = if i % 3 == 2 { 16_666_666 } else { 16_666_667 };
            assert_eq!(pacer.advance(Duration::from_nanos(nanos)), 1, "frame {i}");
        }
    }

    #[test]
    fn short_frames_accumulate() {
        let mut pacer = Pacer::new(60);
        assert_eq!(pacer.advance(Duration::from_millis(10)), 0);
        assert_eq!(pacer.advance(Duration::from_millis(10)), 1);
    }

    #[test]
    fn long_stall_is_capped() {
        let mut pacer = Pacer::new(60);
        assert_eq!(pacer.advance(Duration::from_secs(5)), 15);
        assert_eq!(
            pacer.advance(Duration::ZERO),
            0,
            "backlog is dropped, not deferred"
        );
    }

    #[test]
    fn reset_forgets_partial_progress() {
        let mut pacer = Pacer::new(60);
        assert_eq!(pacer.advance(Duration::from_millis(16)), 0);
        pacer.reset();
        assert_eq!(pacer.advance(Duration::from_millis(16)), 0);
    }
}
