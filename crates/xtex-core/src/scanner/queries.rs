//! Questions other modules ask about a scanned buffer.

use super::boundaries::is_escaped;
use super::regions::{Extent, command_extent};
use super::tables::DEFINITION_COMMANDS;
use super::{Piece, scan};
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
