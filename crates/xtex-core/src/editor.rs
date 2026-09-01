//! What an editor asks about a position, answered without a protocol.
//!
//! An editor asks three questions: what is under the cursor, what may be
//! written here, and where is this name declared. None of them is about
//! JSON-RPC, and none of them needs a running server to test.
//!
//! Keeping them here rather than in `xtex-lsp` is what makes the phase's exit
//! criterion provable instead of argued. "The language server and the CLI
//! report the same thing for the same input" is true by construction when both
//! call this module, and merely testable by comparison when they do not. See
//! `docs/decisions/0005`.

use crate::document::{Document, Node};
use crate::scanner::EntryToken;
use crate::source::{SourceId, Sources, Span};
use crate::symbols::{EntityClass, SymbolTable};
use std::fmt::Write as _;

/// A one-based line and byte column, which is what editors speak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// One-based line.
    pub line: u32,
    /// One-based byte column.
    pub column: u32,
}

/// A construct found under a cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    /// Which construct it is.
    pub kind: EntryToken,
    /// The whole construct, which is what an editor highlights.
    pub span: Span,
    /// The identifier or key it carries.
    pub name: String,
}

/// What to show when the cursor rests on a construct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hover {
    /// The construct being described.
    pub span: Span,
    /// Plain text, one fact per line.
    pub text: String,
}

/// Something an editor may offer inside a construct's parentheses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// The text to insert.
    pub label: String,
    /// What it names, so an editor can group or icon it.
    pub class: EntityClass,
    /// Where it is declared, for a one-line preview.
    pub detail: Option<String>,
}

