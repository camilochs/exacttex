//! Finds where a NextTeX construct may begin.
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
//! in §8, and both need the signature database that arrives with the LaTeX
//! front end. Until then, an entry token inside a command argument is
//! recognised when it should not be. That is a known gap rather than an
//! oversight: it is listed in the module tests as a fixture that does not pass
//! yet.

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
    /// From `\begin{name}` to a line-exact `\end{name}`.
    VerbatimEnvironment { name: Vec<u8> },
    /// From `\makeatletter` to `\makeatother`.
    InternalMacros,
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
/// project by `nextex.toml`.
pub const DEFAULT_VERBATIM_ENVIRONMENTS: &[&str] = &[
    "verbatim",
    "verbatim*",
    "Verbatim",
    "verbatimtab",
    "listing",
    "lstlisting",
    "minted",
];

/// A stretch of the source, classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Piece {
    /// Prose: bytes in which a construct would have been recognised, and none
    /// was.
    Text(Span),
    /// A region §8 excludes, such as a comment, math, or a verbatim block.
    ///
    /// Distinguished from [`Piece::Text`] because a consumer that searches the
    /// source for something other than a construct — a bibliography
    /// declaration, say — must not find it here either.
    Excluded(Span),
    /// A complete NextTeX entry token and everything it delimits.
    Construct {
        /// Which construct the entry token opened.
        kind: EntryToken,
        /// The whole construct, entry token through closing delimiter.
        span: Span,
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
                if let Some((token, end)) = entry_token_at(bytes, at) {
                    if let Some(kind) = block_kind(token) {
                        flush!(at);
                        let piece = match crate::blocks::parse_block(bytes, kind, at, end) {
                            Ok(block) => Piece::Construct {
                                kind: token,
                                span: block.span,
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
                // A command's arguments are an exclusion region. The shape comes
                // from a signature, never from guessing which groups belong to
                // it — see signatures.rs.
                if bytes[at] == b'\\' {
                    match command_extent(bytes, at) {
                        Extent::Through(end) => {
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

            Region::VerbatimEnvironment { name } => {
                if let Some(end) = verbatim_end(bytes, at, name) {
                    at = end;
                    pieces.push(Piece::Excluded(span(excluded_start, at)));
                    text_start = at;
                    region = Region::Prose;
                } else {
                    at += 1;
                }
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
        // A region that never closed still covers its bytes.
        _ => {
            if bytes.len() > excluded_start {
                pieces.push(Piece::Excluded(span(excluded_start, bytes.len())));
            }
        }
    }

    pieces
}

/// The block kind an entry token opens, if it opens one.
const fn block_kind(token: EntryToken) -> Option<crate::blocks::BlockKind> {
    match token {
        EntryToken::Figure => Some(crate::blocks::BlockKind::Figure),
        EntryToken::Table => Some(crate::blocks::BlockKind::Table),
        _ => None,
    }
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
    /// applies only inside a NextTeX block body, never to transported LaTeX,
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
            if bytes[at..].starts_with(b"\\[") {
                return Some((Region::DisplayMath { dollars: false }, at + 2));
            }
            if bytes[at..].starts_with(b"\\makeatletter") {
                return Some((Region::InternalMacros, at + b"\\makeatletter".len()));
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
                        Region::VerbatimEnvironment {
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
fn verbatim_end(bytes: &[u8], at: usize, name: &[u8]) -> Option<usize> {
    let mut needle = Vec::with_capacity(name.len() + 7);
    needle.extend_from_slice(b"\\end{");
    needle.extend_from_slice(name);
    needle.push(b'}');
    bytes[at..]
        .starts_with(&needle)
        .then_some(at + needle.len())
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
                | Piece::Construct { span: s, .. }
                | Piece::Malformed { span: s, .. } => s,
            };
            out.extend_from_slice(&bytes[span.start()..span.end()]);
        }
        out
    }

    /// Where a piece sits, whatever kind it is.
    fn extent(piece: Piece) -> Span {
        match piece {
            Piece::Text(s)
            | Piece::Excluded(s)
            | Piece::Construct { span: s, .. }
            | Piece::Malformed { span: s, .. } => s,
        }
    }

    /// Inputs chosen so that truncating them enters every region the scanner
    /// has, and leaves each one unterminated in turn.
    const AWKWARD: &[&[u8]] = &[
        b"\\section{Caf\xE9}\r\n%% comment\t\n\\ref{a} @ref(b) trailing",
        b"before %comment\n@id(x) $math$ after",
        b"\\verb+@ref(a)+ then @ref(b)",
        b"\\begin{verbatim}\n@id(v)\n\\end{verbatim} @id(real)",
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
                    let span = extent(piece);
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
        scan(bytes)
            .into_iter()
            .filter_map(|p| match p {
                Piece::Construct { kind, .. } => Some(kind),
                _ => None,
            })
            .collect()
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
        // \section is `s o m`: an optional star, an optional bracketed
        // argument, then one mandatory braced one.
        assert_eq!(
            constructs(b"\\section[@ref(short)]{@ref(long)} @ref(after)"),
            [EntryToken::Ref]
        );
        assert_eq!(
            constructs(b"\\section*{@ref(a)} @ref(after)"),
            [EntryToken::Ref]
        );
        // \caption is `o m`; the token after its argument is prose again.
        assert_eq!(
            constructs(b"\\caption{@ref(inside)} @id(after)"),
            [EntryToken::Id]
        );
    }

    #[test]
    fn a_command_takes_only_the_arguments_its_signature_declares() {
        // \emph is `m`. The second group is prose, not a second argument, so a
        // construct inside it is recognised.
        assert_eq!(
            constructs(b"\\emph{@ref(arg)}{@ref(prose)}"),
            [EntryToken::Ref]
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
        assert_eq!(constructs(b"@import(\"a.ntex\")"), [EntryToken::Import]);
        assert_eq!(constructs(b"@cite(k)"), [EntryToken::Cite]);
    }
}
