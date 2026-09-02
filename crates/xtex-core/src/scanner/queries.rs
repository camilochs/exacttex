//! Questions other modules ask about a scanned buffer.

use super::boundaries::{balanced_end, is_escaped};
use super::regions::{Extent, command_extent};
use super::tables::{DEFINITION_COMMANDS, DISPLAY_MATH_ENVIRONMENTS, looks_like_a_construct_word};
use super::{EntryToken, Piece, scan};
use crate::source::Span;

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

/// Whether a region is a display-math body: it opens with `\[`, `$$` or the
/// `\begin{…}` of a display environment, or it closes with that
/// environment's `\end{…}`.
///
/// Both ends are checked because the scanner's excluded piece for an
/// environment begins after the header slot (grammar §4) and so does not
/// carry its own `\begin`; it does carry the `\end`.
#[must_use]
pub fn is_display_math_region(region: &[u8]) -> bool {
    if region.starts_with(b"\\[") || region.starts_with(b"$$") {
        return true;
    }
    if let Some(rest) = region.strip_prefix(b"\\begin{")
        && let Some(close) = rest.iter().position(|byte| *byte == b'}')
        && is_display_math_name(&rest[..close])
    {
        return true;
    }
    let Some(open) = region
        .windows(b"\\end{".len())
        .rposition(|w| w == b"\\end{")
    else {
        return false;
    };
    region.ends_with(b"}")
        && is_display_math_name(&region[open + b"\\end{".len()..region.len() - 1])
}

fn is_display_math_name(name: &[u8]) -> bool {
    DISPLAY_MATH_ENVIRONMENTS
        .iter()
        .any(|(known, _)| known.as_bytes() == name)
}

