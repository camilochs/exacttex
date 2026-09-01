//! What opens each exclusion region, and where each one ends.

use super::EntryToken;
use super::boundaries::{
    balanced_end, close_paren, delimited_end, is_escaped, optional_argument_end,
    skip_ascii_whitespace,
};
use super::tables::AT_TOKENS;
use super::tables::{
    ARGUMENT_OPENERS, DEFAULT_VERBATIM_ENVIRONMENTS, DISPLAY_MATH_ENVIRONMENTS,
    MAX_UNKNOWN_COMMAND_GROUPS, TEXT_MANDATORY_COMMANDS, TEXT_OPTIONAL_COMMANDS,
    if_name_is_not_a_conditional,
};
use super::tables::{DEFAULT_CITE_COMMANDS, DEFAULT_REF_COMMANDS};
use crate::signatures::{Argument, is_known, signature_of};

/// Where the scanner is in the byte stream.
///
/// Each variant except [`Region::Prose`] is a region in which every entry token
/// is ordinary text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Region {
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
    ///
    /// `math` marks a display-math body, inside which `@id(` is still
    /// recognised — an equation is labelled from inside its own body far
    /// more often than from the header slot (17 such labels across 6 papers
    /// in corpus E2). A verbatim body recognises nothing.
    Environment { name: Vec<u8>, math: bool },
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

/// A complete entry token starting at `at`, and the offset just past it.
pub(crate) fn entry_token_at(bytes: &[u8], at: usize) -> Option<(EntryToken, usize)> {
    if bytes[at] == b'@' {
        for token in AT_TOKENS {
            let keyword = token.keyword();
            let end = at + keyword.len();
            if bytes[at..].starts_with(keyword) && bytes.get(end) == Some(&b'(') {
                return Some((*token, end + 1));
            }
        }
        for (commands, token) in [
            (DEFAULT_CITE_COMMANDS, EntryToken::Cite),
            (DEFAULT_REF_COMMANDS, EntryToken::Ref),
        ] {
            for command in commands {
                let end = at + 1 + command.len();
                if bytes[at + 1..].starts_with(command.as_bytes()) && bytes.get(end) == Some(&b'(')
                {
                    return Some((token, end + 1));
                }
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

/// How far a command and its arguments reach.
pub(crate) enum Extent {
    /// The command and its arguments end just before this offset.
    Through(usize),
    /// A boundary could not be located; nothing further is recognised.
    Unbounded,
    /// The backslash does not begin a control word.
    NotACommand,
}

/// The interior of a text-bearing argument, when this call has one.
///
/// Walks the same signature the command was consumed under and returns the
/// byte range inside the prose argument's delimiters. `None` when the
/// command bears no prose, when the argument is absent, or when it is not
/// in delimited form — each of those keeps today's whole-argument
/// exclusion, which is the safe direction.
pub(crate) fn text_argument_interior(bytes: &[u8], at: usize) -> Option<(usize, usize)> {
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
                if bytes.get(cursor) == Some(&b'[')
                    && let Some(end) = optional_argument_end(bytes, cursor)
                {
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

pub(crate) fn command_extent(bytes: &[u8], at: usize) -> Extent {
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

    // `\newif\iffoo` *defines* a conditional; the `\iffoo` after it opens
    // nothing and has no `\fi`. Read as an opener it quarantined the rest of
    // the file — the TACL template does this at line 52, and every construct
    // after it went dark with exit 0 (arXiv 2301.00303, corpus E2). The
    // defined name is claimed here as the command's argument.
    if name == b"newif" {
        let next = skip_ascii_whitespace(bytes, cursor);
        if bytes.get(next) == Some(&b'\\') {
            let mut end = next + 1;
            while bytes.get(end).is_some_and(u8::is_ascii_alphabetic) {
                end += 1;
            }
            if end > next + 1 {
                return Extent::Through(end);
            }
        }
        return Extent::Through(cursor);
    }

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
                    // A `[` that never closes, or closes past a blank line,
                    // is prose and the argument is absent.
                    if bytes.get(cursor) == Some(&b'[')
                        && let Some(end) = optional_argument_end(bytes, cursor)
                    {
                        cursor = end;
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
        let open = match bytes.get(next) {
            Some(b'{') => b'{',
            Some(b'[') => b'[',
            _ => break,
        };
        if groups == MAX_UNKNOWN_COMMAND_GROUPS {
            return Extent::Unbounded;
        }
        if open == b'{' {
            match balanced_end(bytes, next) {
                Some(end) => cursor = end,
                None => return Extent::Unbounded,
            }
        } else {
            // An unclosed bracket is prose, not a group; the claim ends.
            let Some(end) = optional_argument_end(bytes, next) else {
                break;
            };
            cursor = end;
        }
        groups += 1;
    }
    Extent::Through(cursor)
}

/// A region beginning at `at`, and where scanning resumes.
pub(crate) fn region_opening_at(bytes: &[u8], at: usize) -> Option<(Region, usize)> {
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
                            math: false,
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
pub(crate) fn verbatim_command_opening(bytes: &[u8], at: usize) -> Option<(Region, usize)> {
    let (name, mut cursor) = control_word_at(bytes, at)?;
    match name {
        b"verb" => {
            if bytes.get(cursor) == Some(&b'*') {
                cursor += 1;
            }
        }
        b"lstinline" => {
            if bytes.get(cursor) == Some(&b'[') {
                // An unclosed option list is prose; this is then not a
                // verbatim command and the ordinary path reads it.
                cursor = optional_argument_end(bytes, cursor)?;
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
pub(crate) fn control_word_at(bytes: &[u8], at: usize) -> Option<(&[u8], usize)> {
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
pub(crate) fn environment_end(bytes: &[u8], at: usize, name: &[u8]) -> Option<usize> {
    let mut needle = Vec::with_capacity(name.len() + 7);
    needle.extend_from_slice(b"\\end{");
    needle.extend_from_slice(name);
    needle.push(b'}');
    bytes[at..]
        .starts_with(&needle)
        .then_some(at + needle.len())
}

/// A display body start and the environment name that closes it.
pub(crate) fn display_math_environment_opening(
    bytes: &[u8],
    at: usize,
) -> Option<(usize, Vec<u8>)> {
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
pub(crate) fn display_header_whitespace_end(bytes: &[u8], mut at: usize) -> usize {
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
