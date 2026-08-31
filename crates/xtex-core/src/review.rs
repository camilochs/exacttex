//! Revision identity, sidecar parsing, and source rewrites.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use crate::scanner::{EntryToken, Piece, scan, substitution_separator};
use crate::source::Span;

/// Attribution stored beside a revision construct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionRecord {
    /// Construct identifier.
    pub id: String,
    /// `add`, `del`, `sub`, or `note`.
    pub kind: String,
    /// Free-text author name.
    pub author: String,
    /// RFC 3339 timestamp as written.
    pub at: String,
    /// Optional explanation.
    pub message: Option<String>,
    /// Target identifier, present only for notes.
    pub on: Option<String>,
}

/// A parsed `.xtexrev` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sidecar {
    /// Format version. Version 1 is supported.
    pub version: u32,
    /// Document filename named by the sidecar.
    pub document: String,
    /// Live revision records.
    pub revisions: Vec<RevisionRecord>,
}

/// A sidecar or identity error with its stable diagnostic code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewError {
    /// Stable diagnostic identifier.
    pub code: &'static str,
    /// Plain-language detail.
    pub message: String,
}

impl fmt::Display for ReviewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl Error for ReviewError {}

/// A non-fatal loss of attribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advisory {
    /// Construct identifier lacking a record.
    pub id: String,
    /// Plain-language explanation.
    pub message: String,
}

/// Parses the dependency-free TOML subset used by `.xtexrev` files.
///
/// # Errors
///
/// Returns `XT1013` for malformed input or unsupported versions.
pub fn parse_sidecar(bytes: &[u8]) -> Result<Sidecar, ReviewError> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| error("XT1013", "the sidecar is not UTF-8"))?;
    let mut version = None;
    let mut document = None;
    let mut records = Vec::new();
    let mut current: Option<BTreeMap<String, String>> = None;
    for (index, raw) in text.lines().enumerate() {
        let line = strip_toml_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line == "[[revision]]" {
            if let Some(fields) = current.take() {
                records.push(record(fields)?);
            }
            current = Some(BTreeMap::new());
            continue;
        }
        if line.starts_with("[[") {
            if let Some(fields) = current.take() {
                records.push(record(fields)?);
            }
            current = None;
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| error("XT1013", format!("line {} has no '='", index + 1)))?;
        let key = key.trim();
        if let Some(fields) = current.as_mut() {
            if fields
                .insert(key.to_owned(), parse_string(value.trim())?)
                .is_some()
            {
                return Err(error(
                    "XT1013",
                    format!("revision field '{key}' appears more than once"),
                ));
            }
        } else if key == "version" {
            version = Some(
                value
                    .trim()
                    .parse()
                    .map_err(|_| error("XT1013", "version must be an integer"))?,
            );
        } else if key == "document" {
            document = Some(parse_string(value.trim())?);
        }
    }
    if let Some(fields) = current {
        records.push(record(fields)?);
    }
    let version = version.ok_or_else(|| error("XT1013", "the sidecar has no version"))?;
    if version != 1 {
        return Err(error(
            "XT1013",
            format!("sidecar version {version} is not supported"),
        ));
    }
    Ok(Sidecar {
        version,
        document: document.ok_or_else(|| error("XT1013", "the sidecar has no document"))?,
        revisions: records,
    })
}

