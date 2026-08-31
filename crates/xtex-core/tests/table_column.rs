//! A table overfull names its column — when the evidence supports it.
//!
//! Decision 0018, measured against the live engine first: paragraph-shaped
//! columns localize an overfull to the ROW and the box trace prints the
//! offending cell's own content. Attribution is therefore content matching
//! against the row's cells — never width arithmetic, never a guess.

use xtex_core::blame::{merge_records, translate};
use xtex_core::source::Sources;
use xtex_core::sourcemap::emit_with_map;
use xtex_core::{parse, project};

const DOC: &[u8] = b"\\documentclass{article}\n\
\\table(tab:sizes) {\n\
  placement = ht\n\
  caption = {Sizes.}\n\
  body = {\n\
    \\begin{tabular}{p{1cm}p{2cm}p{1cm}}\n\
      short & ThisWordIsFarTooWideForItsColumn & also \\\\\n\
    \\end{tabular}\n\
  }\n\
}\n";

/// The emitted line that carries the too-wide row, found rather than
/// hard-coded so an emitter change cannot silently invalidate the log.
fn emitted_row_line(emitted: &[u8]) -> u32 {
    let text = String::from_utf8_lossy(emitted);
    let line = text
        .lines()
        .position(|l| l.contains("ThisWordIsFarTooWideForItsColumn"))
        .expect("the row is emitted");
    u32::try_from(line + 1).expect("fits")
}

fn translated_for(log: &str) -> Vec<xtex_core::blame::Translated> {
    let mut sources = Sources::new();
    let id = sources.add("t.xtex", DOC.to_vec());
    let document = parse(&sources, id);
    let emission = emit_with_map(&sources, &document).expect("emits");
    let store = xtex_core::io::Memory::new().with_input("t.xtex".to_owned(), DOC.to_vec());
    let analysed = project::analyse(&store, "t.xtex").expect("analyses");
    let records = merge_records(None, Some(log), "t.tex");
    translate(&records, &emission, &sources, &document, &analysed.table)
}

#[test]
fn the_column_is_named_when_the_trace_matches_one_cell() {
    let mut sources = Sources::new();
    let id = sources.add("t.xtex", DOC.to_vec());
    let document = parse(&sources, id);
    let emission = emit_with_map(&sources, &document).expect("emits");
    let row = emitted_row_line(&emission.bytes);
    // The log shapes are transcribed from a live pdfTeX run (decision 0018).
    let log = format!(
        "Overfull \\hbox (137.35315pt too wide) in paragraph at lines {row}--{row}\n\
         []\\OT1/cmr/m/n/10 ThisWordIsFarTooWideForItsColumn|\n\
         []\n"
    );
    let translated = translated_for(&log);
    let record = translated
        .iter()
        .find(|t| t.column.is_some())
        .expect("one record carries the column");
    let (index, content) = record.column.as_ref().expect("checked");
    assert_eq!(*index, 2, "the offending word sits in the second cell");
    assert!(content.contains("ThisWordIsFarTooWide"));
    let (name, ..) = record.entity.as_ref().expect("the table is the entity");
    assert_eq!(name, "tab:sizes");
}

#[test]
fn an_ambiguous_trace_names_no_column() {
    // The same word in two cells: two records with identical traces. A
    // confident wrong column is worse than a located table, so none is named.
    let doc = b"\\documentclass{article}\n\
\\table(tab:twin) {\n\
  placement = ht\n\
  caption = {Twins.}\n\
  body = {\n\
    \\begin{tabular}{p{1cm}p{1cm}}\n\
      SameWordTooWideForBoth & SameWordTooWideForBoth \\\\\n\
    \\end{tabular}\n\
  }\n\
}\n";
    let mut sources = Sources::new();
    let id = sources.add("t.xtex", doc.to_vec());
    let document = parse(&sources, id);
    let emission = emit_with_map(&sources, &document).expect("emits");
    let text = String::from_utf8_lossy(&emission.bytes);
    let row = u32::try_from(
        text.lines()
            .position(|l| l.contains("SameWordTooWideForBoth"))
            .expect("emitted")
            + 1,
    )
    .expect("fits");
    let log = format!(
        "Overfull \\hbox (94.65866pt too wide) in paragraph at lines {row}--{row}\n\
         []|\\OT1/cmr/m/n/10 SameWordTooWideForBoth|\n\
         []\n"
    );
    let store = xtex_core::io::Memory::new().with_input("t.xtex".to_owned(), doc.to_vec());
    let analysed = project::analyse(&store, "t.xtex").expect("analyses");
    let records = merge_records(None, Some(&log), "t.tex");
    let translated = translate(&records, &emission, &sources, &document, &analysed.table);
    assert!(
        translated.iter().all(|t| t.column.is_none()),
        "an ambiguous trace must not name a column"
    );
}
