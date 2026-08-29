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
use nextex_core::scanner::{Piece, scan};
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

/// The pieces an `expect.txt` declares, in order.
///
/// The `scanner:` line is the machine-checked one: `none`, or a space-separated
/// list where `ref` is a construct and `!ref` is a malformed one. It was
/// transcribed from each fixture's own prose, never from what the scanner
/// happened to do — a `scanner:` line written to match the code would document
/// the bug rather than the grammar.
///
/// The `constructs:` line stays as the note for a reader. It says things a
/// checker will verify and a scanner cannot, such as which identifier attached
/// to which caption.
fn declared_pieces(expect: &str, name: &str) -> Vec<String> {
    let line = expect
        .lines()
        .find(|l| l.starts_with("scanner:"))
        .unwrap_or_else(|| panic!("{name}: expect.txt has no scanner line"));
    let body = line.trim_start_matches("scanner:").trim();
    if body == "none" {
        return Vec::new();
    }
    body.split_whitespace().map(str::to_owned).collect()
}

/// The pieces the scanner produced, named as `expect.txt` names them.
fn found_pieces(bytes: &[u8]) -> Vec<String> {
    scan(bytes)
        .into_iter()
        .filter_map(|piece| match piece {
            Piece::Construct { kind, .. } => Some(short(kind)),
            Piece::Malformed { kind, .. } => Some(format!("!{}", short(kind))),
            Piece::Text(_) | Piece::Excluded(_) => None,
        })
        .collect()
}

fn short(kind: nextex_core::scanner::EntryToken) -> String {
    kind.name().trim_start_matches(['@', '\\']).to_owned()
}

/// Fixtures the scanner does not satisfy yet, each with the issue that closes it.
///
/// This is a statement about what is not built. An entry without an issue behind
/// it is an expectation being quietly dropped, which is the anti-pattern in
/// `AGENTS.md` §5.
const NOT_YET: &[(&str, &str)] = &[
    (
        "revisions/01-inline-between-words-and-before-punctuation",
        "#6, #15",
    ),
    ("revisions/02-a-long-multiline-revision", "#6, #15"),
    (
        "revisions/03-nested-constructs-inside-a-revision",
        "#6, #15",
    ),
    (
        "revisions/04-construct-inside-a-command-argument-in-a-revision",
        "#6, #15",
    ),
    ("revisions/05-substitution-with-arrows-at-depth", "#6, #15"),
    (
        "revisions/06-substitution-with-zero-and-two-arrows",
        "#6, #15",
    ),
    ("revisions/07-unmatched-revision-brace", "#6, #15"),
    ("revisions/08-properly-nested-revisions", "#6, #15"),
];

#[test]
fn every_fixture_produces_the_pieces_it_declares() {
    // Until this ran, all 42 fixtures declared their constructs in prose and
    // nothing read it. The grammar was documented and unfalsified at once.
    let mut checked = 0usize;
    let mut skipped = 0usize;
    let mut failures = Vec::new();

    for input in all_fixtures() {
        let name = label(&input);
        if let Some((_, issue)) = NOT_YET.iter().find(|(n, _)| *n == name) {
            println!("{name}: not built yet, {issue}");
            skipped += 1;
            continue;
        }
        let raw = fs::read(&input).unwrap_or_else(|e| panic!("{name}: cannot read ({e})"));
        let expect = fs::read_to_string(input.with_file_name("expect.txt"))
            .unwrap_or_else(|e| panic!("{name}: cannot read expect.txt ({e})"));

        let want = declared_pieces(&expect, &name);
        let got = found_pieces(&raw);
        if want != got {
            failures.push(format!("{name}: declared {want:?}, produced {got:?}"));
        }
        checked += 1;
    }

    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    assert!(
        checked >= 30,
        "only {checked} fixtures ran, {skipped} skipped"
    );
}

#[test]
fn ordinary_latex_yields_no_constructs() {
    // The promise the whole project rests on: rename a `.tex` to `.ntex`,
    // change nothing, and the compiler has nothing to say. It cannot hold if
    // the scanner recognises a construct in bytes the author wrote as LaTeX.
    let ordinary: &[&[u8]] = &[
        br"Write to \texttt{author@example.org} for the data.",
        br"\begin{tabular}{@{}lcc@{}} a & b & c \\ \end{tabular}",
        br"The \verb+@ref+ syntax is not used in this paper.",
        br"Coverage reached 94\% this run, up from 90\%.",
        br"See \ref{fig:main} and \cite{knuth1984}; neither resolves.",
        br"\newcommand{\ref}[1]{see #1} % a package redefining \ref",
        br"\makeatletter \@ifpackageloaded{amsmath}{}{} \makeatother",
        br"An email @ the start of a line, and a lone @ mid-sentence.",
        br"\definecolor[named]{@id(a)}{rgb}{1,0,0} % xcolor form",
        br"latex, plain prose about the word, with no brace after it.",
    ];

    for bytes in ordinary {
        let found = found_pieces(bytes);
        assert!(
            found.is_empty(),
            "recognised {found:?} in ordinary LaTeX: {}",
            String::from_utf8_lossy(bytes)
        );
    }
}

#[test]
fn a_declared_view_is_a_built_document_and_the_two_differ() {
    // The views cannot be derived until revisions are parsed (#15), so they are
    // written by hand — which is the point: an expectation authored after the
    // code would only record what the code does.
    //
    // Two properties are checkable now. A view is a *built* `.tex`, so no
    // NextTeX markup may survive in it. And the two views must differ, or the
    // fixture is not testing a revision at all.
    //
    // The derivation itself — that `--final` really is the input with every
    // revision applied — arrives with #15. Writing a weaker proxy for it here
    // would be worse than saying plainly that it is not checked yet.
    let mut checked = 0usize;

    for input in all_fixtures() {
        let name = label(&input);
        let expect = fs::read_to_string(input.with_file_name("expect.txt"))
            .unwrap_or_else(|e| panic!("{name}: cannot read expect.txt ({e})"));
        if !expect.lines().any(|l| l.starts_with("views:")) {
            continue;
        }

        let mut views = Vec::new();
        for view in ["original.tex", "final.tex"] {
            let bytes = fs::read(input.with_file_name(view))
                .unwrap_or_else(|e| panic!("{name}: declares views but has no {view} ({e})"));
            assert!(
                found_pieces(&bytes).is_empty(),
                "{name}: {view} still carries NextTeX markup; a view is a built document"
            );
            views.push(bytes);
        }

        assert_ne!(
            views[0], views[1],
            "{name}: the two views are identical, so the fixture tests no revision"
        );
        checked += 1;
    }

    assert!(checked >= 5, "only {checked} fixtures declare views");
}
