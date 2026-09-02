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
    ///
    /// Equal classes, either side open, or both in the sectioning family:
    /// an appendix is a section in LaTeX's own terms (the same `\section`
    /// command, after the `\appendix` switch), so `sec:` on an appendix
    /// and `app:` on a section after the switch are both consistent, and
    /// so are the words "Section" and "Appendix" before either. Reading
    /// them as distinct rejected 14 correct references in 3 corpus papers
    /// the day after reading them as one had rejected 98. Figure, table,
    /// equation and algorithm against the family stay errors.
    #[must_use]
    pub const fn is_consistent_with(self, other: Self) -> bool {
        matches!(self, Self::UnknownOpen)
            || matches!(other, Self::UnknownOpen)
            || self as u8 == other as u8
            || (self.is_sectioning() && other.is_sectioning())
    }

    /// Section or appendix.
    #[must_use]
    pub const fn is_sectioning(self) -> bool {
        matches!(self, Self::Section | Self::Appendix)
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

/// A use of a name, by a reference or citation construct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reference {
    /// Which construct made the reference.
    pub kind: EntryToken,
    /// Where the referenced text is.
    pub payload: Payload,
    /// The whole construct, which is what a diagnostic points at.
    pub construct: Span,
    /// The entity-kind word written immediately before a reference, when
    /// there is one: `Figure~@ref(x)` carries `Figure`. See
    /// [`prose_word_before`] and `docs/decisions/0019`.
    pub prose: Option<ProseWord>,
}

/// An entity-kind word the author wrote before a reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProseWord {
    /// The word as written, for the diagnostic to quote.
    pub word: &'static str,
    /// The class the word names.
    pub class: EntityClass,
    /// The word's bytes in the source, for the diagnostic to quote.
    pub span: Span,
}

/// The words the check reads, and the class each names.
///
/// A fixed vocabulary, deliberately: the capitalised forms authors put
/// before a reference, their plurals, and the abbreviations LaTeX
/// manuals use. Lower case is not matched — "figure" mid-sentence is
/// prose about figures, not a label for the next reference — and a class
/// the symbol table cannot assign (theorem, listing) has no word here,
/// because a word with no class to compare against could never fire.
const PROSE_WORDS: &[(&str, EntityClass)] = &[
    ("Figure", EntityClass::Figure),
    ("Figures", EntityClass::Figure),
    ("Fig.", EntityClass::Figure),
    ("Figs.", EntityClass::Figure),
    ("Table", EntityClass::Table),
    ("Tables", EntityClass::Table),
    ("Tab.", EntityClass::Table),
    ("Section", EntityClass::Section),
    ("Sections", EntityClass::Section),
    ("Sec.", EntityClass::Section),
    ("Equation", EntityClass::Equation),
    ("Equations", EntityClass::Equation),
    ("Eq.", EntityClass::Equation),
    ("Algorithm", EntityClass::Algorithm),
    ("Algorithms", EntityClass::Algorithm),
    ("Alg.", EntityClass::Algorithm),
    ("Appendix", EntityClass::Appendix),
    ("Appendices", EntityClass::Appendix),
];