/// Checks the one-to-one relationship between source and sidecar.
///
/// # Errors
///
/// Returns `XT1001`, `XT1010`, `XT1011`, or `XT1012` as specified in
/// `docs/revisions.md` §5.
pub fn validate(bytes: &[u8], sidecar: &Sidecar) -> Result<Vec<Advisory>, ReviewError> {
    let constructs = revision_constructs(bytes);
    let mut declared = BTreeSet::new();
    let identifiers = scan(bytes)
        .into_iter()
        .filter_map(|piece| {
            let Piece::Construct {
                kind: EntryToken::Id,
                span,
                ..
            } = piece
            else {
                return None;
            };
            let source = &bytes[span.start()..span.end()];
            Some(String::from_utf8_lossy(&source[b"@id(".len()..source.len() - 1]).into_owned())
        })
        .chain(constructs.iter().map(|construct| construct.id.clone()));
    for id in identifiers {
        if !declared.insert(id.clone()) {
            return Err(error(
                "XT1001",
                format!("identifier '{id}' is declared more than once"),
            ));
        }
    }
    let mut by_id = BTreeMap::new();
    for construct in &constructs {
        by_id.insert(construct.id.clone(), construct);
    }
    let mut record_ids = BTreeSet::new();
    for record in &sidecar.revisions {
        if !record_ids.insert(record.id.as_str()) {
            return Err(error(
                "XT1010",
                format!("sidecar identifier '{}' appears more than once", record.id),
            ));
        }
        let Some(construct) = by_id.get(&record.id) else {
            return Err(error(
                "XT1012",
                format!("sidecar record '{}' has no construct", record.id),
            ));
        };
        if kind_name(construct.kind) != record.kind {
            return Err(error(
                "XT1011",
                format!(
                    "sidecar says '{}' is {}, but the construct is {}",
                    record.id,
                    record.kind,
                    kind_name(construct.kind)
                ),
            ));
        }
        if record.kind == "note" && record.on.is_none() {
            return Err(error(
                "XT1011",
                format!("note record '{}' has no on field", record.id),
            ));
        }
        if record.kind == "note" && record.on != construct.on {
            return Err(error(
                "XT1011",
                format!("note record '{}' names a different target", record.id),
            ));
        }
        if record.kind != "note" && record.on.is_some() {
            return Err(error(
                "XT1011",
                format!("{} record '{}' has an on field", record.kind, record.id),
            ));
        }
    }
    Ok(constructs
        .into_iter()
        .filter(|construct| !record_ids.contains(construct.id.as_str()))
        .map(|construct| Advisory {
            message: format!(
                "revision '{}' has no sidecar record; its text still builds",
                construct.id
            ),
            id: construct.id,
        })
        .collect())
}

/// Whether a selected change is applied or declined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Keep the proposed final text.
    Accept,
    /// Keep the original text.
    Reject,
}

/// Lists revision identifiers in source order.
#[must_use]
pub fn revision_ids(bytes: &[u8]) -> Vec<String> {
    revision_constructs(bytes)
        .into_iter()
        .map(|construct| construct.id)
        .collect()
}

/// Rewrites one revision construct and returns the removed content, if any.
///
/// The caller owns persistence so it can use a same-directory atomic rename.
///
/// # Errors
///
/// Returns `XT1002` if the identifier is absent.
pub fn resolve(
    bytes: &[u8],
    id: &str,
    resolution: Resolution,
) -> Result<(Vec<u8>, Vec<u8>), ReviewError> {
    let construct = revision_constructs(bytes)
        .into_iter()
        .find(|construct| construct.id == id)
        .ok_or_else(|| error("XT1002", format!("revision '{id}' was not found")))?;
    let source = &bytes[construct.span.start()..construct.span.end()];
    let open = source
        .iter()
        .position(|byte| *byte == b'{')
        .unwrap_or(source.len());
    let end = source.len().saturating_sub(1);
    let (old, new) = if construct.kind == EntryToken::Sub {
        let arrow = substitution_separator(source, open + 1, end).unwrap_or(end);
        (
            trim_end(&source[open + 1..arrow]),
            trim_start(&source[(arrow + 2).min(end)..end]),
        )
    } else {
        (&source[open + 1..end], &[][..])
    };
    let (kept, removed) = match (construct.kind, resolution) {
        (EntryToken::Add, Resolution::Accept) | (EntryToken::Del, Resolution::Reject) => {
            (old, &[][..])
        }
        (EntryToken::Add, Resolution::Reject) | (EntryToken::Del, Resolution::Accept) => {
            (&[][..], old)
        }
        (EntryToken::Sub, Resolution::Accept) => (new, old),
        (EntryToken::Sub, Resolution::Reject) => (old, new),
        (EntryToken::Note, _) => (&[][..], old),
        _ => (&[][..], &[][..]),
    };
    let mut rewritten = Vec::with_capacity(bytes.len() - source.len() + kept.len());
    rewritten.extend_from_slice(&bytes[..construct.span.start()]);
    rewritten.extend_from_slice(kept);
    rewritten.extend_from_slice(&bytes[construct.span.end()..]);
    // A reply is about a change. Once the change is gone the reply points at
    // nothing, and leaving it behind makes the author clean up by hand after
    // every decision (the director, 2026-08-31). Replies leave with it — and
    // their text rides out in `removed`, so nothing said is lost.
    let mut removed = removed.to_vec();
    if construct.kind != EntryToken::Note {
        for reply in replies_to(&rewritten, id) {
            let text = rewritten[reply.span.start()..reply.span.end()].to_vec();
            // A reply takes the blank that separated it. Leaving that byte
            // behind put a stray space where the conversation had been —
            // visible in the source and, inside a title, in the PDF.
            let mut from = reply.span.start();
            if from > 0 && rewritten[from - 1] == b' ' {
                from -= 1;
            }
            let mut without = Vec::with_capacity(rewritten.len());
            without.extend_from_slice(&rewritten[..from]);
            without.extend_from_slice(&rewritten[reply.span.end()..]);
            rewritten = without;
            removed.extend_from_slice(&text);
        }
    }
    Ok((rewritten, removed))
}

