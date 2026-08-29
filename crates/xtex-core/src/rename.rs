//! Renaming an identifier everywhere it is structurally resolved.
//!
//! This is one of the concrete things explicit names buy, and it is only worth
//! having if it is safe. A find-and-replace over `fig:plot` also rewrites the
//! word inside a `\verb`, inside a comment, and inside the caption text of a
//! paper about naming conventions. This does not.
//!
//! # What it will not do, and why it says so
//!
//! `@id(fig:plot)` emits `\label{fig:plot}`. An author may also have written a
//! plain `\ref{fig:plot}`, which is transported LaTeX and therefore `?O` —
//! unchecked, and by the rules in `AGENTS.md` §4, never rewritten.
//!
//! Renaming the construct and silently leaving that `\ref` behind would break
//! a working document. Rewriting it would break the invariant that opaque
//! bytes are never touched. So this does neither: it renames what it resolved
//! and **reports** what it found in opaque text and did not touch, and the
//! caller decides.

use crate::document::{Document, Node};
use crate::scanner::EntryToken;
use crate::source::{SourceId, Sources, Span};

/// One byte range to replace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// Source the range is in.
    pub source: SourceId,
    /// Range holding the old identifier, without its surrounding construct.
    pub span: Span,
    /// What to put there.
    pub replacement: String,
}

/// An occurrence found in bytes this compiler does not model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Untouched {
    /// Source it is in.
    pub source: SourceId,
    /// Where it is, so a caller can point at it.
    pub span: Span,
}

/// The result of planning a rename.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// Ranges to replace, ordered by source and then by position.
    pub edits: Vec<Edit>,
    /// Matching text left alone because it is opaque.
    pub untouched: Vec<Untouched>,
}

impl Plan {
    /// Whether anything would change.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }
}

/// Plans a rename of `from` to `to` across every document in a root.
///
/// Every document reached by `@import` belongs to one root and shares one
/// namespace, so a rename that visited only the open file would leave the rest
/// pointing at a name that no longer exists.
#[must_use]
pub fn plan(sources: &Sources, documents: &[Document], from: &str, to: &str) -> Plan {
    let mut plan = Plan::default();
    for document in documents {
        collect(sources, document, from, &mut plan);
    }
    // Only the sources the root actually reaches, taken from the documents
    // rather than from the arena: a source loaded for something else is not
    // part of this rename.
    let mut roots: Vec<SourceId> = documents
        .iter()
        .filter_map(|document| document.iter().next().map(Node::source))
        .collect();
    roots.sort_by_key(|id| id.index());
    roots.dedup();
    for id in roots {
        find_untouched(sources, id, from, &mut plan);
    }
    plan.edits
        .sort_by_key(|edit| (edit.source.index(), edit.span.start()));
    plan.untouched
        .sort_by_key(|found| (found.source.index(), found.span.start()));
    for edit in &mut plan.edits {
        to.clone_into(&mut edit.replacement);
    }
    plan
}

/// Applies a plan to one source's bytes.
///
/// Edits are applied from the end so that earlier offsets stay valid.
#[must_use]
pub fn apply(bytes: &[u8], source: SourceId, plan: &Plan) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let mut edits: Vec<&Edit> = plan
        .edits
        .iter()
        .filter(|edit| edit.source == source)
        .collect();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.span.start()));
    for edit in edits {
        out.splice(edit.span.start()..edit.span.end(), edit.replacement.bytes());
    }
    out
}

/// Every construct in `document` naming `from`.
fn collect(sources: &Sources, document: &Document, from: &str, plan: &mut Plan) {
    document.walk(|node| {
        let Node::Construct { kind, span, .. } = node else {
            return;
        };
        if !matches!(
            kind,
            EntryToken::Id
                | EntryToken::Ref
                | EntryToken::Figure
                | EntryToken::Table
                | EntryToken::Add
                | EntryToken::Del
                | EntryToken::Sub
                | EntryToken::Note
        ) {
            return;
        }
        // A citation names a bibliography key, not an identifier. Renaming one
        // here would rewrite a key that lives in a `.bib` this does not own.
        for candidate in name_spans(sources, node.source(), *span, *kind) {
            let Some(text) = sources
                .get(node.source())
                .and_then(|source| source.slice(candidate))
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
            else {
                continue;
            };
            if text.trim() == from {
                // Trim inside the span too, so `on = c1` replaces `c1` and
                // leaves the spacing the author wrote.
                let lead = u32::try_from(text.len() - text.trim_start().len()).unwrap_or(0);
                let trail = u32::try_from(text.len() - text.trim_end().len()).unwrap_or(0);
                let start = u32::try_from(candidate.start()).unwrap_or(u32::MAX);
                let end = u32::try_from(candidate.end()).unwrap_or(u32::MAX);
                plan.edits.push(Edit {
                    source: node.source(),
                    span: Span::new(start + lead, end - trail),
                    replacement: String::new(),
                });
            }
        }
    });
}

