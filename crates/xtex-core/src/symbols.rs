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

/// The class of a declared entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EntityClass {
    /// Unmodelled LaTeX, consistent with every known class.
    UnknownOpen,
    /// A typed figure or an annotation attached to a figure.
    Figure,
    /// A typed table or an annotation attached to a table.
    Table,
    /// A displayed equation.
    Equation,
    /// A sectioning command, including an appendix.
    Section,
    /// An appendix.
    Appendix,
    /// An algorithm environment.
    Algorithm,
    /// A citation key.
    Citation,
    /// A dimension or percentage.
    Length,
}

impl EntityClass {
    /// Whether two classes can safely be used together.
    #[must_use]
    pub const fn is_consistent_with(self, other: Self) -> bool {
        matches!(self, Self::UnknownOpen)
            || matches!(other, Self::UnknownOpen)
            || self as u8 == other as u8
    }

    /// Stable spelling used by diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::UnknownOpen => "unknown-open",
            Self::Figure => "figure",
            Self::Table => "table",
            Self::Equation => "equation",
            Self::Section => "section",
            Self::Appendix => "appendix",
            Self::Algorithm => "algorithm",
            Self::Citation => "citation",
            Self::Length => "length",
        }
    }
}

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
    /// What the declaration names, or the open unknown class for unmodelled LaTeX.
    pub class: EntityClass,
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
        /// Source holding the first declaration.
        first_source: SourceId,
        /// The `@id` being rejected.
        second: Span,
        /// Source holding the rejected declaration.
        second_source: SourceId,
    },
    /// An identifier is empty, or contains bytes an identifier may not.
    Malformed {
        /// Source holding the construct.
        source: SourceId,
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
    prefixes: PrefixMap,
}

/// Project-defined mapping from identifier prefixes to entity classes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixMap(BTreeMap<String, EntityClass>);

impl Default for PrefixMap {
    fn default() -> Self {
        let mut map = BTreeMap::new();
        for (class, prefixes) in [
            (EntityClass::Figure, &["fig"][..]),
            (EntityClass::Table, &["tab"][..]),
            (EntityClass::Section, &["sec", "subsec", "ch"][..]),
            (EntityClass::Appendix, &["app"][..]),
            (EntityClass::Algorithm, &["alg"][..]),
            (EntityClass::Equation, &["eq"][..]),
        ] {
            for prefix in prefixes {
                map.insert((*prefix).to_owned(), class);
            }
        }
        Self(map)
    }
}

impl PrefixMap {
    /// Creates an empty map. Supplying a `[prefixes]` table uses this rather
    /// than the default because project configuration replaces the convention.
    #[must_use]
    pub const fn empty() -> Self {
        Self(BTreeMap::new())
    }

    /// Associates `prefix` with `class`.
    pub fn insert(&mut self, prefix: impl Into<String>, class: EntityClass) {
        self.0.insert(prefix.into(), class);
    }

    fn demand_of(&self, name: &str) -> EntityClass {
        name.split_once(':')
            .and_then(|(prefix, _)| self.0.get(prefix).copied())
            .unwrap_or(EntityClass::UnknownOpen)
    }
}

impl SymbolTable {
    /// Creates an empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a table using the project's complete prefix map.
    #[must_use]
    pub fn with_prefixes(prefixes: PrefixMap) -> Self {
        Self {
            prefixes,
            ..Self::default()
        }
    }