/// Replies whose `on` names `id`, latest first so removing one does not move
/// the next one's span.
fn replies_to(bytes: &[u8], id: &str) -> Vec<RevisionConstruct> {
    let mut replies: Vec<RevisionConstruct> = revision_constructs(bytes)
        .into_iter()
        .filter(|construct| {
            construct.kind == EntryToken::Note && construct.on.as_deref() == Some(id)
        })
        .collect();
    replies.sort_by_key(|construct| std::cmp::Reverse(construct.span.start()));
    replies
}

/// Moves a resolved live record to sidecar history.
///
/// # Errors
///
/// Returns `XT1013` when the sidecar cannot be parsed and `XT1002` when it has
/// no matching live record.
pub fn resolve_sidecar(
    bytes: &[u8],
    id: &str,
    resolution: Resolution,
    by: &str,
    at: &str,
    removed: &[u8],
) -> Result<Vec<u8>, ReviewError> {
    let sidecar = parse_sidecar(bytes)?;
    let record = sidecar
        .revisions
        .iter()
        .find(|record| record.id == id)
        .ok_or_else(|| error("XT1002", format!("sidecar record '{id}' was not found")))?;
    move_record(
        bytes,
        id,
        &record.kind,
        match resolution {
            Resolution::Accept => "accepted",
            Resolution::Reject => "rejected",
        },
        by,
        at,
        removed,
    )
}

/// Moves records whose constructs are gone to history.
///
/// # Errors
///
/// Returns `XT1013` if the sidecar is malformed.
pub fn prune_sidecar(
    bytes: &[u8],
    source: &[u8],
    by: &str,
    at: &str,
) -> Result<Vec<u8>, ReviewError> {
    let sidecar = parse_sidecar(bytes)?;
    let live: BTreeSet<String> = revision_constructs(source)
        .into_iter()
        .map(|construct| construct.id)
        .collect();
    let mut output = bytes.to_vec();
    for record in sidecar
        .revisions
        .iter()
        .filter(|record| !live.contains(&record.id))
    {
        output = move_record(&output, &record.id, &record.kind, "pruned", by, at, b"")?;
    }
    Ok(output)
}

fn move_record(
    bytes: &[u8],
    id: &str,
    kind: &str,
    resolution: &str,
    by: &str,
    at: &str,
    removed: &[u8],
) -> Result<Vec<u8>, ReviewError> {
    let removed = std::str::from_utf8(removed).map_err(|_| {
        error(
            "XT1013",
            "rejected text is not UTF-8 and cannot be stored losslessly in TOML",
        )
    })?;
    let text =
        std::str::from_utf8(bytes).map_err(|_| error("XT1013", "the sidecar is not UTF-8"))?;
    let starts = table_starts(text);
    let mut output = String::from(&text[..starts.first().copied().unwrap_or(text.len())]);
    for (index, start) in starts.iter().copied().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(text.len());
        let block = &text[start..end];
        if !block.starts_with("[[revision]]") || !block_has_id(block, id) {
            output.push_str(block);
        }
    }
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str("\n[[history]]\n");
    write_field(&mut output, "id", id);
    write_field(&mut output, "kind", kind);
    write_field(&mut output, "resolution", resolution);
    write_field(&mut output, "by", by);
    write_field(&mut output, "at", at);
    write_field(&mut output, "removed", removed);
    Ok(output.into_bytes())
}

fn table_starts(text: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with("[[") {
            starts.push(offset + line.len() - line.trim_start().len());
        }
        offset += line.len();
    }
    starts
}

#[derive(Debug, Clone)]
struct RevisionConstruct {
    id: String,
    kind: EntryToken,
    span: Span,
    on: Option<String>,
}

