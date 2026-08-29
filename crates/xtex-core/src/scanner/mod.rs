//! Finds where a ExactTeX construct may begin.
//!
//! This is the half of the parser that decides whether `@ref(` is syntax or
//! ordinary text. `docs/grammar.md` §8 lists the regions in which it is text,
//! and getting that wrong is not a missing feature — it is a parser that
//! rewrites the inside of somebody's `verbatim` block.
//!
//! The scanner walks bytes once, left to right, holding a small amount of
//! state. It never looks ahead beyond a bounded window, and when it cannot
//! locate the end of a region it stops recognising rather than guessing.
//!
//! # What it does not do
//!
//! Command arguments and display-math environment bodies are exclusion regions
//! in §8. Their boundaries come from the signature tables below and in
//! [`crate::signatures`].

use crate::source::Span;

mod boundaries;
mod queries;
mod regions;
mod tables;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

pub use boundaries::{CommentRule, balanced_end, balanced_end_with};
pub use queries::{listing_header_labels, readable_content, readable_for, substitution_separator};
pub use tables::{DEFAULT_CITE_COMMANDS, DEFAULT_VERBATIM_ENVIRONMENTS, DEFINITION_COMMANDS};

use boundaries::{close_import, close_paren, ident_end, is_escaped, span, span_at};
use queries::substitution_arrows;
use regions::{
    Extent, Region, command_extent, display_math_environment_opening, entry_token_at,
    environment_end, region_opening_at, text_argument_interior,
};
use tables::if_name_is_not_a_conditional;

/// A stretch of the source, classified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    /// Prose: bytes in which a construct would have been recognised, and none
    /// was.
    Text(Span),
    /// A region where recognition stopped for good.
    ///
    /// Distinguished from [`Piece::Excluded`] because an excluded region ends
    /// and recognition resumes after it, while this one does not: its boundary
    /// could not be located, so every byte after it is unrecognisable rather
    /// than merely unrecognised. `ROADMAP.md` calls it `OpaqueToEof`.
    Quarantined(Span),
    /// A command and its arguments.
    ///
    /// Excluded from *construct recognition* like any other §8 region, and
    /// separate from [`Piece::Excluded`] because it is the only exclusion whose
    /// bytes are still document content. A `\label` inside `\caption{…}` is a
    /// real declaration; the same bytes inside a comment or a verbatim block
    /// are not, and one variant for both made those indistinguishable.
    Arguments(Span),
    /// A region §8 excludes, such as a comment, math, or a verbatim block.
    ///
    /// Distinguished from [`Piece::Text`] because a consumer that searches the
    /// source for something other than a construct — a bibliography
    /// declaration, say — must not find it here either.
    Excluded(Span),
    /// A complete ExactTeX entry token and everything it delimits.
    Construct {
        /// Which construct the entry token opened.
        kind: EntryToken,
        /// The whole construct, entry token through closing delimiter.
        span: Span,
        /// Constructs recognised inside this construct's document content.
        ///
        /// Typed block fields use this to keep their outer boundary as one
        /// piece while exposing constructs in braced values.
        children: Vec<Piece>,
    },
    /// An entry token whose construct could not be closed.
    ///
    /// The bytes are still transported; the diagnostic is what changes.
    Malformed {
        /// Which construct the entry token opened.
        kind: EntryToken,
        /// The entry token itself, which is where the diagnostic points.
        span: Span,
    },
}

/// The byte sequences that can begin a construct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EntryToken {
    /// `@add(`
    Add,
    /// `@del(`
    Del,
    /// `@sub(`
    Sub,
    /// `@note(`
    Note,
    /// `@id(`
    Id,
    /// `@ref(`
    Ref,
    /// `@cite(`
    Cite,
    /// `@import(`
    Import,
    /// `latex {`
    Raw,
    /// `\figure(`
    Figure,
    /// `\table(`
    Table,
}

