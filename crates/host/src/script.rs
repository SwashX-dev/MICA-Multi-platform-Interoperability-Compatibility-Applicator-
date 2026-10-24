//! Scripted input: which buttons are held during which tick.
//!
//! A script is plain text with one directive per line:
//!
//! ```text
//! # Lines starting with '#' are comments.
//! ticks 240          simulate 240 ticks in total
//! 0 right            from tick 0, hold right
//! 60 right down      from tick 60, hold right and down
//! 120 -              from tick 120, hold nothing
//! ```
//!
//! Buttons stay held until a later line changes them; before the first line
//! nothing is held. Button names may also be joined with `+` (`right+down`).

use anyhow::{Context as _, Result, bail, ensure};

use crate::runtime::Buttons;

const BUTTONS: [(&str, Buttons); 4] = [
    ("up", Buttons::UP),
    ("down", Buttons::DOWN),
    ("left", Buttons::LEFT),
    ("right", Buttons::RIGHT),
];

/// A complete, validated input sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    ticks: u64,
    /// `(first tick, buttons held from then on)`, strictly increasing ticks.
    changes: Vec<(u64, Buttons)>,
}

impl Script {
    pub fn parse(text: &str) -> Result<Self> {
        let mut ticks = None;
        let mut changes: Vec<(u64, Buttons)> = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let parsed: Result<()> = (|| {
                let mut words = line.split_whitespace();
                let first = words.next().unwrap_or_default();
                if first == "ticks" {
                    ensure!(ticks.is_none(), "`ticks` is given twice");
                    let count = words.next().context("`ticks` needs a count")?;
                    ensure!(words.next().is_none(), "`ticks` takes a single count");
                    ticks = Some(parse_number(count)?);
                    return Ok(());
                }
                let tick = parse_number(first)?;
                if let Some(&(previous, _)) = changes.last() {
                    ensure!(
                        tick > previous,
                        "tick {tick} does not come after the previous entry's tick {previous}"
                    );
                }
                changes.push((tick, parse_buttons(words)?));
                Ok(())
            })();
            parsed.with_context(|| format!("line {}: `{}`", index + 1, raw.trim()))?;
        }
        let ticks = ticks.context("the script has no `ticks N` line giving its length")?;
        if let Some(&(last, _)) = changes.last() {
            ensure!(
                last < ticks,
                "the entry for tick {last} is beyond the script's length of {ticks} ticks"
            );
        }
        Ok(Self { ticks, changes })
    }

    /// Number of ticks the script lasts.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// The buttons held during each tick, in tick order.
    pub fn inputs(&self) -> impl Iterator<Item = Buttons> + '_ {
        let mut next = 0;
        let mut held = Buttons::empty();
        (0..self.ticks).map(move |tick| {
            while let Some(&(start, buttons)) = self.changes.get(next) {
                if start > tick {
                    break;
                }
                held = buttons;
                next += 1;
            }
            held
        })
    }

    /// The script in canonical text form, which [`Script::parse`] reads back.
    pub fn to_text(&self) -> String {
        let mut text = format!("ticks {}\n", self.ticks);
        for &(tick, held) in &self.changes {
            text.push_str(&format!("{tick} {}\n", format_buttons(held, " ")));
        }
        text
    }
}

/// Builds a [`Script`] from live input, one tick at a time.
#[derive(Debug, Default)]
pub struct Recorder {
    ticks: u64,
    changes: Vec<(u64, Buttons)>,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Note the buttons held during the next tick.
    pub fn record(&mut self, held: Buttons) {
        let current = self
            .changes
            .last()
            .map_or(Buttons::empty(), |&(_, buttons)| buttons);
        if held != current {
            self.changes.push((self.ticks, held));
        }
        self.ticks += 1;
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    pub fn finish(self) -> Script {
        Script {
            ticks: self.ticks,
            changes: self.changes,
        }
    }
}

/// Button names joined by `separator`, in a fixed order; `-` when none.
pub fn format_buttons(held: Buttons, separator: &str) -> String {
    let names: Vec<&str> = BUTTONS
        .iter()
        .filter(|(_, button)| held.contains(*button))
        .map(|(name, _)| *name)
        .collect();
    if names.is_empty() {
        "-".to_string()
    } else {
        names.join(separator)
    }
}

fn parse_buttons<'a>(words: impl Iterator<Item = &'a str>) -> Result<Buttons> {
    let mut held = Buttons::empty();
    for name in words.flat_map(|word| word.split('+')) {
        if matches!(name, "-" | "none" | "") {
            continue;
        }
        match BUTTONS.iter().find(|(known, _)| *known == name) {
            Some((_, button)) => held |= *button,
            None => bail!("unknown button `{name}` (expected up, down, left, right, or -)"),
        }
    }
    Ok(held)
}

fn parse_number(word: &str) -> Result<u64> {
    word.parse()
        .with_context(|| format!("`{word}` is not a tick number"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_buttons_until_changed() {
        let script = Script::parse("ticks 5\n1 right\n3 up left\n4 -\n").unwrap();
        let inputs: Vec<Buttons> = script.inputs().collect();
        assert_eq!(
            inputs,
            [
                Buttons::empty(),
                Buttons::RIGHT,
                Buttons::RIGHT,
                Buttons::UP | Buttons::LEFT,
                Buttons::empty(),
            ]
        );
    }

    #[test]
    fn accepts_comments_blank_lines_and_plus_joined_names() {
        let script = Script::parse("# demo\n\nticks 2  # two\n0 down+right\n").unwrap();
        assert_eq!(
            script.inputs().collect::<Vec<_>>(),
            [Buttons::DOWN | Buttons::RIGHT; 2]
        );
    }

    #[test]
    fn text_round_trips() {
        let script = Script::parse("ticks 10\n0 right\n4 up+left\n7 none\n").unwrap();
        assert_eq!(Script::parse(&script.to_text()).unwrap(), script);
        assert_eq!(script.to_text(), "ticks 10\n0 right\n4 up left\n7 -\n");
    }

    #[test]
    fn recorder_produces_the_script_it_saw() {
        let script = Script::parse("ticks 6\n0 right\n2 right down\n5 -\n").unwrap();
        let mut recorder = Recorder::new();
        for held in script.inputs() {
            recorder.record(held);
        }
        assert_eq!(recorder.finish(), script);
    }

    #[test]
    fn errors_name_the_line() {
        let cases = [
            ("ticks 5\n0 rigth\n", "line 2", "unknown button `rigth`"),
            ("ticks 5\n3 up\n2 down\n", "line 3", "does not come after"),
            ("ticks 5\nfive up\n", "line 2", "not a tick number"),
            ("0 up\n", "", "no `ticks N` line"),
            ("ticks 5\n5 up\n", "", "beyond the script's length"),
        ];
        for (text, line, message) in cases {
            let error = format!("{:#}", Script::parse(text).unwrap_err());
            assert!(error.contains(line), "{error:?} should mention {line:?}");
            assert!(
                error.contains(message),
                "{error:?} should mention {message:?}"
            );
        }
    }
}