/// Every revision construct the scanner recognises.
///
/// Recognition is the scanner's business and only the scanner's: a construct
/// inside `\author{…}` or a caption is real because those arguments are
/// prose (`TEXT_MANDATORY_COMMANDS`), and one inside a comment or a
/// `\newcommand` body is not. Deciding it again here would let review and
/// emission disagree — which is how a proposed deletion of an author name
/// came to be resolvable and, at the same time, printed into the PDF as
/// `@del(change:…)` (the director's book, 2026-08-31).
fn revision_constructs(bytes: &[u8]) -> Vec<RevisionConstruct> {
    scan(bytes)
        .into_iter()
        .filter_map(|piece| {
            let Piece::Construct { kind, span, .. } = piece else {
                return None;
            };
            if !matches!(
                kind,
                EntryToken::Add | EntryToken::Del | EntryToken::Sub | EntryToken::Note
            ) {
                return None;
            }
            let source = &bytes[span.start()..span.end()];
            let open = source.iter().position(|byte| *byte == b'(')? + 1;
            let end = source[open..].iter().position(|byte| {
                !byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b':' | b'.' | b'-')
            })? + open;
            Some(RevisionConstruct {
                id: String::from_utf8_lossy(&source[open..end]).into_owned(),
                kind,
                span,
                on: if kind == EntryToken::Note {
                    let equals = source.iter().position(|byte| *byte == b'=')? + 1;
                    let target_start = equals
                        + source[equals..]
                            .iter()
                            .position(|byte| !byte.is_ascii_whitespace())?;
                    let target_end = target_start
                        + source[target_start..].iter().position(|byte| {
                            !byte.is_ascii_alphanumeric()
                                && !matches!(byte, b'_' | b':' | b'.' | b'-')
                        })?;
                    Some(String::from_utf8_lossy(&source[target_start..target_end]).into_owned())
                } else {
                    None
                },
            })
        })
        .collect()
}

fn kind_name(kind: EntryToken) -> &'static str {
    match kind {
        EntryToken::Add => "add",
        EntryToken::Del => "del",
        EntryToken::Sub => "sub",
        EntryToken::Note => "note",
        _ => "",
    }
}

fn record(mut fields: BTreeMap<String, String>) -> Result<RevisionRecord, ReviewError> {
    let id = take_field(&mut fields, "id")?;
    let kind = take_field(&mut fields, "kind")?;
    let author = take_field(&mut fields, "author")?;
    let at = take_field(&mut fields, "at")?;
    if !looks_like_rfc3339(&at) {
        return Err(error(
            "XT1013",
            format!("revision '{id}' has an invalid RFC 3339 timestamp"),
        ));
    }
    let message = fields.remove("message");
    let on = fields.remove("on");
    if let Some(name) = fields.keys().next() {
        return Err(error(
            "XT1013",
            format!("revision record has unknown field '{name}'"),
        ));
    }
    Ok(RevisionRecord {
        id,
        kind,
        author,
        at,
        message,
        on,
    })
}

fn take_field(fields: &mut BTreeMap<String, String>, name: &str) -> Result<String, ReviewError> {
    fields
        .remove(name)
        .ok_or_else(|| error("XT1013", format!("revision record has no {name}")))
}

fn looks_like_rfc3339(value: &str) -> bool {
    value.len() >= 20
        && value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
        && value.as_bytes().get(10) == Some(&b'T')
        && value.as_bytes().get(13) == Some(&b':')
        && value.as_bytes().get(16) == Some(&b':')
        && (value.ends_with('Z')
            || value
                .get(19..)
                .is_some_and(|zone| zone.starts_with('+') || zone.starts_with('-')))
}

fn strip_toml_comment(line: &str) -> &str {
    let mut quoted = false;
    let mut escaped = false;
    for (index, character) in line.char_indices() {
        if character == '"' && !escaped {
            quoted = !quoted;
        }
        if character == '#' && !quoted {
            return &line[..index];
        }
        escaped = character == '\\' && !escaped;
        if character != '\\' {
            escaped = false;
        }
    }
    line
}

fn parse_string(value: &str) -> Result<String, ReviewError> {
    let inner = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .ok_or_else(|| error("XT1013", "sidecar strings must use double quotes"))?;
    let mut result = String::new();
    let mut chars = inner.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            result.push(character);
            continue;
        }
        match chars.next() {
            Some('n') => result.push('\n'),
            Some('r') => result.push('\r'),
            Some('t') => result.push('\t'),
            Some('"') => result.push('"'),
            Some('\\') => result.push('\\'),
            _ => {
                return Err(error(
                    "XT1013",
                    "the sidecar contains an unsupported string escape",
                ));
            }
        }
    }
    Ok(result)
}

fn error(code: &'static str, message: impl Into<String>) -> ReviewError {
    ReviewError {
        code,
        message: message.into(),
    }
}

fn trim_start(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    bytes
}
fn trim_end(mut bytes: &[u8]) -> &[u8] {
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn block_has_id(block: &str, id: &str) -> bool {
    block
        .lines()
        .filter_map(|line| line.split_once('='))
        .any(|(key, value)| key.trim() == "id" && parse_string(value.trim()).as_deref() == Ok(id))
}

fn write_field(output: &mut String, name: &str, value: &str) {
    output.push_str(name);
    output.push_str(" = \"");
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            _ => output.push(character),
        }
    }
    output.push_str("\"\n");
}
