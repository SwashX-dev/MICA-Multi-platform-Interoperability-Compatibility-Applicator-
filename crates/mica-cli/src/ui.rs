//! Output and prompts.

use std::io::{BufRead, IsTerminal, Write};

use serde::Serialize;

/// A `label      value` line, the layout Portable Host uses too.
pub fn line(label: &str, value: impl std::fmt::Display) {
    println!("{label:<11}{value}");
}

/// A continuation line under a label.
pub fn more(value: impl std::fmt::Display) {
    println!("{:<11}{value}", "");
}

pub fn json<T: Serialize>(value: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// Status markers that read the same in every terminal.
pub fn mark(status: mica_security::Status) -> &'static str {
    match status {
        mica_security::Status::Met => "[ ok ]",
        mica_security::Status::NotMet => "[FAIL]",
        mica_security::Status::Unknown => "[ ?? ]",
        mica_security::Status::NotApplicable => "[ -- ]",
    }
}

pub fn yes_no(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    }
}

/// Ask a yes/no question; `assume_yes` answers yes. Without a terminal and
/// without `--yes`, the answer is no.
pub fn confirm(question: &str, assume_yes: bool) -> bool {
    if assume_yes {
        return true;
    }
    if !std::io::stdin().is_terminal() {
        eprintln!("{question} [y/N] — no terminal to ask; re-run with --yes to proceed");
        return false;
    }
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Wrap text to `width` columns with an indent, for long explanations.
pub fn wrap(text: &str, indent: usize, width: usize) -> String {
    let mut out = String::new();
    let mut column = 0;
    for word in text.split_whitespace() {
        if column > 0 && column + 1 + word.len() > width.saturating_sub(indent) {
            out.push('\n');
            out.push_str(&" ".repeat(indent));
            column = 0;
        } else if column > 0 {
            out.push(' ');
            column += 1;
        }
        out.push_str(word);
        column += word.len();
    }
    out
}

/// A wrapped `label      value` paragraph.
pub fn para(label: &str, text: &str) {
    println!("{label:<11}{}", wrap(text, 11, 100));
}
