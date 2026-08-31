//! TeX's diagnostics, translated to the author's terms.
//!
//! The engine reports against a file the author has never seen. Everything
//! here closes that gap and nothing here rewrites the evidence: the engine's
//! own sentence is carried unchanged, and the location and blame are added
//! beside it — `author-latex`, `xtex-construct`, `xtex-generated`, or
//! `unresolved` when no map segment supports an attribution. A confident
//! wrong attribution is worst exactly where the user cannot check it against
//! a terminal, so the rule is the CLI's, moved here so both hosts share it:
//! **never attribute blame without a map segment supporting it.**

use std::fmt::Write as _;

use crate::diagnostics::{Blame, DiagnosticSpan, map_emitted_diagnostic};
use crate::document::Document;
use crate::editor::entity_at;
use crate::source::Sources;
use crate::sourcemap::MappedEmission;
use crate::symbols::{EntityClass, SymbolTable};
use crate::texlog::{Record, Severity, Visual};

/// One engine record, with everything a reader needs beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Translated {
    /// `"error"`, `"warning"`, or `"unrecognised"`.
    pub kind: &'static str,
    /// The engine's own words, unchanged. For an unrecognised record, the
    /// whole line — an engine's output is evidence, and a line that cannot be
    /// placed is still a line it said.
    pub message: String,
    /// One-based line in the emitted file, where the engine gave one.
    pub emitted_line: Option<u32>,
    /// Which side of the boundary the finding belongs to, where a record has
    /// a location at all.
    pub blame: Option<Blame>,
    /// The author's position, when a map segment supports one.
    pub span: Option<DiagnosticSpan>,
    /// The author's entity, when a map segment and a declaration both supply
    /// the evidence: its name, its class, how the failure reads, and how
    /// much — the engine's own quantity, where its message named one.
    pub entity: Option<(String, EntityClass, Visual, Option<String>)>,
    /// For a table overfull whose box trace named the offending content:
    /// the column it sits in (one-based) and that content, quoted. Only
    /// present when the content matches exactly ONE cell of the author's
    /// row — a confident wrong column is worse than a located table
    /// (decision 0018).
    pub column: Option<(u32, String)>,
}

/// Merges the engine's stderr with its log file, the way the CLI always has.
///
/// The raw log carries records stderr never shows — `Float too large for
/// page` is one, checked against a live run — and a typesetting failure
/// supersedes the plainer stderr line for the same message, so the restated
/// form is the one an author reads.
#[must_use]
pub fn merge_records(stderr: Option<&str>, log: Option<&str>, emitted_name: &str) -> Vec<Record> {
    let mut records = stderr.map(crate::texlog::parse).unwrap_or_default();
    if let Some(text) = log {
        for record in crate::texlog::parse_log(text, emitted_name) {
            let Record::Typeset { message, .. } = &record else {
                continue;
            };
            records.retain(|existing| {
                !matches!(
                    existing,
                    Record::Located { message: other, .. } if other == message
                )
            });
            if !records.contains(&record) {
                records.push(record);
            }
        }
    }
    records
}

/// Translates merged records against one emission.
#[must_use]
pub fn translate(
    records: &[Record],
    emission: &MappedEmission,
    sources: &Sources,
    document: &Document,
    table: &SymbolTable,
) -> Vec<Translated> {
    records
        .iter()
        .map(|record| translate_one(record, emission, sources, document, table))
        .collect()
}

fn translate_one(
    record: &Record,
    emission: &MappedEmission,
    sources: &Sources,
    document: &Document,
    table: &SymbolTable,
) -> Translated {
    let (kind, line, message, visual) = match record {
        Record::Located {
            severity,
            line,
            message,
            ..
        } => (
            match severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            },
            *line,
            message.clone(),
            None,
        ),
        Record::Typeset {
            visual,
            line,
            message,
            trace,
            ..
        } => (
            "warning",
            *line,
            message.clone(),
            Some((*visual, trace.clone())),
        ),
        Record::Unrecognised(text) => {
            return Translated {
                kind: "unrecognised",
                message: text.clone(),
                emitted_line: None,
                blame: None,
                span: None,
                entity: None,
                column: None,
            };
        }
    };

    // Read the amount before the message moves into the mapper.
    let amount = crate::texlog::amount_in(&message).map(str::to_owned);
    let (visual, trace) = match visual {
        Some((visual, trace)) => (Some(visual), trace),
        None => (None, None),
    };
    let mapped = map_emitted_diagnostic(message, &emission.bytes, line, 1, &emission.map);

    // A typesetting failure is restated in the author's terms only when a map
    // segment and a declared entity both supply the evidence. Where either is
    // missing the engine's own sentence stands, because "something overflows"
    // is worse than a message that at least locates a box.
    let entity = visual.and_then(|visual| {
        let span = mapped.span.as_ref()?;
        let (name, class) = entity_at(sources, document, table, span.offset as usize)?;
        Some((name, class, visual, amount.clone()))
    });
    let column = entity
        .as_ref()
        .filter(|(_, class, ..)| *class == EntityClass::Table)
        .and_then(|_| {
            let trace = trace.as_deref()?;
            column_of(&emission.bytes, line, trace)
        });

    Translated {
        kind,
        message: mapped.message,
        emitted_line: Some(line),
        blame: Some(mapped.blame),
        span: mapped.span,
        entity,
        column,
    }
}

