//! Text traces of an application's output, for golden-file tests and for
//! comparing runs across hosts and CPU architectures.
//!
//! A trace has one line per tick: the tick number, the buttons held during it,
//! and the frame rendered after it. The text is the canonical form; its SHA-256
//! is the fingerprint compared between machines.

use crate::runtime::{Buttons, Color, DrawCommand};
use crate::script::format_buttons;

pub const HEADER: &str =
    "# portable-host trace v1: <tick> <held> | <frame rendered after the tick>";

#[derive(Debug, Clone)]
pub struct Trace {
    text: String,
    frames: u64,
}

impl Default for Trace {
    fn default() -> Self {
        Self::new()
    }
}

impl Trace {
    pub fn new() -> Self {
        Self {
            text: format!("{HEADER}\n"),
            frames: 0,
        }
    }

    pub fn record(&mut self, tick: u64, held: Buttons, frame: &[DrawCommand]) {
        self.text.push_str(&format!(
            "{tick} {} | {}\n",
            format_buttons(held, "+"),
            format_frame(frame)
        ));
        self.frames += 1;
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn sha256(&self) -> String {
        crate::sha256_hex(self.text.as_bytes())
    }
}

/// A frame as one line of text, for example
/// `clear #14161c; fill 152,82 16x16 #f2c14e`.
pub fn format_frame(frame: &[DrawCommand]) -> String {
    let commands: Vec<String> = frame
        .iter()
        .map(|command| match command {
            DrawCommand::Clear(color) => format!("clear {}", hex(color)),
            DrawCommand::FillRect(fill) => format!(
                "fill {},{} {}x{} {}",
                fill.rect.x,
                fill.rect.y,
                fill.rect.width,
                fill.rect.height,
                hex(&fill.color)
            ),
        })
        .collect();
    if commands.is_empty() {
        "(empty)".to_string()
    } else {
        commands.join("; ")
    }
}

/// Fingerprint of a single frame.
pub fn frame_sha256(frame: &[DrawCommand]) -> String {
    crate::sha256_hex(format_frame(frame).as_bytes())
}

fn hex(color: &Color) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

/// The first line where two traces differ, as `(line number, expected, actual)`.
pub fn first_difference<'a>(
    expected: &'a str,
    actual: &'a str,
) -> Option<(usize, &'a str, &'a str)> {
    let mut expected_lines = expected.lines();
    let mut actual_lines = actual.lines();
    for number in 1.. {
        match (expected_lines.next(), actual_lines.next()) {
            (None, None) => return None,
            (e, a) if e == a => continue,
            (e, a) => {
                return Some((
                    number,
                    e.unwrap_or("<end of trace>"),
                    a.unwrap_or("<end of trace>"),
                ));
            }
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{FilledRect, Rect};

    #[test]
    fn formats_frames_compactly() {
        let frame = [
            DrawCommand::Clear(Color { r: 1, g: 2, b: 3 }),
            DrawCommand::FillRect(FilledRect {
                rect: Rect {
                    x: -4,
                    y: 5,
                    width: 6,
                    height: 7,
                },
                color: Color {
                    r: 0xff,
                    g: 0x80,
                    b: 0,
                },
            }),
        ];
        assert_eq!(format_frame(&frame), "clear #010203; fill -4,5 6x7 #ff8000");
        assert_eq!(format_frame(&[]), "(empty)");
    }

    #[test]
    fn records_one_line_per_tick() {
        let mut trace = Trace::new();
        trace.record(0, Buttons::RIGHT | Buttons::UP, &[]);
        trace.record(1, Buttons::empty(), &[]);
        assert_eq!(
            trace.text(),
            format!("{HEADER}\n0 up+right | (empty)\n1 - | (empty)\n")
        );
        assert_eq!(trace.frames(), 2);
    }

    #[test]
    fn finds_first_difference() {
        assert_eq!(first_difference("a\nb\n", "a\nb\n"), None);
        assert_eq!(first_difference("a\nb\n", "a\nc\n"), Some((2, "b", "c")));
        assert_eq!(
            first_difference("a\n", "a\nb\n"),
            Some((2, "<end of trace>", "b"))
        );
    }
}
