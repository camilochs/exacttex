//! What a construct names, and whether the name resolves.
//!
//! The scanner finds where a construct is. This reads what is inside it — the
//! identifier, the bibliography key, the import path — and builds the table
//! that `@ref` is checked against.
//!
//! # Scope
//!
//! An identifier is scoped to **one document root**, not one file and not the
//! whole project. A root is its root file plus every file reached through
//! `@import`, and its table is the merge of theirs, per `docs/grammar.md` §4.
//!
//! That matches LaTeX, where a label is document-wide. It also means two
//! separately emitted papers in one project may reuse an identifier, which
//! matters: the corpus contains a project with five document roots sharing a
//! directory.

use std::collections::BTreeMap;

use crate::document::{Document, Node};
use crate::scanner::EntryToken;
use crate::source::{SourceId, Sources, Span};

/// The text a construct carries between its parentheses.
///
/// Borrowed from the source rather than copied, because the source outlives the
/// table and copying would put a second version of every identifier in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Payload {
    /// Source the construct came from.
    pub source: SourceId,
    /// The bytes between `(` and `)`, excluding both.
    pub span: Span,
}

/// A declaration made by `@id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Declaration {
    /// Where the identifier text is.
    pub payload: Payload,
    /// The whole `@id(...)` construct, which is what a diagnostic points at.
    pub construct: Span,
}

/// A use of a name, by `@ref` or `@cite`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reference {
    /// Which construct made the reference.
    pub kind: EntryToken,
    /// Where the referenced text is.
    pub payload: Payload,
    /// The whole construct, which is what a diagnostic points at.
    pub construct: Span,
}

/// Something the table cannot accept.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SymbolError {
    /// Two `@id` constructs declare the same identifier in one root.
    ///
    /// Blamed on the later one: the first declaration is not at fault for
    /// existing.
    Duplicate {
        /// The identifier, as written.
        name: String,
        /// The `@id` that declared it first.
        first: Span,
        /// The `@id` being rejected.
        second: Span,
    },
    /// An identifier is empty, or contains bytes an identifier may not.
    Malformed {
        /// The construct at fault.
        construct: Span,
    },
}

/// Names declared and used within one document root.
#[derive(Debug, Default, Clone)]
pub struct SymbolTable {
    declarations: BTreeMap<String, Declaration>,
    references: Vec<(String, Reference)>,
    errors: Vec<SymbolError>,
}

impl SymbolTable {
    /// Creates an empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds every construct in `document` to the table.
    ///
    /// Merging a second document into the same table is how a root absorbs an
    /// imported file: the scope is the root, so the two share one namespace and
    /// a clash between them is a real clash.
    pub fn merge(&mut self, sources: &Sources, document: &Document) {
        for node in document.iter() {
            let (kind, construct) = match node {
                Node::Construct { kind, span, .. } => (*kind, *span),
                _ => continue,
            };
            let Some(payload) = payload_of(sources, node.source(), construct, kind) else {
                continue;
            };
            let Some(text) = text_of(sources, payload) else {
                continue;
            };

            match kind {
                EntryToken::Id | EntryToken::Figure | EntryToken::Table => {
                    if !is_identifier(text.as_bytes()) {
                        self.errors.push(SymbolError::Malformed { construct });
                        continue;
                    }
                    if let Some(first) = self.declarations.get(&text) {
                        self.errors.push(SymbolError::Duplicate {
                            name: text,
                            first: first.construct,
                            second: construct,
                        });
                        continue;
                    }
                    self.declarations
                        .insert(text, Declaration { payload, construct });
                }
                EntryToken::Ref | EntryToken::Cite => {
                    if text.is_empty() {
                        self.errors.push(SymbolError::Malformed { construct });
                        continue;
                    }
                    self.references.push((
                        text,
                        Reference {
                            kind,
                            payload,
                            construct,
                        },
                    ));
                }
                _ => {}
            }
        }
    }

