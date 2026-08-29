//! Typed blocks and their fields.
//!
//! `\figure(id) { … }` and `\table(id) { … }`, per `docs/grammar.md` §5. A
//! field's value kind is fixed by its name, so the parser reads the kind the
//! specification declares rather than trying alternatives until one succeeds —
//! which would make a malformed value silently become a different valid one.
//!
//! Three field names are rejected rather than ignored. `columns` restated what
//! a tabular column specification already says, so the two could disagree;
//! `needs` would have emitted a `\usepackage` the source did not contain, which
//! the no-injection rule forbids. A document carrying either is malformed, and
//! saying so is how an author learns the field is gone.

use crate::scanner::{CommentRule, balanced_end, balanced_end_with};
use crate::source::Span;

/// Which typed block this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlockKind {
    /// `\figure(id) { … }`
    Figure,
    /// `\table(id) { … }`
    Table,
}

impl BlockKind {
    /// Name used in diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Figure => "\\figure",
            Self::Table => "\\table",
        }
    }

    /// The value kind this block requires for `key`, if it accepts it at all.
    const fn value_kind(self, key: &[u8]) -> Option<ValueKind> {
        match (self, key) {
            (Self::Figure, b"src") => Some(ValueKind::Str),
            (Self::Figure, b"width") => Some(ValueKind::LengthOrPercentage),
            (Self::Figure | Self::Table, b"caption") | (Self::Table, b"body" | b"trailing") => {
                Some(ValueKind::Braced)
            }
            _ => None,
        }
    }
}

/// The shape a field's value must take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueKind {
    Str,
    LengthOrPercentage,
    Braced,
}

/// A field's value, as it appears in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Value {
    /// A quoted string. The span covers the quotes.
    Str(Span),
    /// A number and a unit.
    Length(Span),
    /// A number and a percent sign.
    Percentage(Span),
    /// Balanced braces. The span covers them.
    Braced(Span),
}

impl Value {
    /// Byte range this value covers.
    #[must_use]
    pub const fn span(self) -> Span {
        match self {
            Self::Str(s) | Self::Length(s) | Self::Percentage(s) | Self::Braced(s) => s,
        }
    }
}

/// One `key = value` pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    /// The key.
    pub key: Span,
    /// The value.
    pub value: Value,
}

/// A parsed typed block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// Which block it is.
    pub kind: BlockKind,
    /// The identifier between the parentheses.
    pub id: Span,
    /// Fields in source order.
    pub fields: Vec<Field>,
    /// The whole construct, entry token through closing brace.
    pub span: Span,
}

/// Why a block could not be parsed.
///
/// Every variant carries the span a diagnostic points at, which is the field or
/// token at fault rather than the whole block. A reader who is told "this block
/// is wrong" has to find the reason; one who is told which field is wrong does
/// not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlockError {
    /// The identifier is empty or contains bytes an identifier may not.
    BadIdentifier(Span),
    /// No `{` follows the identifier.
    MissingBody(Span),
    /// The body's braces never balance before end of file.
    UnclosedBody(Span),
    /// A field name this block does not accept.
    UnknownField {
        /// The offending key.
        key: Span,
        /// Why it is not accepted.
        reason: RemovedField,
    },
    /// The value is not the kind this field requires.
    WrongValueKind {
        /// The offending key.
        key: Span,
        /// What the specification requires.
        expected: &'static str,
    },
    /// A field has no `=`.
    MissingEquals(Span),
}

/// Whether a rejected field was removed from the specification, or never in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RemovedField {
    /// `columns` — the count is read from the tabular column specification.
    Columns,
    /// `needs` — emitting `\usepackage` would inject bytes the source lacks.
    Needs,
    /// Not a field of this block.
    Unknown,
}

impl RemovedField {
    /// One sentence a reader can act on.
    #[must_use]
    pub const fn explanation(self) -> &'static str {
        match self {
            Self::Columns => {
                "the column count is read from the tabular column specification, \
                 so declaring it here lets the two disagree"
            }
            Self::Needs => {
                "a block cannot add a \\usepackage the source does not contain; \
                 declare the package in the preamble"
            }
            Self::Unknown => "this block has no such field",
        }
    }
}