/// Occurrences of `from` in bytes no construct covers.
///
/// These are what the rename deliberately leaves alone: a `\label{fig:plot}`
/// the author wrote, the same word inside a `\verb`, or a sentence that
/// happens to contain it.
fn find_untouched(sources: &Sources, id: SourceId, from: &str, plan: &mut Plan) {
    let Some(source) = sources.get(id) else {
        return;
    };
    let bytes = source.bytes();
    let covered: Vec<Span> = plan
        .edits
        .iter()
        .filter(|edit| edit.source == id)
        .map(|edit| edit.span)
        .collect();
    let needle = from.as_bytes();
    let mut at = 0usize;
    while let Some(found) = bytes[at..]
        .windows(needle.len())
        .position(|window| window == needle)
    {
        let start = at + found;
        let end = start + needle.len();
        at = start + 1;
        // A longer identifier that merely contains this one is not this one.
        let bounded =
            |byte: u8| !byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b':' | b'.' | b'-');
        if start > 0 && !bounded(bytes[start - 1]) {
            continue;
        }
        if end < bytes.len() && !bounded(bytes[end]) {
            continue;
        }
        if covered
            .iter()
            .any(|span| span.start() <= start && end <= span.end())
        {
            continue;
        }
        plan.untouched.push(Untouched {
            source: id,
            span: Span::new(
                u32::try_from(start).unwrap_or(u32::MAX),
                u32::try_from(end).unwrap_or(u32::MAX),
            ),
        });
    }
}