/// Byte offset of a one-based line and column.
///
/// Returns `None` past the end of the file, which is what an editor sends
/// while a keystroke is in flight.
#[must_use]
pub fn offset_at(bytes: &[u8], position: Position) -> Option<usize> {
    if position.line == 0 || position.column == 0 {
        return None;
    }
    let mut at = 0usize;
    for _ in 1..position.line {
        at += bytes[at..].iter().position(|byte| *byte == b'\n')? + 1;
    }
    let end = bytes[at..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(bytes.len(), |line| at + line);
    let offset = at + (position.column as usize - 1);
    (offset <= end).then_some(offset)
}

/// The construct covering `offset`, innermost first.
///
/// Innermost, because a `@cite` inside a `\figure` caption is what the cursor
/// is on; the block around it is context, not the answer.
#[must_use]
pub fn construct_at(sources: &Sources, document: &Document, offset: usize) -> Option<Located> {
    let mut best: Option<Located> = None;
    // `walk` descends into a block's fields, which is where a citation inside
    // a caption lives. Iterating the top level only would answer "the table"
    // for a cursor sitting on the `@cite` inside it.
    document.walk(|node| {
        let Node::Construct { kind, span, .. } = node else {
            return;
        };
        if offset < span.start() || offset >= span.end() {
            return;
        }
        let narrower = best
            .as_ref()
            .is_none_or(|found| span.end() - span.start() < found.span.end() - found.span.start());
        if narrower {
            best = Some(Located {
                kind: *kind,
                span: *span,
                name: name_under(sources, node.source(), *span, *kind, offset).unwrap_or_default(),
            });
        }
    });
    best
}

/// The name the cursor is on.
///
/// A citation or a `cref` names several keys, and the question an editor asks
/// — what is this, where is it declared — is about the one under the cursor.
/// On the entry token, before any key, the first key answers.
fn name_under(
    sources: &Sources,
    source: SourceId,
    span: Span,
    kind: EntryToken,
    offset: usize,
) -> Option<String> {
    if !matches!(kind, EntryToken::Ref | EntryToken::Cite) {
        return payload_text_for(sources, source, span, kind);
    }
    let bytes = sources.get(source)?.slice(span)?;
    if !crate::scanner::names_a_list(bytes) && kind != EntryToken::Cite {
        return payload_text_for(sources, source, span, kind);
    }
    let open = bytes.iter().position(|byte| *byte == b'(')? + 1;
    let close = open + bytes[open..].iter().position(|byte| *byte == b')')?;
    let relative = offset.saturating_sub(span.start());
    let keys = crate::scanner::split_keys(&bytes[open..close]);
    let (start, end) = keys
        .iter()
        .copied()
        .find(|(_, end)| relative <= open + end)
        .or_else(|| keys.first().copied())?;
    std::str::from_utf8(&bytes[open + start..open + end])
        .ok()
        .map(|key| key.trim().to_owned())
}

/// What to show for the construct at `offset`.
///
/// Every line is a fact the compiler already holds. Nothing here is computed
/// for the editor's benefit, which is why it cannot drift from what `check`
/// reports.
#[must_use]
pub fn hover(
    sources: &Sources,
    document: &Document,
    table: &SymbolTable,
    offset: usize,
) -> Option<Hover> {
    let located = construct_at(sources, document, offset)?;
    let mut text = String::new();

    // The construct's own command word — `@Cref`, `@citep` — rather than the
    // family: the hover describes what the author wrote.
    let command = sources
        .get(document.source())
        .and_then(|source| source.slice(located.span))
        .map(crate::scanner::at_command)
        .map(|command| String::from_utf8_lossy(command).into_owned())
        .unwrap_or_default();
    match located.kind {
        EntryToken::Ref => {
            let _ = writeln!(text, "@{command}({})", located.name);
            let demanded = table.demand_of(&located.name);
            if demanded != EntityClass::UnknownOpen {
                let _ = writeln!(text, "requires: {}", demanded.name());
            }
            match table.declaration(&located.name) {
                Some(declaration) => {
                    let _ = write!(text, "declared as: {}", declaration.class.name());
                }
                // An editor showing nothing here is the common case while the
                // author is still typing the name, and saying so beats an
                // empty popup that looks like a broken server.
                None => text.push_str("not declared in this document root"),
            }
        }
        EntryToken::Cite => {
            let _ = write!(text, "@{command}({})\ncitation key", located.name);
        }
        EntryToken::Id | EntryToken::Figure | EntryToken::Table => {
            let class = table
                .declaration(&located.name)
                .map_or(EntityClass::UnknownOpen, |declaration| declaration.class);
            let _ = write!(text, "{}\ndeclares: {}", located.name, class.name());
        }
        EntryToken::Import => {
            let _ = write!(text, "@import({})\nimported into this root", located.name);
        }
        _ => return None,
    }

    Some(Hover {
        span: located.span,
        text,
    })
}

/// What may be written at `offset`.
///
/// Empty unless the cursor is inside a construct that names something, because
/// offering every identifier in a document while the author writes prose is
/// worse than offering nothing.
#[must_use]
pub fn completions(
    sources: &Sources,
    document: &Document,
    table: &SymbolTable,
    offset: usize,
) -> Vec<Completion> {
    let Some(located) = construct_at(sources, document, offset) else {
        return Vec::new();
    };
    if located.kind != EntryToken::Ref {
        return Vec::new();
    }

    // The prefix already typed is the demand, so a reference that says `fig:`
    // is offered figures and nothing else. That is the type system paying for
    // itself in the editor rather than only in the exit code.
    let demanded = table.demand_of(&located.name);
    table
        .declared()
        .filter_map(|name| {
            let declaration = table.declaration(name)?;
            // The same relation the checker uses, not a second copy of it.
            let compatible = declaration.class.is_consistent_with(demanded);
            compatible.then(|| Completion {
                label: name.to_owned(),
                class: declaration.class,
                detail: Some(declaration.class.name().to_owned()),
            })
        })
        .collect()
}

/// The declared entity whose construct encloses `offset`, innermost first.
///
/// What turns "an overfull box at line 12" into "the caption of figure
/// `fig:runtime` overflows its line". Returns `None` where no declared entity
/// encloses the position, and the caller must then leave the engine's message
/// alone rather than name something it cannot support.
#[must_use]
pub fn entity_at(
    sources: &Sources,
    document: &Document,
    table: &SymbolTable,
    offset: usize,
) -> Option<(String, EntityClass)> {
    let mut best: Option<(String, EntityClass, usize)> = None;
    document.walk(|node| {
        let Node::Construct { kind, span, .. } = node else {
            return;
        };
        if !matches!(
            kind,
            EntryToken::Id | EntryToken::Figure | EntryToken::Table
        ) {
            return;
        }
        if offset < span.start() || offset >= span.end() {
            return;
        }
        let Some(name) = payload_text(sources, node.source(), *span) else {
            return;
        };
        let class = table
            .declaration(&name)
            .map_or(EntityClass::UnknownOpen, |declaration| declaration.class);
        let width = span.end() - span.start();
        if best.as_ref().is_none_or(|(_, _, best)| width < *best) {
            best = Some((name, class, width));
        }
    });
    best.map(|(name, class, _)| (name, class))
}

/// Where the name at `offset` is declared.
#[must_use]
pub fn definition(
    sources: &Sources,
    document: &Document,
    table: &SymbolTable,
    offset: usize,
) -> Option<Span> {
    definition_site(sources, document, table, offset).map(|(_, span)| span)
}

/// As [`definition`], with the source the declaration lives in.
///
/// The span alone answered a single-file editor. A project-wide table can
/// resolve a name declared in another file, and a location without its file
/// sends the editor to the wrong buffer.
#[must_use]
pub fn definition_site(
    sources: &Sources,
    document: &Document,
    table: &SymbolTable,
    offset: usize,
) -> Option<(crate::source::SourceId, Span)> {
    let located = construct_at(sources, document, offset)?;
    let declaration = table.declaration(&located.name)?;
    Some((declaration.payload.source, declaration.construct))
}

/// A citation's definition site: the key's own line in the `.bib`.
///
/// The symbol table holds constructs, and a citation's declaration is not a
/// construct — it is the `@entry{key,` line of a resource the document
/// declared. The resources consulted are exactly [`declared_in`]'s, loaded
/// through the same loader the checker uses, and interned into `sources` so
/// the ordinary definition JSON path speaks for the answer too.
///
/// `None` when the position is not a citation, or no declared resource
/// holds the key — the same silence as an unknown identifier.
///
/// [`declared_in`]: crate::bibliography::declared_in
pub fn citation_definition_site(
    sources: &mut Sources,
    loader: &impl crate::io::SourceLoader,
    document: &Document,
    id: crate::source::SourceId,
    offset: usize,
) -> Option<(crate::source::SourceId, Span)> {
    let key = match construct_at(sources, document, offset) {
        Some(located) if located.kind == EntryToken::Cite => located.name,
        // Plain `\cite{…}`: no construct and no checker guarantee, but
        // "where is this defined?" has the same answer either way. The
        // recognition is navigational only; diagnostics are unchanged.
        _ => crate::bibliography::latex_citation_key_at(sources.get(id)?.bytes(), offset)?,
    };
    let declared = crate::bibliography::declared_in(sources, id);
    for resource in &declared.resources {
        let Ok(bib) = loader.load(&resource.name, Some(id), sources) else {
            continue;
        };
        let Some(bytes) = sources.get(bib).map(crate::source::Source::bytes) else {
            continue;
        };
        if let Some(span) = crate::bibliography::entry_span_in_bib(bytes, &key) {
            return Some((bib, span));
        }
    }
    None
}

/// The text between a construct's parentheses.
fn payload_text(sources: &Sources, source: SourceId, span: Span) -> Option<String> {
    payload_text_for(sources, source, span, EntryToken::Id)
}

fn payload_text_for(
    sources: &Sources,
    source: SourceId,
    span: Span,
    kind: EntryToken,
) -> Option<String> {
    let bytes = sources.get(source)?.slice(span)?;
    let open = bytes.iter().position(|byte| *byte == b'(')? + 1;
    // An identifier cannot contain `)`, so the first close ends it — and for
    // a block, whose span covers its whole body, the last `)` in the span may
    // sit inside a caption's own construct. Scanning from the right returned
    // `fig:plot) { … caption = {… @ref(sec:intro` as a figure's name, found
    // by the blame parity fixture. Only `@import` reads to the last close,
    // because a `)` inside its quoted string is data per grammar §4.
    let close = if kind == EntryToken::Import {
        bytes.iter().rposition(|byte| *byte == b')')?
    } else {
        open + bytes[open..].iter().position(|byte| *byte == b')')?
    };
    if open > close {
        return None;
    }
    std::str::from_utf8(&bytes[open..close])
        .ok()
        .map(|text| text.trim().to_owned())
}

/// Renders a hover as JSON, one renderer for both hosts.
pub fn hover_to_json(found: &Hover, out: &mut String) {
    out.push_str("{\"text\":");
    push_json_string(&found.text, out);
    out.push('}');
}

/// Renders the whole identity inventory: every declaration, sorted by name,
/// with its class, its declaration site, and its reference count.
pub fn inventory_to_json(sources: &Sources, table: &SymbolTable, out: &mut String) {
    out.push('[');
    for (index, (name, declaration)) in table.declarations().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"name\":");
        crate::check::write_json_string(name, out);
        let _ = write!(
            out,
            ",\"class\":\"{}\",\"references\":{}",
            declaration.class.name(),
            table.reference_count(name)
        );
        crate::check::write_span(
            sources,
            declaration.payload.source,
            declaration.payload.span,
            out,
        );
        out.push('}');
    }
    out.push(']');
}

