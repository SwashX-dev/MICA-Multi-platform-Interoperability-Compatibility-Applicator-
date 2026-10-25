//! Valve's text KeyValues format (`.vdf`, `.acf`), as Steam writes it.
//!
//! ```text
//! "libraryfolders"
//! {
//!     "0"
//!     {
//!         "path"    "/home/me/.local/share/Steam"
//!     }
//! }
//! ```
//!
//! Keys and values are quoted strings (unquoted tokens are accepted too),
//! `{`/`}` open and close objects, `//` starts a comment, and platform
//! conditionals such as `[$WIN32]` are skipped. Keys may repeat; order is
//! kept.

use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Text(String),
    Object(Vec<(String, Value)>),
}

impl Value {
    /// The first value under `key` (case-insensitive, as Steam compares).
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(entries) => entries
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(key))
                .map(|(_, value)| value),
            Value::Text(_) => None,
        }
    }

    /// Follow a path of keys.
    pub fn path(&self, keys: &[&str]) -> Option<&Value> {
        keys.iter().try_fold(self, |value, key| value.get(key))
    }

    pub fn text(&self) -> Option<&str> {
        match self {
            Value::Text(text) => Some(text),
            Value::Object(_) => None,
        }
    }

    pub fn entries(&self) -> &[(String, Value)] {
        match self {
            Value::Object(entries) => entries,
            Value::Text(_) => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub offset: usize,
    pub message: &'static str,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at byte {}", self.message, self.offset)
    }
}

impl std::error::Error for ParseError {}

#[derive(Debug, PartialEq)]
enum Token {
    Text(String),
    Open,
    Close,
}

struct Lexer<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Lexer<'_> {
    fn next(&mut self) -> Result<Option<(usize, Token)>, ParseError> {
        loop {
            while self.at < self.bytes.len() && self.bytes[self.at].is_ascii_whitespace() {
                self.at += 1;
            }
            let Some(&byte) = self.bytes.get(self.at) else {
                return Ok(None);
            };
            let start = self.at;
            match byte {
                b'/' if self.bytes.get(self.at + 1) == Some(&b'/') => {
                    while self.at < self.bytes.len() && self.bytes[self.at] != b'\n' {
                        self.at += 1;
                    }
                }
                // Platform conditionals: [$WIN32], [!$OSX], ...
                b'[' => {
                    while self.at < self.bytes.len() && self.bytes[self.at] != b']' {
                        self.at += 1;
                    }
                    self.at += 1;
                }
                b'{' => {
                    self.at += 1;
                    return Ok(Some((start, Token::Open)));
                }
                b'}' => {
                    self.at += 1;
                    return Ok(Some((start, Token::Close)));
                }
                b'"' => {
                    self.at += 1;
                    let mut text = Vec::new();
                    loop {
                        let Some(&byte) = self.bytes.get(self.at) else {
                            return Err(ParseError {
                                offset: start,
                                message: "unterminated string",
                            });
                        };
                        self.at += 1;
                        match byte {
                            b'"' => break,
                            b'\\' => {
                                let escaped = self.bytes.get(self.at).copied().unwrap_or(b'\\');
                                self.at += 1;
                                text.push(match escaped {
                                    b'n' => b'\n',
                                    b't' => b'\t',
                                    other => other,
                                });
                            }
                            other => text.push(other),
                        }
                    }
                    return Ok(Some((
                        start,
                        Token::Text(String::from_utf8_lossy(&text).into_owned()),
                    )));
                }
                _ => {
                    while self.at < self.bytes.len()
                        && !self.bytes[self.at].is_ascii_whitespace()
                        && !matches!(self.bytes[self.at], b'{' | b'}' | b'"')
                    {
                        self.at += 1;
                    }
                    return Ok(Some((
                        start,
                        Token::Text(
                            String::from_utf8_lossy(&self.bytes[start..self.at]).into_owned(),
                        ),
                    )));
                }
            }
        }
    }
}

