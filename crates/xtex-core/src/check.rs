//! Hard diagnostics substantiated by explicit ExactTeX constructs.

use crate::bibliography::{Bibliography, missing_citations};
use crate::blocks::{BlockError, BlockKind, Value, parse_block};
use crate::document::{Document, Node};
use crate::scanner::EntryToken;
use crate::source::{SourceId, Sources, Span};
use crate::symbols::{EntityClass, SymbolError, SymbolTable};
use std::collections::BTreeMap;

/// A location used to explain a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Related {
    /// Source holding the bytes.
    pub source: SourceId,
    /// Bytes being referred to.
    pub span: Span,
    /// One sentence without a trailing period.
    pub message: String,
}

/// One checker finding, independent of its human or JSON rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Stable checker code.
    pub code: &'static str,
    /// Entity involved in the finding.
    pub entity: EntityClass,
    /// Identifier or citation key, when there is one.
    pub name: Option<String>,
    /// Source holding the primary span.
    pub source: SourceId,
    /// Primary bytes at fault.
    pub span: Span,
    /// One sentence without a trailing period.
    pub message: String,
    /// Locations that explain the primary finding.
    pub related: Vec<Related>,
    /// Whether the finding can change the process exit code.
    pub severity: Severity,
    /// Which side of the source boundary the finding belongs to.
    pub blame: Blame,
}

/// How strongly a diagnostic is substantiated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A substantiated failure in an explicit ExactTeX construct.
    Error,
    /// A condition worth reporting that does not change the exit code.
    Advisory,
}

impl Severity {
    /// The stable spelling used by both renderers.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Advisory => "advisory",
        }
    }
}

/// The side of the source boundary responsible for a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blame {
    /// An explicit ExactTeX construct.
    XtexConstruct,
    /// The available evidence does not establish a side.
    Unresolved,
}

impl Blame {
    /// The stable spelling used by both renderers.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::XtexConstruct => "xtex-construct",
            Self::Unresolved => "unresolved",
        }
    }
}

/// Runs the checks whose evidence is already assembled for one document root.
#[must_use]
pub fn check(table: &SymbolTable, bibliography: &Bibliography) -> Vec<Diagnostic> {
    check_with_labels(
        table,
        bibliography,
        &crate::labels::Inventory::Complete(BTreeMap::new()),
    )
}

/// Checks with the author's own `\\label` commands available to resolve `@ref`.
///
/// Without them, referencing a figure the author has not annotated yet is a
/// hard error on a document LaTeX resolves without complaint, and annotating a
/// document one figure at a time — the whole on-ramp — does not work.
#[must_use]
pub fn check_with_labels(
    table: &SymbolTable,
    bibliography: &Bibliography,
    labels: &crate::labels::Inventory,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for error in table.errors() {
        match error {
            SymbolError::Duplicate {
                name,
                first,
                first_source,
                second,
                second_source,
            } => {
                let Some(declaration) = table.declaration(name) else {
                    continue;
                };
                diagnostics.push(Diagnostic {
                    code: "XT1001",
                    entity: declaration.class,
                    name: Some(name.clone()),
                    source: *second_source,
                    span: *second,
                    message: format!("identifier `{name}` is already declared"),
                    related: vec![Related {
                        source: *first_source,
                        span: *first,
                        message: "first declared here".to_owned(),
                    }],
                    severity: Severity::Error,
                    blame: Blame::XtexConstruct,
                });
            }
            SymbolError::Malformed { source, construct } => diagnostics.push(Diagnostic {
                code: "XT1002",
                entity: EntityClass::UnknownOpen,
                name: None,
                source: *source,
                span: *construct,
                message: "identifier is empty or contains unsupported bytes".to_owned(),
                related: Vec::new(),
                severity: Severity::Error,
                blame: Blame::XtexConstruct,
            }),
        }
    }
    for (name, reference) in table.unresolved_against(labels) {
        diagnostics.push(Diagnostic {
            code: "XT1003",
            entity: table.demand_of(name),
            name: Some(name.to_owned()),
            source: reference.payload.source,
            span: reference.payload.span,
            message: format!("identifier `{name}` is not declared"),
            related: Vec::new(),
            severity: Severity::Error,
            blame: Blame::XtexConstruct,
        });
    }
    for (name, reference, demand, declaration) in table.inconsistent_references() {
        diagnostics.push(Diagnostic {
            code: "XT1004",
            entity: demand,
            name: Some(name.to_owned()),
            source: reference.payload.source,
            span: reference.payload.span,
            message: format!(
                "reference `{name}` requires {}, but its target is {}",
                demand.name(),
                declaration.class.name()
            ),
            related: vec![Related {
                source: declaration.payload.source,
                span: declaration.payload.span,
                message: format!("{} declared here", declaration.class.name()),
            }],
            severity: Severity::Error,
            blame: Blame::XtexConstruct,
        });
    }
    for (key, reference) in missing_citations(table, bibliography) {
        diagnostics.push(Diagnostic {
            code: "XT1005",
            entity: EntityClass::Citation,
            name: Some(key.to_owned()),
            source: reference.payload.source,
            span: reference.payload.span,
            message: format!("citation key `{key}` is not in the bibliography"),
            related: Vec::new(),
            severity: Severity::Error,
            blame: Blame::XtexConstruct,
        });
    }
    diagnostics
}