impl EntryToken {
    /// The literal that opens this construct, without its `(` or `{`.
    const fn keyword(self) -> &'static [u8] {
        match self {
            Self::Add => b"@add",
            Self::Del => b"@del",
            Self::Sub => b"@sub",
            Self::Note => b"@note",
            Self::Id => b"@id",
            Self::Ref => b"@ref",
            Self::Cite => b"@cite",
            Self::Import => b"@import",
            Self::Raw => b"latex",
            Self::Figure => b"\\figure",
            Self::Table => b"\\table",
        }
    }

    /// Name used in diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Add => "@add",
            Self::Del => "@del",
            Self::Sub => "@sub",
            Self::Note => "@note",
            Self::Id => "@id",
            Self::Ref => "@ref",
            Self::Cite => "@cite",
            Self::Import => "@import",
            Self::Raw => "latex",
            Self::Figure => "\\figure",
            Self::Table => "\\table",
        }
    }
}

/// Splits a source into text and constructs.
///
/// The result covers every byte exactly once and in order, so concatenating the
/// pieces reproduces the input. That is asserted by the tests rather than
/// assumed, because it is the property emission depends on.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn scan(bytes: &[u8]) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut region = Region::Prose;
    let mut text_start = 0usize;
    let mut excluded_start = 0usize;
    let mut at = 0usize;
    let mut display_opening: Option<(usize, Vec<u8>)> = None;
    // Text-bearing arguments being scanned as prose. Each entry is the byte
    // where the argument's interior ends and the byte where the whole call
    // ends; the tail between them is the closing delimiter, emitted as an
    // `Arguments` piece when prose reaches it. A region that overruns the
    // interior (an unbalanced `$` inside a caption) simply passes the
    // boundary: the entry is dropped and whatever tail remains is still
    // classified, so every byte stays covered exactly once.
    let mut text_arguments: Vec<(usize, usize)> = Vec::new();

    /// Closes the run of ordinary bytes that ends here.
    macro_rules! flush {
        ($end:expr) => {
            if $end > text_start {
                pieces.push(Piece::Text(span(text_start, $end)));
            }
        };
    }

    /// Leaves prose for an excluded region beginning at `$start`.
    ///
    /// Both halves are required and neither is optional: the prose has to end,
    /// and the region has to know where it began. A site that sets one without
    /// the other makes the next piece start where an older one did, and the
    /// same bytes are emitted twice. That has happened twice, which is why
    /// leaving prose goes through here instead of being written out by hand.
    macro_rules! enter {
        ($start:expr) => {
            flush!($start);
            excluded_start = $start;
        };
    }

    while at < bytes.len() {
        match &region {
            Region::Prose => {
                if let Some(&(interior_end, call_end)) = text_arguments.last() {
                    if at >= interior_end {
                        flush!(at.min(interior_end));
                        text_arguments.pop();
                        let tail = at.max(interior_end);
                        if tail < call_end {
                            pieces.push(Piece::Arguments(span(tail, call_end)));
                            at = call_end;
                        }
                        text_start = at;
                        continue;
                    }
                }
                if let Some((start, name)) = display_opening.as_ref() {
                    if at == *start {
                        enter!(at);
                        region = Region::Environment { name: name.clone() };
                        display_opening = None;
                        continue;
                    }
                }
                if display_opening.is_none()
                    && let Some(opening) = display_math_environment_opening(bytes, at)
                {
                    display_opening = Some(opening);
                }
                if let Some((token, end)) = entry_token_at(bytes, at) {
                    if matches!(
                        token,
                        EntryToken::Add | EntryToken::Del | EntryToken::Sub | EntryToken::Note
                    ) {
                        flush!(at);
                        let (piece, resume) = revision_piece(bytes, token, at, end);
                        pieces.push(piece);
                        at = resume;
                        text_start = at;
                        continue;
                    }
                    if let Some(kind) = block_kind(token) {
                        flush!(at);
                        let piece = match crate::blocks::parse_block(bytes, kind, at, end) {
                            Ok(block) => Piece::Construct {
                                kind: token,
                                span: block.span,
                                children: block_children(bytes, &block),
                            },
                            Err(_) => Piece::Malformed {
                                kind: token,
                                span: span(at, end),
                            },
                        };
                        let resume = match piece {
                            Piece::Construct { span, .. } => span.end(),
                            _ => end,
                        };
                        pieces.push(piece);
                        at = resume;
                        text_start = at;
                        continue;
                    }
                    if token == EntryToken::Raw {
                        flush!(at);
                        region = Region::Raw { depth: 1 };
                        text_start = at;
                        at = end;
                        continue;
                    }
                    flush!(at);
                    // A construct that never closes on its line is reported at
                    // its entry token, and its bytes are still transported.
                    let close = if token == EntryToken::Import {
                        close_import(bytes, end)
                    } else {
                        close_paren(bytes, end)
                    };
                    let (piece, resume) = match close {
                        Some(close) => (
                            Piece::Construct {
                                kind: token,
                                span: span(at, close + 1),
                                children: Vec::new(),
                            },
                            close + 1,
                        ),
                        None => (
                            Piece::Malformed {
                                kind: token,
                                span: span(at, end),
                            },
                            end,
                        ),
                    };
                    pieces.push(piece);
                    at = resume;
                    text_start = at;
                    continue;
                }
                if let Some((new_region, resume)) = region_opening_at(bytes, at) {
                    enter!(at);
                    region = new_region;
                    at = resume;
                    continue;
                }
                // A category-code assignment changes what a byte *means* from
                // here on, and TeX's grouping after expansion can differ from
                // the braces we can see. There is no boundary to trust, so
                // recognition stops rather than resuming somewhere plausible.
                // `ROADMAP.md`'s hazard table, and the plan's declared
                // uncertainty, both land here.
                if bytes[at..].starts_with(b"\\catcode") {
                    enter!(at);
                    region = Region::Quarantine;
                    continue;
                }
                // A command's arguments are an exclusion region. The shape comes
                // from a signature, never from guessing which groups belong to
                // it — see signatures.rs.
                if bytes[at] == b'\\' {
                    match command_extent(bytes, at) {
                        Extent::Through(end) => {
                            // A caption is prose. The head of the call — the
                            // command, its data arguments, the opening
                            // delimiter — stays an exclusion region, and the
                            // interior is scanned exactly as the prose
                            // outside it, every inner region included.
                            // Issue #83; grammar §8's composition rule.
                            if let Some((interior_start, interior_end)) =
                                text_argument_interior(bytes, at)
                            {
                                enter!(at);
                                pieces.push(Piece::Arguments(span(at, interior_start)));
                                text_arguments.push((interior_end, end));
                                text_start = interior_start;
                                at = interior_start;
                                continue;
                            }
                            // §8 makes a command's arguments an exclusion
                            // region, and `Piece::Excluded` is what tells a
                            // consumer searching the source not to look here.
                            // Leaving these bytes inside a `Text` piece made
                            // that promise false: a `\label` in a
                            // `\newcommand` body was found by a reader that
                            // was doing exactly what the documentation said
                            // was safe.
                            enter!(at);
                            pieces.push(Piece::Arguments(span(at, end)));
                            text_start = end;
                            at = end;
                            continue;
                        }
                        Extent::Unbounded => {
                            // Quarantine is an excluded region like any other.
                            enter!(at);
                            region = Region::Quarantine;
                            continue;
                        }
                        Extent::NotACommand => {}
                    }
                }
                at += 1;
            }

            Region::Comment => {
                at += 1;
                if bytes[at - 1] == b'\n' {
                    pieces.push(Piece::Excluded(span(excluded_start, at)));
                    text_start = at;
                    region = Region::Prose;
                }
            }

            Region::InlineMath => {
                let closes = bytes[at] == b'$' && !is_escaped(bytes, at);
                at += 1;
                if closes {
                    pieces.push(Piece::Excluded(span(excluded_start, at)));
                    text_start = at;
                    region = Region::Prose;
                }
            }

            Region::DisplayMath { dollars } => {
                if *dollars {
                    if bytes[at] == b'$'
                        && !is_escaped(bytes, at)
                        && bytes.get(at + 1) == Some(&b'$')
                    {
                        at += 2;
                        pieces.push(Piece::Excluded(span(excluded_start, at)));
                        text_start = at;
                        region = Region::Prose;
                        continue;
                    }
                } else if bytes[at] == b'\\' && bytes.get(at + 1) == Some(&b']') {
                    at += 2;
                    pieces.push(Piece::Excluded(span(excluded_start, at)));
                    text_start = at;
                    region = Region::Prose;
                    continue;
                }
                at += 1;
            }

            Region::Verb { delimiter } => {
                let closes = bytes[at] == *delimiter;
                at += 1;
                if closes {
                    pieces.push(Piece::Excluded(span(excluded_start, at)));
                    text_start = at;
                    region = Region::Prose;
                }
            }

            Region::Environment { name } => {
                if let Some(end) = environment_end(bytes, at, name) {
                    at = end;
                    pieces.push(Piece::Excluded(span(excluded_start, at)));
                    text_start = at;
                    region = Region::Prose;
                } else {
                    at += 1;
                }
            }

            Region::ControlName => {
                if bytes[at..].starts_with(b"\\endcsname") {
                    at += b"\\endcsname".len();
                    pieces.push(Piece::Excluded(span(excluded_start, at)));
                    text_start = at;
                    region = Region::Prose;
                } else {
                    at += 1;
                }
            }

            Region::Conditional { depth } => {
                let mut depth = *depth;
                if bytes[at..].starts_with(b"\\fi") {
                    at += b"\\fi".len();
                    depth -= 1;
                    if depth == 0 {
                        pieces.push(Piece::Excluded(span(excluded_start, at)));
                        text_start = at;
                        region = Region::Prose;
                        continue;
                    }
                } else if bytes[at..].starts_with(b"\\if") {
                    // A nested conditional. Counting them is what keeps the
                    // first `\fi` of an inner one from closing the outer.
                    let rest = &bytes[at + 3..];
                    let name_len = rest.iter().take_while(|b| b.is_ascii_alphabetic()).count();
                    if !if_name_is_not_a_conditional(&rest[..name_len]) {
                        depth += 1;
                    }
                    at += 3 + name_len;
                } else {
                    at += 1;
                }
                region = Region::Conditional { depth };
            }

            Region::InternalMacros => {
                if bytes[at..].starts_with(b"\\makeatother") {
                    at += b"\\makeatother".len();
                    pieces.push(Piece::Excluded(span(excluded_start, at)));
                    text_start = at;
                    region = Region::Prose;
                } else {
                    at += 1;
                }
            }

            Region::Quarantine => {
                at = bytes.len();
            }

            Region::Raw { depth } => {
                let mut depth = *depth;
                if !is_escaped(bytes, at) {
                    match bytes[at] {
                        b'%' => {
                            // A comment inside a raw block hides its braces.
                            while at < bytes.len() && bytes[at] != b'\n' {
                                at += 1;
                            }
                            continue;
                        }
                        b'{' => depth += 1,
                        b'}' => depth -= 1,
                        _ => {}
                    }
                }
                at += 1;
                if depth == 0 {
                    pieces.push(Piece::Construct {
                        kind: EntryToken::Raw,
                        span: span(text_start, at),
                        children: Vec::new(),
                    });
                    text_start = at;
                    region = Region::Prose;
                } else {
                    region = Region::Raw { depth };
                }
            }
        }
    }

    // An unterminated region is still transported; only its classification
    // changes, and the diagnostic belongs to whatever opened it.
    match region {
        Region::Raw { .. } => pieces.push(Piece::Malformed {
            kind: EntryToken::Raw,
            span: span(text_start, bytes.len()),
        }),
        Region::Prose => flush!(bytes.len()),
        // A comment ends at end of file the same way it ends at a newline, so
        // its boundary was found and nothing is in doubt.
        Region::Comment => {
            if bytes.len() > excluded_start {
                pieces.push(Piece::Excluded(span(excluded_start, bytes.len())));
            }
        }
        // Every other region needed a terminator and did not get one. Its
        // boundary was never located, so recognition stopped where it opened
        // rather than resuming at a byte nobody can justify.
        _ => {
            if bytes.len() > excluded_start {
                pieces.push(Piece::Quarantined(span(excluded_start, bytes.len())));
            }
        }
    }

    pieces
}