/// Parse a document: a sequence of `key value` or `key { ... }` pairs at the
/// top level, returned as one object.
pub fn parse(text: &str) -> Result<Value, ParseError> {
    let mut lexer = Lexer {
        bytes: text.as_bytes(),
        at: 0,
    };
    let mut stack: Vec<Vec<(String, Value)>> = vec![Vec::new()];
    let mut keys: Vec<String> = Vec::new();
    while let Some((offset, token)) = lexer.next()? {
        match token {
            Token::Text(key) => match lexer.next()? {
                Some((_, Token::Text(value))) => {
                    stack.last_mut().expect("never empty").push((key, Value::Text(value)));
                }
                Some((_, Token::Open)) => {
                    if stack.len() > 64 {
                        return Err(ParseError {
                            offset,
                            message: "nesting too deep",
                        });
                    }
                    keys.push(key);
                    stack.push(Vec::new());
                }
                Some((offset, Token::Close)) => {
                    return Err(ParseError {
                        offset,
                        message: "a key must be followed by a value or {",
                    });
                }
                None => {
                    return Err(ParseError {
                        offset,
                        message: "a key at the end of the input has no value",
                    });
                }
            },
            Token::Close => {
                let Some(key) = keys.pop() else {
                    return Err(ParseError {
                        offset,
                        message: "unbalanced }",
                    });
                };
                let object = stack.pop().expect("pushed with the key");
                stack
                    .last_mut()
                    .expect("never empty")
                    .push((key, Value::Object(object)));
            }
            Token::Open => {
                return Err(ParseError {
                    offset,
                    message: "{ without a key",
                });
            }
        }
    }
    if !keys.is_empty() {
        return Err(ParseError {
            offset: text.len(),
            message: "unclosed {",
        });
    }
    Ok(Value::Object(stack.pop().expect("never empty")))
}

fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Write entries the way Steam does: tabs, one pair per line.
pub fn write(entries: &[(String, Value)]) -> String {
    fn write_level(out: &mut String, entries: &[(String, Value)], depth: usize) {
        let indent = "\t".repeat(depth);
        for (key, value) in entries {
            match value {
                Value::Text(text) => {
                    let _ = writeln!(out, "{indent}{}\t\t{}", quote(key), quote(text));
                }
                Value::Object(children) => {
                    let _ = writeln!(out, "{indent}{}", quote(key));
                    let _ = writeln!(out, "{indent}{{");
                    write_level(out, children, depth + 1);
                    let _ = writeln!(out, "{indent}}}");
                }
            }
        }
    }
    let mut out = String::new();
    write_level(&mut out, entries, 0);
    out
}

/// Build an object from `(key, value)` text pairs.
pub fn object(pairs: &[(&str, &str)]) -> Value {
    Value::Object(
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), Value::Text(value.to_string())))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIBRARY: &str = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"/home/me/.local/share/Steam"
		"label"		""
		"apps"
		{
			"228980"		"417631547"
			"1172470"		"76503420109"
		}
	}
	"1"
	{
		"path"		"/mnt/games/Steam\\Library"   // a second drive
		"contentid"		"123"
	}
}
"#;

    #[test]
    fn parses_steam_library_folders() {
        let document = parse(LIBRARY).unwrap();
        let folders = document.get("libraryfolders").unwrap();
        assert_eq!(folders.entries().len(), 2);
        assert_eq!(
            folders.path(&["0", "path"]).unwrap().text(),
            Some("/home/me/.local/share/Steam")
        );
        assert_eq!(
            folders.path(&["1", "PATH"]).unwrap().text(),
            Some("/mnt/games/Steam\\Library")
        );
        let apps: Vec<&str> = folders
            .path(&["0", "apps"])
            .unwrap()
            .entries()
            .iter()
            .map(|(key, _)| key.as_str())
            .collect();
        assert_eq!(apps, vec!["228980", "1172470"]);
    }

    #[test]
    fn round_trips_and_skips_conditionals() {
        let manifest = object(&[("version", "2"), ("commandline", "/mica-steam %verb%")]);
        let text = write(&[("manifest".into(), manifest.clone())]);
        assert_eq!(parse(&text).unwrap().get("manifest"), Some(&manifest));
        let conditional = parse("\"a\" \"1\" [$WIN32]\n\"b\" { \"c\" \"x\\\"y\" }").unwrap();
        assert_eq!(conditional.path(&["b", "c"]).unwrap().text(), Some("x\"y"));
    }

    #[test]
    fn rejects_malformed_input() {
        assert!(parse("\"a\" {").is_err());
        assert!(parse("}").is_err());
        assert!(parse("\"a\"").is_err());
        assert!(parse("\"unterminated").is_err());
        assert!(parse("{").is_err());
        assert_eq!(parse("").unwrap(), Value::Object(Vec::new()));
        let deep = "\"k\" {".repeat(100) + &"}".repeat(100);
        assert!(parse(&deep).is_err());
    }
}