    /// Declaration of `name`, if one exists.
    #[must_use]
    pub fn declaration(&self, name: &str) -> Option<&Declaration> {
        self.declarations.get(name)
    }

    /// Every declared identifier, sorted.
    pub fn declared(&self) -> impl Iterator<Item = &str> {
        self.declarations.keys().map(String::as_str)
    }

    /// Every `@ref` whose identifier no `@id` declares.
    ///
    /// `@cite` is excluded: its keys come from a bibliography, and reporting a
    /// missing key needs that bibliography to have been read successfully.
    /// Reporting one here would call an unread bibliography an absent key.
    pub fn unresolved_references(&self) -> impl Iterator<Item = (&str, &Reference)> {
        self.references.iter().filter_map(|(name, reference)| {
            (reference.kind == EntryToken::Ref && !self.declarations.contains_key(name))
                .then_some((name.as_str(), reference))
        })
    }

    /// Every `@cite`, for a checker that has a key set to compare against.
    pub fn citations(&self) -> impl Iterator<Item = (&str, &Reference)> {
        self.references.iter().filter_map(|(name, reference)| {
            (reference.kind == EntryToken::Cite).then_some((name.as_str(), reference))
        })
    }

    /// Problems found while building the table.
    pub fn errors(&self) -> impl Iterator<Item = &SymbolError> {
        self.errors.iter()
    }
}

/// The span between `(` and `)` of a construct covering `construct`.
///
/// Returns `None` for a construct that carries no payload, such as the raw
/// escape.
fn payload_of(
    sources: &Sources,
    source: SourceId,
    construct: Span,
    kind: EntryToken,
) -> Option<Payload> {
    let keyword = match kind {
        EntryToken::Id => "@id(",
        EntryToken::Ref => "@ref(",
        EntryToken::Cite => "@cite(",
        EntryToken::Import => "@import(",
        // A block declares its identifier just as `@id` does: the name between
        // its parentheses is what a reference resolves to.
        EntryToken::Figure => "\\figure(",
        EntryToken::Table => "\\table(",
        _ => return None,
    };
    let bytes = sources.get(source)?.slice(construct)?;
    if !bytes.starts_with(keyword.as_bytes()) {
        return None;
    }
    let start = construct.start() + keyword.len();
    // An inline construct ends at its `)`. A block ends at its closing brace,
    // so its identifier ends at the first `)` instead.
    let end = match kind {
        EntryToken::Figure | EntryToken::Table => {
            construct.start() + bytes.iter().position(|b| *b == b')')?
        }
        _ if bytes.ends_with(b")") => construct.end() - 1,
        _ => return None,
    };
    if start > end {
        return None;
    }
    Some(Payload {
        source,
        span: Span::new(u32::try_from(start).ok()?, u32::try_from(end).ok()?),
    })
}

/// The payload's bytes as text, if they are valid UTF-8.
///
/// An identifier is ASCII by the grammar, so anything that is not valid UTF-8
/// is not an identifier and is reported rather than lossily converted.
fn text_of(sources: &Sources, payload: Payload) -> Option<String> {
    let bytes = sources.get(payload.source)?.slice(payload.span)?;
    // A citation key may carry a comma and fields after it; the key is what
    // precedes the first comma.
    let key = bytes.split(|b| *b == b',').next().unwrap_or(bytes);
    let trimmed = key
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .map_or(&[][..], |start| {
            let end = key
                .iter()
                .rposition(|b| !b.is_ascii_whitespace())
                .unwrap_or(start);
            &key[start..=end]
        });
    std::str::from_utf8(trimmed).ok().map(str::to_owned)
}