/// Parses a block whose entry token starts at `start`.
///
/// `after_entry` is the offset just past `\figure(` or `\table(`.
///
/// # Errors
///
/// Returns [`BlockError`] pointing at the field or token at fault. The bytes
/// are transported either way; only the diagnostic changes.
pub fn parse_block(
    bytes: &[u8],
    kind: BlockKind,
    start: usize,
    after_entry: usize,
) -> Result<Block, BlockError> {
    let id_end = bytes[after_entry..]
        .iter()
        .position(|b| *b == b')' || *b == b'\n')
        .map(|i| after_entry + i)
        .filter(|i| bytes[*i] == b')')
        .ok_or_else(|| BlockError::BadIdentifier(span(start, after_entry)))?;

    let id = span(after_entry, id_end);
    if id.is_empty() || !is_identifier(&bytes[after_entry..id_end]) {
        return Err(BlockError::BadIdentifier(id));
    }

    let mut at = skip_whitespace(bytes, id_end + 1);
    if bytes.get(at) != Some(&b'{') {
        return Err(BlockError::MissingBody(span(start, at.min(bytes.len()))));
    }
    let body_open = at;
    // A block body is scanned with the percent-after-digit rule, because the
    // grammar admits `width = 80%` and LaTeX's own rule would let that `%`
    // swallow the closing brace.
    let body_end = balanced_end_with(bytes, body_open, CommentRule::PercentAfterDigit)
        .ok_or_else(|| BlockError::UnclosedBody(span(start, bytes.len())))?;

    at = skip_whitespace(bytes, body_open + 1);
    let mut fields = Vec::new();

    while at < body_end - 1 {
        let key_start = at;
        while at < body_end && (bytes[at].is_ascii_lowercase() || bytes[at] == b'_') {
            at += 1;
        }
        if at == key_start {
            break;
        }
        let key = span(key_start, at);

        at = skip_whitespace(bytes, at);
        if bytes.get(at) != Some(&b'=') {
            return Err(BlockError::MissingEquals(key));
        }
        at = skip_whitespace(bytes, at + 1);

        let Some(expected) = kind.value_kind(&bytes[key.start()..key.end()]) else {
            return Err(BlockError::UnknownField {
                key,
                reason: removed_field(&bytes[key.start()..key.end()]),
            });
        };

        let (value, next) = read_value(bytes, at, body_end, expected).ok_or({
            BlockError::WrongValueKind {
                key,
                expected: describe(expected),
            }
        })?;
        fields.push(Field { key, value });
        at = skip_whitespace(bytes, next);
    }

    Ok(Block {
        kind,
        id,
        fields,
        span: span(start, body_end),
    })
}

fn removed_field(key: &[u8]) -> RemovedField {
    match key {
        b"columns" => RemovedField::Columns,
        b"needs" => RemovedField::Needs,
        _ => RemovedField::Unknown,
    }
}

const fn describe(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Str => "a quoted string",
        ValueKind::LengthOrPercentage => "a length such as 4cm, or a percentage such as 80%",
        ValueKind::Braced => "a braced group, because it may span lines and contain LaTeX",
    }
}

fn read_value(
    bytes: &[u8],
    at: usize,
    limit: usize,
    expected: ValueKind,
) -> Option<(Value, usize)> {
    match expected {
        ValueKind::Str => {
            if bytes.get(at) != Some(&b'"') {
                return None;
            }
            let mut i = at + 1;
            while i < limit {
                match bytes[i] {
                    b'\\' => i += 2,
                    b'"' => return Some((Value::Str(span(at, i + 1)), i + 1)),
                    b'\n' => return None,
                    _ => i += 1,
                }
            }
            None
        }
        ValueKind::LengthOrPercentage => {
            let mut i = at;
            while i < limit && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            if i == at {
                return None;
            }
            if bytes.get(i) == Some(&b'%') {
                return Some((Value::Percentage(span(at, i + 1)), i + 1));
            }
            for unit in [b"pt", b"mm", b"cm", b"in", b"em", b"ex"] {
                if bytes[i..].starts_with(unit) {
                    return Some((Value::Length(span(at, i + 2)), i + 2));
                }
            }
            None
        }
        ValueKind::Braced => {
            if bytes.get(at) != Some(&b'{') {
                return None;
            }
            let end = balanced_end(bytes, at)?;
            Some((Value::Braced(span(at, end)), end))
        }
    }
}

fn is_identifier(bytes: &[u8]) -> bool {
    matches!(bytes.first(), Some(b) if b.is_ascii_alphabetic())
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b'.' | b'-'))
}

