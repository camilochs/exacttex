//! Revision views and identity behavior against the handwritten artifacts.

use std::fs;
use std::path::Path;

use xtex_core::review::{
    Resolution, parse_sidecar, prune_sidecar, resolve, resolve_sidecar, validate,
};
use xtex_core::source::Sources;
use xtex_core::{RevisionView, emit_view, parse};

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
        let input = fixture(name, "input.xtex");
        let mut sources = Sources::new();
        let id = sources.add("paper.xtex", input);
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
    let sidecar = parse_sidecar(b"version = 1\ndocument = \"paper.xtex\"\n").unwrap();
    assert_eq!(validate(source, &sidecar).unwrap().len(), 1);
    let orphan = parse_sidecar(b"version = 1\ndocument = \"paper.xtex\"\n[[revision]]\nid=\"gone\"\nkind=\"add\"\nauthor=\"r\"\nat=\"2026-08-29T00:00:00Z\"\n").unwrap();
    assert_eq!(validate(source, &orphan).unwrap_err().code, "XT1012");
}

#[test]
fn rejection_moves_attribution_and_removed_text_to_history() {
    let sidecar = b"version = 1\ndocument = \"paper.xtex\"\n\n[[revision]]\nid = \"change:x\"\nkind = \"add\"\nauthor = \"r\"\nat = \"2026-08-28T00:00:00Z\"\n";
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
        "paper.xtex",
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
    let sidecar = b"version=1\ndocument=\"paper.xtex\"\n[[revision]]\nid=\"gone\"\nkind=\"add\"\nauthor=\"r\"\nat=\"2026-08-28T00:00:00Z\"\n";
    let output = prune_sidecar(sidecar, b"plain", "editor", "2026-08-29T00:00:00Z").unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(!text.contains("[[revision]]"));
    assert!(text.contains("resolution = \"pruned\""));
}

#[test]
fn marked_imports_target_the_distinct_marked_artifact() {
    let mut sources = Sources::new();
    let id = sources.add("paper.xtex", b"@import(\"part.xtex\")".as_slice());
    let document = parse(&sources, id);
    let mut output = Vec::new();
    emit_view(&sources, &document, RevisionView::Marked, &mut output).unwrap();
    assert!(output.ends_with(b"\\input{part.marked.tex}"));
}

#[test]
fn a_revision_inside_a_command_argument_is_a_real_revision() {
    // The director's book, 2026-08-31: a deletion proposed on an author name
    // inside `\\author{…}` showed in the margin and could never be accepted —
    // `xtex revise` answered XT1012 "sidecar record has no construct", because
    // only the top level was scanned. Arguments are document content (see
    // `Piece::Arguments`), so a revision written there is a real revision.
    // The same reasoning fixed rename in #83.
    let text = b"\\author{@del(change:gabriela) {Gabriela Ochoa}, Camilo}\n".to_vec();

    let (rewritten, removed) =
        resolve(&text, "change:gabriela", Resolution::Accept).expect("the construct is found");
    assert_eq!(
        String::from_utf8_lossy(&rewritten),
        "\\author{, Camilo}\n",
        "accepting a deletion removes the payload where it stands"
    );
    assert_eq!(String::from_utf8_lossy(&removed), "Gabriela Ochoa");
}

#[test]
fn revisions_are_found_wherever_the_document_carries_text() {
    // Generality, demanded after the \author{…} case: a revision is a
    // revision wherever the DOCUMENT carries text — a command argument, a
    // nested argument, a caption inside an environment. And it is still not
    // one where the document does not carry text: a comment, verbatim.
    let cases: [(&str, &[u8]); 5] = [
        ("prosa suelta", b"Texto @del(a:one) {fuera} normal.\n"),
        (
            "argumento simple",
            b"\\author{@del(a:two) {Nombre}, Otro}\n",
        ),
        (
            "argumento anidado",
            b"\\textbf{\\footnote{@del(a:three) {al fondo}}}\n",
        ),
        (
            "caption dentro de un entorno",
            b"\\begin{figure}\n\\caption{Pie con @del(a:four) {sobra}.}\n\\end{figure}\n",
        ),
        (
            "segundo argumento",
            b"\\newcommand{\\x}{@del(a:five) {en el segundo}}\n",
        ),
    ];
    for (what, text) in cases {
        let ids = xtex_core::review::revision_ids(text);
        assert_eq!(
            ids.len(),
            1,
            "{what}: la revision debe verse, se vio {ids:?}"
        );
        let id = &ids[0];
        resolve(text, id, Resolution::Accept)
            .unwrap_or_else(|error| panic!("{what}: no se pudo aceptar: {error:?}"));
    }

    let blind: [(&str, &[u8]); 2] = [
        ("comentario", b"% @del(a:six) {en un comentario}\nTexto.\n"),
        (
            "verbatim",
            b"\\begin{verbatim}\n@del(a:seven) {literal}\n\\end{verbatim}\n",
        ),
    ];
    for (what, text) in blind {
        assert!(
            xtex_core::review::revision_ids(text).is_empty(),
            "{what}: no es contenido del documento y no debe contar como revision"
        );
    }
}