fn is_identifier(bytes: &[u8]) -> bool {
    matches!(bytes.first(), Some(b) if b.is_ascii_alphabetic())
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b'.' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn table(files: &[(&str, &str)]) -> (Sources, SymbolTable) {
        let mut sources = Sources::new();
        let mut table = SymbolTable::new();
        for (name, text) in files {
            let id = sources.add(*name, text.as_bytes().to_vec());
            let document = parse(&sources, id);
            table.merge(&sources, &document);
        }
        (sources, table)
    }

    #[test]
    fn a_declaration_is_found_by_its_name() {
        let (_, t) = table(&[("a.ntex", "\\section{X} @id(sec:intro)")]);
        assert_eq!(t.declared().collect::<Vec<_>>(), ["sec:intro"]);
        assert!(t.declaration("sec:intro").is_some());
        assert!(t.errors().next().is_none());
    }

    #[test]
    fn a_reference_to_nothing_is_unresolved() {
        let (_, t) = table(&[(
            "a.ntex",
            "See @ref(missing) and @ref(present). @id(present)",
        )]);
        let unresolved: Vec<_> = t.unresolved_references().map(|(n, _)| n).collect();
        assert_eq!(unresolved, ["missing"]);
    }

    #[test]
    fn the_scope_is_the_root_so_two_files_share_one_namespace() {
        // Both files belong to one root, so the second declaration clashes.
        let (_, t) = table(&[("a.ntex", "@id(fig:main)"), ("b.ntex", "@id(fig:main)")]);
        let errors: Vec<_> = t.errors().collect();
        assert_eq!(errors.len(), 1);
        assert!(matches!(errors[0], SymbolError::Duplicate { name, .. } if name == "fig:main"));
    }

    #[test]
    fn separate_roots_may_reuse_an_identifier() {
        // A project with several roots — the corpus has one with five. Separate
        // tables, so the same name in each is not a clash.
        let (_, first) = table(&[("paper_jss/main.ntex", "@id(fig:main)")]);
        let (_, second) = table(&[("paper_tse/main.ntex", "@id(fig:main)")]);
        assert!(first.errors().next().is_none());
        assert!(second.errors().next().is_none());
    }

    #[test]
    fn a_duplicate_is_blamed_on_the_later_declaration() {
        let (_, t) = table(&[("a.ntex", "@id(x) then later @id(x)")]);
        let Some(SymbolError::Duplicate { first, second, .. }) = t.errors().next() else {
            panic!("expected a duplicate")
        };
        assert!(
            first.start() < second.start(),
            "the first declaration is not at fault for existing"
        );
    }

    #[test]
    fn a_citation_is_not_reported_as_an_unresolved_reference() {
        // Its key comes from a bibliography. Reporting it here would call an
        // unread bibliography an absent key.
        let (_, t) = table(&[("a.ntex", "@cite(knuth1984)")]);
        assert_eq!(t.unresolved_references().count(), 0);
        assert_eq!(
            t.citations().map(|(n, _)| n).collect::<Vec<_>>(),
            ["knuth1984"]
        );
    }

    #[test]
    fn a_citation_key_stops_at_its_first_field() {
        let (_, t) = table(&[("a.ntex", "@cite(knuth1984, style=textual)")]);
        assert_eq!(
            t.citations().map(|(n, _)| n).collect::<Vec<_>>(),
            ["knuth1984"]
        );
    }

    #[test]
    fn a_malformed_identifier_is_reported_rather_than_declared() {
        let (_, t) = table(&[("a.ntex", "@id(9starts-with-a-digit)")]);
        assert_eq!(t.declared().count(), 0);
        assert!(matches!(
            t.errors().next(),
            Some(SymbolError::Malformed { .. })
        ));
    }

    #[test]
    fn a_construct_inside_an_excluded_region_never_reaches_the_table() {
        let (_, t) = table(&[(
            "a.ntex",
            "% @id(commented)\n$@id(math)$\n\\begin{verbatim}\n@id(verb)\n\\end{verbatim}\n@id(real)",
        )]);
        assert_eq!(t.declared().collect::<Vec<_>>(), ["real"]);
    }
}