fn skip_whitespace(bytes: &[u8], mut at: usize) -> usize {
    while matches!(bytes.get(at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        at += 1;
    }
    at
}

fn span(start: usize, end: usize) -> Span {
    Span::new(
        u32::try_from(start).unwrap_or(u32::MAX),
        u32::try_from(end).unwrap_or(u32::MAX),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> Result<Block, BlockError> {
        let bytes = input.as_bytes();
        let (kind, entry) = if bytes.starts_with(b"\\figure(") {
            (BlockKind::Figure, b"\\figure(".len())
        } else {
            (BlockKind::Table, b"\\table(".len())
        };
        parse_block(bytes, kind, 0, entry)
    }

    fn keys(input: &str) -> Vec<String> {
        let bytes = input.as_bytes();
        parse(input)
            .unwrap()
            .fields
            .iter()
            .map(|f| String::from_utf8_lossy(&bytes[f.key.start()..f.key.end()]).into_owned())
            .collect()
    }

    #[test]
    fn a_figure_reads_its_three_fields() {
        let input =
            "\\figure(fig:r) {\n  src = \"r.pdf\"\n  width = 80%\n  caption = {A caption}\n}";
        assert_eq!(keys(input), ["src", "width", "caption"]);
        assert_eq!(parse(input).unwrap().kind, BlockKind::Figure);
    }

    #[test]
    fn a_caption_may_span_lines_and_nest_braces() {
        let input =
            "\\table(t) {\n  caption = {A caption across\n  two lines with {nested {groups}}}\n}";
        assert_eq!(keys(input), ["caption"]);
    }

    #[test]
    fn a_caption_containing_an_escaped_percent_is_not_truncated() {
        let input = "\\figure(f) {\n  caption = {Accuracy rose 12\\% and {10\\%} of runs}\n}";
        let block = parse(input).unwrap();
        let Value::Braced(span) = block.fields[0].value else {
            panic!("caption is braced")
        };
        assert!(span.len() > 30, "caption was cut short at the percent sign");
    }

    #[test]
    fn a_comment_inside_a_caption_hides_its_braces() {
        let input = "\\figure(f) {\n  caption = {before\n  % a comment with } inside\n  after}\n}";
        assert_eq!(keys(input), ["caption"]);
    }

    #[test]
    fn width_takes_a_length_or_a_percentage() {
        for value in ["80%", "4cm", "12.5pt"] {
            let input = format!("\\figure(f) {{ width = {value} }}");
            assert_eq!(keys(&input), ["width"], "{value} was refused");
        }
    }

    #[test]
    fn a_bare_caption_is_refused_because_it_cannot_span_lines() {
        let input = "\\figure(f) { caption = unbraced text }";
        assert!(matches!(
            parse(input),
            Err(BlockError::WrongValueKind { .. })
        ));
    }

    #[test]
    fn columns_is_rejected_and_says_why() {
        let input = "\\table(t) { columns = 7 }";
        let Err(BlockError::UnknownField { reason, .. }) = parse(input) else {
            panic!("columns must be rejected")
        };
        assert_eq!(reason, RemovedField::Columns);
        assert!(reason.explanation().contains("column specification"));
    }

    #[test]
    fn needs_is_rejected_and_says_why() {
        let input = "\\table(t) { needs = booktabs }";
        let Err(BlockError::UnknownField { reason, .. }) = parse(input) else {
            panic!("needs must be rejected")
        };
        assert_eq!(reason, RemovedField::Needs);
        assert!(reason.explanation().contains("usepackage"));
    }

    #[test]
    fn an_unclosed_body_reports_rather_than_scanning_forever() {
        let input = "\\figure(f) {\n  caption = {never closed\n";
        assert!(matches!(parse(input), Err(BlockError::UnclosedBody(_))));
    }

    #[test]
    fn a_bad_identifier_is_reported_at_the_identifier() {
        assert!(matches!(
            parse("\\figure() { }"),
            Err(BlockError::BadIdentifier(_))
        ));
        assert!(matches!(
            parse("\\figure(9bad) { }"),
            Err(BlockError::BadIdentifier(_))
        ));
    }

    #[test]
    fn a_field_without_an_equals_is_reported_at_the_key() {
        assert!(matches!(
            parse("\\figure(f) { src \"x.pdf\" }"),
            Err(BlockError::MissingEquals(_))
        ));
    }

    #[test]
    fn a_table_accepts_trailing_content() {
        let input =
            "\\table(t) {\n  body = { x }\n  trailing = { \\vspace{2pt} {\\scriptsize note} }\n}";
        assert_eq!(keys(input), ["body", "trailing"]);
    }
}
