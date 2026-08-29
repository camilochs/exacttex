//! Reads a .ntex file and prints what its symbol table holds.
use nextex_core::{parse, source::Sources, symbols::SymbolTable};

fn main() {
    let path = std::env::args().nth(1).expect("usage: symbols <file>");
    let bytes = std::fs::read(&path).expect("readable");
    let mut sources = Sources::new();
    let id = sources.add(path.clone(), bytes);
    let document = parse(&sources, id);
    let mut table = SymbolTable::new();
    table.merge(&sources, &document);

    println!("  cobertura            {:.0}%", document.coverage() * 100.0);
    println!(
        "  declarados           {:?}",
        table.declared().collect::<Vec<_>>()
    );
    println!(
        "  citas                {:?}",
        table.citations().map(|(n, _)| n).collect::<Vec<_>>()
    );
    println!(
        "  referencias rotas    {:?}",
        table
            .unresolved_references()
            .map(|(n, _)| n)
            .collect::<Vec<_>>()
    );
    println!("  errores              {}", table.errors().count());
}
