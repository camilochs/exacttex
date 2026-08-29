//! The whole-project pipeline behind `xtex check`.
//!
//! One root, its transitive imports, the author's own `\include`/`\input`
//! edges, the bibliography and the label inventory — assembled through a
//! [`SourceLoader`], so the same pipeline serves the CLI's filesystem and the
//! WebAssembly build's caller-supplied bundle. Moved here from the CLI when
//! issue #69 needed it from both sides of that boundary.

use std::collections::{BTreeMap, BTreeSet};

use crate::bibliography::{Bibliography, Declared, assemble, declared_in};
use crate::check::{Blame, Diagnostic, Severity, check_documents, check_with_labels};
use crate::document::Node;
use crate::io::{IoError, SourceLoader};
use crate::parse;
use crate::scanner::EntryToken;
use crate::source::{SourceId, Sources};
use crate::symbols::{EntityClass, SymbolTable};

/// Checks one document root, wherever its bytes live.
///
/// This is the whole pipeline behind `xtex check`, host-independent: the
/// loader answers every question about the outside world. The CLI hands it a
/// filesystem; the WebAssembly build hands it the bundle a browser supplied.
///
/// # Errors
///
/// Returns [`IoError`] when the root itself cannot be loaded, or an import
/// fails for a reason other than not existing.
pub fn check_project(
    loader: &impl SourceLoader,
    root: &str,
    prefixes: crate::symbols::PrefixMap,
) -> Result<(Sources, Vec<Diagnostic>, f64, Bibliography), IoError> {
    let mut sources = Sources::new();
    let root_id = loader.load(root, None, &mut sources)?;
    let mut table = SymbolTable::with_prefixes(prefixes);
    let mut documents = Vec::new();
    let mut pending = vec![root_id];
    let mut merged = BTreeSet::new();
    let mut import_diagnostics = Vec::new();
    let mut declared = Declared::default();
    // One inventory for the whole root, merged like the symbol table: a
    // `\label` in an imported file declares its name for the root, exactly as
    // an `@id` there does.
    let mut labels: BTreeMap<String, crate::source::Span> = BTreeMap::new();
    let mut labels_unavailable = None;

    while let Some(id) = pending.pop() {
        let name = sources
            .get(id)
            .map(|s| s.name().to_owned())
            .unwrap_or_default();
        let canonical = loader.canonical(&name);
        if !merged.insert(canonical) {
            continue;
        }
        let document = parse(&sources, id);
        table.merge(&sources, &document);
        match crate::labels::inventory(&sources, &document, id) {
            crate::labels::Inventory::Complete(found) => labels.extend(found),
            // One file that went dark makes the root's inventory a subset, and
            // a subset that looks complete turns every name it missed into a
            // false "not declared".
            crate::labels::Inventory::Unavailable(reason) => labels_unavailable = Some(reason),
        }
        merge_declared(&mut declared, declared_in(&sources, id));
        follow_latex_edges(
            loader,
            id,
            &mut sources,
            &mut pending,
            &mut labels_unavailable,
        )?;
        let mut imports = Vec::new();
        document.walk(|node| {
            if let Node::Construct {
                kind: EntryToken::Import,
                span,
                ..
            } = node
                && let Some(path) = literal_import(&sources, id, *span)
            {
                imports.push((*span, path));
            }
        });
        for (span, path) in imports {
            match loader.load(&path, Some(id), &mut sources) {
                Ok(imported) => pending.push(imported),
                Err(IoError::NotFound { .. } | IoError::Unresolvable { .. }) => {
                    labels_unavailable = Some(crate::labels::Unavailable::UnreadableEdge);
                    import_diagnostics.push(Diagnostic {
                        code: "XT1009",
                        entity: EntityClass::UnknownOpen,
                        name: Some(path.clone()),
                        source: id,
                        span,
                        message: format!("import path `{path}` does not resolve"),
                        related: Vec::new(),
                        severity: Severity::Error,
                        blame: Blame::XtexConstruct,
                    });
                }
                Err(error) => return Err(error),
            }
        }
        documents.push(document);
    }

    let bibliography = assemble(&declared, |name| loader.read_aux(root, name));
    // The author's own `\label` commands resolve `@ref` too, so annotating a
    // document one figure at a time does not report the unannotated ones as
    // missing. Merged across the root, like every other declaration.
    let inventory = root_inventory(labels, labels_unavailable);
    let mut diagnostics = check_with_labels(&table, &bibliography, &inventory);
    diagnostics.extend(bibliography_advisory(&table, &bibliography));
    diagnostics.extend(check_documents(&sources, &documents, |source, name| {
        let beside = sources
            .get(source)
            .map(|s| s.name().to_owned())
            .unwrap_or_default();
        loader.file_exists(&beside, name)
    }));
    diagnostics.extend(import_diagnostics);
    let coverage = root_coverage(&sources, &documents);
    Ok((sources, diagnostics, coverage, bibliography))
}

