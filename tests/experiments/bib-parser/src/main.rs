use std::collections::BTreeSet;
fn main() {
    let mut files = 0; let mut agree = 0;
    for path in std::env::args().skip(1) {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        files += 1;
        let mine: BTreeSet<String> = xtex_core::bibliography::keys_in_bib(&bytes)
            .unwrap_or_default().into_iter().collect();
        let text = String::from_utf8_lossy(&bytes).to_string();
        let theirs: BTreeSet<String> = match biblatex::Bibliography::parse(&text) {
            Ok(b) => b.iter().map(|e| e.key.clone()).collect(),
            Err(e) => { println!("BIBLATEX-FAIL {path}: {e:?} (mine found {})", mine.len()); continue }
        };
        if mine == theirs { agree += 1; }
        else {
            println!("DIFF {path}: mine {} theirs {}", mine.len(), theirs.len());
            for k in mine.difference(&theirs) { println!("   only-mine: {k}"); }
            for k in theirs.difference(&mine) { println!("   only-theirs: {k}"); }
        }
    }
    println!("--- {agree}/{files} files agree exactly ---");
}
