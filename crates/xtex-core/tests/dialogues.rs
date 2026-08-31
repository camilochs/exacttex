//! Conversations in difficult places.
//!
//! A review is a dialogue: someone proposes a change, someone else replies to
//! it, and both live inside the document, sometimes in the least convenient
//! corner of it — a title page, a caption, a table cell, the argument of a
//! command inside another command. Every case here was chosen because the
//! document carries text there, so the compiler owes the same answer it gives
//! in running prose: the construct is recognised, it can be resolved, and it
//! never reaches the PDF as itself.
//!
//! Origin: 2026-08-31. A proposed deletion of an author's name was resolvable
//! in the margin and printed into the PDF as `@del(change:…)` at the same
//! time, because recognition and emission disagreed about `\author{…}`.

use xtex_core::review::{Resolution, resolve, revision_ids};
use xtex_core::source::Sources;
use xtex_core::{RevisionView, emit_view, parse};

/// The Final view of one source, which is what the PDF is built from.
fn final_view(source: &[u8]) -> String {
    let mut sources = Sources::new();
    let id = sources.add("t.xtex", source.to_vec());
    let document = parse(&sources, id);
    let mut out = Vec::new();
    emit_view(&sources, &document, RevisionView::Final, &mut out).expect("emits");
    String::from_utf8_lossy(&out).into_owned()
}

/// The Marked view, which shows the change rather than applying it.
fn marked_view(source: &[u8]) -> String {
    let mut sources = Sources::new();
    let id = sources.add("t.xtex", source.to_vec());
    let document = parse(&sources, id);
    let mut out = Vec::new();
    emit_view(&sources, &document, RevisionView::Marked, &mut out).expect("emits");
    String::from_utf8_lossy(&out).into_owned()
}

/// A dialogue in a hard place: what it is, its source, and the ids in it.
struct Dialogue {
    what: &'static str,
    source: &'static [u8],
    ids: &'static [&'static str],
}

const DIALOGUES: &[Dialogue] = &[
    Dialogue {
        what: "the title page: a deleted author, and a reply about it",
        source: b"\\author{Gabriela, @del(d:blum) {Christian} @note(n:why, on = d:blum) {moved to acknowledgements}}\n\\title{Search @sub(d:title) {Trees -> Trajectory} Networks}\n",
        ids: &["d:blum", "n:why", "d:title"],
    },
    Dialogue {
        what: "a caption, with the reply beside the change",
        source: b"\\caption{The run @sub(d:hedge) {proves -> suggests} it @note(n:hedge, on = d:hedge) {softer}}\n",
        ids: &["d:hedge", "n:hedge"],
    },
    Dialogue {
        what: "a command inside a command",
        source: b"\\textbf{\\footnote{@del(d:deep) {a whole aside} @note(n:deep, on = d:deep) {agreed}}}\n",
        ids: &["d:deep", "n:deep"],
    },
    Dialogue {
        what: "a table cell",
        source: b"\\begin{tabular}{ll}\na & @del(d:cell) {12.4} \\\\\nb & 9.1 \\\\\n\\end{tabular}\n",
        ids: &["d:cell"],
    },
    Dialogue {
        what: "a list item's own argument",
        source: b"\\begin{description}\n\\item[@sub(d:item) {Fast -> Quick}] the label is prose\n\\end{description}\n",
        ids: &["d:item"],
    },
    Dialogue {
        what: "two changes side by side, and a reply to the second",
        source: b"\\caption{@del(d:one) {first} and @del(d:two) {second} @note(n:two, on = d:two) {this one}}\n",
        ids: &["d:one", "d:two", "n:two"],
    },
    Dialogue {
        what: "a change beside a citation and a reference",
        source: b"Section~@ref(sec:x) shows @cite(knuth) and @del(d:beside) {an aside}.\n",
        ids: &["d:beside"],
    },
];

#[test]
fn every_dialogue_is_recognised_where_it_stands() {
    for case in DIALOGUES {
        let mut ids = revision_ids(case.source);
        ids.sort();
        let mut expected: Vec<String> = case.ids.iter().map(|id| (*id).to_owned()).collect();
        expected.sort();
        assert_eq!(ids, expected, "{}: recognised the wrong set", case.what);
    }
}

#[test]
fn no_dialogue_reaches_the_pdf_as_itself() {
    for case in DIALOGUES {
        for (view, text) in [
            ("Final", final_view(case.source)),
            ("Marked", marked_view(case.source)),
        ] {
            for marker in ["@del(", "@sub(", "@add(", "@note("] {
                assert!(
                    !text.contains(marker),
                    "{}: the {view} view printed {marker} into the document:\n{text}",
                    case.what
                );
            }
        }
    }
}

#[test]
fn every_change_can_be_accepted_and_rejected_where_it_stands() {
    for case in DIALOGUES {
        for id in case.ids.iter().filter(|id| !id.starts_with("n:")) {
            for resolution in [Resolution::Accept, Resolution::Reject] {
                let (rewritten, _) = resolve(case.source, id, resolution).unwrap_or_else(|error| {
                    panic!("{}: {id} could not be resolved: {error:?}", case.what)
                });
                assert!(
                    !String::from_utf8_lossy(&rewritten).contains(&format!("({id})")),
                    "{}: {id} survived its own resolution",
                    case.what
                );
            }
        }
    }
}

#[test]
fn a_resolved_change_leaves_the_document_around_it_untouched() {
    // The invariant that makes this safe to use on someone's book: resolving
    // one change rewrites that construct and nothing else.
    let source = b"\\author{Gabriela, @del(d:blum) {Christian}}\n\\title{Networks}\n";
    let (rewritten, removed) = resolve(source, "d:blum", Resolution::Accept).expect("resolves");
    assert_eq!(
        String::from_utf8_lossy(&rewritten),
        "\\author{Gabriela, }\n\\title{Networks}\n"
    );
    assert_eq!(String::from_utf8_lossy(&removed), "Christian");
}

#[test]
fn where_the_document_carries_no_text_a_dialogue_is_not_one() {
    // The other half of the rule, asserted so it cannot drift: a comment, a
    // verbatim block and a definition body are not places a change lives.
    let blind: [(&str, &[u8]); 3] = [
        ("a comment", b"% @del(d:no) {invisible}\nProse.\n"),
        (
            "verbatim",
            b"\\begin{verbatim}\n@del(d:no) {literal}\n\\end{verbatim}\n",
        ),
        (
            "a definition body",
            b"\\newcommand{\\x}{@del(d:no) {nothing has happened yet}}\n",
        ),
    ];
    for (what, source) in blind {
        assert!(
            revision_ids(source).is_empty(),
            "{what}: must not be read as a change"
        );
        assert_eq!(
            final_view(source).as_bytes(),
            source,
            "{what}: the bytes must be transported exactly"
        );
    }
}

#[test]
fn resolving_a_change_does_not_leave_its_replies_dangling() {
    // The director, 2026-08-31: "cuando se responde y después de reject la
    // root, la nota de respuesta queda ahí suelta". A reply is about a change;
    // once that change is gone, a reply pointing at nothing is debris the
    // author has to clean by hand.
    let source =
        b"\\author{Gabriela, @del(d:blum) {Christian} @note(n:why, on = d:blum) {moved}}\n";
    let (rewritten, _) = resolve(source, "d:blum", Resolution::Reject).expect("resolves");
    let text = String::from_utf8_lossy(&rewritten);
    assert!(
        !text.contains("n:why"),
        "the reply must go with the change it answered:\n{text}"
    );
}
