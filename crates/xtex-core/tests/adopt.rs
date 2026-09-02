//! The mechanical ramp over the fixtures under `tests/fixtures/adopt/`.
//!
//! Each fixture directory is a `.tex` project whose root is `main.tex`, with
//! an `expect/` directory beside it holding the `.xtex` files the ramp must
//! produce and `report.txt`, the report it must print. One rule or one
//! exclusion per fixture, so a failure names the rule that stopped holding.
//!
//! The guarantee is checked here a second time, without the module's own
//! gate: every converted file is emitted through the public emitter and
//! compared with its original, admitting only the `.tex` an `@import`
//! writes back. A rule that breaks the bytes fails this test even if the
//! gate that should have caught it were broken too.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use xtex_core::adopt::{adopt, render};
use xtex_core::io::Memory;
use xtex_core::source::Sources;
use xtex_core::{RevisionView, emit_view, parse};

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/adopt")
}

fn cases() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(fixtures_root())
        .expect("the adopt fixtures")
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| path.is_dir())
        .collect();
    found.sort();
    found
}

fn files_under(dir: &Path, base: &Path, skip: Option<&str>, out: &mut BTreeMap<String, Vec<u8>>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .expect("readable")
        .map(|entry| entry.expect("an entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            if skip.is_some_and(|name| path.file_name().is_some_and(|f| f == name)) {
                continue;
            }
            files_under(&path, base, skip, out);
        } else {
            let name = path
                .strip_prefix(base)
                .expect("inside the case")
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(name, fs::read(&path).expect("readable"));
        }
    }
}

/// The project as a store, and what `expect/` says about it.
fn load(case: &Path) -> (Memory, BTreeMap<String, Vec<u8>>, String) {
    let mut inputs = BTreeMap::new();
    files_under(case, case, Some("expect"), &mut inputs);
    let mut store = Memory::new();
    for (name, bytes) in &inputs {
        store = store.with_input(name.clone(), bytes.clone());
    }
    let expect = case.join("expect");
    let mut expected = BTreeMap::new();
    files_under(&expect, &expect, None, &mut expected);
    let report = expected
        .remove("report.txt")
        .map(|bytes| String::from_utf8(bytes).expect("the report is UTF-8"))
        .expect("expect/report.txt");
    (store, expected, report)
}

fn label(case: &Path) -> String {
    case.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

#[test]
fn the_fixture_directory_is_not_empty() {
    assert!(
        cases().len() >= 10,
        "expected the adopt fixtures under {}",
        fixtures_root().display()
    );
}

#[test]
fn every_fixture_converts_to_its_expected_files_and_report() {
    for case in cases() {
        let name = label(&case);
        let (store, mut expected, report) = load(&case);
        let adopted = adopt(&store, "main.tex").unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut rendered = String::new();
        render(&adopted, &mut rendered);
        assert_eq!(rendered, report, "{name}: the report differs");
        for file in &adopted.files {
            match &file.converted {
                Some(bytes) => {
                    let want = expected
                        .remove(&file.output)
                        .unwrap_or_else(|| panic!("{name}: {} was not expected", file.output));
                    assert_eq!(
                        String::from_utf8_lossy(bytes),
                        String::from_utf8_lossy(&want),
                        "{name}: {} differs",
                        file.output
                    );
                    assert_eq!(bytes, &want, "{name}: {} differs in bytes", file.output);
                }
                None => assert!(
                    !expected.contains_key(&file.output),
                    "{name}: {} was expected and not produced: {:?}",
                    file.output,
                    file.failure
                ),
            }
        }
        assert!(
            expected.is_empty(),
            "{name}: expected outputs nothing produced: {:?}",
            expected.keys().collect::<Vec<_>>()
        );
    }
}

/// Whether `emitted` is `original` with `.tex` appended inside some of its
/// `\input{…}` arguments and nothing else changed.
fn differs_only_by_input_extension(original: &[u8], emitted: &[u8]) -> bool {
    let (mut o, mut e) = (0usize, 0usize);
    while o < original.len() || e < emitted.len() {
        if original[o..].starts_with(b"\\input{") && emitted[e..].starts_with(b"\\input{") {
            let close_o = original[o..].iter().position(|b| *b == b'}');
            let close_e = emitted[e..].iter().position(|b| *b == b'}');
            let (Some(close_o), Some(close_e)) = (close_o, close_e) else {
                return false;
            };
            let path_o = &original[o + 7..o + close_o];
            let path_e = &emitted[e + 7..e + close_e];
            let with_tex = [path_o, b".tex".as_slice()].concat();
            if path_e != path_o && path_e != with_tex.as_slice() {
                return false;
            }
            o += close_o + 1;
            e += close_e + 1;
            continue;
        }
        if original.get(o) != emitted.get(e) {
            return false;
        }
        o += 1;
        e += 1;
    }
    true
}

#[test]
fn the_guarantee_holds_over_every_fixture_without_the_gate() {
    let mut imports_seen = 0usize;
    for case in cases() {
        let name = label(&case);
        let (store, _, _) = load(&case);
        let adopted = adopt(&store, "main.tex").unwrap_or_else(|e| panic!("{name}: {e}"));
        for file in &adopted.files {
            let Some(converted) = &file.converted else {
                continue;
            };
            let original = adopted
                .sources
                .get(file.source)
                .map(|source| source.bytes().to_vec())
                .expect("the source was read");
            let mut sources = Sources::new();
            let id = sources.add(file.output.clone(), converted.clone());
            let document = parse(&sources, id);
            let mut emitted = Vec::new();
            emit_view(&sources, &document, RevisionView::Original, &mut emitted).expect("emits");
            imports_seen += file.counts.imports;
            assert!(
                differs_only_by_input_extension(&original, &emitted),
                "{name}: emitting {} does not return {}:\n{}",
                file.output,
                file.name,
                String::from_utf8_lossy(&emitted)
            );
        }
    }
    assert!(
        imports_seen >= 3,
        "the fixtures must exercise the one admitted difference, or the check proves less than it claims"
    );
}

#[test]
fn a_fixture_carrying_a_byte_outside_utf8_keeps_it() {
    let case = fixtures_root().join("10-project");
    let (store, _, _) = load(&case);
    let original = fs::read(case.join("main.tex")).expect("the root");
    assert!(
        std::str::from_utf8(&original).is_err(),
        "the fixture must not be valid UTF-8, or it tests nothing"
    );
    let adopted = adopt(&store, "main.tex").expect("adopts");
    let root = adopted.files[0]
        .converted
        .as_deref()
        .expect("the root passed");
    assert!(root.contains(&0xE9), "the Latin-1 byte did not survive");
}
