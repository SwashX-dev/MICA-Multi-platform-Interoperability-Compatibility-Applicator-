//! The moving-square simulation: integer state, integer math, no host APIs.
//!
//! Everything here is deterministic. The same sequence of inputs produces the
//! same sequence of frames on every host and every CPU architecture.

use crate::portable::platform::types::{Buttons, Color, DrawCommand, FilledRect, Rect};

pub const CANVAS_WIDTH: u32 = 320;
pub const CANVAS_HEIGHT: u32 = 180;
pub const TICKS_PER_SECOND: u32 = 60;

/// Edge length of the square, in logical pixels.
pub const SIZE: i32 = 16;
/// Distance moved per tick along each held axis, in logical pixels.
pub const SPEED: i32 = 2;

const BACKGROUND: Color = Color {
    r: 0x14,
    g: 0x16,
    b: 0x1c,
};
const SQUARE: Color = Color {
    r: 0xf2,
    g: 0xc1,
    b: 0x4e,
};

const MAX_X: i32 = CANVAS_WIDTH as i32 - SIZE;
const MAX_Y: i32 = CANVAS_HEIGHT as i32 - SIZE;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    x: i32,
    y: i32,
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

impl Game {
    /// A square resting in the middle of the canvas.
    pub fn new() -> Self {
        Self {
            x: MAX_X / 2,
            y: MAX_Y / 2,
        }
    }

    #[cfg(test)]
    pub fn position(&self) -> (i32, i32) {
        (self.x, self.y)
    }

    /// Advance one tick. Opposite directions cancel; the square stays on the
    /// canvas.
    pub fn update(&mut self, held: Buttons) {
        let axis = |negative: Buttons, positive: Buttons| {
            i32::from(held.contains(positive)) - i32::from(held.contains(negative))
        };
        self.x = (self.x + SPEED * axis(Buttons::LEFT, Buttons::RIGHT)).clamp(0, MAX_X);
        self.y = (self.y + SPEED * axis(Buttons::UP, Buttons::DOWN)).clamp(0, MAX_Y);
    }

    /// The current state as a frame: background first, then the square.
    pub fn render(&self) -> Vec<DrawCommand> {
        vec![
            DrawCommand::Clear(BACKGROUND),
            DrawCommand::FillRect(FilledRect {
                rect: Rect {
                    x: self.x,
                    y: self.y,
                    width: SIZE as u32,
                    height: SIZE as u32,
                },
                color: SQUARE,
            }),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(game: &mut Game, held: Buttons, ticks: u32) {
        for _ in 0..ticks {
            game.update(held);
        }
    }

    #[test]
    fn starts_centered() {
        assert_eq!(Game::new().position(), (152, 82));
    }

    #[test]
    fn idle_input_keeps_position() {
        let mut game = Game::new();
        run(&mut game, Buttons::empty(), 100);
        assert_eq!(game, Game::new());
    }

    #[test]
    fn moves_speed_pixels_per_tick_per_axis() {
        let mut game = Game::new();
        game.update(Buttons::RIGHT);
        assert_eq!(game.position(), (152 + SPEED, 82));
        game.update(Buttons::UP | Buttons::LEFT);
        assert_eq!(game.position(), (152, 82 - SPEED));
        game.update(Buttons::DOWN);
        assert_eq!(game.position(), (152, 82));
    }

    #[test]
    fn opposite_directions_cancel() {
        let mut game = Game::new();
        run(&mut game, Buttons::all(), 10);
        run(&mut game, Buttons::LEFT | Buttons::RIGHT, 10);
        run(&mut game, Buttons::UP | Buttons::DOWN, 10);
        assert_eq!(game, Game::new());
    }

    #[test]
    fn stays_on_the_canvas() {
        let mut game = Game::new();
        run(&mut game, Buttons::LEFT | Buttons::UP, 1000);
        assert_eq!(game.position(), (0, 0));
        run(&mut game, Buttons::RIGHT | Buttons::DOWN, 1000);
        assert_eq!(game.position(), (MAX_X, MAX_Y));
    }

    #[test]
    fn state_accumulates_across_ticks() {
        let mut game = Game::new();
        run(&mut game, Buttons::RIGHT, 30);
        run(&mut game, Buttons::DOWN, 20);
        assert_eq!(game.position(), (152 + 30 * SPEED, 82 + 20 * SPEED));
    }

    #[test]
    fn frame_is_background_then_square() {
        let frame = Game::new().render();
        assert_eq!(
            frame,
            vec![
                DrawCommand::Clear(BACKGROUND),
                DrawCommand::FillRect(FilledRect {
                    rect: Rect {
                        x: 152,
                        y: 82,
                        width: 16,
                        height: 16,
                    },
                    color: SQUARE,
                }),
            ]
        );
    }
}
