//! The parser hazard table, as a test.
//!
//! `ROADMAP.md` lists what the shallow LaTeX parser must do at each dangerous
//! boundary, and gives, for each, the observation that would show the handling
//! is wrong. `tests/corpus/hazards/` turns each row into a file, and this runs
//! them.
//!
//! Each `expect.txt` opens with two machine-checked lines, transcribed from
//! that table before any of this was implemented:
//!
//! ```text
//! quarantine: quarantines | no
//! recognised: ref ref | none
//! ```
//!
//! Every fixture failing is the correct state until #21 is finished; the list
//! below is what remains, and an entry leaving it is the unit of progress.

use std::fs;
use std::path::{Path, PathBuf};

use xtex_core::document::Node;
use xtex_core::parse;
use xtex_core::source::Sources;

/// Hazards the parser does not handle yet.
///
/// Empty, which is what closes #21. An entry removed while its fixture still
/// fails would be an expectation being dropped, which is the anti-pattern in
/// `AGENTS.md` §5 — so each was removed only after its fixture passed, and two
/// of the fixtures had to be strengthened first because they passed while
/// testing nothing.
const NOT_YET: &[&str] = &[];

fn hazards() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/hazards");
    let mut found: Vec<PathBuf> = fs::read_dir(root)
        .expect("the hazard directory")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join("input.xtex").exists())
        .collect();
    found.sort();
    found
}

fn declared(expect: &str, key: &str) -> String {
    expect
        .lines()
        .find(|line| line.starts_with(key))
        .unwrap_or_else(|| panic!("no `{key}` line"))
        .trim_start_matches(key)
        .trim()
        .to_owned()
}

#[test]
fn every_hazard_behaves_as_the_roadmap_specifies() {
    let mut failures = Vec::new();
    let mut checked = 0usize;

    for dir in hazards() {
        let name = dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if NOT_YET.contains(&name.as_str()) {
            println!("{name}: not handled yet, #21");
            continue;
        }
        let bytes = fs::read(dir.join("input.xtex")).expect("input");
        let expect = fs::read_to_string(dir.join("expect.txt")).expect("expect.txt");

        let mut sources = Sources::new();
        let id = sources.add(name.clone(), bytes);
        let document = parse(&sources, id);

        let quarantines = document.quarantine_position().is_some();
        let wants_quarantine = declared(&expect, "quarantine:") == "quarantines";
        if quarantines != wants_quarantine {
            failures.push(format!(
                "{name}: expected quarantine {wants_quarantine}, got {quarantines}"
            ));
        }

        let mut found = Vec::new();
        document.walk(|node| {
            if let Node::Construct { kind, .. } = node {
                found.push(kind.name().trim_start_matches(['@', '\\']).to_owned());
            }
        });
        let want = declared(&expect, "recognised:");
        let want: Vec<&str> = if want == "none" {
            Vec::new()
        } else {
            want.split_whitespace().collect()
        };
        if found != want {
            failures.push(format!("{name}: expected {want:?}, recognised {found:?}"));
        }
        checked += 1;
    }

    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    assert!(checked >= 1, "every hazard is on the not-yet list");
}