/// Checks typed blocks in every source belonging to one document root.
///
/// `resolves` receives literal paths and the source that wrote them. A false
/// result is evidence for XT1006; computed paths never reach this callback.
pub fn check_documents(
    sources: &Sources,
    documents: &[Document],
    mut resolves: impl FnMut(SourceId, &str) -> bool,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for document in documents {
        document.walk(|node| {
            let (source, span, kind, malformed) = match node {
                Node::Construct {
                    source, span, kind, ..
                } => (*source, *span, *kind, false),
                Node::Malformed { source, span, kind } => (*source, *span, *kind, true),
                Node::Opaque { .. } => return,
            };
            let Some(block_kind) = block_kind(kind) else {
                return;
            };
            let Some(bytes) = sources.get(source).map(crate::source::Source::bytes) else {
                return;
            };
            let start = span.start();
            let entry = start
                + match block_kind {
                    BlockKind::Figure => b"\\figure(".len(),
                    BlockKind::Table => b"\\table(".len(),
                };
            match parse_block(bytes, block_kind, start, entry) {
                Ok(block) if !malformed => check_block(
                    sources,
                    source,
                    bytes,
                    &block,
                    &mut resolves,
                    &mut diagnostics,
                ),
                Err(error) => diagnostics.push(block_error(source, block_kind, error, bytes)),
                Ok(_) => {}
            }
        });
    }
    diagnostics
}

fn block_kind(kind: EntryToken) -> Option<BlockKind> {
    match kind {
        EntryToken::Figure => Some(BlockKind::Figure),
        EntryToken::Table => Some(BlockKind::Table),
        _ => None,
    }
}

fn check_block(
    _sources: &Sources,
    source: SourceId,
    bytes: &[u8],
    block: &crate::blocks::Block,
    resolves: &mut impl FnMut(SourceId, &str) -> bool,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for field in &block.fields {
        let key = &bytes[field.key.start()..field.key.end()];
        if let Value::Percentage(span) = field.value {
            let number = &bytes[span.start()..span.end() - 1];
            if std::str::from_utf8(number)
                .ok()
                .and_then(|n| n.parse::<f64>().ok())
                .is_some_and(|n| !(0.0..=100.0).contains(&n))
            {
                diagnostics.push(Diagnostic {
                    code: "XT1007",
                    entity: EntityClass::Length,
                    name: None,
                    source,
                    span,
                    message: "percentage must be between 0 and 100".to_owned(),
                    related: Vec::new(),
                    severity: Severity::Error,
                    blame: Blame::XtexConstruct,
                });
            }
        }
        if block.kind == BlockKind::Figure && key == b"src" {
            let Value::Str(span) = field.value else {
                continue;
            };
            let raw = &bytes[span.start() + 1..span.end() - 1];
            let Ok(path) = std::str::from_utf8(raw) else {
                continue;
            };
            if !path.contains('\\') && !resolves(source, path) {
                diagnostics.push(Diagnostic {
                    code: "XT1006",
                    entity: EntityClass::Figure,
                    name: Some(path.to_owned()),
                    source,
                    span,
                    message: format!("image file `{path}` does not resolve"),
                    related: Vec::new(),
                    severity: Severity::Error,
                    blame: Blame::XtexConstruct,
                });
            }
        }
    }
}

