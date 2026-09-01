//! Byte-level delimiters: where a group, an argument or an escape ends.

use crate::source::Span;

pub(crate) fn span_at(start: usize, end: usize) -> Span {
    Span::new(
        u32::try_from(start).unwrap_or(u32::MAX),
        u32::try_from(end).unwrap_or(u32::MAX),
    )
}

pub(crate) fn ident_end(bytes: &[u8], start: usize) -> Option<usize> {
    if !bytes.get(start).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut at = start + 1;
    while bytes.get(at).is_some_and(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'.' | b'-')
    }) {
        at += 1;
    }
    Some(at)
}

/// Offset just past the `}` that closes the group opening at `open`.
///
/// Counts with LaTeX's own escaping: `\{`, `\}` and `\%` do not count, decided
/// by backslash-run parity, and an unescaped `%` opens a comment through the
/// line ending so braces inside it are invisible.
///
/// Returns `None` when the braces never balance, which is a boundary that could
/// not be located rather than an error in the LaTeX.
#[must_use]
pub fn balanced_end(bytes: &[u8], open: usize) -> Option<usize> {
    balanced_end_with(bytes, open, CommentRule::Latex)
}

/// How `%` is read while scanning a balanced region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentRule {
    /// LaTeX's own rule: an unescaped `%` opens a comment.
    Latex,
    /// As above, except that a `%` immediately after an ASCII digit is a
    /// percent sign.
    ///
    /// This exists because the grammar admits `width = 80%` as a value and also
    /// scans block bodies with LaTeX's comment rule, and those two cannot both
    /// hold: the `%` swallows the closing brace and the block never ends. It is
    /// the same collision the tool baseline measured in plain LaTeX, where
    /// `width=120%` fails with `File ended while scanning use of \Gin@ii`.
    ///
    /// One byte of context decides it, so it stays a left-to-right rule. It
    /// applies only inside a ExactTeX block body, never to transported LaTeX,
    /// where a comment must keep meaning what TeX says it means.
    PercentAfterDigit,
}

/// [`balanced_end`] with an explicit rule for `%`.
///
/// # Panics
///
/// Never; the signature returns `None` for every failure.
#[must_use]
pub fn balanced_end_with(bytes: &[u8], open: usize, rule: CommentRule) -> Option<usize> {
    if bytes.get(open) != Some(&b'{') {
        return None;
    }
    let mut depth = 1u32;
    let mut at = open + 1;
    while at < bytes.len() {
        if !is_escaped(bytes, at) {
            match bytes[at] {
                b'%' => {
                    let is_percent_sign = rule == CommentRule::PercentAfterDigit
                        && at > 0
                        && bytes[at - 1].is_ascii_digit();
                    if !is_percent_sign {
                        while at < bytes.len() && bytes[at] != b'\n' {
                            at += 1;
                        }
                        continue;
                    }
                }
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(at + 1);
                    }
                }
                _ => {}
            }
        }
        at += 1;
    }
    None
}

pub(crate) fn span(start: usize, end: usize) -> Span {
    Span::new(
        u32::try_from(start).unwrap_or(u32::MAX),
        u32::try_from(end).unwrap_or(u32::MAX),
    )
}

/// Whether the byte at `at` is preceded by an odd run of backslashes.
///
/// Parity, not presence: `\\%` is a line break followed by a real comment,
/// while `\%` is a literal percent sign. The corpus contains 120 escaped
/// percent signs inside captions and no real comments there, so treating every
/// `%` as a comment truncates real content.
pub(crate) fn is_escaped(bytes: &[u8], at: usize) -> bool {
    let mut run = 0usize;
    let mut i = at;
    while i > 0 && bytes[i - 1] == b'\\' {
        run += 1;
        i -= 1;
    }
    run % 2 == 1
}

/// Offset of the `)` closing a construct opened at `from`, on the same line.
///
/// A construct does not scan past a line ending: an unterminated one ends
/// there, is reported, and parsing resumes on the next line.
pub(crate) fn close_paren(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i < bytes.len() {
        match bytes[i] {
            b')' => return Some(i),
            b'\n' => return None,
            _ => i += 1,
        }
    }
    None
}

pub(crate) fn close_import(bytes: &[u8], from: usize) -> Option<usize> {
    if bytes.get(from) != Some(&b'"') {
        return None;
    }
    let mut i = from + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' if bytes.get(i + 1) == Some(&b')') => return Some(i + 1),
            b'"' | b'\n' => return None,
            _ => i += 1,
        }
    }
    None
}

/// Offset just past the `close` matching the `open` at `from`.
pub(crate) fn delimited_end(bytes: &[u8], from: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 1u32;
    let mut at = from + 1;
    while at < bytes.len() {
        if !is_escaped(bytes, at) {
            if bytes[at] == open {
                depth += 1;
            } else if bytes[at] == close {
                depth -= 1;
                if depth == 0 {
                    return Some(at + 1);
                }
            }
        }
        at += 1;
    }
    None
}

/// Offset just past the `]` closing an optional argument opened at `from`,
/// when it closes before a blank line.
///
/// A `[` with no `]` before the end of the file, or one that would reach
/// across a paragraph break, is not an optional argument; it is a bracket
/// in prose. Read as an argument it had no boundary, and the rest of the
/// file went opaque with no diagnostic — `\foo{}[never closed` hid a
/// planted broken reference in a real paper (corpus E2). A real optional
/// argument may span lines (`\caption[short\ntitle]{…}`), never a blank one.
pub(crate) fn optional_argument_end(bytes: &[u8], from: usize) -> Option<usize> {
    let end = delimited_end(bytes, from, b'[', b']')?;
    let mut at = from;
    while at < end {
        if bytes[at] == b'\n' {
            let mut next = at + 1;
            while matches!(bytes.get(next), Some(b' ' | b'\t' | b'\r')) {
                next += 1;
            }
            if bytes.get(next) == Some(&b'\n') {
                return None;
            }
        }
        at += 1;
    }
    Some(end)
}

pub(crate) fn skip_ascii_whitespace(bytes: &[u8], mut at: usize) -> usize {
    while matches!(bytes.get(at), Some(b' ' | b'\t')) {
        at += 1;
    }
    at
}
