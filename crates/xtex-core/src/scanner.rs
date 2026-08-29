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

use crate::signatures::{Argument, is_known, signature_of};
use crate::source::Span;

/// Where the scanner is in the byte stream.
///
/// Each variant except [`Region::Prose`] is a region in which every entry token
/// is ordinary text.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Region {
    /// Constructs are recognised here.
    Prose,
    /// From an unescaped `%` to the line ending.
    Comment,
    /// From `$` to the matching `$`.
    InlineMath,
    /// From `$$` or `\[` to its match.
    DisplayMath { dollars: bool },
    /// From a verbatim command plus a delimiter byte to its next occurrence.
    Verb { delimiter: u8 },
    /// An environment body whose bytes are copied through its matching end.
    Environment { name: Vec<u8> },
    /// From `\makeatletter` to `\makeatother`.
    InternalMacros,
    /// From `\csname` to `\endcsname`.
    ///
    /// The name between them is built by expansion, so it is not knowable
    /// here and is never inferred.
    ControlName,
    /// From a `\if…` primitive to its matching `\fi`, counting nesting.
    ///
    /// Every branch is preserved and the condition is not evaluated, so both
    /// arms stay opaque rather than one being chosen.
    Conditional { depth: u32 },
    /// From a `latex {` entry to the matching `}`.
    Raw { depth: u32 },
    /// Nothing further is recognised in this file.
    ///
    /// Entered when a boundary could not be located. Preserving is always
    /// available; guessing is not.
    Quarantine,
}

/// How many adjacent groups an unknown command may claim before the parser
/// stops rather than guess that the next one is prose.
///
/// `docs/grammar.md` §8 fixes this at sixteen and says what would change it: a
/// real command absent from the databases with more arguments, or a documented
/// collision after a shorter run.
const MAX_UNKNOWN_COMMAND_GROUPS: usize = 16;

/// Environments whose bodies are copied rather than read.
///
/// Seeded from what a shallow LaTeX parser that handles real papers already
/// skips — `TexSoup`'s list, which names three this project had missed:
/// `Verbatim` from `fancyvrb`, `verbatimtab`, and `listing`. Extended per
/// project by `xtex.toml`.
pub const DEFAULT_VERBATIM_ENVIRONMENTS: &[&str] = &[
    "verbatim",
    "verbatim*",
    "Verbatim",
    "verbatimtab",
    "listing",
    "lstlisting",
    "minted",
];

/// Environments whose bodies are display math.
///
/// Transcribed from amsmath 2.17z (`amsmath.dtx`, 2025-07-09): its top-level
/// display definitions are equation, gather, align, alignat, xalignat,
/// xxalignat, flalign, and multline, with starred definitions where listed
/// here. `aligned`, `gathered`, `split`, and `cases` are inner structures, so
/// an occurrence of one cannot close the outer display region.
const DISPLAY_MATH_ENVIRONMENTS: &[(&str, &[Argument])] = &[
    ("align", &[]),
    ("align*", &[]),
    ("alignat", &[Argument::Mandatory]),
    ("alignat*", &[Argument::Mandatory]),
    ("equation", &[]),
    ("equation*", &[]),
    ("flalign", &[]),
    ("flalign*", &[]),
    ("gather", &[]),
    ("gather*", &[]),
    ("multline", &[]),
    ("multline*", &[]),
    ("xalignat", &[Argument::Mandatory]),
    ("xalignat*", &[Argument::Mandatory]),
    ("xxalignat", &[Argument::Mandatory]),
];

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

/// Every `@`-keyword, longest first so that `@import` is tried before `@id`.
const AT_TOKENS: &[EntryToken] = &[
    EntryToken::Import,
    EntryToken::Note,
    EntryToken::Add,
    EntryToken::Del,
    EntryToken::Sub,
    EntryToken::Cite,
    EntryToken::Ref,
    EntryToken::Id,
];

/// The citation commands a construct may name, longest first.
///
/// A citation construct is a LaTeX citation command written with `@`, so this
/// is a list of real command names rather than a vocabulary of our own. The
/// kernel provides `cite`; `natbib` provides `citep` and `citet`; `biblatex`
/// provides `textcite` and `parencite`. `docs/grammar.md` §4.
///
/// All five map to one [`EntryToken::Cite`]. Which command was written is in
/// the construct's own bytes, and the emitter reads it there — a variant per
/// command would put the same information in two places.
pub const DEFAULT_CITE_COMMANDS: &[&str] = &["parencite", "textcite", "citep", "citet", "cite"];

/// Regions a reader looking for `commands` may search.
///
/// Prose, plus any excluded region that *is* one of those commands — a
/// command's arguments are an exclusion region, and the only reader entitled to
/// look inside one is the reader that asked for that command by name.
///
/// The distinction matters: `\bibliography{refs}` is a declaration, and the
/// same bytes inside a `\newcommand` body are not. Searching every excluded
/// region would find both; searching none would find neither.
#[must_use]
pub fn readable_for(bytes: &[u8], commands: &[&str]) -> Vec<Span> {
    let mut spans = Vec::new();
    for piece in scan(bytes) {
        match piece {
            Piece::Text(span) => spans.push(span),
            Piece::Arguments(span) => {
                let region = &bytes[span.start()..span.end()];
                if commands.iter().any(|command| {
                    region
                        .strip_prefix(b"\\")
                        .is_some_and(|rest| rest.starts_with(command.as_bytes()))
                }) {
                    spans.push(span);
                }
            }
            _ => {}
        }
    }
    spans
}

/// Commands whose argument is a *definition* rather than content.
///
/// The distinction §8 is actually about. A `\label` inside `\caption{…}` is a
/// real declaration — that argument is typeset. The same `\label` inside a
/// `\newcommand` body is not: nothing there has happened yet, and it may never
/// happen.
pub const DEFINITION_COMMANDS: &[&str] = &[
    "newcommand",
    "renewcommand",
    "providecommand",
    "newenvironment",
    "renewenvironment",
    "def",
    "edef",
    "gdef",
    "xdef",
    "csname",
];

