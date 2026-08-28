//! Runs every fixture the grammar promises.
//!
//! `docs/grammar.md` ends six of its sections with "Fixtures must include…".
//! Each of those entries is a directory under `tests/fixtures/` holding an
//! `input.ntex` and an `expect.txt`.
//!
//! Today only one line of each expectation can be checked — `transport:
//! identical` — because nothing is recognised yet and every byte is opaque.
//! The rest of the expectation is written down beside the input so that it
//! cannot be authored after the parser exists, to match whatever the parser
//! happens to do.
//!
//! The transport assertion is not a placeholder either. A fixture is chosen to
//! be awkward: verbatim delimiters that are entry tokens, comments containing
//! closing braces, backslash runs before delimiters. Anything that decodes or
//! normalises fails here first.
//!
//! The harness drives the public pipeline rather than its parts, so it keeps
//! asserting the same property as the middle of that pipeline grows.

use std::fs;
use std::path::{Path, PathBuf};

use nextex_core::io::Memory;
use nextex_core::transport;

fn fixtures_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/nextex-core; the fixtures are repo-relative
    // so that they are shared with any future front end rather than owned by
    // one crate.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures")
}

fn collect(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, found);
        } else if path.file_name().is_some_and(|n| n == "input.ntex") {
            found.push(path);
        }
    }
}

fn all_fixtures() -> Vec<PathBuf> {
    let mut found = Vec::new();
    collect(&fixtures_root(), &mut found);
    found.sort();
    found
}

/// Name a failure reports, e.g. `exclusions/03-entry-token-as-a-verb-delimiter`.
fn label(input: &Path) -> String {
    let dir = input.parent().expect("input.ntex has a parent");
    let group = dir
        .parent()
        .and_then(Path::file_name)
        .map_or_else(String::new, |g| format!("{}/", g.to_string_lossy()));
    format!(
        "{group}{}",
        dir.file_name().unwrap_or_default().to_string_lossy()
    )
}

#[test]
fn the_fixture_directory_is_not_empty() {
    // A harness that silently finds nothing passes forever. This is the guard
    // against a path that stops resolving.
    let fixtures = all_fixtures();
    assert!(
        fixtures.len() >= 40,
        "expected the grammar's fixtures, found {} under {}",
        fixtures.len(),
        fixtures_root().display()
    );
}

#[test]
fn every_fixture_declares_what_it_expects() {
    for input in all_fixtures() {
        let expect = input.with_file_name("expect.txt");
        let text = fs::read_to_string(&expect)
            .unwrap_or_else(|e| panic!("{}: no expect.txt ({e})", label(&input)));
        assert!(
            text.lines().any(|l| l.starts_with("transport:")),
            "{}: expect.txt has no transport line",
            label(&input)
        );
        assert!(
            text.lines().any(|l| l.starts_with("constructs:")),
            "{}: expect.txt has no constructs line",
            label(&input)
        );
    }
}

#[test]
fn every_fixture_transports_byte_identical() {
    let mut checked = 0usize;

    for input in all_fixtures() {
        let name = label(&input);
        let raw = fs::read(&input).unwrap_or_else(|e| panic!("{name}: cannot read ({e})"));

        let declared = fs::read_to_string(input.with_file_name("expect.txt"))
            .unwrap_or_else(|e| panic!("{name}: cannot read expect.txt ({e})"));
        assert!(
            declared.contains("transport: identical"),
            "{name}: this harness only knows how to check identical transport"
        );

        let mut store = Memory::new().with_input(name.clone(), raw.clone());
        transport(&name, &store.clone(), &mut store)
            .unwrap_or_else(|e| panic!("{name}: transport failed ({e})"));
        let out = store.output(&name).unwrap_or_default().to_vec();

        assert_eq!(
            out,
            raw,
            "{name}: transport changed the bytes ({} in, {} out)",
            raw.len(),
            out.len()
        );
        checked += 1;
    }

    assert!(checked >= 40, "only {checked} fixtures ran");
}

#[test]
fn every_fixture_transports_at_every_truncation() {
    // Truncating manufactures unterminated constructs of every shape from
    // inputs that already contain the awkward ones. It is the cheapest
    // generator that finds boundary bugs, and it will keep finding them once
    // the parser stops treating everything as one region.
    for input in all_fixtures() {
        let name = label(&input);
        let raw = fs::read(&input).unwrap_or_else(|e| panic!("{name}: cannot read ({e})"));

        for cut in 0..=raw.len() {
            let slice = &raw[..cut];
            let mut store = Memory::new().with_input(name.clone(), slice.to_vec());
            transport(&name, &store.clone(), &mut store)
                .unwrap_or_else(|e| panic!("{name}: transport failed at cut {cut} ({e})"));

            assert_eq!(
                store.output(&name),
                Some(slice),
                "{name}: transport changed bytes at cut {cut}"
            );
        }
    }
}