/// Every range in a construct that holds an identifier.
///
/// Usually one. A `@note(n1, on = c1)` holds two, and both are structural: the
/// note's own name, and the revision it is about. Renaming a revision without
/// following its `on` field orphans the note, which is `XT1012` on a document
/// that was correct a moment earlier.
fn name_spans(sources: &Sources, source: SourceId, construct: Span, kind: EntryToken) -> Vec<Span> {
    let Some(bytes) = sources.get(source).and_then(|s| s.slice(construct)) else {
        return Vec::new();
    };
    let Some(open) = bytes.iter().position(|byte| *byte == b'(').map(|at| at + 1) else {
        return Vec::new();
    };
    // A block ends at its closing brace, so its name ends at the first `)`.
    // An inline construct ends at its own `)`.
    let close = match kind {
        EntryToken::Figure | EntryToken::Table => bytes.iter().position(|byte| *byte == b')'),
        _ => bytes.iter().rposition(|byte| *byte == b')'),
    };
    let Some(close) = close.filter(|close| open <= *close) else {
        return Vec::new();
    };

    let at = |from: usize, to: usize| {
        Span::new(
            u32::try_from(construct.start() + from).unwrap_or(u32::MAX),
            u32::try_from(construct.start() + to).unwrap_or(u32::MAX),
        )
    };
    let Some(comma) = bytes[open..close].iter().position(|byte| *byte == b',') else {
        return vec![at(open, close)];
    };
    let mut found = vec![at(open, open + comma)];
    // The value of `on = <id>`. Any other field names nothing renameable.
    if kind == EntryToken::Note {
        let tail = &bytes[open + comma..close];
        if let Some(equals) = tail.iter().position(|byte| *byte == b'=') {
            found.push(at(open + comma + equals + 1, close));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_construct_in_a_caption_is_a_real_occurrence_not_untouched() {
        // Issue #83: a text-bearing argument is prose, so rename must reach
        // it. Before, the caption's @ref sat in an Arguments piece and was
        // reported untouched — an editor renaming 2 of 3 places silently.
        let mut sources = Sources::new();
        let text = "\\section{S}@id(sec:s)\nProsa @ref(sec:s).\n\\caption{Pie con @ref(sec:s).}\n";
        let id = sources.add("r.xtex", text.as_bytes().to_vec());
        let document = crate::parse(&sources, id);
        let plan = plan(&sources, &[document], "sec:s", "sec:nuevo");
        assert_eq!(plan.edits.len(), 3, "the caption occurrence is an edit");
        assert!(
            plan.untouched.is_empty(),
            "nothing opaque holds this identifier: {:?}",
            plan.untouched
        );
    }
    use crate::parse;

    fn renamed(text: &str, from: &str, to: &str) -> (String, usize) {
        let mut sources = Sources::new();
        let id = sources.add("paper.xtex", text.as_bytes().to_vec());
        let document = parse(&sources, id);
        let plan = plan(&sources, std::slice::from_ref(&document), from, to);
        let out = apply(text.as_bytes(), id, &plan);
        (String::from_utf8(out).expect("utf-8"), plan.untouched.len())
    }

    #[test]
    fn a_declaration_and_every_reference_to_it_change_together() {
        let (out, _) = renamed(
            "\\figure(fig:plot) { caption = {C} }\nSee @ref(fig:plot) twice: @ref(fig:plot).",
            "fig:plot",
            "fig:runtime",
        );
        assert!(!out.contains("fig:plot"), "{out}");
        assert_eq!(out.matches("fig:runtime").count(), 3, "{out}");
    }

    #[test]
    fn a_different_identifier_that_contains_this_one_is_left_alone() {
        // `fig:plot2` contains `fig:plot`. A find-and-replace renames it too.
        let (out, _) = renamed(
            "@id(fig:plot) @id(fig:plot2) @ref(fig:plot) @ref(fig:plot2)",
            "fig:plot",
            "fig:x",
        );
        assert!(out.contains("@id(fig:plot2)"), "{out}");
        assert!(out.contains("@ref(fig:plot2)"), "{out}");
        assert_eq!(out.matches("fig:x)").count(), 2, "{out}");
    }

    #[test]
    fn opaque_text_is_never_rewritten_and_is_reported_instead() {
        // The whole point. `\label` is the author's LaTeX, the `\verb` is
        // verbatim, and the sentence is prose. None may change, and an author
        // who is not told about the first one has a document that used to work.
        let text = "@id(fig:plot)\n\
                    \\label{fig:plot}\n\
                    \\verb|fig:plot|\n\
                    We call it fig:plot in the text.";
        let (out, untouched) = renamed(text, "fig:plot", "fig:new");
        assert!(out.contains("\\label{fig:plot}"), "{out}");
        assert!(out.contains("\\verb|fig:plot|"), "{out}");
        assert!(out.contains("call it fig:plot in the text"), "{out}");
        assert_eq!(untouched, 3, "each one is reported: {out}");
    }

    #[test]
    fn a_citation_key_is_not_an_identifier_and_is_not_renamed() {
        // Its key lives in a `.bib` this does not own.
        let (out, _) = renamed("@cite(fig:plot) @id(fig:plot)", "fig:plot", "fig:new");
        assert!(out.contains("@cite(fig:plot)"), "{out}");
        assert!(out.contains("@id(fig:new)"), "{out}");
    }

    #[test]
    fn a_block_keeps_its_body_and_changes_only_its_name() {
        let (out, _) = renamed(
            "\\table(tab:x) { caption = {A caption mentioning tab:x} }",
            "tab:x",
            "tab:y",
        );
        assert!(out.starts_with("\\table(tab:y)"), "{out}");
        assert!(
            out.contains("mentioning tab:x"),
            "the caption is content: {out}"
        );
    }

    #[test]
    fn a_note_follows_the_revision_it_is_about() {
        // The `on =` field is a structural reference: XT1011 and XT1012 are
        // both about that pairing. A rename that moved the revision and left
        // the note behind would orphan it, on a document that was correct a
        // moment earlier.
        let (out, untouched) = renamed(
            "@add(c1) {new text} @note(n1, on = c1) {why}",
            "c1",
            "change:qualified",
        );
        assert_eq!(
            out,
            "@add(change:qualified) {new text} @note(n1, on = change:qualified) {why}"
        );
        assert_eq!(untouched, 0, "nothing was left behind");
    }

    #[test]
    fn a_note_keeps_its_own_name_when_the_revision_moves() {
        let (out, _) = renamed("@add(c1) {t} @note(c1x, on = c1) {why}", "c1", "c2");
        assert!(out.contains("@note(c1x, on = c2)"), "{out}");
    }

    #[test]
    fn renaming_something_that_does_not_exist_changes_nothing() {
        let (out, _) = renamed("@id(fig:plot)", "fig:absent", "fig:new");
        assert_eq!(out, "@id(fig:plot)");
    }
}
