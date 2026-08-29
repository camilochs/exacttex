//! Hard diagnostics substantiated by explicit NextTeX constructs.

use crate::bibliography::{Bibliography, missing_citations};
use crate::source::{SourceId, Span};
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
                    code: "NT1001",
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
                code: "NT1002",
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
            code: "NT1003",
            entity: super::symbols::demand_of(name),
            name: Some(name.to_owned()),
            source: reference.payload.source,
            span: reference.payload.span,
            message: format!("identifier `{name}` is not declared"),
            related: Vec::new(),
        });
    }
    for (name, reference, demand, declaration) in table.inconsistent_references() {
        diagnostics.push(Diagnostic {
            code: "NT1004",
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
            code: "NT1005",
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::bibliography::Unavailable;
    use crate::parse;
    use crate::source::Sources;

    fn diagnostics(source: &str, bibliography: &Bibliography) -> Vec<Diagnostic> {
        let mut sources = Sources::new();
        let id = sources.add("main.ntex", source.as_bytes().to_vec());
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
        assert_eq!(found[0].code, "NT1005");
    }
}
