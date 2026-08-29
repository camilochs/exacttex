//! The `\label` commands the author already wrote.
//!
//! An author does not migrate a document in one sitting. They rename a `.tex`,
//! annotate a figure, and reference it — and the next figure is still carrying
//! a plain `\label`. If `@ref` only saw `@id`, referencing that second figure
//! would be a hard error on a document LaTeX resolves without complaint, and
//! partial annotation — the whole on-ramp — would not work.
//!
//! So `@ref(x)` resolves against this inventory as well. What it does *not*
//! get is a class: a `\label` says a name exists and nothing about what it
//! names, so the entity is `?O` and a class comparison against it stays
//! silent rather than guessing from the prefix.
//!
//! # Complete or unavailable, never partial
//!
//! The same rule as the bibliography, for the same reason. A partial inventory
//! looks complete and turns every name it missed into a false "not declared".
//! So a document whose recognition stopped has no inventory at all, and
//! nothing is reported absent from it.

use std::collections::BTreeMap;

use crate::document::{Document, Node, ParseConfidence};

use crate::source::{SourceId, Sources, Span};

/// Why an inventory could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Unavailable {
    /// Recognition stopped before the end of the file.
    ///
    /// A `\label` after that point cannot be seen, so the ones before it are a
    /// subset rather than a set.
    Quarantined,
    /// A listing header's `label` option could not be read literally.
    ///
    /// A computed value, or an option list that never closes. What could not
    /// be read might declare a label, so nothing may be called absent.
    UnreadableLabelOption,
    /// A literal LaTeX project edge did not resolve or could not be read.
    UnreadableEdge,
}

/// What a document's own `\label` commands amount to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inventory {
    /// Every `\label` in the document, with where it is.
    Complete(BTreeMap<String, Span>),
    /// Nothing may be called absent.
    Unavailable(Unavailable),
}

impl Inventory {
    /// Where `name` is labelled, when the inventory is known to be complete.
    #[must_use]
    pub fn declaration(&self, name: &str) -> Option<Span> {
        match self {
            Self::Complete(labels) => labels.get(name).copied(),
            Self::Unavailable(_) => None,
        }
    }

    /// Whether `name` is labelled.
    ///
    /// `false` under [`Inventory::Unavailable`], which is what keeps a
    /// document that went dark from reporting every name as missing.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.declaration(name).is_some()
    }
}

/// Reads the `\label` commands `document` declares.
#[must_use]
pub fn inventory(sources: &Sources, document: &Document, id: SourceId) -> Inventory {
    // Recognition stopping is what makes the set a subset. `grammar.md` §4
    // names this as the condition, and it is the same shape as an unreadable
    // `.bib` making every citation key unjudgeable.
    if document.iter().any(|node| {
        matches!(node, Node::Opaque { confidence, .. } if *confidence == ParseConfidence::OpaqueToEof)
    }) {
        return Inventory::Unavailable(Unavailable::Quarantined);
    }

    let mut labels = BTreeMap::new();
    let Some(source) = sources.get(id) else {
        return Inventory::Complete(labels);
    };
    let bytes = source.bytes();

    // Only prose: a `\label` inside a comment, a verbatim block, a macro body
    // or an inactive branch is not a declaration, and the scanner has already
    // separated those out.
    for span in crate::scanner::readable_content(bytes) {
        collect(&bytes[span.start()..span.end()], span.start(), &mut labels);
    }
    // A listing may declare its label as a package option in its header,
    // with no `\label` anywhere. Grammar §4; decided in issue #82.
    match crate::scanner::listing_header_labels(bytes) {
        Ok(found) => {
            for (name, span) in found {
                labels.entry(name).or_insert(span);
            }
        }
        Err(_) => return Inventory::Unavailable(Unavailable::UnreadableLabelOption),
    }
    Inventory::Complete(labels)
}

/// Finds `\label{name}` and `\label[opt]{name}` in one region.
fn collect(region: &[u8], base: usize, labels: &mut BTreeMap<String, Span>) {
    let keyword = b"\\label";
    let mut at = 0usize;
    while let Some(hit) = find(region, at, keyword) {
        let mut cursor = hit + keyword.len();
        // `\label` is `o m`: an optional argument may precede the name.
        if region.get(cursor) == Some(&b'[') {
            let Some(close) = region[cursor..].iter().position(|byte| *byte == b']') else {
                at = cursor;
                continue;
            };
            cursor += close + 1;
        }
        if region.get(cursor) != Some(&b'{') {
            at = cursor;
            continue;
        }
        let open = cursor + 1;
        let Some(close) = region[open..].iter().position(|byte| *byte == b'}') else {
            at = open;
            continue;
        };
        let end = open + close;
        if let Ok(name) = std::str::from_utf8(&region[open..end]) {
            let name = name.trim();
            if !name.is_empty() {
                labels.entry(name.to_owned()).or_insert_with(|| {
                    Span::new(
                        u32::try_from(base + open).unwrap_or(u32::MAX),
                        u32::try_from(base + end).unwrap_or(u32::MAX),
                    )
                });
            }
        }
        at = end;
    }
}

