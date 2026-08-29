//! Just enough JSON to speak LSP.
//!
//! Not a JSON library and not on its way to becoming one. It reads the handful
//! of shapes the protocol sends us and writes the handful we send back, and a
//! message using something outside that is refused rather than guessed at.
//!
//! Written by hand for the reason in `docs/decisions/0005`: the compiler core
//! has no dependencies and the server is small enough to keep it that way.

use std::fmt::Write as _;

/// A parsed JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// Any number, held as written so an integer survives a round trip.
    Number(f64),
    /// A string with its escapes resolved.
    Text(String),
    /// An array.
    List(Vec<Value>),
    /// An object, in the order its keys arrived.
    Map(Vec<(String, Value)>),
}

impl Value {
    /// The value at `key`, if this is an object that has one.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Map(entries) => entries
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// This value as text.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }

    /// This value as an integer, when it is one.
    #[must_use]
    pub fn integer(&self) -> Option<i64> {
        match self {
            #[allow(clippy::cast_possible_truncation)]
            Self::Number(number) if number.fract() == 0.0 => Some(*number as i64),
            _ => None,
        }
    }
}

/// Parses one JSON value, or `None` if the bytes are not one.
#[must_use]
pub fn parse(bytes: &[u8]) -> Option<Value> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut chars: Vec<char> = text.chars().collect();
    chars.push('\0');
    let mut at = 0usize;
    let value = value(&chars, &mut at)?;
    skip_space(&chars, &mut at);
    (chars[at] == '\0').then_some(value)
}

fn skip_space(chars: &[char], at: &mut usize) {
    while chars[*at].is_ascii_whitespace() {
        *at += 1;
    }
}

fn value(chars: &[char], at: &mut usize) -> Option<Value> {
    skip_space(chars, at);
    match chars[*at] {
        '"' => text(chars, at).map(Value::Text),
        '{' => map(chars, at),
        '[' => list(chars, at),
        't' => literal(chars, at, "true").then_some(Value::Bool(true)),
        'f' => literal(chars, at, "false").then_some(Value::Bool(false)),
        'n' => literal(chars, at, "null").then_some(Value::Null),
        _ => number(chars, at),
    }
}

fn literal(chars: &[char], at: &mut usize, word: &str) -> bool {
    if chars[*at..].starts_with(&word.chars().collect::<Vec<_>>()[..]) {
        *at += word.len();
        return true;
    }
    false
}

fn number(chars: &[char], at: &mut usize) -> Option<Value> {
    let start = *at;
    while matches!(chars[*at], '0'..='9' | '-' | '+' | '.' | 'e' | 'E') {
        *at += 1;
    }
    (start < *at)
        .then(|| chars[start..*at].iter().collect::<String>())
        .and_then(|text| text.parse().ok())
        .map(Value::Number)
}

fn text(chars: &[char], at: &mut usize) -> Option<String> {
    if chars[*at] != '"' {
        return None;
    }
    *at += 1;
    let mut out = String::new();
    loop {
        match chars[*at] {
            '\0' => return None,
            '"' => {
                *at += 1;
                return Some(out);
            }
            '\\' => {
                *at += 1;
                let resolved = match chars[*at] {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    'b' => '\u{8}',
                    'f' => '\u{c}',
                    'u' => {
                        let hex: String = chars.get(*at + 1..*at + 5)?.iter().collect();
                        let code = u32::from_str_radix(&hex, 16).ok()?;
                        *at += 4;
                        char::from_u32(code)?
                    }
                    other => other,
                };
                out.push(resolved);
                *at += 1;
            }
            other => {
                out.push(other);
                *at += 1;
            }
        }
    }
}

fn list(chars: &[char], at: &mut usize) -> Option<Value> {
    *at += 1;
    let mut items = Vec::new();
    loop {
        skip_space(chars, at);
        if chars[*at] == ']' {
            *at += 1;
            return Some(Value::List(items));
        }
        items.push(value(chars, at)?);
        skip_space(chars, at);
        match chars[*at] {
            ',' => *at += 1,
            ']' => {}
            _ => return None,
        }
    }
}

fn map(chars: &[char], at: &mut usize) -> Option<Value> {
    *at += 1;
    let mut entries = Vec::new();
    loop {
        skip_space(chars, at);
        if chars[*at] == '}' {
            *at += 1;
            return Some(Value::Map(entries));
        }
        let key = text(chars, at)?;
        skip_space(chars, at);
        if chars[*at] != ':' {
            return None;
        }
        *at += 1;
        entries.push((key, value(chars, at)?));
        skip_space(chars, at);
        match chars[*at] {
            ',' => *at += 1,
            '}' => {}
            _ => return None,
        }
    }
}

/// Appends `text` to `out` as a quoted JSON string.
pub fn write_text(text: &str, out: &mut String) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // The protocol is UTF-8, so only what JSON itself forbids is
            // escaped. Escaping more would be correct and unreadable.
            control if (control as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", control as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
}