fn root_coverage(sources: &Sources, documents: &[crate::document::Document]) -> f64 {
    let mut total = 0.0;
    let mut checked = 0.0;
    for document in documents {
        let Some(source) = sources.get(document.source()) else {
            continue;
        };
        let bytes =
            f64::from(u32::try_from(source.bytes().len()).expect("source exceeds u32 addressing"));
        total += bytes;
        checked += document.coverage() * bytes;
    }
    if total == 0.0 { 1.0 } else { checked / total }
}

fn follow_latex_edges(
    loader: &impl SourceLoader,
    id: SourceId,
    sources: &mut Sources,
    pending: &mut Vec<SourceId>,
    unavailable: &mut Option<crate::labels::Unavailable>,
) -> Result<(), IoError> {
    let (edges, computed) = sources
        .get(id)
        .map(|source| latex_inventory_edges(source.bytes()))
        .unwrap_or_default();
    if computed {
        *unavailable = Some(crate::labels::Unavailable::UnreadableEdge);
    }
    for path in edges {
        match load_latex_edge(loader, &path, id, sources) {
            Ok(included) => pending.push(included),
            Err(error @ IoError::TooLarge { .. }) => return Err(error),
            Err(_) => *unavailable = Some(crate::labels::Unavailable::UnreadableEdge),
        }
    }
    Ok(())
}

/// Loads a `\include`/`\input` target, trying `.tex` then `.xtex`.
///
/// # Errors
///
/// Returns [`IoError`] when neither spelling can be loaded.
pub fn load_latex_edge(
    loader: &impl SourceLoader,
    path: &str,
    relative_to: SourceId,
    sources: &mut Sources,
) -> Result<SourceId, IoError> {
    // An extension is a dot in the final component. `Path` semantics are a
    // host notion; names here are logical.
    let final_component = path.rsplit('/').next().unwrap_or(path);
    if final_component.contains('.') {
        return loader.load(path, Some(relative_to), sources);
    }
    match loader.load(&format!("{path}.tex"), Some(relative_to), sources) {
        Ok(id) => Ok(id),
        Err(IoError::NotFound { .. }) => {
            loader.load(&format!("{path}.xtex"), Some(relative_to), sources)
        }
        Err(error) => Err(error),
    }
}

fn latex_inventory_edges(bytes: &[u8]) -> (Vec<String>, bool) {
    let mut edges = Vec::new();
    let mut computed = false;
    for span in crate::scanner::readable_for(bytes, &["include", "input"]) {
        let region = &bytes[span.start()..span.end()];
        computed |= collect_latex_edges(region, &mut edges);
    }
    (edges, computed)
}