fn block_error(source: SourceId, kind: BlockKind, error: BlockError, bytes: &[u8]) -> Diagnostic {
    let (span, message, length_error, identifier_error) = match error {
        BlockError::BadIdentifier(span) => (
            span,
            "identifier is empty or contains unsupported bytes".to_owned(),
            false,
            true,
        ),
        BlockError::MissingBody(span) => (span, "block body is required".to_owned(), false, false),
        BlockError::UnclosedBody(span) => {
            (span, "block body is not closed".to_owned(), false, false)
        }
        BlockError::UnknownField { key, reason } => {
            (key, reason.explanation().to_owned(), false, false)
        }
        BlockError::WrongValueKind { key, expected } => {
            let name = bytes.get(key.start()..key.end()).unwrap_or_default();
            (
                key,
                format!("field requires {expected}"),
                name == b"width" || name == b"height",
                false,
            )
        }
        BlockError::MissingEquals(span) => (
            span,
            "field requires `=` and a value".to_owned(),
            false,
            false,
        ),
    };
    Diagnostic {
        code: if identifier_error {
            "XT1002"
        } else if length_error {
            "XT1007"
        } else {
            "XT1008"
        },
        entity: if length_error {
            EntityClass::Length
        } else if kind == BlockKind::Figure {
            EntityClass::Figure
        } else {
            EntityClass::Table
        },
        name: None,
        source,
        span,
        message,
        related: Vec::new(),
        severity: Severity::Error,
        blame: Blame::XtexConstruct,
    }
}

#[cfg(test)]
mod label_tests {
    use super::*;
    use crate::bibliography::{Bibliography, Unavailable};
    use crate::labels::inventory;
    use crate::parse;
    use crate::symbols::SymbolTable;

    fn diagnose(text: &str) -> Vec<String> {
        let mut sources = Sources::new();
        let id = sources.add("a.xtex", text.as_bytes().to_vec());
        let document = parse(&sources, id);
        let mut table = SymbolTable::new();
        table.merge(&sources, &document);
        let labels = inventory(&sources, &document, id);
        check_with_labels(
            &table,
            &Bibliography::Unavailable(Unavailable::NoneDeclared),
            &labels,
        )
        .into_iter()
        .map(|diagnostic| diagnostic.name.unwrap_or_default())
        .collect()
    }

    #[test]
    fn a_reference_to_the_authors_own_label_resolves() {
        // Annotating a document one figure at a time is the on-ramp, and it
        // does not work if referencing an unannotated figure is a hard error
        // on a document LaTeX resolves without complaint.
        assert!(diagnose("\\label{fig:old}\nSee @ref(fig:old).").is_empty());
    }

    #[test]
    fn a_reference_to_nothing_still_fails() {
        // The fix widened what resolves. It must not have weakened what fails.
        assert_eq!(
            diagnose("\\label{fig:old}\nSee @ref(fig:ghost)."),
            ["fig:ghost"]
        );
    }

    #[test]
    fn a_label_in_a_macro_body_does_not_resolve_a_reference() {
        assert_eq!(
            diagnose("\\newcommand{\\m}[1]{\\label{fig:inside}}\nSee @ref(fig:inside)."),
            ["fig:inside"]
        );
    }

    #[test]
    fn a_commented_label_does_not_resolve_a_reference() {
        assert_eq!(
            diagnose("% \\label{fig:hidden}\nSee @ref(fig:hidden)."),
            ["fig:hidden"]
        );
    }
}

#[cfg(test)]
mod json_tests {
    use super::*;
    use crate::bibliography::{Bibliography, Unavailable};
    use crate::parse;
    use crate::symbols::SymbolTable;

    /// The JSON was unrendered by any test until the WebAssembly build needed
    /// it. A scripted refactor scrambled the field order and every test still
    /// passed, which is how this one came to exist.
    #[test]
    fn the_json_is_well_formed_and_carries_every_field() {
        let mut sources = Sources::new();
        let id = sources.add("j.xtex", b"@id(a) @ref(b)\n".to_vec());
        let document = parse(&sources, id);
        let mut table = SymbolTable::new();
        table.merge(&sources, &document);
        let diagnostics = check(
            &table,
            &Bibliography::Unavailable(Unavailable::NoneDeclared),
        );

        let mut json = String::new();
        to_json(&sources, &diagnostics, document.coverage(), &mut json);

        assert!(json.starts_with("{\"coverage\":"), "{json}");
        assert!(json.ends_with("]}"), "{json}");
        for field in [
            "\"code\":\"XT1003\"",
            "\"severity\":\"error\"",
            "\"blame\":\"xtex-construct\"",
            "\"entity\":",
            "\"name\":\"b\"",
            "\"span\":{\"file\":\"j.xtex\"",
            "\"offset\":",
            "\"length\":",
            "\"line\":1",
            "\"column\":",
            "\"message\":",
            "\"related\":[]",
        ] {
            assert!(json.contains(field), "missing {field} in {json}");
        }
        // Braces balance, which a scrambled render does not.
        assert_eq!(
            json.matches('{').count(),
            json.matches('}').count(),
            "{json}"
        );
    }