/// The one-based column whose cell carries the trace content — and the
/// content itself — when exactly one cell of the row does.
///
/// The row is the EMITTED line the engine named: the trace content came
/// from that very line, and in a transported table body its `&`-separated
/// cells are the author's own. Matching is containment over
/// whitespace-collapsed text, both directions. Zero matches or several
/// answer `None`: the honest sentence then names the table, never a
/// guessed column (decision 0018).
fn column_of(emitted: &[u8], line: u32, trace: &str) -> Option<(u32, String)> {
    let text = std::str::from_utf8(emitted).ok()?;
    let row = text.lines().nth(line.checked_sub(1)? as usize)?;
    let collapse = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let needle = collapse(trace);
    if needle.len() < 4 {
        return None;
    }
    let mut hit = None;
    for (index, cell) in row.split('&').enumerate() {
        let cell_text = collapse(cell);
        if cell_text.is_empty() {
            continue;
        }
        if cell_text.contains(&needle) || needle.contains(&cell_text) {
            if hit.is_some() {
                return None;
            }
            hit = Some((u32::try_from(index + 1).ok()?, needle.clone()));
        }
    }
    hit
}

/// Renders translations as JSON, one renderer for both hosts.
///
/// The same discipline as [`crate::check::to_json`]: written by hand, field
/// order fixed, so the native build and the WebAssembly build cannot drift
/// apart in a way a byte comparison would miss.
pub fn to_json(translated: &[Translated], out: &mut String) {
    out.push_str("{\"records\":[");
    for (index, record) in translated.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"kind\":\"");
        out.push_str(record.kind);
        out.push_str("\",\"message\":");
        push_json_string(&record.message, out);
        out.push_str(",\"emitted_line\":");
        match record.emitted_line {
            Some(line) => out.push_str(&line.to_string()),
            None => out.push_str("null"),
        }
        out.push_str(",\"blame\":");
        match record.blame {
            Some(blame) => {
                out.push('"');
                out.push_str(blame.as_str());
                out.push('"');
            }
            None => out.push_str("null"),
        }
        out.push_str(",\"span\":");
        match &record.span {
            Some(span) => {
                out.push_str("{\"file\":");
                push_json_string(&span.file, out);
                let _ = write!(
                    out,
                    ",\"offset\":{},\"length\":{},\"line\":{},\"column\":{}}}",
                    span.offset, span.length, span.line, span.column
                );
            }
            None => out.push_str("null"),
        }
        out.push_str(",\"column\":");
        match &record.column {
            Some((index, content)) => {
                use std::fmt::Write as _;
                let _ = write!(out, "{{\"index\":{index},\"content\":");
                push_json_string(content, out);
                out.push('}');
            }
            None => out.push_str("null"),
        }
        out.push_str(",\"entity\":");
        match &record.entity {
            Some((name, class, visual, amount)) => {
                out.push_str("{\"name\":");
                push_json_string(name, out);
                out.push_str(",\"class\":\"");
                out.push_str(class.name());
                out.push_str("\",\"reads\":\"");
                out.push_str(visual.name());
                out.push_str("\",\"amount\":");
                match amount {
                    Some(amount) => push_json_string(amount, out),
                    None => out.push_str("null"),
                }
                out.push('}');
            }
            None => out.push_str("null"),
        }
        out.push('}');
    }
    out.push_str("]}");
}

fn push_json_string(text: &str, out: &mut String) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}
