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
            // A percentage takes the reference its field fixes at emission —
            // width against \linewidth, height against \textheight. Here they
            // only have to be the same kind of value. See docs/decisions/0004.
            (Self::Figure, b"width" | b"height") => Some(ValueKind::LengthOrPercentage),
            // The remaining `\includegraphics` options an author writes by
            // hand, passed through as written. Each takes the shape graphicx
            // gives it, compiled to check: `scale` and `angle` are numbers,
            // `trim` is four lengths, `clip` is a boolean.
            (Self::Figure, b"scale" | b"angle") => Some(ValueKind::Number),
            (Self::Figure, b"trim") => Some(ValueKind::Trim),
            (Self::Figure, b"clip") => Some(ValueKind::Boolean),
            (Self::Figure | Self::Table, b"caption") | (Self::Table, b"body" | b"trailing") => {
                Some(ValueKind::Braced)
            }
            // Decided by the maintainer, 2026-08-30 (issue #81), after Phase 0a
            // measured that 96% of the corpus's floats carry explicit
            // placement and the block could not say it.
            (Self::Figure | Self::Table, b"placement") => Some(ValueKind::Placement),
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
    Placement,
    Number,
    Trim,
    Boolean,
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
    /// A float placement specifier: bytes from `htbp!H`, unquoted.
    Placement(Span),
    /// A decimal number, optionally negative: `0.5`, `-90`.
    Number(Span),
    /// Four lengths separated by spaces — left, bottom, right, top — as
    /// `\includegraphics`'s `trim` takes them.
    Trim(Span),
    /// `true` or `false`, unquoted.
    Boolean(Span),
}

impl Value {
    /// Byte range this value covers.
    #[must_use]
    pub const fn span(self) -> Span {
        match self {
            Self::Str(s)
            | Self::Length(s)
            | Self::Percentage(s)
            | Self::Braced(s)
            | Self::Placement(s)
            | Self::Number(s)
            | Self::Trim(s)
            | Self::Boolean(s) => s,
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
    /// A placement letter LaTeX's float mechanism does not accept.
    BadPlacementByte {
        /// The offending byte's span, one byte long.
        at: Span,
        /// The byte itself, for the message.
        byte: u8,
    },
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
        let after_equals = at + 1;
        at = skip_whitespace(bytes, after_equals);

        let Some(expected) = kind.value_kind(&bytes[key.start()..key.end()]) else {
            return Err(BlockError::UnknownField {
                key,
                reason: removed_field(&bytes[key.start()..key.end()]),
            });
        };

        // A bare value ends at its line, so a bare field whose line holds
        // nothing is empty — not a licence to take the next line's key as the
        // value. Writing the field and saying nothing is malformed; silence
        // stays the author's explicit choice of omitting the field.
        if matches!(
            expected,
            ValueKind::Placement | ValueKind::Number | ValueKind::Trim | ValueKind::Boolean
        ) && bytes[after_equals..at.min(body_end)].contains(&b'\n')
        {
            return Err(BlockError::WrongValueKind {
                key,
                expected: describe(expected),
            });
        }
        let (value, next) = read_value(bytes, at, body_end, expected).ok_or({
            BlockError::WrongValueKind {
                key,
                expected: describe(expected),
            }
        })?;
        if let Value::Placement(placement) = value {
            for (offset, byte) in bytes[placement.start()..placement.end()].iter().enumerate() {
                if !is_placement_byte(*byte) {
                    let at = placement.start() + offset;
                    return Err(BlockError::BadPlacementByte {
                        at: span(at, at + 1),
                        byte: *byte,
                    });
                }
            }
        }
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
        ValueKind::LengthOrPercentage => {
            "a length such as 4cm or 0.8\\columnwidth, or a percentage such as 80%"
        }
        ValueKind::Braced => "a braced group, because it may span lines and contain LaTeX",
        ValueKind::Placement => {
            "one or more of the letters h, t, b, p, H and `!`, written without quotes"
        }
        ValueKind::Number => "a number such as 0.5 or -90, written without quotes",
        ValueKind::Trim => {
            "four lengths separated by spaces — left, bottom, right, top — such as \
             1cm 0 0 2mm, where a bare number is in big points as graphicx reads it"
        }
        ValueKind::Boolean => "true or false, written without quotes",
    }
}

/// The units a `length` may carry, transcribed from grammar §5.
const LENGTH_UNITS: [&[u8; 2]; 6] = [b"pt", b"mm", b"cm", b"in", b"em", b"ex"];

/// The end of a decimal number at `at`, with an optional leading `-`.
///
/// `None` when no digit is present: a sign or a lone `.` is not a number.
fn number_end(bytes: &[u8], at: usize, limit: usize) -> Option<usize> {
    let mut i = at;
    if bytes.get(i) == Some(&b'-') {
        i += 1;
    }
    let digits_start = i;
    while i < limit && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
        i += 1;
    }
    bytes[digits_start..i]
        .iter()
        .any(u8::is_ascii_digit)
        .then_some(i)
}

