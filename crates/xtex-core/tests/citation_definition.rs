//! A citation's definition is the key's own line in the declared `.bib`.

use xtex_core::editor::citation_definition_site;
use xtex_core::io::{Memory, SourceLoader};
use xtex_core::parse;
use xtex_core::source::Sources;

const DOC: &str = "\\documentclass{article}\n\\begin{document}\nSee @cite(knuth1984).\n\\bibliography{refs}\n\\end{document}\n";
const BIB: &str = "% typesetting classics\n@book{lamport1994, title={LaTeX}}\n@book{knuth1984, title={The TeXbook}}\n";

fn project() -> (
    Memory,
    Sources,
    xtex_core::document::Document,
    xtex_core::source::SourceId,
) {
    let memory = Memory::new()
        .with_input("main.xtex", DOC.as_bytes().to_vec())
        .with_input("refs.bib", BIB.as_bytes().to_vec());
    let mut sources = Sources::new();
    let id = memory
        .load("main.xtex", None, &mut sources)
        .expect("root loads");
    let document = parse(&sources, id);
    (memory, sources, document, id)
}

#[test]
fn a_cite_lands_on_its_bib_entry() {
    let (memory, mut sources, document, id) = project();
    let offset = DOC.find("@cite").expect("cite is in the doc") + 7;
    let (bib, span) = citation_definition_site(&mut sources, &memory, &document, id, offset)
        .expect("the key is declared in refs.bib");
    let bytes = sources.get(bib).expect("bib interned").bytes();
    assert_eq!(
        &bytes[span.start()..span.end()],
        b"knuth1984",
        "the span is the key's own token"
    );
    let before = &BIB[..BIB.find("knuth1984").expect("key in bib")];
    assert_eq!(
        span.start(),
        before.len(),
        "and it sits at the right offset"
    );
}

#[test]
fn a_position_outside_a_cite_stays_silent() {
    let (memory, mut sources, document, id) = project();
    let offset = DOC.find("See").expect("prose is in the doc");
    assert!(citation_definition_site(&mut sources, &memory, &document, id, offset).is_none());
}

#[test]
fn an_unknown_key_stays_silent() {
    let (memory, sources, document, id) = project();
    let doc2 = DOC.replace("knuth1984", "knuth1985");
    let memory2 = Memory::new()
        .with_input("main.xtex", doc2.as_bytes().to_vec())
        .with_input("refs.bib", BIB.as_bytes().to_vec());
    let mut sources2 = Sources::new();
    let id2 = memory2
        .load("main.xtex", None, &mut sources2)
        .expect("root loads");
    let document2 = parse(&sources2, id2);
    let offset = doc2.find("@cite").expect("cite is in the doc") + 7;
    assert!(citation_definition_site(&mut sources2, &memory2, &document2, id2, offset).is_none());
    let _ = (memory, sources, document, id);
}