fn block_children(bytes: &[u8], block: &crate::blocks::Block) -> Vec<Piece> {
    block
        .fields
        .iter()
        .filter_map(|field| match field.value {
            crate::blocks::Value::Braced(value) => Some(value),
            _ => None,
        })
        .flat_map(|value| {
            let start = value.start() + 1;
            scan(&bytes[start..value.end() - 1])
                .into_iter()
                .map(move |piece| piece.shifted(start))
        })
        .collect()
}

impl Piece {
    fn shifted(self, by: usize) -> Self {
        let shift = |span: Span| span_at(span.start() + by, span.end() + by);
        match self {
            Self::Text(span) => Self::Text(shift(span)),
            Self::Excluded(span) => Self::Excluded(shift(span)),
            Self::Arguments(span) => Self::Arguments(shift(span)),
            Self::Quarantined(span) => Self::Quarantined(shift(span)),
            Self::Construct {
                kind,
                span,
                children,
            } => Self::Construct {
                kind,
                span: shift(span),
                children: children
                    .into_iter()
                    .map(|child| child.shifted(by))
                    .collect(),
            },
            Self::Malformed { kind, span } => Self::Malformed {
                kind,
                span: shift(span),
            },
        }
    }

    /// Visits this piece and every nested piece in source order.
    pub fn walk(&self, visit: &mut impl FnMut(&Self)) {
        visit(self);
        if let Self::Construct { children, .. } = self {
            for child in children {
                child.walk(visit);
            }
        }
    }
}

