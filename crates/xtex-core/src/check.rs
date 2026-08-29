//! Hard diagnostics substantiated by explicit ExactTeX constructs.

use crate::bibliography::{Bibliography, missing_citations};
use crate::blocks::{BlockError, BlockKind, Value, parse_block};
use crate::document::{Document, Node};
use crate::scanner::EntryToken;
use crate::source::{SourceId, Sources, Span};
use crate::symbols::{EntityClass, SymbolError, SymbolTable};

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
}

/// Runs the checks whose evidence is already assembled for one document root.
#[must_use]
pub fn check(table: &SymbolTable, bibliography: &Bibliography) -> Vec<Diagnostic> {
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
            }),
        }
    }
    for (name, reference) in table.unresolved_references() {
        diagnostics.push(Diagnostic {
            code: "XT1003",
            entity: table.demand_of(name),
            name: Some(name.to_owned()),
            source: reference.payload.source,
            span: reference.payload.span,
            message: format!("identifier `{name}` is not declared"),
            related: Vec::new(),
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