fn collect_latex_edges(mut region: &[u8], edges: &mut Vec<String>) -> bool {
    let mut computed = false;
    while let Some(at) = region.windows(2).position(|window| window == b"\\i") {
        region = &region[at..];
        let command_len = if region.starts_with(b"\\include") {
            b"\\include".len()
        } else if region.starts_with(b"\\input") {
            b"\\input".len()
        } else {
            region = &region[2..];
            continue;
        };
        if region
            .get(command_len)
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'@')
        {
            region = &region[command_len..];
            continue;
        }
        let rest = &region[command_len..];
        let whitespace = rest
            .iter()
            .position(|byte| !byte.is_ascii_whitespace())
            .unwrap_or(rest.len());
        let rest = &rest[whitespace..];
        let Some(body) = rest.strip_prefix(b"{") else {
            computed = true;
            region = rest;
            continue;
        };
        let Some(close) = body.iter().position(|byte| *byte == b'}') else {
            return true;
        };
        let path = &body[..close];
        if path
            .iter()
            .any(|byte| matches!(byte, b'\\' | b'{' | b'}' | b'#'))
        {
            computed = true;
        } else if let Ok(path) = std::str::from_utf8(path) {
            let path = path.trim();
            if path.is_empty() {
                computed = true;
            } else {
                edges.push(path.to_owned());
            }
        } else {
            computed = true;
        }
        region = &body[close + 1..];
    }
    computed
}

/// The literal path inside an `@import`, when it is one.
#[must_use]
pub fn literal_import(
    sources: &Sources,
    source: SourceId,
    span: crate::source::Span,
) -> Option<String> {
    let bytes = sources.get(source)?.slice(span)?;
    let first = bytes.iter().position(|byte| *byte == b'"')?;
    let last = bytes.iter().rposition(|byte| *byte == b'"')?;
    (last > first)
        .then(|| String::from_utf8(bytes[first + 1..last].to_vec()).ok())
        .flatten()
}

fn merge_declared(target: &mut Declared, found: Declared) {
    target.resources.extend(found.resources);
    target.inline_keys.extend(found.inline_keys);
    target.computed = target.computed.or(found.computed);
}

pub(crate) fn bibliography_advisory(
    table: &SymbolTable,
    bibliography: &Bibliography,
) -> Option<Diagnostic> {
    let Bibliography::Unavailable(reason) = bibliography else {
        return None;
    };
    let (_, citation) = table.citations().next()?;
    Some(Diagnostic {
        code: "XT2001",
        entity: EntityClass::Citation,
        name: None,
        source: citation.payload.source,
        span: citation.payload.span,
        message: format!("citation checking unavailable: {}", reason.reason()),
        related: Vec::new(),
        severity: Severity::Advisory,
        blame: Blame::Unresolved,
    })
}

/// One inventory for a whole document root.
///
/// Complete or unavailable, never partial: a subset that looks complete turns
/// every name it missed into a false "not declared", which is the failure this
/// whole inventory exists to prevent.
fn root_inventory(
    labels: BTreeMap<String, crate::source::Span>,
    unavailable: Option<crate::labels::Unavailable>,
) -> crate::labels::Inventory {
    match unavailable {
        Some(reason) => crate::labels::Inventory::Unavailable(reason),
        None => crate::labels::Inventory::Complete(labels),
    }
}

/// Loads a root and its transitive `@import`s, keeping each parsed document.
///
/// This is the traversal `xtex rename` has always used — imports only, not
/// the author's `\include` edges, because a rename rewrites `.xtex`
/// constructs and transported LaTeX is never rewritten. Returns each
/// source's resolved name beside its id, because a caller that writes files
/// must write to the name that actually resolved, not the one requested.
///
/// # Errors
///
/// Returns [`IoError`] when the root or any import cannot be loaded.
#[allow(clippy::type_complexity)]
pub fn load_imports(
    loader: &impl SourceLoader,
    root: &str,
) -> Result<
    (
        Sources,
        Vec<crate::document::Document>,
        Vec<(SourceId, String)>,
    ),
    IoError,