/// Renders completions as JSON, one renderer for both hosts.
pub fn completions_to_json(items: &[Completion], out: &mut String) {
    out.push('[');
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"label\":");
        push_json_string(&item.label, out);
        out.push_str(",\"class\":\"");
        out.push_str(item.class.name());
        out.push_str("\",\"detail\":");
        match &item.detail {
            Some(detail) => push_json_string(detail, out),
            None => out.push_str("null"),
        }
        out.push('}');
    }
    out.push(']');
}

/// Renders a definition site as JSON, one renderer for both hosts.
pub fn definition_to_json(sources: &Sources, source: SourceId, span: Span, out: &mut String) {
    use std::fmt::Write as _;
    let (name, line, column) = sources.get(source).map_or_else(
        || (String::new(), 0, 0),
        |s| {
            let bytes = s.bytes();
            let upto = &bytes[..span.start().min(bytes.len())];
            // The dependency the lint suggests is a dependency; this
            // repository holds none.
            #[allow(clippy::naive_bytecount)]
            let line = upto.iter().filter(|b| **b == b'\n').count() + 1;
            let column =
                span.start() - upto.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1) + 1;
            (s.name().to_owned(), line, column)
        },
    );
    out.push_str("{\"file\":");
    push_json_string(&name, out);
    let _ = write!(
        out,
        ",\"offset\":{},\"length\":{},\"line\":{line},\"column\":{column}}}",
        span.start(),
        span.len(),
    );
}

