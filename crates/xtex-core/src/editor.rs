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
                name: payload_text_for(sources, node.source(), *span, *kind).unwrap_or_default(),
            });
        }
    });
    best
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

    match located.kind {
        EntryToken::Ref => {
            let _ = writeln!(text, "@ref({})", located.name);
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
            let _ = write!(text, "@cite({})\ncitation key", located.name);
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
    let located = construct_at(sources, document, offset)?;
    Some(table.declaration(&located.name)?.construct)
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