> {
    let mut sources = Sources::new();
    let mut documents = Vec::new();
    let mut names = Vec::new();
    let mut pending = vec![(root.to_owned(), None)];
    let mut seen = BTreeSet::new();
    while let Some((name, parent)) = pending.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let id = loader.load(&name, parent, &mut sources)?;
        let document = parse(&sources, id);
        let mut imports = Vec::new();
        document.walk(|node| {
            if let Node::Construct {
                kind: EntryToken::Import,
                span,
                ..
            } = node
                && let Some(path) = literal_import(&sources, id, *span)
            {
                imports.push(path);
            }
        });
        for path in imports {
            pending.push((path, Some(id)));
        }
        let resolved = sources
            .get(id)
            .map_or_else(|| name.clone(), |source| source.name().to_owned());
        names.push((id, resolved));
        documents.push(document);
    }
    Ok((sources, documents, names))
}

#[cfg(test)]
mod bibliography_advisory_tests {
    use super::*;
    use crate::bibliography::Unavailable;
    use crate::check::to_json;

    fn table(text: &str) -> (Sources, SymbolTable) {
        let mut sources = Sources::new();
        let id = sources.add("main.xtex", text.as_bytes().to_vec());
        let document = parse(&sources, id);
        let mut table = SymbolTable::new();
        table.merge(&sources, &document);
        (sources, table)
    }

    #[test]
    fn unreadable_bibliography_with_a_citation_is_an_advisory_in_json() {
        let (sources, table) = table("See @cite(knuth1984).");
        let bibliography = Bibliography::Unavailable(Unavailable::Unreadable {
            name: "refs.bib".to_owned(),
        });
        let advisory = bibliography_advisory(&table, &bibliography)
            .expect("the explicit citation requests bibliography checking");

        assert_eq!(advisory.severity, Severity::Advisory);
        assert_eq!(advisory.blame, Blame::Unresolved);
        assert_eq!(advisory.name, None, "the advisory is not about a key");
        let mut json = String::new();
        to_json(&sources, &[advisory], 1.0, &mut json);
        assert!(json.contains("\"severity\":\"advisory\""), "{json}");
        assert!(
            json.contains("citation checking unavailable: `refs.bib` could not be read"),
            "{json}"
        );
    }

    #[test]
    fn no_advisory_message_carries_a_derived_debug_spelling() {
        // The reader of this line is the author, not the compiler. Field names
        // and byte offsets belong in neither output form.
        let (_, table) = table("See @cite(knuth1984).");
        for reason in [
            Unavailable::NoneDeclared,
            Unavailable::ComputedPath {
                span: crate::source::Span::new(0, 1),
            },
            Unavailable::Unreadable {
                name: "refs.bib".to_owned(),
            },
            Unavailable::UnparsableEntry {
                name: "refs.bib".to_owned(),
                detail: "a value opened at line 3 is never closed".to_owned(),
            },
        ] {
            let message = bibliography_advisory(&table, &Bibliography::Unavailable(reason.clone()))
                .expect("the explicit citation requests bibliography checking")
                .message;
            for tell in ["{", "}", "Span", "start:", "name:"] {
                assert!(
                    !message.contains(tell),
                    "`{tell}` leaked into `{message}` from `{reason:?}`"
                );
            }
        }
    }

    #[test]
    fn unreadable_bibliography_without_a_citation_says_nothing() {
        // Declarations, references and a plain LaTeX `\cite` — every kind of
        // construct except the one that asks about a bibliography. Anchoring on
        // any of them would report an unreadable file at a document that never
        // asked, which is the invariant in `AGENTS.md` §4.
        let (_, table) = table(
            "@id(intro) here, @ref(intro) and @ref(absent).\n\\cite{knuth1984}\n\\bibliography{refs}",
        );
        let bibliography = Bibliography::Unavailable(Unavailable::Unreadable {
            name: "refs.bib".to_owned(),
        });

        assert!(bibliography_advisory(&table, &bibliography).is_none());
    }

    #[test]
    fn complete_bibliography_says_nothing() {
        let (_, table) = table("See @cite(knuth1984).");
        let bibliography = Bibliography::Complete(BTreeSet::from(["knuth1984".to_owned()]));

        assert!(bibliography_advisory(&table, &bibliography).is_none());
    }
}