fn push_json_string(text: &str, out: &mut String) {
    use std::fmt::Write as _;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn a_blocks_name_is_its_identifier_even_when_its_caption_holds_a_construct() {
        // Found by the blame parity fixture: with the block's span covering
        // its whole body, scanning for the LAST `)` returned everything up to
        // the caption's own `@ref` as the figure's "name". The identifier
        // ends at the first close, because an identifier cannot contain one.
        let text = "\\figure(fig:plot) {\n  caption = {Con @ref(sec:x) dentro.}\n}\n";
        let mut sources = Sources::new();
        let id = sources.add("a.xtex", text.as_bytes().to_vec());
        let document = parse(&sources, id);
        let mut table = SymbolTable::new();
        table.merge(&sources, &document);
        let offset = text.find("caption").unwrap();
        let (name, _) = entity_at(&sources, &document, &table, offset).expect("an entity");
        assert_eq!(name, "fig:plot");
    }

    const DOC: &str = "\\section{Intro} @id(sec:intro)\n\
                       \\figure(fig:plot) { src = \"p.pdf\" caption = {C} }\n\
                       See @ref(fig:plot) and @ref(tab:none) and @cite(knuth1984).\n";

    fn document() -> (Sources, Document, SymbolTable) {
        let mut sources = Sources::new();
        let id = sources.add("paper.xtex", DOC.as_bytes().to_vec());
        let document = parse(&sources, id);
        let mut table = SymbolTable::new();
        table.merge(&sources, &document);
        (sources, document, table)
    }

    fn at(text: &str) -> usize {
        DOC.find(text).expect("the fixture contains it") + 1
    }

    #[test]
    fn a_position_maps_to_the_byte_the_editor_meant() {
        let bytes = DOC.as_bytes();
        assert_eq!(offset_at(bytes, Position { line: 1, column: 1 }), Some(0));
        // Column one of line two is the byte after the first line ending.
        let second = DOC.find('\n').expect("two lines") + 1;
        assert_eq!(
            offset_at(bytes, Position { line: 2, column: 1 }),
            Some(second)
        );
    }

    #[test]
    fn a_position_past_the_end_is_none_rather_than_clamped() {
        // An editor sends these while a keystroke is in flight. Clamping would
        // answer a question about a position that does not exist.
        let bytes = DOC.as_bytes();
        assert_eq!(
            offset_at(
                bytes,
                Position {
                    line: 99,
                    column: 1
                }
            ),
            None
        );
        assert_eq!(
            offset_at(
                bytes,
                Position {
                    line: 1,
                    column: 999
                }
            ),
            None
        );
        assert_eq!(offset_at(bytes, Position { line: 0, column: 1 }), None);
    }

    #[test]
    fn the_construct_under_the_cursor_is_the_innermost_one() {
        // The cursor is on the caption's own bytes, inside the figure block.
        // The block is context; the answer is what the cursor is actually on.
        let mut sources = Sources::new();
        let text = "\\table(tab:x) { caption = {See @cite(knuth1984)} }";
        let id = sources.add("a.xtex", text.as_bytes().to_vec());
        let document = parse(&sources, id);
        let offset = text.find("@cite").expect("present") + 2;
        let found = construct_at(&sources, &document, offset).expect("a construct");
        assert_eq!(found.kind, EntryToken::Cite);
        assert_eq!(found.name, "knuth1984");
    }

    #[test]
    fn hover_on_a_resolved_reference_names_both_sides() {
        let (sources, document, table) = document();
        let hover = hover(&sources, &document, &table, at("@ref(fig:plot)")).expect("hover");
        assert!(hover.text.contains("requires: figure"), "{}", hover.text);
        assert!(hover.text.contains("declared as: figure"), "{}", hover.text);
    }

    #[test]
    fn hover_on_an_unresolved_reference_says_so_rather_than_showing_nothing() {
        let (sources, document, table) = document();
        let hover = hover(&sources, &document, &table, at("@ref(tab:none)")).expect("hover");
        assert!(hover.text.contains("not declared"), "{}", hover.text);
    }

    #[test]
    fn hover_outside_a_construct_is_none() {
        let (sources, document, table) = document();
        assert!(hover(&sources, &document, &table, at("See ")).is_none());
    }

    #[test]
    fn completion_offers_only_what_the_prefix_demands() {
        // `@ref(tab:` demands a table, and the document declares a figure and a
        // section. Offering them would be offering an error.
        let (sources, document, table) = document();
        let offered: Vec<_> = completions(&sources, &document, &table, at("@ref(tab:none)"))
            .into_iter()
            .map(|item| item.label)
            .collect();
        assert!(offered.is_empty(), "{offered:?}");
    }

    #[test]
    fn completion_offers_the_matching_class() {
        let (sources, document, table) = document();
        let offered: Vec<_> = completions(&sources, &document, &table, at("@ref(fig:plot)"))
            .into_iter()
            .map(|item| item.label)
            .collect();
        assert_eq!(offered, ["fig:plot"]);
    }

    #[test]
    fn completion_in_prose_offers_nothing() {
        // Offering every identifier in the document while someone writes a
        // sentence is worse than offering nothing.
        let (sources, document, table) = document();
        assert!(completions(&sources, &document, &table, at("See ")).is_empty());
    }

    #[test]
    fn an_entity_is_found_only_where_one_encloses_the_position() {
        let text = "\\table(tab:x) { caption = {C} }\nOutside any entity.\n";
        let mut sources = Sources::new();
        let id = sources.add("a.xtex", text.as_bytes().to_vec());
        let document = parse(&sources, id);
        let mut table = SymbolTable::new();
        table.merge(&sources, &document);

        let inside = text.find("caption").expect("present");
        let (name, class) =
            entity_at(&sources, &document, &table, inside).expect("inside the block");
        assert_eq!(name, "tab:x");
        assert_eq!(class, EntityClass::Table);

        // The negative case carries the weight: a position outside every
        // declared entity must yield nothing, so a typesetting failure there
        // keeps TeX's own sentence instead of naming something it cannot
        // support.
        let outside = text.find("Outside").expect("present");
        assert!(entity_at(&sources, &document, &table, outside).is_none());
    }

    #[test]
    fn definition_points_at_the_declaring_construct() {
        let (sources, document, table) = document();
        let span = definition(&sources, &document, &table, at("@ref(fig:plot)")).expect("found");
        let declared = DOC.find("\\figure(fig:plot)").expect("present");
        assert_eq!(span.start(), declared);
    }
}