    #[test]
    fn a_message_with_a_quote_or_a_newline_is_escaped() {
        let mut out = String::new();
        write_json_string("a \"quoted\" line\nand another", &mut out);
        assert_eq!(out, "\"a \\\"quoted\\\" line\\nand another\"");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::bibliography::Unavailable;
    use crate::parse;
    use crate::source::Sources;

    fn diagnostics(source: &str, bibliography: &Bibliography) -> Vec<Diagnostic> {
        let mut sources = Sources::new();
        let id = sources.add("main.xtex", source.as_bytes().to_vec());
        let document = parse(&sources, id);
        let mut table = SymbolTable::new();
        table.merge(&sources, &document);
        check(&table, bibliography)
    }

    #[test]
    fn ordinary_latex_with_unresolved_names_has_no_errors() {
        let found = diagnostics(
            "See \\ref{missing} and \\cite{invented}.",
            &Bibliography::Complete(BTreeSet::default()),
        );
        assert!(found.is_empty());
    }

    #[test]
    fn an_unreadable_bibliography_silences_missing_citations() {
        let found = diagnostics(
            "@cite(invented)",
            &Bibliography::Unavailable(Unavailable::Unreadable {
                name: "missing.bib".to_owned(),
            }),
        );
        assert!(found.is_empty());
    }

    #[test]
    fn a_complete_bibliography_substantiates_a_missing_citation() {
        let found = diagnostics(
            "@cite(invented)",
            &Bibliography::Complete(BTreeSet::default()),
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].code, "XT1005");
    }
}

use std::fmt::Write as _;

/// File, one-based line and one-based column of a span.
fn location(
    sources: &Sources,
    source: SourceId,
    span: crate::source::Span,
) -> (&str, usize, usize) {
    let Some(source) = sources.get(source) else {
        return ("<unresolved>", 1, 1);
    };
    let before = &source.bytes()[..span.start().min(source.bytes().len())];
    #[allow(clippy::naive_bytecount)]
    let line = before.iter().filter(|byte| **byte == b'\n').count() + 1;
    let column = before
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(before.len() + 1, |at| before.len() - at);
    (source.name(), line, column)
}

/// Renders diagnostics as the JSON `xtex check --json` prints.
///
/// It returns bytes rather than printing them, because the WebAssembly build
/// has no stdout and the exit criterion of #18 is that its JSON equals the
/// native tool's byte for byte. One implementation makes that true by
/// construction; two would make it a coincidence to be re-checked.
pub fn to_json(sources: &Sources, diagnostics: &[Diagnostic], coverage: f64, out: &mut String) {
    // Six decimals, not the sixteen an f64 prints. The extra digits are false
    // precision on a ratio of byte counts, and they are not reproducible: the
    // WebAssembly build computes `1.0 - opaque/total` on 32-bit `usize` and
    // lands one bit away from the native build, which made the two outputs
    // differ at the sixteenth digit and nowhere else.
    let _ = write!(out, "{{\"coverage\":{coverage:.6},\"diagnostics\":[");
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"code\":\"{}\",\"severity\":\"{}\",\"blame\":\"{}\",\"entity\":\"{}\",\"name\":",
            diagnostic.code,
            diagnostic.severity.as_str(),
            diagnostic.blame.as_str(),
            diagnostic.entity.name()
        );
        match &diagnostic.name {
            Some(name) => write_json_string(name, out),
            None => out.push_str("null"),
        }
        write_span(sources, diagnostic.source, diagnostic.span, out);
        out.push_str(",\"message\":");
        write_json_string(&diagnostic.message, out);
        out.push_str(",\"related\":[");
        for (related_index, related) in diagnostic.related.iter().enumerate() {
            if related_index > 0 {
                out.push(',');
            }
            out.push('{');
            // A related note has a span and a message and no code of its own.
            write_span(sources, related.source, related.span, out);
            out.push_str(",\"message\":");
            write_json_string(&related.message, out);
            out.push('}');
        }
        out.push_str("]}");
    }
    out.push_str("]}");
}

/// Appends `,"span":{...}` for one span. The leading comma is included because
/// every caller has already written a field before it.
fn write_span(sources: &Sources, source: SourceId, span: crate::source::Span, out: &mut String) {
    let (file, line, column) = location(sources, source, span);
    out.push_str(",\"span\":{\"file\":");
    write_json_string(file, out);
    let _ = write!(
        out,
        ",\"offset\":{},\"length\":{},\"line\":{line},\"column\":{column}}}",
        span.start(),
        span.len()
    );
}

/// Appends `value` as a quoted JSON string.
fn write_json_string(value: &str, out: &mut String) {
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if control.is_control() => {
                let _ = write!(out, "\\u{:04x}", control as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
}
