//! Reads a .xtex file and prints what its symbol table and bibliography hold.
use std::collections::BTreeMap;
use std::path::Path;

use xtex_core::bibliography::{Bibliography, assemble_from, declared_in, missing_citations};
use xtex_core::{parse, source::Sources, symbols::SymbolTable};

fn main() {
    let path = std::env::args().nth(1).expect("usage: symbols <file>");
    let bytes = std::fs::read(&path).expect("readable");
    let mut sources = Sources::new();
    let id = sources.add(path.clone(), bytes);
    let document = parse(&sources, id);
    let mut table = SymbolTable::new();
    table.merge(&sources, &document);

    // Declared resources are relative to the document, so they are read from
    // beside it. A resource that is not there stays unread on purpose.
    let base = Path::new(&path).parent().unwrap_or_else(|| Path::new("."));
    let declared = declared_in(&sources, id);
    let files: BTreeMap<String, Vec<u8>> = declared
        .resources
        .iter()
        .filter_map(|r| {
            std::fs::read(base.join(&r.name))
                .ok()
                .map(|b| (r.name.clone(), b))
        })
        .collect();
    let bibliography = assemble_from(&declared, &files);

    println!("  coverage             {:.0}%", document.coverage() * 100.0);
    println!(
        "  declared             {:?}",
        table.declared().collect::<Vec<_>>()
    );
    println!(
        "  citations            {:?}",
        table.citations().map(|(n, _)| n).collect::<Vec<_>>()
    );
    println!(
        "  broken references    {:?}",
        table
            .unresolved_references()
            .map(|(n, _)| n)
            .collect::<Vec<_>>()
    );
    match &bibliography {
        Bibliography::Complete(keys) => println!("  bibliography         {} keys", keys.len()),
        Bibliography::Unavailable(why) => println!("  bibliography         unavailable: {why:?}"),
    }
    println!(
        "  missing citations    {:?}",
        missing_citations(&table, &bibliography)
            .map(|(n, _)| n)
            .collect::<Vec<_>>()
    );
    println!("  errors               {}", table.errors().count());
}