/// Whether a scalar value ends cleanly at `at`: whitespace, the block's
/// closing brace, or the end of the buffer. `0.5x` is not a number.
fn ends_a_scalar(bytes: &[u8], at: usize) -> bool {
    bytes
        .get(at)
        .is_none_or(|byte| byte.is_ascii_whitespace() || *byte == b'}')
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
            let has_number = i > at;
            if has_number {
                if bytes.get(i) == Some(&b'%') {
                    return Some((Value::Percentage(span(at, i + 1)), i + 1));
                }
                for unit in LENGTH_UNITS {
                    if bytes[i..].starts_with(unit) {
                        return Some((Value::Length(span(at, i + 2)), i + 2));
                    }
                }
            }
            // A TeX length, with or without a coefficient: `0.8\\columnwidth`,
            // `\\linewidth`. The percentage form covers the common case against
            // a reference the field fixes; this is how an author names a
            // different one without a new keyword. See docs/decisions/0004.
            if bytes.get(i) == Some(&b'\\') {
                let name_start = i + 1;
                let mut end = name_start;
                while end < limit && bytes[end].is_ascii_alphabetic() {
                    end += 1;
                }
                // A control word taking an argument is a command, not a length.
                if end > name_start && bytes.get(end) != Some(&b'{') {
                    return Some((Value::Length(span(at, end)), end));
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
        ValueKind::Number | ValueKind::Trim | ValueKind::Boolean => {
            read_option_value(bytes, at, limit, expected)
        }
        ValueKind::Placement => {
            let mut i = at;
            while i < limit && !bytes[i].is_ascii_whitespace() && bytes[i] != b'}' {
                i += 1;
            }
            // `placement =` followed by nothing is malformed, not "no
            // brackets": silence must stay the author's explicit choice of
            // omitting the field.
            if i == at {
                return None;
            }
            Some((Value::Placement(span(at, i)), i))
        }
    }
}

/// The `\includegraphics` option shapes: a number, four lengths, a boolean.
fn read_option_value(
    bytes: &[u8],
    at: usize,
    limit: usize,
    expected: ValueKind,
) -> Option<(Value, usize)> {
    match expected {
        ValueKind::Number => {
            let end = number_end(bytes, at, limit)?;
            ends_a_scalar(bytes, end).then_some((Value::Number(span(at, end)), end))
        }
        ValueKind::Trim => {
            // Four lengths on one line. graphicx reads a bare number as big
            // points, so a unit is optional here and required for `width`,
            // where TeX itself rejects `width=3` — the asymmetry is the
            // packages' own, not this parser's.
            let mut i = at;
            for n in 0..4 {
                if n > 0 {
                    let spaces = i;
                    while i < limit && matches!(bytes[i], b' ' | b'\t') {
                        i += 1;
                    }
                    if i == spaces {
                        return None;
                    }
                }
                let end = number_end(bytes, i, limit)?;
                i = match bytes.get(end..end + 2) {
                    Some(unit) if LENGTH_UNITS.iter().any(|known| *known == unit) => end + 2,
                    _ => end,
                };
                if !ends_a_scalar(bytes, i) {
                    return None;
                }
            }
            // A fifth value is not a fifth length; the line must be done.
            let mut rest = i;
            while rest < limit && matches!(bytes[rest], b' ' | b'\t') {
                rest += 1;
            }
            if !bytes
                .get(rest)
                .is_none_or(|byte| matches!(byte, b'\n' | b'\r' | b'}'))
            {
                return None;
            }
            Some((Value::Trim(span(at, i)), i))
        }
        ValueKind::Boolean => {
            for word in [&b"true"[..], &b"false"[..]] {
                let end = at + word.len();
                if bytes.get(at..end) == Some(word) && ends_a_scalar(bytes, end) {
                    return Some((Value::Boolean(span(at, end)), end));
                }
            }
            None
        }
        _ => None,
    }
}

/// The bytes LaTeX's float mechanism accepts in a placement specifier.
///
/// `h t b p !` are LaTeX's own; `H` is the `float` package's. Whether that
/// package is loaded is a package fact and stays unvalidated, like every
/// other package fact. The value is emitted verbatim — no reordering, no
/// deduplication — because the compiler does not improve author intent.
const fn is_placement_byte(byte: u8) -> bool {
    matches!(byte, b'h' | b't' | b'b' | b'p' | b'!' | b'H')
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
    fn placement_reads_the_corpus_forms_and_stops_at_the_line() {
        // The corpus's frequent forms, per issue #81. The value is bare and
        // ends at its line.
        for form in ["t", "h", "ht", "!htbp", "!ht", "b", "H"] {
            let input = format!("\\figure(f) {{\n  placement = {form}\n  caption = {{x}}\n}}");
            let block = parse_block(input.as_bytes(), BlockKind::Figure, 0, b"\\figure(".len())
                .expect(form);
            let field = &block.fields[0];
            let Value::Placement(span) = field.value else {
                panic!("{form} did not parse as a placement")
            };
            assert_eq!(&input.as_bytes()[span.start()..span.end()], form.as_bytes());
            assert_eq!(block.fields.len(), 2, "{form} swallowed the caption");
        }
    }

    #[test]
    fn a_placement_byte_latex_would_reject_names_itself() {
        let input = "\\table(t) {\n  placement = htq\n  caption = {x}\n}";
        let error = parse_block(input.as_bytes(), BlockKind::Table, 0, b"\\table(".len())
            .expect_err("q is not a placement letter");
        let BlockError::BadPlacementByte { at, byte } = error else {
            panic!("wrong error: {error:?}")
        };
        assert_eq!(byte, b'q');
        assert_eq!(&input.as_bytes()[at.start()..at.end()], b"q");
    }

    #[test]
    fn an_empty_placement_is_malformed_not_a_licence_to_read_the_next_line() {
        // `placement =` followed by a line ending must not take the next
        // line's key as its value. Writing the field and saying nothing is
        // malformed; omitting the field is how silence is asked for.
        let input = "\\figure(f) {\n  placement =\n  caption = {x}\n}";
        let error = parse_block(input.as_bytes(), BlockKind::Figure, 0, b"\\figure(".len())
            .expect_err("an empty placement is malformed");
        assert!(
            matches!(error, BlockError::WrongValueKind { .. }),
            "wrong error: {error:?}"
        );
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
    fn inclusion_options_read_the_shapes_graphicx_gives_them() {
        let input = "\\figure(f) {\n  scale = 0.5\n  angle = -90\n  trim = 1cm 0 0.5in 2\n  clip = true\n  caption = {x}\n}";
        assert_eq!(keys(input), ["scale", "angle", "trim", "clip", "caption"]);
        let block = parse(input).unwrap();
        let bytes = input.as_bytes();
        let text = |value: Value| {
            String::from_utf8_lossy(&bytes[value.span().start()..value.span().end()]).into_owned()
        };
        assert!(matches!(block.fields[0].value, Value::Number(_)));
        assert_eq!(text(block.fields[1].value), "-90");
        assert!(matches!(block.fields[2].value, Value::Trim(_)));
        assert_eq!(text(block.fields[2].value), "1cm 0 0.5in 2");
        assert!(matches!(block.fields[3].value, Value::Boolean(_)));
    }

    #[test]
    fn inclusion_options_refuse_what_graphicx_would_choke_on() {
        // Three lengths, a unit TeX does not know, a number with a tail, a
        // boolean in quotes, and an empty value: each is malformed at its key.
        for (field, value) in [
            ("trim", "1cm 0 0"),
            ("trim", "1qm 0 0 0"),
            ("trim", "1cm 0 0 0 0"),
            ("scale", "0.5x"),
            ("angle", "ninety"),
            ("clip", "\"true\""),
            ("clip", "yes"),
            ("scale", ""),
        ] {
            let input = format!("\\figure(f) {{\n  {field} = {value}\n  caption = {{x}}\n}}");
            let Err(BlockError::WrongValueKind { key, .. }) = parse(&input) else {
                panic!("{field} = {value:?} was accepted")
            };
            assert_eq!(&input.as_bytes()[key.start()..key.end()], field.as_bytes());
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