/// The entity-kind word immediately before a construct starting at `at`.
///
/// Immediately: separated by nothing, one space, or one or more `~`. A word
/// further away is not about this reference, and a word after a line
/// ending is not either. The byte before the word must not be a letter,
/// so `Subfigure` does not end in a `figure` this reads. A construct is
/// only recognised in prose, so a word inside a comment or verbatim text
/// never precedes one.
#[must_use]
pub fn prose_word_before(bytes: &[u8], at: usize) -> Option<ProseWord> {
    let mut end = at;
    if bytes.get(end.wrapping_sub(1)) == Some(&b' ') {
        end -= 1;
    }
    while end > 0 && bytes[end - 1] == b'~' {
        end -= 1;
    }
    let (word, class) = PROSE_WORDS
        .iter()
        .find(|(word, _)| bytes[..end].ends_with(word.as_bytes()))?;
    let start = end - word.len();
    if start > 0 && bytes[start - 1].is_ascii_alphanumeric() {
        return None;
    }
    Some(ProseWord {
        word,
        class: *class,
        span: Span::new(u32::try_from(start).ok()?, u32::try_from(end).ok()?),
    })
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
        self.merge_from(sources, document, false);
    }

    /// As [`merge`](Self::merge), for a document that begins inside the
    /// appendices.
    ///
    /// `\appendix` is a switch, not a scope: everything after it is an
    /// appendix, including the files imported after it. A file that begins
    /// after the switch has no `\appendix` of its own, so its importer has to
    /// say so. See [`appendix_switch_at`].
    pub fn merge_from(&mut self, sources: &Sources, document: &Document, in_appendix: bool) {
        let switch = if in_appendix {
            Some(0)
        } else {
            sources
                .get(document.source())
                .and_then(|source| appendix_switch_at(source.bytes()))
        };
        let floats = sources
            .get(document.source())
            .map_or_else(Vec::new, |source| float_bodies(source.bytes()));
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
                                prose: None,
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
                        EntryToken::Id => attached_class(
                            sources,
                            payload.source,
                            construct,
                            switch.is_some_and(|at| at < construct.start()),
                            &floats,
                        ),
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
                EntryToken::Ref => self.merge_reference(sources, node.source(), construct, payload),
                _ => {}
            }
        });
    }

    /// Adds one reference construct's names to the table.
    ///
    /// `@cref(a, b)` names two identifiers and each is checked on its own,
    /// like a citation's keys. The other reference commands take one, so
    /// their whole payload is the name: `\autoref{a,b}` is one undefined
    /// reference in LaTeX too, and this reports it as one.
    fn merge_reference(
        &mut self,
        sources: &Sources,
        source: SourceId,
        construct: Span,
        payload: Payload,
    ) {
        let list = sources
            .get(source)
            .and_then(|s| s.slice(construct))
            .is_some_and(crate::scanner::names_a_list);
        let names = if list {
            keys_of(sources, payload)
        } else {
            text_of(sources, payload)
                .filter(|text| !text.is_empty())
                .into_iter()
                .collect()
        };
        if names.is_empty() {
            self.errors
                .push(SymbolError::Malformed { source, construct });
            return;
        }
        // The word before the construct applies to every name it lists:
        // "Figures~@cref(a, b)" says both are figures.
        let prose = sources
            .get(source)
            .and_then(|s| prose_word_before(s.bytes(), construct.start()));
        for name in names {
            self.references.push((
                name,
                Reference {
                    kind: EntryToken::Ref,
                    payload,
                    construct,
                    prose,
                },
            ));
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

    /// Every declaration with its name, sorted by name.
    pub fn declarations(&self) -> impl Iterator<Item = (&str, &Declaration)> {
        self.declarations
            .iter()
            .map(|(name, declaration)| (name.as_str(), declaration))
    }

    /// How many references demand `name`.
    #[must_use]
    pub fn reference_count(&self, name: &str) -> usize {
        self.references
            .iter()
            .filter(|(reference, _)| reference == name)
            .count()
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
            (matches!(labels, crate::labels::Inventory::Complete(_))
                && reference.kind == EntryToken::Ref
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

    /// References whose preceding entity-kind word contradicts the known
    /// class of their target. `docs/decisions/0019`.
    pub fn prose_mismatches(
        &self,
    ) -> impl Iterator<Item = (&str, &Reference, ProseWord, &Declaration)> {
        self.references.iter().filter_map(|(name, reference)| {
            let prose = reference.prose?;
            let declaration = self.declarations.get(name)?;
            (!prose.class.is_consistent_with(declaration.class)).then_some((
                name.as_str(),
                reference,
                prose,
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

/// Where `\appendix` switches a file into its appendices, if it does.
///
/// The first `\appendix` control word in readable content — not one in a
/// comment, a verbatim body or a macro definition. After it, sectioning
/// commands declare appendices: authors label them `app:x`, and reading
/// them as sections raised 98 false `XT1004` on three correct papers
/// (corpus E2: 2212.13570, 2212.14882, 2603.02873).
#[must_use]
pub fn appendix_switch_at(bytes: &[u8]) -> Option<usize> {
    let needle = b"\\appendix";
    for span in crate::scanner::readable_content(bytes) {
        let region = &bytes[span.start()..span.end()];
        let mut from = 0usize;
        while let Some(hit) = region[from..]
            .windows(needle.len())
            .position(|window| window == needle)
            .map(|at| from + at)
        {
            let after = region.get(hit + needle.len());
            if !after.is_some_and(u8::is_ascii_alphabetic) {
                return Some(span.start() + hit);
            }
            from = hit + 1;
        }
    }
    None
}

/// Whether `at` sits inside a display-math body that opened before it and
/// has not closed: a `\begin{align}` with no `\end{align}` between, or a
/// `\[` with no `\]` between.
fn inside_display_math(bytes: &[u8], at: usize) -> bool {
    let before = &bytes[..at];
    let last_open = before
        .windows(b"\\begin{".len())
        .rposition(|window| window == b"\\begin{");
    if let Some(open) = last_open {
        let rest = &before[open + b"\\begin{".len()..];
        if let Some(close) = rest.iter().position(|byte| *byte == b'}') {
            let name = &rest[..close];
            let mut end = b"\\end{".to_vec();
            end.extend_from_slice(name);
            end.push(b'}');
            let closed = rest.windows(end.len()).any(|window| window == end);
            if !closed && crate::scanner::is_display_math_region(&before[open..]) {
                return true;
            }
        }
    }
    // `\[` opens a display only when its backslash is not itself escaped:
    // `\\[2pt]` is a tabular line break with spacing, and reading it as an
    // opener classed every later declaration in the file as an equation
    // (47 XT1004 and 61 XT1020 on correct documents, corpus E2 re-run).
    let mut from = before.len();
    while let Some(open) = before[..from]
        .windows(2)
        .rposition(|window| window == b"\\[")
    {
        if !crate::scanner::is_escaped(before, open) {
            let closed = before[open..].windows(2).enumerate().any(|(at, window)| {
                window == b"\\]" && !crate::scanner::is_escaped(before, open + at)
            });
            return !closed;
        }
        from = open;
    }
    false
}

/// The environments whose whole body classes an `@id` inside it.
///
/// The label of a real float follows its `\caption`, or its
/// `\includegraphics` lines, or closes the environment; only the header
/// slot was read, so after `xtex adopt` every `fig:` and `alg:` identifier
/// in a converted paper was unknown-open, and `@ref(fig:x)` on a table
/// declared that way was never `XT1004` (issue #161). Nested `subfigure`,
/// `minipage` and `tabular` bodies belong to the float around them; a
/// display body inside a float keeps its own class.
///
/// The rotated (`rotating`), wrapped (`wrapfig`) and long (`longtable`)
/// floats are floats in practice and carry the class of the counter they
/// step; five census identifiers stayed unknown-open with only the three
/// standard names listed (issue #163).
const FLOAT_ENVIRONMENTS: &[(&str, EntityClass)] = &[
    ("figure", EntityClass::Figure),
    ("figure*", EntityClass::Figure),
    ("table", EntityClass::Table),
    ("table*", EntityClass::Table),
    ("algorithm", EntityClass::Algorithm),
    ("algorithm*", EntityClass::Algorithm),
    ("sidewaysfigure", EntityClass::Figure),
    ("sidewaysfigure*", EntityClass::Figure),
    ("sidewaystable", EntityClass::Table),
    ("sidewaystable*", EntityClass::Table),
    ("wrapfigure", EntityClass::Figure),
    ("wraptable", EntityClass::Table),
    ("longtable", EntityClass::Table),
    ("longtable*", EntityClass::Table),
];

/// Every closed float in `bytes`, with the class it gives and its span.
fn float_bodies(bytes: &[u8]) -> Vec<(EntityClass, Span)> {
    crate::scanner::closed_environments(bytes)
        .into_iter()
        .filter_map(|(name, body)| {
            let name = &bytes[name.start()..name.end()];
            FLOAT_ENVIRONMENTS
                .iter()
                .find(|(known, _)| known.as_bytes() == name)
                .map(|(_, class)| (*class, body))
        })
        .collect()
}

/// The class of the innermost float whose body holds `at`.
fn enclosing_float(floats: &[(EntityClass, Span)], at: usize) -> Option<EntityClass> {
    floats
        .iter()
        .filter(|(_, body)| body.start() <= at && at < body.end())
        .min_by_key(|(_, body)| body.len())
        .map(|(class, _)| *class)
}

/// The class an `@id` at `construct` declares.
///
/// In order: a display-math body around it; the construct it attaches
/// backwards to within the bounded whitespace of `docs/grammar.md` §4 — a
/// sectioning command, a caption written by hand, a closed display, an
/// environment header; then the float whose body holds it. An `@id` on
/// nothing the compiler models is unknown-open.
fn attached_class(
    sources: &Sources,
    source: SourceId,
    construct: Span,
    in_appendix: bool,
    floats: &[(EntityClass, Span)],
) -> EntityClass {
    let Some(bytes) = sources.get(source).map(crate::source::Source::bytes) else {
        return EntityClass::UnknownOpen;
    };
    if inside_display_math(bytes, construct.start()) {
        return EntityClass::Equation;
    }
    header_class(bytes, construct.start(), in_appendix)
        .or_else(|| enclosing_float(floats, construct.start()))
        .unwrap_or(EntityClass::UnknownOpen)
}

/// The class of the attachable construct within the bounded whitespace
/// before `at`, if there is one.
fn header_class(bytes: &[u8], at: usize, in_appendix: bool) -> Option<EntityClass> {
    let mut start = at;
    let mut lines = 0usize;
    while start > 0 && at - start < 256 && bytes[start - 1].is_ascii_whitespace() {
        start -= 1;
        if bytes[start] == b'\n' {
            lines += 1;
            if lines > 2 {
                return None;
            }
        }
    }
    let before = &bytes[..start];
    if before.ends_with(b"\\]") || before.ends_with(b"$$") {
        return Some(EntityClass::Equation);
    }
    for command in [
        b"\\section".as_slice(),
        b"\\subsection",
        b"\\subsubsection",
        b"\\chapter",
        b"\\part",
    ] {
        if last_command_with_balanced_argument(before, command) {
            return Some(if in_appendix {
                EntityClass::Appendix
            } else {
                EntityClass::Section
            });
        }
    }
    // `\captionof{figure}{…}` steps the figure counter wherever it is
    // written, so the kind in its first argument is the class, and the
    // construct attaches to the second argument as it does to a section
    // title. A figure set in a `minipage` or `center` with no float around
    // it is classed by this rule alone.
    for (kind, class) in [
        ("figure", EntityClass::Figure),
        ("table", EntityClass::Table),
    ] {
        if last_command_with_balanced_argument(before, format!("\\captionof{{{kind}}}").as_bytes())
        {
            return Some(class);
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
            return Some(class);
        }
    }
    None
}

/// Whether `bytes` ends with `command` followed by one balanced braced
/// argument.
///
/// The argument's `{` is found by walking back from the end over the
/// groups inside it, so a title or caption holding `\textbf{…}`,
/// `\cite{…}` or `$x^{2}$` attaches; the last `{` in the prefix, which is
/// what was read before, is that inner group's. The forward scan then
/// confirms the group, with its own rule for `%`. The last `{` stays as
/// the second reading, so that nothing that attached before stops
/// attaching: it is what reaches a command on a line that a `%` opens.
fn last_command_with_balanced_argument(bytes: &[u8], command: &[u8]) -> bool {
    let attaches = |open: usize| {
        bytes[..open].ends_with(command)
            && crate::scanner::balanced_end(bytes, open) == Some(bytes.len())
    };
    argument_opening(bytes).is_some_and(attaches)
        || bytes
            .iter()
            .rposition(|byte| *byte == b'{')
            .is_some_and(attaches)
}

/// The `{` opening the braced group `bytes` ends with, if it ends with one.
///
/// Walked back one line at a time, and on each line only the part before
/// an unescaped `%` counts, since the forward scan that confirms the group
/// drops a comment too; an escaped brace is text.
fn argument_opening(bytes: &[u8]) -> Option<usize> {
    let mut depth = 0usize;
    let mut end = bytes.len();
    loop {
        let start = bytes[..end]
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |at| at + 1);
        let live = bytes[start..end]
            .iter()
            .enumerate()
            .position(|(offset, byte)| {
                *byte == b'%' && !crate::scanner::is_escaped(bytes, start + offset)
            })
            .map_or(end, |offset| start + offset);
        for at in (start..live).rev() {
            match bytes[at] {
                b'}' if !crate::scanner::is_escaped(bytes, at) => depth += 1,
                b'{' if !crate::scanner::is_escaped(bytes, at) => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return Some(at);
                    }
                }
                _ if depth == 0 => return None,
                _ => {}
            }
        }
        if start == 0 || depth == 0 {
            return None;
        }
        end = start - 1;
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
    // A citation or reference names its own command, so its keyword is not
    // fixed: the payload starts after the construct's first `(`.
    if matches!(kind, EntryToken::Cite | EntryToken::Ref) {
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
    fn an_unavailable_label_inventory_reports_nothing_missing() {
        let (_, t) = table(&[("a.xtex", "@ref(possibly-in-an-unread-file)")]);
        let labels =
            crate::labels::Inventory::Unavailable(crate::labels::Unavailable::UnreadableEdge);

        assert_eq!(t.unresolved_against(&labels).count(), 0);
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
    fn every_reference_command_is_a_construct_checked_like_ref() {
        let (_, t) = table(&[(
            "a.xtex",
            "@ref(a) @cref(b) @Cref(c) @autoref(d) @pageref(e) @id(a)",
        )]);
        let unresolved: Vec<_> = t.unresolved_references().map(|(n, _)| n).collect();
        assert_eq!(unresolved, ["b", "c", "d", "e"]);
    }

    #[test]
    fn a_cleveref_reference_may_name_several_identifiers() {
        // `\cref{a,b}` is cleveref's own form, compiled to check. Each name
        // is one reference, so one absent name is one diagnostic naming it.
        let (_, t) = table(&[("a.xtex", "@id(a) @cref(a, b) @Cref(b,c)")]);
        let unresolved: Vec<_> = t.unresolved_references().map(|(n, _)| n).collect();
        assert_eq!(unresolved, ["b", "b", "c"]);
        assert_eq!(t.reference_count("a"), 1);
    }

    #[test]
    fn a_single_identifier_command_does_not_split_on_commas() {
        // `\autoref{a,b}` is one undefined reference in LaTeX, measured with
        // hyperref; reporting two would describe a document TeX never sees.
        let (_, t) = table(&[("a.xtex", "@id(a) @id(b) @autoref(a, b)")]);
        let unresolved: Vec<_> = t.unresolved_references().map(|(n, _)| n).collect();
        assert_eq!(unresolved, ["a, b"]);
    }

    #[test]
    fn a_prefix_demand_applies_to_every_reference_command() {
        let (_, t) = table(&[(
            "a.xtex",
            "\\table(fig:x) { caption = {X} } @cref(fig:x) @autoref(fig:x) @pageref(fig:x)",
        )]);
        assert_eq!(t.inconsistent_references().count(), 3);
    }

    fn prose_mismatch_names(text: &str) -> Vec<String> {
        let (_, t) = table(&[("a.xtex", text)]);
        t.prose_mismatches()
            .map(|(name, _, _, _)| name.to_owned())
            .collect()
    }

    #[test]
    fn a_prose_word_naming_another_class_than_the_target_is_a_mismatch() {
        let table_decl = "\\table(tab:main) { caption = {T} }";
        assert_eq!(
            prose_mismatch_names(&format!("{table_decl} Figure~@ref(tab:main)")),
            ["tab:main"]
        );
        // Nothing, one space, or tildes between the word and the construct.
        for form in [
            "Figure@ref(tab:main)",
            "Figure @ref(tab:main)",
            "Figure~~@ref(tab:main)",
        ] {
            assert_eq!(
                prose_mismatch_names(&format!("{table_decl} {form}")),
                ["tab:main"],
                "{form}"
            );
        }
    }

    #[test]
    fn a_prose_word_matching_the_target_is_silent() {
        assert!(
            prose_mismatch_names("\\table(tab:main) { caption = {T} } Table~@ref(tab:main)")
                .is_empty()
        );
    }

    #[test]
    fn plurals_and_abbreviations_name_the_same_class() {
        let decl = "\\table(tab:a) { caption = {A} } \\table(tab:b) { caption = {B} }";
        assert_eq!(
            prose_mismatch_names(&format!("{decl} Figures~@cref(tab:a, tab:b)")),
            ["tab:a", "tab:b"]
        );
        assert_eq!(
            prose_mismatch_names(&format!("{decl} Fig.~@ref(tab:a)")),
            ["tab:a"]
        );
        assert_eq!(
            prose_mismatch_names(&format!("{decl} Figs.~@Cref(tab:a,tab:b)")),
            ["tab:a", "tab:b"]
        );
        assert!(
            prose_mismatch_names(&format!(
                "{decl} Tab.~@ref(tab:a) Tables~@cref(tab:a, tab:b)"
            ))
            .is_empty()
        );
    }

    #[test]
    fn an_unknown_open_target_never_fires() {
        // A `\label` supplies no class, and neither does an `@id` on
        // unmodelled LaTeX; both sides must be known.
        let (_, t) = table(&[(
            "a.xtex",
            "\\newtheorem{X}{Open}\n@id(tab:x) Figure~@ref(tab:x)",
        )]);
        assert_eq!(
            t.declaration("tab:x").map(|d| d.class),
            Some(EntityClass::UnknownOpen),
            "the control must be a declared, unknown-open target"
        );
        assert_eq!(t.prose_mismatches().count(), 0);
        assert!(prose_mismatch_names("\\label{tab:x} Figure~@ref(tab:x)").is_empty());
    }

    #[test]
    fn no_word_or_a_word_that_is_not_immediately_before_is_silent() {
        let decl = "\\table(tab:x) { caption = {T} }";
        for text in [
            "see @ref(tab:x)",
            "Figure 3 and @ref(tab:x)",
            "Figure~\n@ref(tab:x)",
            "figure~@ref(tab:x)",
            "Subfigure~@ref(tab:x)",
            "% Figure\n@ref(tab:x)",
            "\\emph{Figure}~@ref(tab:x)",
            "@cref(tab:x)",
        ] {
            assert!(
                prose_mismatch_names(&format!("{decl} {text}")).is_empty(),
                "{text}"
            );
        }
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
    fn an_id_anywhere_inside_a_float_takes_the_floats_class() {
        // The label of a real float follows its caption, its graphics
        // lines, or closes the environment; only the header slot was read,
        // so every one of these was unknown-open (issue #161).
        for (text, class) in [
            (
                "\\begin{figure}\n\\centering\n\\includegraphics{y.png}\n\\caption{A}@id(x)\n\\end{figure}",
                EntityClass::Figure,
            ),
            (
                "\\begin{table*}\n\\caption{A}\n\\begin{tabular}{cc}\na & b @id(x)\n\\end{tabular}\n\\end{table*}",
                EntityClass::Table,
            ),
            (
                "\\begin{algorithm}[H]\n\\caption{A}\n\\begin{algorithmic}\n\\State x\n\\end{algorithmic}\n@id(x)\n\\end{algorithm}",
                EntityClass::Algorithm,
            ),
            (
                "\\begin{figure*}\n\\begin{minipage}{0.5\\linewidth}\n\\begin{subfigure}{\\linewidth}\n\\caption{L}@id(x)\n\\end{subfigure}\n\\end{minipage}\n\\end{figure*}",
                EntityClass::Figure,
            ),
            // The header slot and a display body keep their precedence.
            (
                "\\begin{table}@id(x)\n\\caption{A}\n\\end{table}",
                EntityClass::Table,
            ),
            (
                "\\begin{table}\n\\caption{A}\n\\begin{equation} y @id(x) \\end{equation}\n\\end{table}",
                EntityClass::Equation,
            ),
        ] {
            let (_, t) = table(&[("a.xtex", text)]);
            assert_eq!(t.declaration("x").map(|d| d.class), Some(class), "{text}");
        }
        // Prose between two floats, a float whose `\end` is never read, and
        // a `\begin{figure}` in a comment or a listing class nothing.
        for text in [
            "\\begin{figure}\\caption{A}\\end{figure}\n@id(x)\n\\begin{table}\\caption{B}\\end{table}",
            "\\begin{figure}\n\\caption{A}@id(x)\n",
            "% \\begin{figure}\n\\caption{A}@id(x)",
            "\\begin{lstlisting}\n\\begin{figure}\n\\end{lstlisting}\n@id(x)",
        ] {
            let (_, t) = table(&[("a.xtex", text)]);
            assert_eq!(
                t.declaration("x").map(|d| d.class),
                Some(EntityClass::UnknownOpen),
                "{text}"
            );
        }
        // Inside a listing the construct is not live text at all.
        let (_, t) = table(&[(
            "a.xtex",
            "\\begin{figure}\n\\begin{lstlisting}\n@id(x)\n\\end{lstlisting}\n\\end{figure}",
        )]);
        assert!(t.declaration("x").is_none());
    }

    #[test]
    fn the_rotated_wrapped_and_long_floats_and_a_caption_by_hand_class_an_id() {
        // Five census identifiers stayed unknown-open with only `figure`,
        // `table` and `algorithm` listed: their float is one a package
        // adds, or their caption is `\captionof` (issue #163).
        for (text, class) in [
            (
                "\\begin{sidewaysfigure}\n\\includegraphics{y.png}\n\\caption{A}@id(x)\n\\end{sidewaysfigure}",
                EntityClass::Figure,
            ),
            (
                "\\begin{sidewaysfigure*}\n\\caption{A}\n@id(x)\n\\end{sidewaysfigure*}",
                EntityClass::Figure,
            ),
            (
                "\\begin{sidewaystable}\n\\caption{A}@id(x)\n\\begin{tabular}{cc}\na & b\n\\end{tabular}\n\\end{sidewaystable}",
                EntityClass::Table,
            ),
            (
                "\\begin{sidewaystable*}\n\\caption{A}@id(x)\n\\end{sidewaystable*}",
                EntityClass::Table,
            ),
            (
                "\\begin{wrapfigure}{r}{0.5\\textwidth}\n\\includegraphics{y.png}\n\\caption{A}@id(x)\n\\end{wrapfigure}",
                EntityClass::Figure,
            ),
            (
                "\\begin{wraptable}{l}{0.4\\textwidth}\n\\caption{A}@id(x)\n\\end{wraptable}",
                EntityClass::Table,
            ),
            (
                "\\begin{longtable}{cc}\n\\caption{A}@id(x) \\\\\na & b \\\\\n\\end{longtable}",
                EntityClass::Table,
            ),
            (
                "\\begin{longtable*}{cc}\n\\caption{A}@id(x) \\\\\n\\end{longtable*}",
                EntityClass::Table,
            ),
            // The caption written by hand attaches as a section title does:
            // right after it, or on the next line.
            (
                "\\begin{minipage}{\\linewidth}\n\\includegraphics{y.png}\n\\captionof{figure}{A}@id(x)\n\\end{minipage}",
                EntityClass::Figure,
            ),
            (
                "\\begin{center}\n\\captionof{table}{A}\n@id(x)\n\\end{center}",
                EntityClass::Table,
            ),
            // Its kind is the class even inside a float of the other kind,
            // as the header rules keep their precedence over the body.
            (
                "\\begin{figure}\n\\captionof{table}{A}@id(x)\n\\end{figure}",
                EntityClass::Table,
            ),
            // The header slot of a package float.
            (
                "\\begin{sidewaystable}@id(x)\n\\caption{A}\n\\end{sidewaystable}",
                EntityClass::Table,
            ),
        ] {
            let (_, t) = table(&[("a.xtex", text)]);
            assert_eq!(t.declaration("x").map(|d| d.class), Some(class), "{text}");
        }
        // Prose between two package floats, and a `\captionof` two blank
        // lines back, class nothing.
        for text in [
            "\\begin{wrapfigure}{r}{1in}\\caption{A}\\end{wrapfigure}\n@id(x)\n\\begin{longtable}{c}\\caption{B}\\end{longtable}",
            "\\captionof{figure}{A}\n\n\n@id(x)",
        ] {
            let (_, t) = table(&[("a.xtex", text)]);
            assert_eq!(
                t.declaration("x").map(|d| d.class),
                Some(EntityClass::UnknownOpen),
                "{text}"
            );
        }
    }

    #[test]
    fn an_argument_with_a_braced_group_inside_still_attaches() {
        // The last `{` in the prefix was taken as the argument's, so a title
        // or caption holding `\textbf{…}` or `\cite{…}` never attached.
        for (text, class) in [
            ("\\section{A \\textbf{b}}@id(x)", EntityClass::Section),
            (
                "\\subsection{Growth as $n^{2}$ and \\emph{more}}\n@id(x)",
                EntityClass::Section,
            ),
            (
                "\\captionof{figure}{A \\cite{k} figure}@id(x)",
                EntityClass::Figure,
            ),
            // An escaped brace is text, not a group, and a brace inside a
            // comment is not one either.
            ("\\section{A \\} b}@id(x)", EntityClass::Section),
            ("\\section{A % }\nB}@id(x)", EntityClass::Section),
            ("\\section{A \\% b}@id(x)", EntityClass::Section),
        ] {
            let (_, t) = table(&[("a.xtex", text)]);
            assert_eq!(t.declaration("x").map(|d| d.class), Some(class), "{text}");
        }
        // Braces that do not balance attach nothing. (An unclosed group
        // runs to the line's end and takes the `@id` with it before this
        // rule is reached, so the unbalanced case here is a stray `}`.)
        for text in [
            "\\section{A}}@id(x)",
            "\\captionof{figure}{A} b}@id(x)",
            "\\section{A} % }\n@id(x)",
        ] {
            let (_, t) = table(&[("a.xtex", text)]);
            assert_eq!(
                t.declaration("x").map(|d| d.class),
                Some(EntityClass::UnknownOpen),
                "{text}"
            );
        }
    }

    #[test]
    fn a_section_after_the_appendix_switch_is_an_appendix() {
        let text = "\\section{A}@id(sec:a) @ref(app:a)\n\\appendix\n\\section{B}@id(app:b)\n\\subsection{C}@id(app:c)\n@ref(app:b) @ref(app:c) @ref(sec:a)";
        let (_, t) = table(&[("a.xtex", text)]);
        assert_eq!(t.declaration("sec:a").unwrap().class, EntityClass::Section);
        assert_eq!(t.declaration("app:b").unwrap().class, EntityClass::Appendix);
        assert_eq!(t.declaration("app:c").unwrap().class, EntityClass::Appendix);
        assert_eq!(t.inconsistent_references().count(), 0);

        // Before the switch a section is a section; `app:` on it is
        // consistent all the same, because the two are one family.
        let (_, t) = table(&[("a.xtex", "\\section{A}@id(app:a) @ref(app:a)\n\\appendix")]);
        assert_eq!(t.declaration("app:a").unwrap().class, EntityClass::Section);
        assert_eq!(t.inconsistent_references().count(), 0);

        // A commented-out switch switches nothing.
        let (_, t) = table(&[("a.xtex", "% \\appendix\n\\section{A}@id(app:a) @ref(app:a)")]);
        assert_eq!(t.declaration("app:a").unwrap().class, EntityClass::Section);
        assert_eq!(appendix_switch_at(b"\\appendixname"), None);
    }

    #[test]
    fn section_and_appendix_are_one_family_for_prefix_and_prose() {
        let text = "\\section{M}@id(sec:m) Section~@ref(sec:m) Appendix~@ref(sec:m)\n\\appendix\n\
                    \\section{E}@id(sec:e) Section~@ref(sec:e) @ref(app:e2)\n\\section{F}@id(app:f) Appendix~@ref(app:f) Section~@ref(app:f)";
        let (_, t) = table(&[("a.xtex", text)]);
        assert_eq!(t.declaration("sec:e").unwrap().class, EntityClass::Appendix);
        assert_eq!(
            t.inconsistent_references().count(),
            0,
            "sec: on an appendix is consistent"
        );
        assert_eq!(t.prose_mismatches().count(), 0);
        // The family does not reach the other classes: both references
        // carry the figure prefix, one carries the word.
        let (_, t) = table(&[(
            "a.xtex",
            "\\appendix\n\\section{E}@id(fig:e) @ref(fig:e) Figure~@ref(fig:e)",
        )]);
        assert_eq!(t.inconsistent_references().count(), 2);
        assert_eq!(t.prose_mismatches().count(), 1);
    }

    #[test]
    fn a_file_imported_after_the_switch_begins_in_the_appendices() {
        let mut sources = Sources::new();
        let mut t = SymbolTable::new();
        let id = sources.add("app.xtex", b"\\section{B}@id(app:b) @ref(app:b)".to_vec());
        let document = parse(&sources, id);
        t.merge_from(&sources, &document, true);
        assert_eq!(t.declaration("app:b").unwrap().class, EntityClass::Appendix);
        assert_eq!(t.inconsistent_references().count(), 0);
    }

    #[test]
    fn an_id_inside_a_display_body_declares_an_equation() {
        for text in [
            "\\begin{equation}\n x = 1 @id(eq:x)\n\\end{equation}",
            "\\begin{align}\n y &= 2 \\\\\n z &= 3 @id(eq:x)\n\\end{align}",
            "\\begin{align*} y = 2 @id(eq:x) \\end{align*}",
            "\\[ z = 3 @id(eq:x) \\]",
        ] {
            let (_, t) = table(&[("a.xtex", text)]);
            assert_eq!(
                t.declaration("eq:x").map(|d| d.class),
                Some(EntityClass::Equation),
                "{text}"
            );
        }
        // A closed display before the construct does not enclose it.
        let (_, t) = table(&[(
            "a.xtex",
            "\\begin{equation} x \\end{equation}\n\\newtheorem{X}{Y}\n@id(eq:x)",
        )]);
        assert_eq!(
            t.declaration("eq:x").unwrap().class,
            EntityClass::UnknownOpen
        );
    }

    #[test]
    fn a_tabular_line_break_with_spacing_opens_no_display() {
        // `\\[2pt]` is `\\` followed by an optional argument; the `\[`
        // inside it is not the display opener. A section after it is a
        // section, and a genuine display after it still declares an equation.
        let text = "\\begin{tabular}{cc}\na & b \\\\[2pt]\nc & d\n\\end{tabular}\n\
                    \\section{M}@id(sec:m) Section~@ref(sec:m)\n\
                    \\[ x = 1 @id(eq:x) \\] @ref(eq:x)\n\
                    a \\\\[4pt] b \\section{N}@id(sec:n)";
        let (_, t) = table(&[("a.xtex", text)]);
        assert_eq!(t.declaration("sec:m").unwrap().class, EntityClass::Section);
        assert_eq!(t.declaration("eq:x").unwrap().class, EntityClass::Equation);
        assert_eq!(t.declaration("sec:n").unwrap().class, EntityClass::Section);
        assert_eq!(t.inconsistent_references().count(), 0);
        assert_eq!(t.prose_mismatches().count(), 0);
    }

    #[test]
    fn an_id_inside_a_caption_is_declared() {
        // Recognised because a caption is prose, and classed by the float
        // around the caption. Declared is what keeps a reference to it
        // from being a false XT1003.
        let (_, t) = table(&[(
            "a.xtex",
            "\\begin{figure}\\caption{A figure @id(fig:c)}\\end{figure} @ref(fig:c)",
        )]);
        assert_eq!(
            t.declaration("fig:c").map(|d| d.class),
            Some(EntityClass::Figure)
        );
        assert_eq!(t.unresolved_references().count(), 0);
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