/// Regions holding content a reader may search.
///
/// Prose, plus every excluded region except a definition's, plus display-math
/// bodies. A command's arguments are an exclusion region so that *constructs*
/// are not recognised there; a reader looking for the author's own `\label`
/// is asking a different question, and the answer differs only for
/// definitions. A display body is excluded for the same reason and read for
/// the same one: `\label{eq:x}` before `\end{align}` is where equations are
/// labelled, and an inventory that skipped it called correct references
/// undeclared (corpus E2, 17 labels in 6 papers).
#[must_use]
pub fn readable_content(bytes: &[u8]) -> Vec<Span> {
    let mut spans = Vec::new();
    for piece in scan(bytes) {
        match piece {
            Piece::Text(span) => spans.push(span),
            Piece::Excluded(span) if is_display_math_region(&bytes[span.start()..span.end()]) => {
                spans.push(span);
            }
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

/// Every environment the scanner read both ends of: the span of its name,
/// and the span from its `\begin` through its `\end`.
///
/// A `\begin` or `\end` counts only where the scanner read it as a command,
/// so one inside a comment, a verbatim body or a macro definition opens or
/// closes nothing. A display-math body is an excluded region that carries
/// its own `\end`, and closes here too. An environment whose `\end` was
/// never read — the author never wrote it, or the file went to quarantine
/// before it — has no extent the scanner located, and is left out. An
/// `@id` takes the class of the float around it from this list, which is
/// what keeps a second walk over the bytes from meeting a `\begin{figure}`
/// the scanner had excluded.
#[must_use]
pub fn closed_environments(bytes: &[u8]) -> Vec<(Span, Span)> {
    let mut open: Vec<(Span, usize)> = Vec::new();
    let mut closed = Vec::new();
    for piece in scan(bytes) {
        let (span, at) = match piece {
            Piece::Arguments(span) => (span, span.start()),
            Piece::Excluded(span) if is_display_math_region(&bytes[span.start()..span.end()]) => {
                let Some(end) = bytes[span.start()..span.end()]
                    .windows(b"\\end{".len())
                    .rposition(|w| w == b"\\end{")
                else {
                    continue;
                };
                (span, span.start() + end)
            }
            _ => continue,
        };
        if let Some(name) = environment_name(bytes, at, b"\\begin") {
            open.push((name, span.start()));
        } else if let Some(name) = environment_name(bytes, at, b"\\end") {
            let wanted = &bytes[name.start()..name.end()];
            // The innermost open environment of that name. Anything opened
            // after it and still open was never closed, and is dropped. The
            // body ends at the `}` closing the name: `\end` has no signature,
            // so its piece also claims any braced group that follows it.
            if let Some(index) = open
                .iter()
                .rposition(|(opened, _)| &bytes[opened.start()..opened.end()] == wanted)
            {
                let (opened, begin) = open[index];
                open.truncate(index);
                closed.push((
                    opened,
                    Span::new(
                        u32::try_from(begin).unwrap_or(u32::MAX),
                        u32::try_from(name.end() + 1).unwrap_or(u32::MAX),
                    ),
                ));
            }
        }
    }
    closed
}

/// The name in `\begin{name}` or `\end{name}` at `at`, given the keyword.
fn environment_name(bytes: &[u8], at: usize, keyword: &[u8]) -> Option<Span> {
    let mut cursor = at + keyword.len();
    if bytes.get(at..cursor)? != keyword {
        return None;
    }
    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'{') {
        return None;
    }
    let start = cursor + 1;
    let end = start
        + bytes[start..]
            .iter()
            .position(|byte| matches!(byte, b'}' | b'{' | b'\n' | b'\r'))?;
    (bytes[end] == b'}' && end > start).then(|| {
        Span::new(
            u32::try_from(start).unwrap_or(u32::MAX),
            u32::try_from(end).unwrap_or(u32::MAX),
        )
    })
}

/// The comma-separated keys in a construct payload, as byte ranges relative
/// to the payload, untrimmed.
///
/// One splitter for the checker, the editor and rename, so that "which key
/// is this" has one answer. A payload with no comma is one key; an empty
/// payload is one empty key.
#[must_use]
pub fn split_keys(payload: &[u8]) -> Vec<(usize, usize)> {
    let mut keys = Vec::new();
    let mut start = 0usize;
    for (at, byte) in payload.iter().enumerate() {
        if *byte == b',' {
            keys.push((start, at));
            start = at + 1;
        }
    }
    keys.push((start, payload.len()));
    keys
}

/// Every `@word(…)` in prose that no construct claims and that reads like
/// one.
///
/// The shape is the entry token's own — `@`, letters, `(`, a payload, `)` on
/// one line — with a word the grammar does not know. Such bytes are
/// transported, so `@eqref(eq:x)` reaches the PDF as literal text with exit
/// 0; the advisory built on this is what tells the author. Only prose is
/// searched: a comment, verbatim text, math or a command's data argument
/// keeps its bytes, and so does an email address (`a@b(` has a letter
/// before the `@`) or a control symbol (`\@ifnextchar(`). The prose inside
/// a typed block's braced fields and inside revision content is searched
/// too, because a caption is where the shape was first found in the wild.
#[must_use]
pub fn construct_shaped_text(bytes: &[u8]) -> Vec<Span> {
    let mut found = Vec::new();
    shapes_in(bytes, 0, bytes.len(), &mut found);
    found
}

fn shapes_in(bytes: &[u8], start: usize, end: usize, found: &mut Vec<Span>) {
    for piece in scan(&bytes[start..end]) {
        match piece {
            Piece::Text(text) => {
                shapes_in_text(bytes, start + text.start(), start + text.end(), found);
            }
            Piece::Construct { kind, span, .. } => {
                let at = start + span.start();
                let close = start + span.end();
                match kind {
                    EntryToken::Figure | EntryToken::Table => {
                        let (block_kind, entry) = if kind == EntryToken::Figure {
                            (crate::blocks::BlockKind::Figure, b"\\figure(".len())
                        } else {
                            (crate::blocks::BlockKind::Table, b"\\table(".len())
                        };
                        let Ok(block) =
                            crate::blocks::parse_block(bytes, block_kind, at, at + entry)
                        else {
                            continue;
                        };
                        for field in block.fields {
                            if let crate::blocks::Value::Braced(value) = field.value {
                                shapes_in(bytes, value.start() + 1, value.end() - 1, found);
                            }
                        }
                    }
                    EntryToken::Add | EntryToken::Del | EntryToken::Sub | EntryToken::Note => {
                        // The header ends at its first `)`; the content is the
                        // balanced group after it.
                        let Some(header) = bytes[at..close].iter().position(|byte| *byte == b')')
                        else {
                            continue;
                        };
                        let Some(open) = bytes[at + header..close]
                            .iter()
                            .position(|byte| *byte == b'{')
                        else {
                            continue;
                        };
                        let open = at + header + open;
                        if let Some(end) = balanced_end(bytes, open).filter(|end| *end <= close) {
                            shapes_in(bytes, open + 1, end - 1, found);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn shapes_in_text(bytes: &[u8], start: usize, end: usize, found: &mut Vec<Span>) {
    let mut at = start;
    while at < end {
        if bytes[at] != b'@' {
            at += 1;
            continue;
        }
        let before = at.checked_sub(1).map(|i| bytes[i]);
        if before.is_some_and(|b| b == b'\\' || b == b'@' || b.is_ascii_alphanumeric()) {
            at += 1;
            continue;
        }
        let word_start = at + 1;
        let mut word_end = word_start;
        while word_end < end && bytes[word_end].is_ascii_alphabetic() {
            word_end += 1;
        }
        if word_end == word_start || bytes.get(word_end) != Some(&b'(') {
            at = word_end.max(at + 1);
            continue;
        }
        let Some(close) = bytes[word_end..end]
            .iter()
            .position(|byte| matches!(byte, b')' | b'\n' | b'\r'))
            .map(|i| word_end + i)
            .filter(|i| bytes[*i] == b')')
        else {
            at = word_end;
            continue;
        };
        if looks_like_a_construct_word(&bytes[word_start..word_end]) {
            found.push(Span::new(
                u32::try_from(at).unwrap_or(u32::MAX),
                u32::try_from(close + 1).unwrap_or(u32::MAX),
            ));
        }
        at = close + 1;
    }
}

pub(crate) fn substitution_arrows(bytes: &[u8], start: usize, end: usize) -> usize {
    let mut depth = 1u32;
    let mut count = 0usize;
    for piece in scan(&bytes[start..end]) {
        let Piece::Text(text) = piece else { continue };
        let mut at = start + text.start();
        let text_end = start + text.end();
        while at + 1 < text_end {
            if bytes[at] == b'\\'
                && let Extent::Through(next) = command_extent(bytes, at)
                && next <= text_end
            {
                at = next;
                continue;
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
            if bytes[at] == b'\\'
                && let Extent::Through(next) = command_extent(bytes, at)
                && next <= text_end
            {
                at = next;
                continue;
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
pub(crate) fn read_label_option(
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