fn find(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| from + at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn built(text: &str) -> Inventory {
        let mut sources = Sources::new();
        let id = sources.add("a.xtex", text.as_bytes().to_vec());
        let document = parse(&sources, id);
        inventory(&sources, &document, id)
    }

    #[test]
    fn a_listing_header_label_is_a_declaration() {
        // The Phase 0a paper's exact shape: options across three lines with a
        // `%` comment inside, and the braced label form. No `\label` exists
        // anywhere in this input.
        let text = "\\begin{lstlisting}[language=Python, style=pythonstyle, % <-- estilo\n    caption=Python function.,\n    label={lst:runs_decomposition}]\nbody\n\\end{lstlisting}";
        assert!(built(text).contains("lst:runs_decomposition"));

        // The unbraced form occurs in the wild too.
        assert!(
            built("\\begin{lstlisting}[label=lst:plain]\nx\n\\end{lstlisting}")
                .contains("lst:plain")
        );
    }

    #[test]
    fn a_computed_label_option_makes_the_inventory_unavailable() {
        // What cannot be read might declare a label, so nothing may be called
        // absent. The same all-or-nothing rule as the bibliography.
        let found = built("\\begin{lstlisting}[label=\\jobname]\nx\n\\end{lstlisting}");
        assert_eq!(
            found,
            Inventory::Unavailable(Unavailable::UnreadableLabelOption)
        );
    }

    #[test]
    fn a_label_in_a_listing_body_or_inside_another_value_declares_nothing() {
        // The body is raw bytes; `label=` there is code being displayed. And
        // `label` as a substring of another option's value is prose about the
        // syntax, not a use of it. Both fixtures contain exactly the bytes a
        // wrong implementation would read.
        let body = built("\\begin{lstlisting}\nlabel={lst:in-body}\n\\end{lstlisting}");
        assert!(!body.contains("lst:in-body"));

        let value = built(
            "\\begin{lstlisting}[caption={the label={lst:in-value} syntax}]\nx\n\\end{lstlisting}",
        );
        assert!(!value.contains("lst:in-value"));
    }

    #[test]
    fn a_lstlisting_inside_a_comment_or_verbatim_body_is_not_a_header() {
        // Only regions the scanner itself opened are read. Both inputs carry
        // a well-formed header a scan of raw bytes would find.
        let comment = built("% \\begin{lstlisting}[label={lst:commented}]\ntext\n");
        assert!(!comment.contains("lst:commented"));

        let nested =
            built("\\begin{verbatim}\n\\begin{lstlisting}[label={lst:shown}]\n\\end{verbatim}\n");
        assert!(!nested.contains("lst:shown"));
    }

    #[test]
    fn an_ordinary_label_is_found() {
        assert!(built("\\label{fig:x}").contains("fig:x"));
    }

    #[test]
    fn the_optional_argument_is_skipped() {
        assert!(built("\\label[eq]{eq:one}").contains("eq:one"));
    }

    #[test]
    fn a_label_in_a_comment_declares_nothing() {
        let found = built("% \\label{fig:hidden}\n\\label{fig:real}");
        assert!(found.contains("fig:real"));
        assert!(!found.contains("fig:hidden"));
    }

    #[test]
    fn a_label_in_a_macro_body_declares_nothing() {
        // The body is opaque, so what is written there is not a declaration —
        // the same reason `checking.md` forbids scanning opaque regions.
        let found = built("\\newcommand{\\mine}[1]{\\label{fig:inside}}\n\\label{fig:real}");
        assert!(found.contains("fig:real"));
        assert!(!found.contains("fig:inside"));
    }

    #[test]
    fn a_label_in_verbatim_declares_nothing() {
        let found =
            built("\\begin{verbatim}\n\\label{fig:shown}\n\\end{verbatim}\n\\label{fig:real}");
        assert!(found.contains("fig:real"));
        assert!(!found.contains("fig:shown"));
    }

    #[test]
    fn a_label_inside_a_caption_is_a_declaration() {
        // The case this whole module exists for. A command's argument is
        // excluded from *construct recognition*, but it is still content, and
        // a `\\label` there is as real as one in prose. Treating it like a
        // macro body reported a false "not declared" on a document LaTeX
        // resolves without complaint.
        assert!(built("\\caption{\\label{fig:x} A caption}").contains("fig:x"));
    }

    #[test]
    fn a_document_that_went_dark_has_no_inventory_at_all() {
        // The labels before the quarantine are a subset, and a subset that
        // looks complete turns every name after it into a false "not declared".
        let found = built("\\label{fig:before}\n\\catcode`\\@=11\n\\label{fig:after}");
        assert!(matches!(found, Inventory::Unavailable(_)), "{found:?}");
        assert!(!found.contains("fig:before"));
    }
}