    /// Adds every construct in `document` to the table.
    ///
    /// Merging a second document into the same table is how a root absorbs an
    /// imported file: the scope is the root, so the two share one namespace and
    /// a clash between them is a real clash.
    pub fn merge(&mut self, sources: &Sources, document: &Document) {
        document.walk(|node| {
            let (kind, construct) = match node {
                Node::Construct { kind, span, .. } => (*kind, *span),
                _ => return,
            };
            let Some(payload) = payload_of(sources, node.source(), construct, kind) else {
                return;
            };
            match kind {
                EntryToken::Cite => {
                    // One reference per key, so an absent key is a diagnostic
                    // naming that key rather than the whole construct.
                    let keys = keys_of(sources, payload);
                    if keys.is_empty() {
                        self.errors.push(SymbolError::Malformed {
                            source: node.source(),
                            construct,
                        });
                        return;
                    }
                    for key in keys {
                        self.references.push((
                            key,
                            Reference {
                                kind,
                                payload,
                                construct,
                            },
                        ));
                    }
                }
                EntryToken::Id | EntryToken::Figure | EntryToken::Table => {
                    let Some(text) = text_of(sources, payload) else {
                        return;
                    };
                    if !is_identifier(text.as_bytes()) {
                        self.errors.push(SymbolError::Malformed {
                            source: node.source(),
                            construct,
                        });
                        return;
                    }
                    if let Some(first) = self.declarations.get(&text) {
                        self.errors.push(SymbolError::Duplicate {
                            name: text,
                            first: first.construct,
                            first_source: first.payload.source,
                            second: construct,
                            second_source: node.source(),
                        });
                        return;
                    }
                    let class = match kind {
                        EntryToken::Figure => EntityClass::Figure,
                        EntryToken::Table => EntityClass::Table,
                        EntryToken::Id => attached_class(sources, payload.source, construct),
                        _ => EntityClass::UnknownOpen,
                    };
                    self.declarations.insert(
                        text,
                        Declaration {
                            payload,
                            construct,
                            class,
                        },
                    );
                }
                EntryToken::Ref => {
                    let Some(text) = text_of(sources, payload) else {
                        return;
                    };
                    if text.is_empty() {
                        self.errors.push(SymbolError::Malformed {
                            source: node.source(),
                            construct,
                        });
                        return;
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
        });
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

    /// Class demanded by `name` under this table's project prefix map.
    #[must_use]
    pub fn demand_of(&self, name: &str) -> EntityClass {
        self.prefixes.demand_of(name)
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

    /// Every `@ref` that neither an `@id` nor a `\\label` declares.
    ///
    /// The inventory is the author's own `\\label` commands. Without it,
    /// referencing a figure they have not annotated yet is a hard error on a
    /// document LaTeX resolves without complaint — and annotating a document
    /// one figure at a time is the whole on-ramp.
    ///
    /// A `\\label` says a name exists and nothing about what it names, so it
    /// resolves the reference and supplies no class. `?O` is consistent with
    /// everything, so no class comparison fires against it.
    pub fn unresolved_against<'a>(
        &'a self,
        labels: &'a crate::labels::Inventory,
    ) -> impl Iterator<Item = (&'a str, &'a Reference)> {
        self.references.iter().filter_map(move |(name, reference)| {
            (reference.kind == EntryToken::Ref
                && !self.declarations.contains_key(name)
                && !labels.contains(name))
            .then_some((name.as_str(), reference))
        })
    }

    /// Every `@cite`, for a checker that has a key set to compare against.
    pub fn citations(&self) -> impl Iterator<Item = (&str, &Reference)> {
        self.references.iter().filter_map(|(name, reference)| {
            (reference.kind == EntryToken::Cite).then_some((name.as_str(), reference))
        })
    }

    /// References whose known prefix contradicts their known target class.
    pub fn inconsistent_references(
        &self,
    ) -> impl Iterator<Item = (&str, &Reference, EntityClass, &Declaration)> {
        self.references.iter().filter_map(|(name, reference)| {
            if reference.kind != EntryToken::Ref {
                return None;
            }
            let demand = self.prefixes.demand_of(name);
            let declaration = self.declarations.get(name)?;
            (!demand.is_consistent_with(declaration.class)).then_some((
                name.as_str(),
                reference,
                demand,
                declaration,
            ))
        })
    }

    /// Problems found while building the table.
    pub fn errors(&self) -> impl Iterator<Item = &SymbolError> {
        self.errors.iter()
    }
}

/// Class demanded by the prefix before the first colon.
#[must_use]
pub fn demand_of(name: &str) -> EntityClass {
    PrefixMap::default().demand_of(name)
}

fn attached_class(sources: &Sources, source: SourceId, construct: Span) -> EntityClass {
    let Some(bytes) = sources.get(source).map(crate::source::Source::bytes) else {
        return EntityClass::UnknownOpen;
    };
    let mut start = construct.start();
    let mut lines = 0usize;
    while start > 0 && construct.start() - start < 256 && bytes[start - 1].is_ascii_whitespace() {
        start -= 1;
        if bytes[start] == b'\n' {
            lines += 1;
            if lines > 2 {
                return EntityClass::UnknownOpen;
            }
        }
    }
    let before = &bytes[..start];
    if before.ends_with(b"\\]") || before.ends_with(b"$$") {
        return EntityClass::Equation;
    }
    for command in [
        b"\\section".as_slice(),
        b"\\subsection",
        b"\\subsubsection",
        b"\\chapter",
        b"\\part",
    ] {
        if last_command_with_balanced_argument(before, command) {
            return EntityClass::Section;
        }
    }
    for (environment, class) in [
        ("algorithm", EntityClass::Algorithm),
        ("figure", EntityClass::Figure),
        ("table", EntityClass::Table),
        ("equation", EntityClass::Equation),
        ("align", EntityClass::Equation),
        ("gather", EntityClass::Equation),
        ("multline", EntityClass::Equation),
    ] {
        let opening = format!("\\begin{{{environment}}}");
        if before.ends_with(opening.as_bytes())
            || before.ends_with(format!("\\begin{{{environment}*}}").as_bytes())
        {
            return class;
        }
    }
    EntityClass::UnknownOpen
}

fn last_command_with_balanced_argument(bytes: &[u8], command: &[u8]) -> bool {
    let Some(open) = bytes.iter().rposition(|byte| *byte == b'{') else {
        return false;
    };
    bytes[..open].ends_with(command)
        && crate::scanner::balanced_end(bytes, open) == Some(bytes.len())
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
    // A citation names its own command, so its keyword is not fixed: the
    // payload starts after the construct's first `(`.
    if kind == EntryToken::Cite {
        let bytes = sources.get(source)?.slice(construct)?;
        if !bytes.ends_with(b")") {
            return None;
        }
        let open = construct.start() + bytes.iter().position(|b| *b == b'(')? + 1;
        let end = construct.end() - 1;
        if open > end {
            return None;
        }
        return Some(Payload {
            source,
            span: Span::new(u32::try_from(open).ok()?, u32::try_from(end).ok()?),
        });
    }

    let keyword = match kind {
        EntryToken::Id => "@id(",
        EntryToken::Ref => "@ref(",
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
    std::str::from_utf8(trim(bytes)).ok().map(str::to_owned)
}

/// Every comma-separated key in a citation payload.
///
/// `\cite{a,b}` is ordinary LaTeX and 13% of measured citations use it, one of
/// them with seven keys. Reading only the first would leave the rest unchecked
/// while reporting success, which is worse than not checking at all.
fn keys_of(sources: &Sources, payload: Payload) -> Vec<String> {
    let Some(bytes) = sources
        .get(payload.source)
        .and_then(|s| s.slice(payload.span))
    else {
        return Vec::new();
    };
    bytes
        .split(|b| *b == b',')
        .filter_map(|key| std::str::from_utf8(trim(key)).ok())
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `bytes` without leading or trailing ASCII whitespace.
fn trim(bytes: &[u8]) -> &[u8] {
    bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .map_or(&[][..], |start| {
            let end = bytes
                .iter()
                .rposition(|b| !b.is_ascii_whitespace())
                .unwrap_or(start);
            &bytes[start..=end]
        })
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
        let (_, t) = table(&[("a.xtex", "\\section{X} @id(sec:intro)")]);
        assert_eq!(t.declared().collect::<Vec<_>>(), ["sec:intro"]);
        assert!(t.declaration("sec:intro").is_some());
        assert!(t.errors().next().is_none());
    }

    #[test]
    fn a_reference_to_nothing_is_unresolved() {
        let (_, t) = table(&[(
            "a.xtex",
            "See @ref(missing) and @ref(present). @id(present)",
        )]);
        let unresolved: Vec<_> = t.unresolved_references().map(|(n, _)| n).collect();
        assert_eq!(unresolved, ["missing"]);
    }

    #[test]
    fn the_scope_is_the_root_so_two_files_share_one_namespace() {
        // Both files belong to one root, so the second declaration clashes.
        let (_, t) = table(&[("a.xtex", "@id(fig:main)"), ("b.xtex", "@id(fig:main)")]);
        let errors: Vec<_> = t.errors().collect();
        assert_eq!(errors.len(), 1);
        assert!(matches!(errors[0], SymbolError::Duplicate { name, .. } if name == "fig:main"));
    }

    #[test]
    fn separate_roots_may_reuse_an_identifier() {
        // A project with several roots — the corpus has one with five. Separate
        // tables, so the same name in each is not a clash.
        let (_, first) = table(&[("paper_jss/main.xtex", "@id(fig:main)")]);
        let (_, second) = table(&[("paper_tse/main.xtex", "@id(fig:main)")]);
        assert!(first.errors().next().is_none());
        assert!(second.errors().next().is_none());
    }

    #[test]
    fn a_duplicate_is_blamed_on_the_later_declaration() {
        let (_, t) = table(&[("a.xtex", "@id(x) then later @id(x)")]);
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
        let (_, t) = table(&[("a.xtex", "@cite(knuth1984)")]);
        assert_eq!(t.unresolved_references().count(), 0);
        assert_eq!(
            t.citations().map(|(n, _)| n).collect::<Vec<_>>(),
            ["knuth1984"]
        );
    }

    #[test]
    fn a_citation_may_name_several_keys() {
        // Replaces a test that asserted the key stopped at the first comma,
        // because a `style` field followed it. The grammar no longer has
        // fields: `\\cite{a,b}` is ordinary LaTeX, 13% of measured citations
        // use it, and one names seven keys. Reading only the first would leave
        // the rest unchecked while reporting success.
        let (_, t) = table(&[("a.xtex", "@citep(knuth1984, lamport1994)")]);
        assert_eq!(
            t.citations().map(|(n, _)| n).collect::<Vec<_>>(),
            ["knuth1984", "lamport1994"]
        );
    }

    #[test]
    fn every_citation_command_is_a_construct() {
        let (_, t) = table(&[(
            "a.xtex",
            "@cite(a) @citep(b) @citet(c) @textcite(d) @parencite(e)",
        )]);
        assert_eq!(
            t.citations().map(|(n, _)| n).collect::<Vec<_>>(),
            ["a", "b", "c", "d", "e"]
        );
    }

    #[test]
    fn a_malformed_identifier_is_reported_rather_than_declared() {
        let (_, t) = table(&[("a.xtex", "@id(9starts-with-a-digit)")]);
        assert_eq!(t.declared().count(), 0);
        assert!(matches!(
            t.errors().next(),
            Some(SymbolError::Malformed { .. })
        ));
    }

    #[test]
    fn a_construct_inside_an_excluded_region_never_reaches_the_table() {
        let (_, t) = table(&[(
            "a.xtex",
            "% @id(commented)\n$@id(math)$\n\\begin{verbatim}\n@id(verb)\n\\end{verbatim}\n@id(real)",
        )]);
        assert_eq!(t.declared().collect::<Vec<_>>(), ["real"]);
    }

    #[test]
    fn typed_blocks_retain_their_classes() {
        let (_, t) = table(&[("a.xtex", "\\figure(fig:x) { caption = {X} }")]);
        assert_eq!(t.declaration("fig:x").unwrap().class, EntityClass::Figure);
    }

    #[test]
    fn an_id_takes_a_known_attachment_class() {
        let (_, t) = table(&[("a.xtex", "\\section{X}\n@id(sec:x)")]);
        assert_eq!(t.declaration("sec:x").unwrap().class, EntityClass::Section);
    }

    #[test]
    fn an_id_on_unmodelled_latex_is_unknown_open() {
        let (_, t) = table(&[("a.xtex", "\\newtheorem{X}\n@id(fig:x)")]);
        assert_eq!(
            t.declaration("fig:x").unwrap().class,
            EntityClass::UnknownOpen
        );
    }

    #[test]
    fn only_two_known_inconsistent_classes_conflict() {
        let (_, known) = table(&[("a.xtex", "\\table(fig:x) { caption = {X} } @ref(fig:x)")]);
        assert_eq!(known.inconsistent_references().count(), 1);

        let (_, unknown_target) = table(&[("a.xtex", "\\newtheorem{X} @id(fig:x) @ref(fig:x)")]);
        assert_eq!(unknown_target.inconsistent_references().count(), 0);

        let (_, unknown_demand) =
            table(&[("a.xtex", "\\table(def:x) { caption = {X} } @ref(def:x)")]);
        assert_eq!(unknown_demand.inconsistent_references().count(), 0);
    }
}