/// The block kind an entry token opens, if it opens one.
const fn block_kind(token: EntryToken) -> Option<crate::blocks::BlockKind> {
    match token {
        EntryToken::Figure => Some(crate::blocks::BlockKind::Figure),
        EntryToken::Table => Some(crate::blocks::BlockKind::Table),
        _ => None,
    }
}

fn revision_piece(
    bytes: &[u8],
    token: EntryToken,
    start: usize,
    after_open: usize,
) -> (Piece, usize) {
    let Some(header_end) = revision_header_end(bytes, token, after_open) else {
        return (
            Piece::Malformed {
                kind: token,
                span: span(start, after_open),
            },
            after_open,
        );
    };
    let mut open = header_end;
    while bytes.get(open).is_some_and(u8::is_ascii_whitespace) {
        open += 1;
    }
    if bytes.get(open) != Some(&b'{') {
        let resume = (open + usize::from(open < bytes.len())).max(after_open);
        return (
            Piece::Malformed {
                kind: token,
                span: span(start, resume),
            },
            resume,
        );
    }
    let Some(end) = balanced_end(bytes, open) else {
        return (
            Piece::Malformed {
                kind: token,
                span: span(start, bytes.len()),
            },
            bytes.len(),
        );
    };
    let nested = nested_pieces(bytes, open + 1, end - 1);
    let valid = token != EntryToken::Sub || substitution_arrows(bytes, open + 1, end - 1) == 1;
    let piece = if valid {
        Piece::Construct {
            kind: token,
            span: span(start, end),
            children: nested,
        }
    } else {
        Piece::Malformed {
            kind: token,
            span: span(start, end),
        }
    };
    (piece, end)
}

fn revision_header_end(bytes: &[u8], token: EntryToken, mut at: usize) -> Option<usize> {
    at = ident_end(bytes, at)?;
    if token == EntryToken::Note {
        if bytes.get(at) != Some(&b',') {
            return None;
        }
        at += 1;
        while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        if !bytes.get(at..)?.starts_with(b"on") {
            return None;
        }
        at += 2;
        while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        if bytes.get(at) != Some(&b'=') {
            return None;
        }
        at += 1;
        while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        at = ident_end(bytes, at)?;
    }
    (bytes.get(at) == Some(&b')')).then_some(at + 1)
}

fn nested_pieces(bytes: &[u8], start: usize, end: usize) -> Vec<Piece> {
    scan(&bytes[start..end])
        .into_iter()
        .filter_map(|piece| match piece {
            Piece::Construct { .. } => Some(piece.shifted(start)),
            Piece::Malformed { kind, span: inner } => Some(Piece::Malformed {
                kind,
                span: span(start + inner.start(), start + inner.end()),
            }),
            Piece::Text(_) | Piece::Excluded(_) | Piece::Arguments(_) | Piece::Quarantined(_) => {
                None
            }
        })
        .collect()
}
