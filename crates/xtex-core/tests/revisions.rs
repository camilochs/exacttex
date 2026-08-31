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
fn recognition_and_emission_agree_about_where_a_document_carries_text() {
    // One rule decides both: `TEXT_MANDATORY_COMMANDS`. When the two paths
    // disagreed, a proposed deletion of an author name could be accepted in
    // the margin AND printed into the PDF as `@del(change:…)` — the director
    // saw both on 2026-08-31.
    let prose: [(&str, &[u8], &[u8]); 5] = [
        (
            "prosa suelta",
            b"Texto @del(a:one) {fuera} normal.\n",
            b"Texto  normal.\n",
        ),
        (
            "el titulo",
            b"\\title{Redes @del(a:two) {que sobran} de busqueda}\n",
            b"\\title{Redes  de busqueda}\n",
        ),
        (
            "los autores",
            b"\\author{Gabriela, @del(a:three) {Christian}}\n",
            b"\\author{Gabriela, }\n",
        ),
        (
            "un pie de figura",
            b"\\caption{Pie @del(a:four) {con sobra} claro}\n",
            b"\\caption{Pie  claro}\n",
        ),
        (
            "anidado",
            b"\\textbf{\\footnote{@del(a:five) {al fondo}}}\n",
            b"\\textbf{\\footnote{}}\n",
        ),
    ];
    for (what, source, expected_final) in prose {
        let ids = xtex_core::review::revision_ids(source);
        assert_eq!(
            ids.len(),
            1,
            "{what}: la revision debe reconocerse, se vio {ids:?}"
        );
        resolve(source, &ids[0], Resolution::Accept)
            .unwrap_or_else(|error| panic!("{what}: no se pudo aceptar: {error:?}"));

        let mut sources = Sources::new();
        let id = sources.add("t.xtex", source.to_vec());
        let document = parse(&sources, id);
        let mut out = Vec::new();
        emit_view(&sources, &document, RevisionView::Final, &mut out).expect("emits");
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(expected_final),
            "{what}: la vista Final debe borrar el constructo, no imprimirlo"
        );

        let mut marked = Vec::new();
        emit_view(&sources, &document, RevisionView::Marked, &mut marked).expect("emits");
        let marked = String::from_utf8_lossy(&marked);
        assert!(
            !marked.contains("@del("),
            "{what}: la vista Marked tampoco imprime el constructo: {marked}"
        );
    }

    // And where the document carries no text, nothing is a revision — in
    // recognition and in emission alike.
    let blind: [(&str, &[u8]); 3] = [
        ("comentario", b"% @del(a:six) {en un comentario}\nTexto.\n"),
        (
            "verbatim",
            b"\\begin{verbatim}\n@del(a:seven) {literal}\n\\end{verbatim}\n",
        ),
        (
            "cuerpo de una definicion",
            b"\\newcommand{\\x}{@del(a:eight) {no ha pasado nada aun}}\n",
        ),
    ];
    for (what, source) in blind {
        assert!(
            xtex_core::review::revision_ids(source).is_empty(),
            "{what}: no es contenido del documento y no debe contar como revision"
        );
        let mut sources = Sources::new();
        let id = sources.add("t.xtex", source.to_vec());
        let document = parse(&sources, id);
        let mut out = Vec::new();
        emit_view(&sources, &document, RevisionView::Final, &mut out).expect("emits");
        assert_eq!(out, source, "{what}: los bytes se transportan intactos");
    }
}