/// Regions holding content a reader may search.
///
/// Prose, plus every excluded region except a definition's. A command's
/// arguments are an exclusion region so that *constructs* are not recognised
/// there; a reader looking for the author's own `\label` is asking a different
/// question, and the answer differs only for definitions.
#[must_use]
pub fn readable_content(bytes: &[u8]) -> Vec<Span> {
    let mut spans = Vec::new();
    for piece in scan(bytes) {
        match piece {
            Piece::Text(span) => spans.push(span),
            Piece::Arguments(span) => {
                let region = &bytes[span.start()..span.end()];
                let defines = region.strip_prefix(b"\\").is_some_and(|rest| {
                    DEFINITION_COMMANDS.iter().any(|command| {
                        rest.strip_prefix(command.as_bytes()).is_some_and(|after| {
                            !after.first().is_some_and(u8::is_ascii_alphabetic)
                        })
                    })
                });
                if !defines {
                    spans.push(span);
                }
            }
            _ => {}
        }
    }
    spans
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

fn span_at(start: usize, end: usize) -> Span {
    Span::new(
        u32::try_from(start).unwrap_or(u32::MAX),
        u32::try_from(end).unwrap_or(u32::MAX),
    )
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

fn ident_end(bytes: &[u8], start: usize) -> Option<usize> {
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

fn substitution_arrows(bytes: &[u8], start: usize, end: usize) -> usize {
    let mut depth = 1u32;
    let mut count = 0usize;
    for piece in scan(&bytes[start..end]) {
        let Piece::Text(text) = piece else { continue };
        let mut at = start + text.start();
        let text_end = start + text.end();
        while at + 1 < text_end {
            if bytes[at] == b'\\' {
                if let Extent::Through(next) = command_extent(bytes, at) {
                    if next <= text_end {
                        at = next;
                        continue;
                    }
                }
            }
            if !is_escaped(bytes, at) {
                match bytes[at] {
                    b'{' => depth += 1,
                    b'}' => depth = depth.saturating_sub(1),
                    b'-' if depth == 1 && bytes[at + 1] == b'>' => {
                        count += 1;
                        at += 2;
                        continue;
                    }
                    _ => {}
                }
            }
            at += 1;
        }
    }
    count
}

/// The depth-one separator in a valid substitution.
#[must_use]
pub fn substitution_separator(bytes: &[u8], start: usize, end: usize) -> Option<usize> {
    let mut depth = 1u32;
    for piece in scan(&bytes[start..end]) {
        let Piece::Text(text) = piece else { continue };
        let mut at = start + text.start();
        let text_end = start + text.end();
        while at + 1 < text_end {
            if bytes[at] == b'\\' {
                if let Extent::Through(next) = command_extent(bytes, at) {
                    if next <= text_end {
                        at = next;
                        continue;
                    }
                }
            }
            if !is_escaped(bytes, at) {
                match bytes[at] {
                    b'{' => depth += 1,
                    b'}' => depth = depth.saturating_sub(1),
                    b'-' if depth == 1 && bytes[at + 1] == b'>' => return Some(at),
                    _ => {}
                }
            }
            at += 1;
        }
    }
    None
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

fn span(start: usize, end: usize) -> Span {
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
fn is_escaped(bytes: &[u8], at: usize) -> bool {
    let mut run = 0usize;
    let mut i = at;
    while i > 0 && bytes[i - 1] == b'\\' {
        run += 1;
        i -= 1;
    }
    run % 2 == 1
}

/// A complete entry token starting at `at`, and the offset just past it.
fn entry_token_at(bytes: &[u8], at: usize) -> Option<(EntryToken, usize)> {
    if bytes[at] == b'@' {
        for token in AT_TOKENS {
            let keyword = token.keyword();
            let end = at + keyword.len();
            if bytes[at..].starts_with(keyword) && bytes.get(end) == Some(&b'(') {
                return Some((*token, end + 1));
            }
        }
        for command in DEFAULT_CITE_COMMANDS {
            let end = at + 1 + command.len();
            if bytes[at + 1..].starts_with(command.as_bytes()) && bytes.get(end) == Some(&b'(') {
                return Some((EntryToken::Cite, end + 1));
            }
        }
        return None;
    }

    if bytes[at] == b'\\' {
        for token in [EntryToken::Figure, EntryToken::Table] {
            let keyword = token.keyword();
            let end = at + keyword.len();
            if bytes[at..].starts_with(keyword) && bytes.get(end) == Some(&b'(') {
                return Some((token, end + 1));
            }
        }
    }

    if bytes[at] == b'l' && bytes[at..].starts_with(b"latex") {
        // A bare word, so it must not be part of a longer one.
        let before_ok = at == 0 || !bytes[at - 1].is_ascii_alphanumeric() && bytes[at - 1] != b'\\';
        let mut i = at + b"latex".len();
        while matches!(bytes.get(i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            i += 1;
        }
        if before_ok && bytes.get(i) == Some(&b'{') {
            return Some((EntryToken::Raw, i + 1));
        }
    }

    None
}

/// Offset of the `)` closing a construct opened at `from`, on the same line.
///
/// A construct does not scan past a line ending: an unterminated one ends
/// there, is reported, and parsing resumes on the next line.
fn close_paren(bytes: &[u8], from: usize) -> Option<usize> {
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

fn close_import(bytes: &[u8], from: usize) -> Option<usize> {
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

/// How far a command and its arguments reach.
enum Extent {
    /// The command and its arguments end just before this offset.
    Through(usize),
    /// A boundary could not be located; nothing further is recognised.
    Unbounded,
    /// The backslash does not begin a control word.
    NotACommand,
}

/// The offset just past a command at `at` and every argument it claims.
///
/// For a command with a known signature, the arguments are exactly the ones the
/// signature selects. For one with no signature, §8 allows a run of adjacent
/// balanced groups to be treated as its arguments, bounded at sixteen — beyond
/// that the parser stops rather than assume the seventeenth group is prose.
/// Bytes that never begin a single-token mandatory argument.
///
/// A mandatory argument may be one token — `\\newcommand\\foo{}` is real LaTeX.
/// But these bytes open some *other* xparse argument form, so meeting one where
/// a mandatory argument was expected means the signature does not describe this
/// call. See `docs/grammar.md` §8.
const ARGUMENT_OPENERS: &[u8] = b"[<(*";

/// Known commands whose final mandatory argument is prose.
///
/// A caption is a sentence; treating it as data made `@ref(fig:x)` inside
/// one emit literally into the PDF with exit 0 — Phase 0a's gap 3, decided
/// as issue #83. Each entry is transcribed against its signature in the
/// table above; a command absent from the signature table cannot be listed
/// (`title` and `author` are not, and stay excluded — the conservative
/// default the issue fixes for the unclassified).
///
/// `item` is not here because its prose lives in its *optional* argument;
/// see [`TEXT_OPTIONAL_COMMANDS`].
const TEXT_MANDATORY_COMMANDS: &[&[u8]] = &[
    b"caption",
    b"chapter",
    b"emph",
    b"footnote",
    b"footnotetext",
    b"mbox",
    b"paragraph",
    b"part",
    b"section",
    b"subparagraph",
    b"subsection",
    b"subsubsection",
    b"textbf",
    b"textit",
    b"underline",
];
// `\texttt` is deliberately absent, against the issue's first list: it is
// the code font, and `\texttt{@ref(x)}` is how prose *shows* the literal
// token — fixture revisions/04 exists to hold exactly that. Converting it
// would corrupt any document that documents ExactTeX.

/// Known commands whose optional argument is prose.
const TEXT_OPTIONAL_COMMANDS: &[&[u8]] = &[b"item"];

/// The interior of a text-bearing argument, when this call has one.
///
/// Walks the same signature the command was consumed under and returns the
/// byte range inside the prose argument's delimiters. `None` when the
/// command bears no prose, when the argument is absent, or when it is not
/// in delimited form — each of those keeps today's whole-argument
/// exclusion, which is the safe direction.
fn text_argument_interior(bytes: &[u8], at: usize) -> Option<(usize, usize)> {
    let name_start = at + 1;
    let mut name_end = name_start;
    while matches!(bytes.get(name_end), Some(b) if b.is_ascii_alphabetic()) {
        name_end += 1;
    }
    let name = &bytes[name_start..name_end];
    let mandatory = TEXT_MANDATORY_COMMANDS.contains(&name);
    let optional = TEXT_OPTIONAL_COMMANDS.contains(&name);
    if !mandatory && !optional {
        return None;
    }
    let signature = signature_of(name)?;
    let mut cursor = name_end;
    let mut last_mandatory = None;
    let mut first_optional = None;
    for argument in signature {
        cursor = skip_ascii_whitespace(bytes, cursor);
        match argument {
            Argument::Star => {
                if bytes.get(cursor) == Some(&b'*') {
                    cursor += 1;
                }
            }
            Argument::Optional => {
                if bytes.get(cursor) == Some(&b'[') {
                    let end = delimited_end(bytes, cursor, b'[', b']')?;
                    if first_optional.is_none() {
                        first_optional = Some((cursor + 1, end - 1));
                    }
                    cursor = end;
                }
            }
            Argument::Mandatory => {
                if bytes.get(cursor) == Some(&b'{') {
                    let end = balanced_end(bytes, cursor)?;
                    last_mandatory = Some((cursor + 1, end - 1));
                    cursor = end;
                } else {
                    return None;
                }
            }
            Argument::Delimited(open, close) => {
                if bytes.get(cursor) == Some(&open) {
                    cursor = delimited_end(bytes, cursor, open, close)?;
                }
            }
        }
    }
    let interior = if mandatory {
        last_mandatory
    } else {
        first_optional
    }?;
    (interior.0 < interior.1).then_some(interior)
}

fn command_extent(bytes: &[u8], at: usize) -> Extent {
    let name_start = at + 1;
    let mut name_end = name_start;
    while matches!(bytes.get(name_end), Some(b) if b.is_ascii_alphabetic()) {
        name_end += 1;
    }
    if name_end == name_start {
        return Extent::NotACommand;
    }
    let name = &bytes[name_start..name_end];
    let mut cursor = name_end;

    if let Some(signature) = signature_of(name) {
        // A signature is a claim about this call, and the call can refute it.
        // 15 of the built-in commands carry a different signature under another
        // package — `\\definecolor` is `m m m` under `color` and `o m m m` under
        // `xcolor`, and beamer adds a `<overlay>` argument to twelve more. When
        // the bytes do not fit, trusting the signature anyway resumes parsing
        // *inside* an argument and exposes its contents to recognition. Falling
        // through to the unknown-command rule excludes them instead.
        let mut fits = true;
        for argument in signature {
            cursor = skip_ascii_whitespace(bytes, cursor);
            match argument {
                Argument::Star => {
                    if bytes.get(cursor) == Some(&b'*') {
                        cursor += 1;
                    }
                }
                Argument::Optional => {
                    if bytes.get(cursor) == Some(&b'[') {
                        match delimited_end(bytes, cursor, b'[', b']') {
                            Some(end) => cursor = end,
                            None => return Extent::Unbounded,
                        }
                    }
                }
                Argument::Mandatory => {
                    if bytes.get(cursor) == Some(&b'{') {
                        match balanced_end(bytes, cursor) {
                            Some(end) => cursor = end,
                            None => return Extent::Unbounded,
                        }
                    } else if bytes
                        .get(cursor)
                        .is_some_and(|b| ARGUMENT_OPENERS.contains(b))
                    {
                        fits = false;
                        break;
                    } else {
                        // A mandatory argument may be a single token.
                        cursor = (cursor + 1).min(bytes.len());
                    }
                }
                Argument::Delimited(open, close) => {
                    if bytes.get(cursor) == Some(&open) {
                        match delimited_end(bytes, cursor, open, close) {
                            Some(end) => cursor = end,
                            None => return Extent::Unbounded,
                        }
                    }
                }
            }
        }
        if fits {
            return Extent::Through(cursor);
        }
        cursor = name_end;
    } else if is_known(name) {
        return Extent::Through(cursor);
    }

    // No signature, or one this call refuted. Claim adjacent groups, bounded.
    let mut groups = 0usize;
    loop {
        let next = skip_ascii_whitespace(bytes, cursor);
        let (open, close) = match bytes.get(next) {
            Some(b'{') => (b'{', b'}'),
            Some(b'[') => (b'[', b']'),
            _ => break,
        };
        if groups == MAX_UNKNOWN_COMMAND_GROUPS {
            return Extent::Unbounded;
        }
        let end = if open == b'{' {
            balanced_end(bytes, next)
        } else {
            delimited_end(bytes, next, open, close)
        };
        match end {
            Some(e) => cursor = e,
            None => return Extent::Unbounded,
        }
        groups += 1;
    }
    Extent::Through(cursor)
}

/// Offset just past the `close` matching the `open` at `from`.
fn delimited_end(bytes: &[u8], from: usize, open: u8, close: u8) -> Option<usize> {
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

fn skip_ascii_whitespace(bytes: &[u8], mut at: usize) -> usize {
    while matches!(bytes.get(at), Some(b' ' | b'\t')) {
        at += 1;
    }
    at
}

/// A region beginning at `at`, and where scanning resumes.
fn region_opening_at(bytes: &[u8], at: usize) -> Option<(Region, usize)> {
    match bytes[at] {
        b'%' if !is_escaped(bytes, at) => Some((Region::Comment, at + 1)),
        b'$' if !is_escaped(bytes, at) => {
            if bytes.get(at + 1) == Some(&b'$') {
                Some((Region::DisplayMath { dollars: true }, at + 2))
            } else {
                Some((Region::InlineMath, at + 1))
            }
        }
        b'\\' => {
            // `\\[4pt]` is a line break with its optional argument, and the
            // `\[` inside it is not display math. Without this, 448
            // occurrences across 71 files in the measured corpus each sent the
            // rest of their document to quarantine — one real paper went dark
            // at 6% of its bytes.
            //
            // The parity rule already exists for exactly this: a backslash
            // preceded by an odd run of backslashes is escaped.
            if bytes[at..].starts_with(b"\\[") && !is_escaped(bytes, at) {
                return Some((Region::DisplayMath { dollars: false }, at + 2));
            }
            if bytes[at..].starts_with(b"\\makeatletter") {
                return Some((Region::InternalMacros, at + b"\\makeatletter".len()));
            }
            if bytes[at..].starts_with(b"\\csname") {
                return Some((Region::ControlName, at + b"\\csname".len()));
            }
            if let Some(rest) = bytes[at..].strip_prefix(b"\\if") {
                let name_len = rest.iter().take_while(|b| b.is_ascii_alphabetic()).count();
                if !if_name_is_not_a_conditional(&rest[..name_len]) {
                    return Some((
                        Region::Conditional { depth: 1 },
                        at + b"\\if".len() + name_len,
                    ));
                }
            }
            if let Some(opening) = verbatim_command_opening(bytes, at) {
                return Some(opening);
            }
            if let Some(rest) = bytes[at..].strip_prefix(b"\\begin{") {
                let close = rest.iter().position(|b| *b == b'}')?;
                let name = &rest[..close];
                if DEFAULT_VERBATIM_ENVIRONMENTS
                    .iter()
                    .any(|known| known.as_bytes() == name)
                {
                    let consumed = at + b"\\begin{".len() + close + 1;
                    return Some((
                        Region::Environment {
                            name: name.to_vec(),
                        },
                        consumed,
                    ));
                }
            }
            None
        }
        _ => None,
    }
}

/// A verbatim command's delimiter region beginning at `at`.
///
/// `\lstinline` may have an optional argument before the delimiter. `\mint`
/// and `\mintinline` have a mandatory language argument, and `\mintinline`
/// may also use a braced code argument. The braced inline forms return `None`
/// so the ordinary balanced command-argument path handles them.
fn verbatim_command_opening(bytes: &[u8], at: usize) -> Option<(Region, usize)> {
    let (name, mut cursor) = control_word_at(bytes, at)?;
    match name {
        b"verb" => {
            if bytes.get(cursor) == Some(&b'*') {
                cursor += 1;
            }
        }
        b"lstinline" => {
            if bytes.get(cursor) == Some(&b'[') {
                cursor = match delimited_end(bytes, cursor, b'[', b']') {
                    Some(end) => end,
                    None => return Some((Region::Quarantine, cursor)),
                };
            }
            if bytes.get(cursor) == Some(&b'{') {
                return None;
            }
        }
        b"mint" | b"mintinline" => {
            if bytes.get(cursor) != Some(&b'{') {
                return None;
            }
            cursor = match balanced_end(bytes, cursor) {
                Some(end) => end,
                None => return Some((Region::Quarantine, cursor)),
            };
            if name == b"mintinline" && bytes.get(cursor) == Some(&b'{') {
                return None;
            }
        }
        _ => return None,
    }

    // TeX absorbs the spaces after a control word, so the delimiter is the
    // next byte that is not one. Compiled to check rather than recalled:
    // `\verb xCODEx` typesets `CODE`, so the space was skipped and `x` was the
    // delimiter. A line ending is different — `\verb` followed by one opens no
    // region at all, and `|CODE|` after it is typeset as ordinary text. Both
    // observations are fixtures 15 and 16.
    while matches!(bytes.get(cursor), Some(b' ' | b'\t')) {
        cursor += 1;
    }
    let delimiter = match bytes.get(cursor) {
        // No delimiter on this line. Not a verbatim command, so ordinary
        // handling continues — quarantining here would cost recognition for
        // the rest of the file over bytes TeX itself ignores.
        Some(b'\n' | b'\r') | None => return None,
        Some(delimiter) => *delimiter,
    };
    Some((Region::Verb { delimiter }, cursor + 1))
}

/// The alphabetic control word at `at` and the byte just past it.
fn control_word_at(bytes: &[u8], at: usize) -> Option<(&[u8], usize)> {
    if bytes.get(at) != Some(&b'\\') {
        return None;
    }
    let start = at + 1;
    let mut end = start;
    while bytes.get(end).is_some_and(u8::is_ascii_alphabetic) {
        end += 1;
    }
    (end > start).then_some((&bytes[start..end], end))
}

/// Offset just past `\end{name}` if it begins at `at`.
fn environment_end(bytes: &[u8], at: usize, name: &[u8]) -> Option<usize> {
    let mut needle = Vec::with_capacity(name.len() + 7);
    needle.extend_from_slice(b"\\end{");
    needle.extend_from_slice(name);
    needle.push(b'}');
    bytes[at..]
        .starts_with(&needle)
        .then_some(at + needle.len())
}

/// A display body start and the environment name that closes it.
fn display_math_environment_opening(bytes: &[u8], at: usize) -> Option<(usize, Vec<u8>)> {
    let rest = bytes.get(at..)?.strip_prefix(b"\\begin{")?;
    let close = rest.iter().position(|byte| *byte == b'}')?;
    let name = &rest[..close];
    let (_, signature) = DISPLAY_MATH_ENVIRONMENTS
        .iter()
        .find(|(known, _)| known.as_bytes() == name)?;
    let mut cursor = at + b"\\begin{".len() + close + 1;

    for argument in *signature {
        cursor = skip_ascii_whitespace(bytes, cursor);
        cursor = match argument {
            Argument::Mandatory if bytes.get(cursor) == Some(&b'{') => balanced_end(bytes, cursor)?,
            Argument::Mandatory => (cursor + 1).min(bytes.len()),
            _ => unreachable!("display environment signatures contain only mandatory arguments"),
        };
    }

    loop {
        cursor = display_header_whitespace_end(bytes, cursor);
        if cursor == bytes.len() {
            break;
        }
        let Some((EntryToken::Id, after_open)) = entry_token_at(bytes, cursor) else {
            break;
        };
        cursor = close_paren(bytes, after_open).map_or(after_open, |close| close + 1);
    }
    Some((cursor, name.to_vec()))
}

/// The bounded whitespace portion of one display header slot step.
fn display_header_whitespace_end(bytes: &[u8], mut at: usize) -> usize {
    let start = at;
    let mut line_endings = 0usize;
    while at - start < 256 {
        match bytes.get(at) {
            Some(b' ' | b'\t') => at += 1,
            Some(b'\n') if line_endings < 2 => {
                line_endings += 1;
                at += 1;
            }
            Some(b'\r') if bytes.get(at + 1) == Some(&b'\n') && line_endings < 2 => {
                if at + 2 - start > 256 {
                    break;
                }
                line_endings += 1;
                at += 2;
            }
            _ => break,
        }
    }
    at
}

/// The `label` values declared in listing-environment header options.
///
/// `\begin{lstlisting}[…, label={lst:x}]` declares `lst:x` with no `\label`
/// anywhere — the label is a package option, read here so a correct
/// `@ref(lst:x)` is not a false hard error. The environments are only the
/// ones whose `label` option *is* a reference target: `listings` runs
/// `\label` internally for it. `fancyvrb`'s `Verbatim` also takes a `label`
/// option, but there it titles the frame and declares nothing, and plain
/// `verbatim` takes no options at all — reading a `[` from its body as an
/// option list would misread a correct document.
///
/// Only headers the scanner itself opened are read: an excluded region that
/// begins with the environment's `\begin` is one the scanner entered, so a
/// `\begin{lstlisting}` inside a comment or another verbatim body never
/// reaches this.
///
/// # Errors
///
/// A `label` value that is not literal — a control sequence, inner braces,
/// non-ASCII — or an option list whose `]` cannot be found returns the span
/// it stopped at. The caller makes the whole inventory unavailable: a
/// half-read inventory turns correct references into false errors.
pub fn listing_header_labels(bytes: &[u8]) -> Result<Vec<(String, Span)>, Span> {
    const LABEL_OPTION_ENVIRONMENTS: &[&str] = &["lstlisting"];
    let mut found = Vec::new();
    for piece in scan(bytes) {
        let Piece::Excluded(region) = piece else {
            continue;
        };
        let start = region.start();
        let slice = &bytes[start..region.end()];
        let Some(rest) = slice.strip_prefix(b"\\begin{") else {
            continue;
        };
        let Some(close) = rest.iter().position(|byte| *byte == b'}') else {
            continue;
        };
        if !LABEL_OPTION_ENVIRONMENTS
            .iter()
            .any(|name| name.as_bytes() == &rest[..close])
        {
            continue;
        }
        let mut at = b"\\begin{".len() + close + 1;
        // Spaces and tabs may precede the option list; a line ending means
        // the body has begun and there are no options.
        while matches!(slice.get(at), Some(b' ' | b'\t')) {
            at += 1;
        }
        if slice.get(at) != Some(&b'[') {
            continue;
        }
        at += 1;
        let list_start = at;
        // The option list is TeX tokens under default category codes, so a
        // `%` opens a comment to the line ending — the measured header in
        // the Phase 0a paper carries one — and braces nest.
        let mut depth = 0usize;
        let mut segment = list_start;
        let mut closed = None;
        while at < slice.len() {
            match slice[at] {
                b'%' if !is_escaped(slice, at) => {
                    while at < slice.len() && slice[at] != b'\n' {
                        at += 1;
                    }
                    continue;
                }
                b'{' => depth += 1,
                b'}' => depth = depth.saturating_sub(1),
                b',' | b']' if depth == 0 => {
                    read_label_option(&slice[segment..at], start + segment, &mut found).map_err(
                        |offset| {
                            Span::new(
                                u32::try_from(start + segment + offset).unwrap_or(u32::MAX),
                                u32::try_from(start + at).unwrap_or(u32::MAX),
                            )
                        },
                    )?;
                    if slice[at] == b']' {
                        closed = Some(at);
                        break;
                    }
                    segment = at + 1;
                }
                _ => {}
            }
            at += 1;
        }
        if closed.is_none() {
            // The list never closes. Its contents cannot be bounded, so
            // nothing read from it may be trusted.
            return Err(region);
        }
    }
    Ok(found)
}

/// Reads one `key = value` option segment, keeping only a literal `label`.
///
/// Returns the offset of an unusable `label` value inside the segment.
fn read_label_option(
    segment: &[u8],
    base: usize,
    found: &mut Vec<(String, Span)>,
) -> Result<(), usize> {
    let equals = segment.iter().position(|byte| *byte == b'=');
    let Some(equals) = equals else { return Ok(()) };
    let key: Vec<u8> = segment[..equals]
        .iter()
        .copied()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    if key != b"label" {
        return Ok(());
    }
    let raw = &segment[equals + 1..];
    let lead = raw
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(raw.len());
    let trail = raw
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(lead, |i| i + 1);
    let mut value = &raw[lead..trail];
    let mut offset = equals + 1 + lead;
    // The braced and unbraced forms both occur; strip one balanced pair.
    if value.first() == Some(&b'{') && value.last() == Some(&b'}') && value.len() >= 2 {
        value = &value[1..value.len() - 1];
        offset += 1;
    }
    let literal = !value.is_empty()
        && value.iter().all(|byte| {
            byte.is_ascii() && !byte.is_ascii_control() && !matches!(byte, b'\\' | b'{' | b'}')
        });
    if !literal {
        return Err(offset);
    }
    if let Ok(name) = std::str::from_utf8(value) {
        found.push((
            name.trim().to_owned(),
            Span::new(
                u32::try_from(base + offset).unwrap_or(u32::MAX),
                u32::try_from(base + offset + value.len()).unwrap_or(u32::MAX),
            ),
        ));
    }
    Ok(())
}

/// Control words beginning with `if` that are not conditionals.
///
/// The name is everything after `\if`, and the comparison is against the
/// complete name: `\iffalse` is a real kernel conditional whose name also
/// begins with `f`, so a prefix match is exactly the wrong implementation.
/// One rule, called from both the opening site and the nested-count site, so
/// the two cannot drift apart.
///
/// - `thenelse` — `\ifthenelse` takes braced arguments and has no `\fi`; the
///   signature path handles it as a command.
/// - `f` — `\iff` is the kernel's ⟺ symbol, transcribed from `fontmath.ltx`
///   (TeX Live 2026-03-01): `\DeclareRobustCommand \iff{\;\Longleftrightarrow\;}`.
///   No `\fi` exists. Found quarantining real papers by the external corpus
///   (issue #79); package-defined braced conditionals such as `etoolbox`'s
///   `\ifblank` are deliberately not listed — extending this by configuration
///   is a policy decision recorded in the issue, not made here.
fn if_name_is_not_a_conditional(name: &[u8]) -> bool {
    matches!(name, b"thenelse" | b"f")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reassemble(bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for piece in scan(bytes) {
            let span = match piece {
                Piece::Text(s)
                | Piece::Excluded(s)
                | Piece::Arguments(s)
                | Piece::Quarantined(s)
                | Piece::Construct { span: s, .. }
                | Piece::Malformed { span: s, .. } => s,
            };
            out.extend_from_slice(&bytes[span.start()..span.end()]);
        }
        out
    }

    /// Where a piece sits, whatever kind it is.
    fn extent(piece: &Piece) -> Span {
        match piece {
            Piece::Text(s)
            | Piece::Excluded(s)
            | Piece::Arguments(s)
            | Piece::Quarantined(s)
            | Piece::Construct { span: s, .. }
            | Piece::Malformed { span: s, .. } => *s,
        }
    }

    /// Inputs chosen so that truncating them enters every region the scanner
    /// has, and leaves each one unterminated in turn.
    const AWKWARD: &[&[u8]] = &[
        b"\\section{Caf\xE9}\r\n%% comment\t\n\\ref{a} @ref(b) trailing",
        b"before %comment\n@id(x) $math$ after",
        b"\\verb+@ref(a)+ then @ref(b)",
        b"\\begin{verbatim}\n@id(v)\n\\end{verbatim} @id(real)",
        b"\\begin{equation}\n@id(eq:x)\nX \\in [A,B)",
        b"\\makeatletter \\a@b \\makeatother @id(after)",
        b"$$@ref(display)$$ and \\[@ref(bracket)\\] done",
        b"latex { \\raw{@ref(inside)} } @ref(outside)",
        b"\\figure(f) { caption = {C} } @ref(f)",
        b"text \\unknowncommand{a}{b} more @cite(k)",
    ];

    #[test]
    fn the_pieces_cover_every_byte_exactly_once() {
        // The reassembly test above compares the concatenation, which hides
        // *where* coverage broke. This asserts the property directly: pieces
        // run in order, start where the last one ended, and reach the end.
        //
        // It exists because the same defect appeared twice — a region entered
        // without recording where it began, so a later piece restarted inside
        // an earlier one. Both times the bytes were emitted twice and both
        // times it took a fixture to notice.
        for input in AWKWARD {
            for cut in 0..=input.len() {
                let slice = &input[..cut];
                let mut next = 0usize;
                for piece in scan(slice) {
                    let span = extent(&piece);
                    assert_eq!(
                        span.start(),
                        next,
                        "piece {piece:?} does not start where the last one ended, \
                         in {slice:?} cut at {cut}"
                    );
                    assert!(
                        span.end() >= span.start(),
                        "piece {piece:?} ends before it starts"
                    );
                    next = span.end();
                }
                assert_eq!(
                    next,
                    slice.len(),
                    "the pieces stop before the end of {slice:?} cut at {cut}"
                );
            }
        }
    }

    fn constructs(bytes: &[u8]) -> Vec<EntryToken> {
        let mut found = Vec::new();
        for piece in scan(bytes) {
            piece.walk(&mut |piece| {
                if let Piece::Construct { kind, .. } = piece {
                    found.push(*kind);
                }
            });
        }
        found
    }

    #[test]
    fn scanning_covers_every_byte_exactly_once() {
        for input in [
            b"plain text".as_slice(),
            b"@ref(x) and @id(y)",
            b"% @ref(hidden)\n@ref(seen)",
            b"$@ref(math)$ @ref(prose)",
            b"\\verb|@ref(v)| @ref(after)",
            b"latex {@ref(raw)} @ref(after)",
            b"\\makeatletter @ref(no) \\makeatother @ref(yes)",
            b"unterminated @ref(x",
            b"",
        ] {
            assert_eq!(reassemble(input), input, "lost bytes in {input:?}");
        }
    }

    #[test]
    fn a_construct_in_prose_is_recognised() {
        assert_eq!(
            constructs(b"See @ref(a) and @id(b)."),
            [EntryToken::Ref, EntryToken::Id]
        );
    }

    #[test]
    fn a_comment_hides_a_construct() {
        assert_eq!(constructs(b"% @ref(a)\n@ref(b)"), [EntryToken::Ref]);
    }

    #[test]
    fn an_escaped_percent_does_not_open_a_comment() {
        // 120 of these appear inside captions in the corpus; treating them as
        // comments truncates real content.
        assert_eq!(constructs(b"94\\% then @ref(a)"), [EntryToken::Ref]);
        assert_eq!(constructs(b"a \\\\% then @ref(a)"), []);
    }

    #[test]
    fn math_hides_a_construct_and_prose_after_it_does_not() {
        assert_eq!(constructs(b"$@ref(a)$ @ref(b)"), [EntryToken::Ref]);
        assert_eq!(constructs(b"$$@ref(a)$$ @ref(b)"), [EntryToken::Ref]);
        assert_eq!(constructs(b"\\[@ref(a)\\] @ref(b)"), [EntryToken::Ref]);
    }

    #[test]
    fn display_math_environments_hide_commands_and_constructs() {
        for (name, signature) in DISPLAY_MATH_ENVIRONMENTS {
            let argument = if signature.is_empty() { "" } else { "{2}" };
            let input = format!(
                "\\begin{{{name}}}{argument} X \\in [A,B) @id(hidden) @ref(hidden) \\bigg \\end{{{name}}} @ref(after)"
            );
            assert_eq!(
                constructs(input.as_bytes()),
                [EntryToken::Ref],
                "{name} did not hide its body"
            );
            assert!(
                !scan(input.as_bytes())
                    .iter()
                    .any(|piece| matches!(piece, Piece::Quarantined(_))),
                "{name} quarantined a bounded display"
            );
        }
    }

    #[test]
    fn a_display_header_id_is_recognised_before_the_body() {
        let input = b"\\begin{equation}\n @id(eq:x)\n E = @ref(hidden) \\end{equation} @ref(after)";
        assert_eq!(constructs(input), [EntryToken::Id, EntryToken::Ref],);
    }

    #[test]
    fn an_inner_math_environment_does_not_close_the_outer_region() {
        let input = b"\\begin{equation} X=\\begin{cases}a\\end{cases} @ref(hidden) \\
                      \\end{equation} @ref(after)";
        assert_eq!(constructs(input), [EntryToken::Ref]);
    }

    #[test]
    fn an_unterminated_display_environment_quarantines_to_eof() {
        let input = b"\\begin{equation}\n X = @ref(hidden)";
        let pieces = scan(input);
        assert_eq!(constructs(input), []);
        assert!(
            matches!(pieces.last(), Some(Piece::Quarantined(span)) if span.end() == input.len())
        );
    }

    #[test]
    fn iff_in_prose_is_the_kernel_symbol_and_opens_nothing() {
        // `\iff` is ⟺, not a conditional; there is no `\fi` and never will
        // be. Before the fix this quarantined the rest of the file, which is
        // how the external corpus found it in real papers.
        let input = b"A \\iff B en prosa. @ref(after)";
        assert_eq!(constructs(input), [EntryToken::Ref]);
        assert!(
            !scan(input)
                .iter()
                .any(|piece| matches!(piece, Piece::Quarantined(_)))
        );
    }

    #[test]
    fn iffalse_is_a_real_conditional_and_the_exception_must_not_reach_it() {
        // `\iffalse` begins with the same four bytes as `\iff`. A prefix
        // comparison excepts both; only the complete-name comparison excepts
        // one. The closed form scans as a conditional region; the unclosed
        // form quarantines.
        let closed = b"\\iffalse hidden @ref(hidden) \\fi @ref(after)";
        assert_eq!(constructs(closed), [EntryToken::Ref]);

        let unclosed = b"\\iffalse nunca cierra @ref(hidden)";
        assert_eq!(constructs(unclosed), []);
        assert!(
            scan(unclosed)
                .iter()
                .any(|piece| matches!(piece, Piece::Quarantined(_)))
        );
    }

    #[test]
    fn iff_nested_inside_a_real_conditional_does_not_deepen_it() {
        // The nested-count site shares the rule. If `\iff` counted as a
        // nested conditional, this `\fi` would close depth two of three and
        // the region would swallow the file.
        let input = b"\\ifnum x \\iff y \\fi @ref(after)";
        assert_eq!(constructs(input), [EntryToken::Ref]);
        assert!(
            !scan(input)
                .iter()
                .any(|piece| matches!(piece, Piece::Quarantined(_)))
        );
    }

    #[test]
    fn a_verb_delimiter_may_be_the_sigil_itself() {
        assert_eq!(constructs(b"\\verb|@ref(a)| @ref(b)"), [EntryToken::Ref]);
        assert_eq!(constructs(b"\\verb@@ref(a)@ @ref(b)"), [EntryToken::Ref]);
    }

    #[test]
    fn verbatim_environments_hide_constructs() {
        for name in [
            "verbatim",
            "Verbatim",
            "verbatimtab",
            "listing",
            "lstlisting",
        ] {
            let input = format!("\\begin{{{name}}}\n@ref(a)\n\\end{{{name}}}\n@ref(b)");
            assert_eq!(
                constructs(input.as_bytes()),
                [EntryToken::Ref],
                "{name} did not hide its contents"
            );
        }
    }

    #[test]
    fn the_internal_macro_region_hides_constructs() {
        assert_eq!(
            constructs(b"\\makeatletter @ref(a) \\makeatother @ref(b)"),
            [EntryToken::Ref]
        );
    }

    #[test]
    fn a_raw_escape_hides_every_entry_token() {
        let input = b"latex {@id(a) @ref(b) @cite(c) @import(\"d\")} @ref(after)";
        assert_eq!(constructs(input), [EntryToken::Raw, EntryToken::Ref]);
    }

    #[test]
    fn a_comment_inside_a_raw_escape_hides_its_closing_brace() {
        let input = b"latex {\n% a comment with } inside\n} @ref(after)";
        assert_eq!(constructs(input), [EntryToken::Raw, EntryToken::Ref]);
    }

    #[test]
    fn the_bare_word_latex_is_not_an_entry_token() {
        assert_eq!(constructs(b"we use latex here"), []);
        assert_eq!(constructs(b"pdflatex {x}"), []);
        assert_eq!(constructs(b"\\latex {x}"), []);
    }

    #[test]
    fn an_at_shape_that_is_not_a_keyword_is_text() {
        assert_eq!(constructs(b"name@example.org"), []);
        assert_eq!(constructs(b"@{}lcc@{}"), []);
        assert_eq!(constructs(b"@ref alone and @ref{a}"), []);
    }

    #[test]
    fn an_unterminated_construct_is_reported_and_the_next_line_still_parses() {
        let pieces = scan(b"@ref(broken\n@ref(good)");
        let malformed: Vec<_> = pieces
            .iter()
            .filter(|p| matches!(p, Piece::Malformed { .. }))
            .collect();
        assert_eq!(malformed.len(), 1);
        assert_eq!(constructs(b"@ref(broken\n@ref(good)"), [EntryToken::Ref]);
    }

    #[test]
    fn a_known_signature_excludes_exactly_its_arguments() {
        // \section is `s o m`. Since issue #83 its *mandatory* argument is
        // prose — a heading is a sentence — while the optional short title
        // stays data, the conservative default for the unclassified.
        assert_eq!(
            constructs(b"\\section[@ref(short)]{@ref(long)} @ref(after)"),
            [EntryToken::Ref, EntryToken::Ref]
        );
        assert_eq!(
            constructs(b"\\section*{@ref(a)} @ref(after)"),
            [EntryToken::Ref, EntryToken::Ref]
        );
        // A data-bearing command's arguments are still fully excluded.
        assert_eq!(
            constructs(b"\\includegraphics[width=@ref(x)]{@ref(path)} @id(after)"),
            [EntryToken::Id]
        );
    }

    #[test]
    fn a_text_bearing_argument_is_prose_and_its_regions_compose() {
        // Phase 0a's gap 3, decided as issue #83: a caption is prose, so a
        // construct inside one is a construct — while every inner exclusion
        // still excludes. Each hidden case here carries exactly the bytes a
        // wrong implementation would convert.
        assert_eq!(
            constructs(b"\\caption{see @ref(fig:x)} @id(after)"),
            [EntryToken::Ref, EntryToken::Id]
        );
        assert_eq!(
            constructs(b"\\caption{a \\verb|@ref(v)| $@ref(m)$ % @ref(c)\nb @ref(real)}"),
            [EntryToken::Ref]
        );
        // Nested text-bearing commands scan through.
        assert_eq!(
            constructs(b"\\caption{\\textbf{@ref(deep)}}"),
            [EntryToken::Ref]
        );
        // A definition body is not prose, wherever a caption sits inside it.
        assert_eq!(constructs(b"\\newcommand{\\x}{\\caption{@ref(w)}}"), []);
        // `\item`'s prose is its optional argument.
        assert_eq!(
            constructs(b"\\item[see @ref(fig:x)] body"),
            [EntryToken::Ref]
        );
    }

    #[test]
    fn a_command_takes_only_the_arguments_its_signature_declares() {
        // \emph is `m`. The second group is prose, not a second argument —
        // and since #83 the argument itself is prose too, so both refs are
        // recognised and the boundary is proven by their count staying two,
        // not three, when a third group follows nothing.
        assert_eq!(
            constructs(b"\\emph{@ref(arg)}{@ref(prose)}"),
            [EntryToken::Ref, EntryToken::Ref]
        );
    }

    #[test]
    fn an_unknown_command_claims_adjacent_groups_up_to_the_bound() {
        let one = b"\\unknowncmd{@ref(a)} @ref(one)".to_vec();
        assert_eq!(constructs(&one), [EntryToken::Ref]);

        let sixteen: Vec<u8> = format!(
            "\\unknowncmd{} @ref(sixteen)",
            "{x}".repeat(MAX_UNKNOWN_COMMAND_GROUPS)
        )
        .into_bytes();
        assert_eq!(constructs(&sixteen), [EntryToken::Ref]);
    }

    #[test]
    fn one_group_past_the_bound_stops_recognition_rather_than_guessing() {
        let past: Vec<u8> = format!(
            "\\unknowncmd{} @ref(never)",
            "{x}".repeat(MAX_UNKNOWN_COMMAND_GROUPS + 1)
        )
        .into_bytes();
        assert_eq!(constructs(&past), []);
        assert_eq!(reassemble(&past), past, "quarantine still transports");
    }

    #[test]
    fn an_unbounded_argument_quarantines_rather_than_scanning_on() {
        let input = b"\\section{never closed @ref(a)";
        assert_eq!(constructs(input), []);
        assert_eq!(reassemble(input), input);
    }

    #[test]
    fn label_has_an_optional_argument_which_is_easy_to_assume_it_lacks() {
        // The transcribed signature is `o m`, not `m`. A parser that assumed
        // `m` would read `[x]` as prose and recognise a construct inside it.
        assert_eq!(
            constructs(b"\\label[@ref(opt)]{@ref(name)} @ref(after)"),
            [EntryToken::Ref]
        );
    }

    #[test]
    fn longest_keyword_wins() {
        // `@import(` must not be read as `@i` plus text, and `@id(` must not
        // shadow it.
        assert_eq!(constructs(b"@import(\"a.xtex\")"), [EntryToken::Import]);
        assert_eq!(constructs(b"@cite(k)"), [EntryToken::Cite]);
    }
}
