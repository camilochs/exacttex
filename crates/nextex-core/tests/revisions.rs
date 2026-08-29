//! Revision views and identity behavior against the handwritten artifacts.

use std::fs;
use std::path::Path;

use nextex_core::review::{
    Resolution, parse_sidecar, prune_sidecar, resolve, resolve_sidecar, validate,
};
use nextex_core::source::Sources;
use nextex_core::{RevisionView, emit_view, parse};

fn fixture(name: &str, file: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("tests/fixtures/revisions")
            .join(name)
            .join(file),
    )
    .unwrap()
}

#[test]
fn handwritten_revision_views_are_derived_exactly() {
    for name in [
        "01-inline-between-words-and-before-punctuation",
        "03-nested-constructs-inside-a-revision",
        "04-construct-inside-a-command-argument-in-a-revision",
        "05-substitution-with-arrows-at-depth",
        "08-properly-nested-revisions",
    ] {
        let input = fixture(name, "input.ntex");
        let mut sources = Sources::new();
        let id = sources.add("paper.ntex", input);
        let document = parse(&sources, id);
        for (view, expected) in [
            (RevisionView::Original, "original.tex"),
            (RevisionView::Final, "final.tex"),
        ] {
            let mut output = Vec::new();
            emit_view(&sources, &document, view, &mut output).unwrap();
            assert_eq!(output, fixture(name, expected), "{name} {view:?}");
        }
    }
}

#[test]
fn resolving_changes_preserves_the_bytes_around_them() {
    let input = b"before @sub(change:x) {old -> new} after";
    assert_eq!(
        resolve(input, "change:x", Resolution::Accept).unwrap().0,
        b"before new after"
    );
    assert_eq!(
        resolve(input, "change:x", Resolution::Reject).unwrap().0,
        b"before old after"
    );
}

#[test]
fn sidecar_identity_is_asymmetric() {
    let source = b"@add(change:x) {new}";
    let sidecar = parse_sidecar(b"version = 1\ndocument = \"paper.ntex\"\n").unwrap();
    assert_eq!(validate(source, &sidecar).unwrap().len(), 1);
    let orphan = parse_sidecar(b"version = 1\ndocument = \"paper.ntex\"\n[[revision]]\nid=\"gone\"\nkind=\"add\"\nauthor=\"r\"\nat=\"2026-08-29T00:00:00Z\"\n").unwrap();
    assert_eq!(validate(source, &orphan).unwrap_err().code, "NT1012");
}

#[test]
fn rejection_moves_attribution_and_removed_text_to_history() {
    let sidecar = b"version = 1\ndocument = \"paper.ntex\"\n\n[[revision]]\nid = \"change:x\"\nkind = \"add\"\nauthor = \"r\"\nat = \"2026-08-28T00:00:00Z\"\n";
    let history = resolve_sidecar(
        sidecar,
        "change:x",
        Resolution::Reject,
        "editor",
        "2026-08-29T00:00:00Z",
        b"discarded",
    )
    .unwrap();
    let text = String::from_utf8(history).unwrap();
    assert!(!text.contains("[[revision]]"));
    assert!(text.contains("resolution = \"rejected\""));
    assert!(text.contains("removed = \"discarded\""));
}

#[test]
fn marked_packages_follow_the_document_class() {
    let mut sources = Sources::new();
    let id = sources.add(
        "paper.ntex",
        b"\\documentclass{article}\n\\begin{document}@add(c) {new}\\end{document}".as_slice(),
    );
    let document = parse(&sources, id);
    let mut output = Vec::new();
    emit_view(&sources, &document, RevisionView::Marked, &mut output).unwrap();
    assert!(output.starts_with(
        b"\\documentclass{article}\n\\usepackage{xcolor}\n\\usepackage[normalem]{ulem}\n"
    ));
}

#[test]
fn pruning_moves_only_orphan_records_to_history() {
    let sidecar = b"version=1\ndocument=\"paper.ntex\"\n[[revision]]\nid=\"gone\"\nkind=\"add\"\nauthor=\"r\"\nat=\"2026-08-28T00:00:00Z\"\n";
    let output = prune_sidecar(sidecar, b"plain", "editor", "2026-08-29T00:00:00Z").unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(!text.contains("[[revision]]"));
    assert!(text.contains("resolution = \"pruned\""));
}

#[test]
fn marked_imports_target_the_distinct_marked_artifact() {
    let mut sources = Sources::new();
    let id = sources.add("paper.ntex", b"@import(\"part.ntex\")".as_slice());
    let document = parse(&sources, id);
    let mut output = Vec::new();
    emit_view(&sources, &document, RevisionView::Marked, &mut output).unwrap();
    assert!(output.ends_with(b"\\input{part.marked.tex}"));
}
