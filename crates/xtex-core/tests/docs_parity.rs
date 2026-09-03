//! The closed list is only closed if the document says what the code says.
//!
//! `docs/checking.md` §3 claims its table of hard errors is closed: a
//! condition absent from the list is not a hard error. That claim was false
//! for four codes at once — the record family, `XT1016`–`XT1019`, lived in
//! `verification.rs` and in no document — and a reader who checked the code
//! against the prose would have found it before we did. This test makes the
//! claim mechanical: every diagnostic code the crates can emit is named in
//! the document, and every code the document names still exists in the
//! crates.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the workspace root is two levels above the crate")
        .to_path_buf()
}

/// Every `"XTnnnn"` literal in a tree of Rust sources.
fn codes_in_sources(dir: &Path) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(here) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&here) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                // Tests assert on codes they do not emit; sources emit them.
                if path.components().any(|c| c.as_os_str() == "tests") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                found.extend(codes_in(&text));
            }
        }
    }
    found
}

fn codes_in(text: &str) -> BTreeSet<String> {
    let bytes = text.as_bytes();
    let mut found = BTreeSet::new();
    for (i, _) in text.match_indices("XT") {
        let digits = &bytes[(i + 2).min(bytes.len())..(i + 6).min(bytes.len())];
        if digits.len() == 4 && digits.iter().all(u8::is_ascii_digit) {
            found.insert(format!("XT{}", String::from_utf8_lossy(digits)));
        }
    }
    found
}

#[test]
fn every_code_the_compiler_emits_is_documented() {
    let root = root();
    let emitted = codes_in_sources(&root.join("crates"));
    let documented = codes_in(
        &std::fs::read_to_string(root.join("docs/checking.md"))
            .expect("docs/checking.md is readable"),
    );
    let undocumented: Vec<&String> = emitted.difference(&documented).collect();
    assert!(
        undocumented.is_empty(),
        "these codes are emitted by the crates and named in no table of docs/checking.md: {undocumented:?}. \
         The document claims its list is closed, so a code that is not in it makes the claim false."
    );
    let gone: Vec<&String> = documented.difference(&emitted).collect();
    assert!(
        gone.is_empty(),
        "docs/checking.md names codes the crates no longer emit: {gone:?}"
    );
}
